//! MTProto client: grammers wrapper (auth, connect, dialogs, messages).

use std::sync::Arc;

use anyhow::{Context, Result};
use grammers_client::client::{LoginToken, PasswordToken, SignInError};
use grammers_client::media::{Media, Photo, PhotoSize};
use grammers_client::Client;
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
    pub async fn check_password(&self, password_token: PasswordToken, password: &str) -> Result<()> {
        self.client
            .check_password(password_token, password)
            .await
            .context("2FA sign in")
            .map(|_| ())
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
    pub async fn download_photo(
        &self,
        peer_ref: &grammers_session::types::PeerRef,
        msg_id: i32,
        dir: &std::path::Path,
    ) -> Result<Option<std::path::PathBuf>> {
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
        std::fs::create_dir_all(dir)?;
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

/// Chooses a `PhotoSize`: the smallest downloadable one of at least 256 px
/// wide, otherwise the largest available. Returns `(width, height, size)`.
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
        .find(|(w, _, _)| *w >= 256)
        .or_else(|| downloadables.last())
        .cloned()?;
    Some(chosen)
}