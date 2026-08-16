//! MTProto client: grammers wrapper (auth, connect, dialogs, messages).

use std::sync::Arc;

use anyhow::{Context, Result};
use grammers_client::client::{LoginToken, PasswordToken, SignInError};
use grammers_client::media::{Downloadable, Media, Photo, PhotoSize};
use grammers_client::Client;
use grammers_client::tl;
use grammers_mtsender::SenderPool;
use grammers_session::updates::UpdatesLike;

use crate::model::{ChatInfo, MediaKind, MessageInfo};
use crate::session::FileSession;

/// Wrapped Telegram client with the network runtime running in the background.
pub struct Telegram {
    session: Arc<FileSession>,
    client: Client,
    updates: tokio::sync::mpsc::UnboundedReceiver<UpdatesLike>,
    runner: tokio::task::JoinHandle<()>,
}

impl Telegram {
    /// Builds the client from a session and starts the network runtime.
    pub async fn connect(session: Arc<FileSession>, api_id: i32) -> Result<Self> {
        let SenderPool {
            runner,
            handle,
            updates,
        } = SenderPool::new(session.clone(), api_id);
        let client = Client::new(handle);
        let runner = tokio::spawn(runner.run());
        Ok(Self {
            session,
            client,
            updates,
            runner,
        })
    }

    pub fn session(&self) -> &Arc<FileSession> {
        &self.session
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Takes the raw updates stream (consumed once, to feed
    /// `client.stream_updates`).
    pub fn take_updates(&mut self) -> tokio::sync::mpsc::UnboundedReceiver<UpdatesLike> {
        let empty = tokio::sync::mpsc::unbounded_channel::<UpdatesLike>().1;
        std::mem::replace(&mut self.updates, empty)
    }

    /// Returns true if a valid session is already connected.
    pub async fn is_authorized(&self) -> Result<bool> {
        self.client
            .is_authorized()
            .await
            .context("checking authorization")
    }

    /// Step 1: request a verification code for a phone number.
    pub async fn request_login_code(&self, api_hash: &str, phone: &str) -> Result<LoginToken> {
        self.client
            .request_login_code(phone, api_hash)
            .await
            .context("requesting code")
    }

    /// Step 2: sign in with the received code (may require a 2FA password).
    pub async fn sign_in(
        &self,
        token: &LoginToken,
        code: &str,
    ) -> Result<Result<(), SignInError>> {
        Ok(self.client.sign_in(token, code).await.map(|_| ()))
    }

    /// Step 2b: sign in with the 2FA password.
    ///
    /// `Ok(Ok(()))` means signed in; `Ok(Err(new_token))` means the password
    /// was wrong and Telegram supplied a fresh token to retry with.
    pub async fn check_password(
        &self,
        password_token: PasswordToken,
        password: &str,
    ) -> Result<Result<(), PasswordToken>, anyhow::Error> {
        match self.client.check_password(password_token, password).await {
            Ok(_) => Ok(Ok(())),
            Err(SignInError::InvalidPassword(new_token)) => Ok(Err(new_token)),
            Err(SignInError::PasswordRequired(new_token)) => Ok(Err(new_token)),
            Err(e) => Err(anyhow::anyhow!("2FA sign in: {e}")),
        }
    }

    /// Lists the user's chats (dialogs), ordered by activity.
    pub async fn get_dialogs(&self) -> Result<Vec<ChatInfo>> {
        let mut dialogs = self.client.iter_dialogs();
        let mut out = Vec::new();
        while let Some(dialog) = dialogs.next().await? {
            let id = dialog.peer_id();
            let title = dialog
                .peer()
                .name()
                .unwrap_or("Unknown")
                .to_string();
            let unread_count = match &dialog.raw {
                grammers_client::tl::enums::Dialog::Dialog(d) => d.unread_count,
                grammers_client::tl::enums::Dialog::Folder(_) => 0,
            };
            let peer_ref = dialog.peer_ref();
            let last_date = dialog.last_message.as_ref().map(|m| m.date().timestamp() as i32);
            let last_message = dialog.last_message.map(|m| m.text().to_string());
            out.push(ChatInfo {
                id,
                title,
                last_message,
                last_date,
                unread_count,
                peer_ref,
            });
        }
        Ok(out)
    }

    /// Fetches the latest messages of a chat, in chronological order.
    pub async fn get_messages(
        &self,
        peer: &grammers_session::types::PeerRef,
        limit: usize,
    ) -> Result<Vec<MessageInfo>> {
        let mut it = self.client.iter_messages(peer.clone()).limit(limit);
        let mut out = Vec::new();
        while let Some(msg) = it.next().await? {
            out.push(MessageInfo {
                id: msg.id(),
                text: msg.text().to_string(),
                date: msg.date().timestamp() as i32,
                out: msg.outgoing(),
                media: media_kind(msg.media().as_ref()),
            });
        }
        out.reverse();
        Ok(out)
    }

    /// Downloads a message's photo (a small thumbnail, ~256 px) into `dir`,
    /// returning the saved path, or `None` if the message has no photo.
    /// If the thumbnail is already cached, no network request is made.
    pub async fn download_photo(
        &self,
        peer_ref: &grammers_session::types::PeerRef,
        msg_id: i32,
        dir: &std::path::Path,
    ) -> Result<Option<std::path::PathBuf>> {
        std::fs::create_dir_all(dir)?;
        let cached = dir.join(format!("{msg_id}.jpg"));
        if cached.exists() {
            return Ok(Some(cached));
        }
        let msgs = self
            .client
            .get_messages_by_id(peer_ref.clone(), &[msg_id])
            .await
            .context("fetching message")?;
        let Some(Some(msg)) = msgs.into_iter().next() else {
            return Ok(None);
        };
        let Some(size) = media_downloadable(msg.media().as_ref()) else {
            return Ok(None);
        };
        let mut it = self.client.iter_download(&size);
        let mut bytes = Vec::new();
        while let Some(chunk) = it.next().await? {
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            return Ok(None);
        }
        let path = dir.join(format!("{msg_id}.jpg"));
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(Some(path))
    }

    /// Sends a text message to a chat.
    pub async fn send_message(
        &self,
        peer: &grammers_session::types::PeerRef,
        text: &str,
    ) -> Result<()> {
        self.client
            .send_message(
                peer.clone(),
                grammers_client::message::InputMessage::new().text(text),
            )
            .await
            .context("sending message")
            .map(|_| ())
    }

    /// Stops the network runtime.
    pub async fn shutdown(self) {
        drop(self.client);
        let _ = self.runner.await;
    }

    /// Downloads a user's profile photo (small "a" size, ~160 px) into `dir`,
    /// returning the saved path (or `None` for non-user peers / no photo).
    pub async fn download_avatar(
        &self,
        peer_ref: &grammers_session::types::PeerRef,
        dir: &std::path::Path,
    ) -> Result<Option<std::path::PathBuf>> {
        std::fs::create_dir_all(dir)?;
        let cached = dir.join(format!("{}.jpg", peer_ref.id.bot_api_dialog_id()));
        if cached.exists() {
            return Ok(Some(cached));
        }
        if peer_ref.id.kind() != grammers_session::types::PeerKind::User {
            return self
                .download_chat_avatar(peer_ref, &cached)
                .await
                .map(|ok| ok.then_some(cached));
        }

        let input_user = tl::enums::InputUser::User(tl::types::InputUser {
            user_id: peer_ref.id.bare_id(),
            access_hash: peer_ref.auth.hash(),
        });
        let res = self
            .client
            .invoke(&tl::functions::photos::GetUserPhotos {
                user_id: input_user,
                offset: 0,
                max_id: 0,
                limit: 1,
            })
            .await
            .context("fetching avatar")?;
        let photos = match res {
            tl::enums::photos::Photos::Photos(p) => p.photos,
            tl::enums::photos::Photos::Slice(p) => p.photos,
        };
        let Some(base) = photos.into_iter().find_map(|p| match p {
            tl::enums::Photo::Photo(photo) => Some(photo),
            _ => None,
        }) else {
            return Ok(None);
        };
        // Smallest thumbnail size (or the "a" 160 px one when present).
        let size = base
            .sizes
            .iter()
            .filter_map(|s| match s {
                tl::enums::PhotoSize::Size(s) => Some(s),
                _ => None,
            })
            .min_by_key(|s| s.size);
        let Some(size) = size else {
            return Ok(None);
        };

        let location = tl::enums::InputFileLocation::InputPhotoFileLocation(
            tl::types::InputPhotoFileLocation {
                id: base.id,
                access_hash: base.access_hash,
                file_reference: base.file_reference.clone(),
                thumb_size: size.r#type.clone(),
            },
        );
        let mut it = self.client.iter_download(&RawPhoto(location));
        let mut bytes = Vec::new();
        while let Some(chunk) = it.next().await? {
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            return Ok(None);
        }
        let tmp = cached.with_extension("tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &cached)?;
        Ok(Some(cached))
    }

    /// Downloads a group/channel profile photo (small size) into `cached`.
    ///
    /// Groups and channels do not expose their photo through `getUserPhotos`;
    /// we resolve the current `ChatPhoto` (photo_id) through `messages.getChats`
    /// and download it via `inputPeerPhotoFileLocation`.
    async fn download_chat_avatar(
        &self,
        peer_ref: &grammers_session::types::PeerRef,
        cached: &std::path::Path,
    ) -> Result<bool> {
        let kind = peer_ref.id.kind();

        let res = match kind {
            grammers_session::types::PeerKind::Chat => {
                self.client
                    .invoke(&tl::functions::messages::GetChats {
                        id: vec![peer_ref.id.bare_id()],
                    })
                    .await
                    .context("fetching chat")?
            }
            grammers_session::types::PeerKind::Channel => {
                let input_channel = tl::enums::InputChannel::Channel(
                    tl::types::InputChannel {
                        channel_id: peer_ref.id.bare_id(),
                        access_hash: peer_ref.auth.hash(),
                    },
                );
                self.client
                    .invoke(&tl::functions::channels::GetChannels {
                        id: vec![input_channel],
                    })
                    .await
                    .context("fetching channel")?
            }
            _ => return Ok(false),
        };

        let input_peer = match kind {
            grammers_session::types::PeerKind::Chat => {
                tl::enums::InputPeer::Chat(tl::types::InputPeerChat {
                    chat_id: peer_ref.id.bare_id(),
                })
            }
            grammers_session::types::PeerKind::Channel => {
                tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                    channel_id: peer_ref.id.bare_id(),
                    access_hash: peer_ref.auth.hash(),
                })
            }
            _ => return Ok(false),
        };
        let photo_id = res
            .chats()
            .into_iter()
            .find_map(|chat| match chat {
                tl::enums::Chat::Chat(c) => Some(c.photo),
                tl::enums::Chat::Channel(c) => Some(c.photo),
                _ => None,
            })
            .and_then(|photo| match photo {
                tl::enums::ChatPhoto::Photo(p) => Some(p.photo_id),
                tl::enums::ChatPhoto::Empty => None,
            });
        let Some(photo_id) = photo_id else {
            return Ok(false);
        };

        let location = tl::enums::InputFileLocation::InputPeerPhotoFileLocation(
            tl::types::InputPeerPhotoFileLocation {
                big: false,
                peer: input_peer,
                photo_id,
            },
        );
        let mut it = self.client.iter_download(&RawPhoto(location));
        let mut bytes = Vec::new();
        while let Some(chunk) = it.next().await? {
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            return Ok(false);
        }
        let tmp = cached.with_extension("tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, cached)?;
        Ok(true)
    }
}

/// Downloadable wrapper over a raw Telegram photo location.
struct RawPhoto(tl::enums::InputFileLocation);

impl Downloadable for RawPhoto {
    fn to_raw_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        Some(self.0.clone())
    }
}

/// Media kind (for layout) of a message attachment.
pub fn media_kind(media: Option<&Media>) -> Option<MediaKind> {
    let Media::Photo(photo) = media? else {
        return None;
    };
    let thumb = pick_photo_size(photo)?;
    Some(MediaKind::Photo {
        width: thumb.0,
        height: thumb.1,
    })
}

/// A lightweight downloadable for a photo (a ~256 px thumbnail).
fn media_downloadable(media: Option<&Media>) -> Option<PhotoSize> {
    let Media::Photo(photo) = media? else {
        return None;
    };
    let (_, _, size) = pick_photo_size(photo)?;
    Some(size)
}

/// Chooses a `PhotoSize`: the smallest downloadable one of at least 512 px
/// wide (crisp enough for a preview), otherwise the largest available.
/// Returns `(width, height, size)`.
fn pick_photo_size(photo: &Photo) -> Option<(u32, u32, PhotoSize)> {
    let thumbs = photo.thumbs();
    let mut downloadables: Vec<(u32, u32, PhotoSize)> = Vec::new();
    for thumb in thumbs {
        if let PhotoSize::Size(size) = &thumb {
            let (w, h) = (size.width.max(0) as u32, size.height.max(0) as u32);
            downloadables.push((w, h, thumb.clone()));
        }
    }
    if downloadables.is_empty() {
        return None;
    }
    downloadables.sort_by_key(|(w, _, _)| *w);
    let chosen = downloadables
        .iter()
        .find(|(w, _, _)| *w >= 512)
        .or_else(|| downloadables.last())
        .cloned()?;
    Some(chosen)
}