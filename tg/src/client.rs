//! MTProto client: grammers wrapper (connect, dialogs, messages, media).
//!
//! Authentication flows live in the sibling [`crate::auth`] module.

use std::sync::Arc;

use anyhow::{Context, Result};
use grammers_client::media::{Downloadable, Media, Photo, PhotoSize};
use grammers_client::tl;
use grammers_client::Client;
use grammers_mtsender::SenderPool;
use grammers_session::updates::UpdatesLike;

use crate::model::{
    uv16_span_to_bytes, AuthSession, ChatDetail, ChatInfo, ChatKind, CodeEntity, ForwardInfo,
    GlobalHit, MediaKind, MessageInfo, ParticipantRole, ParticipantRow, ReactionInfo, SelfProfile,
    StickerDoc, StickerSetInfo,
};
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

    /// Lists the user's chats (dialogs), ordered by activity.
    pub async fn get_dialogs(&self) -> Result<Vec<ChatInfo>> {
        let mut dialogs = self.client.iter_dialogs();
        let mut out = Vec::new();
        while let Some(dialog) = dialogs.next().await? {
            let id = dialog.peer_id();
            let title = dialog.peer().name().unwrap_or("Unknown").to_string();
            let unread_count = match &dialog.raw {
                grammers_client::tl::enums::Dialog::Dialog(d) => d.unread_count,
                grammers_client::tl::enums::Dialog::Folder(_) => 0,
            };
            let peer_ref = dialog.peer_ref();
            let last_date = dialog
                .last_message
                .as_ref()
                .map(|m| m.date().timestamp() as i32);
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
        let mut it = self.client.iter_messages(*peer).limit(limit);
        let mut out = Vec::new();
        while let Some(msg) = it.next().await? {
            out.push(message_info(&msg));
        }
        out.reverse();
        Ok(out)
    }

    /// Searches a chat's history for `query`, newest-first, mapped to the
    /// display model like [`Telegram::get_messages`].
    pub async fn search_chat(
        &self,
        peer: &grammers_session::types::PeerRef,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MessageInfo>> {
        let mut it = self.client.search_messages(*peer).query(query).limit(limit);
        let mut out = Vec::new();
        while let Some(msg) = it.next().await? {
            out.push(message_info(&msg));
        }
        Ok(out)
    }

    /// Searches all chats by text; each hit carries the originating chat's id
    /// (for the caller to resolve into a title via the dialog list).
    pub async fn search_global(&self, query: &str, limit: usize) -> Result<Vec<GlobalHit>> {
        let mut it = self.client.search_all_messages().query(query).limit(limit);
        let mut out = Vec::new();
        while let Some(msg) = it.next().await? {
            let Some(peer) = msg.peer() else {
                continue;
            };
            out.push(GlobalHit {
                peer_id: peer.id().bot_api_dialog_id(),
                msg: message_info(&msg),
            });
        }
        Ok(out)
    }

    /// Fetches the chat's currently pinned message, if any.
    pub async fn get_pinned_message(
        &self,
        peer: &grammers_session::types::PeerRef,
    ) -> Result<Option<MessageInfo>> {
        let Some(msg) = self
            .client
            .get_pinned_message(*peer)
            .await
            .context("fetching pinned message")?
        else {
            return Ok(None);
        };
        Ok(Some(message_info(&msg)))
    }

    /// Pins a message in a chat (server-side; notifies participants).
    pub async fn pin_message(
        &self,
        peer: &grammers_session::types::PeerRef,
        msg_id: i32,
    ) -> Result<()> {
        self.client
            .pin_message(*peer, msg_id)
            .await
            .context("pinning message")?;
        Ok(())
    }

    /// Unpins a message from a chat.
    pub async fn unpin_message(
        &self,
        peer: &grammers_session::types::PeerRef,
        msg_id: i32,
    ) -> Result<()> {
        self.client
            .unpin_message(*peer, msg_id)
            .await
            .context("unpinning message")?;
        Ok(())
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
            .get_messages_by_id(*peer_ref, &[msg_id])
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

    /// Marks all messages in a chat as read (server side).
    ///
    /// Channels use `channels.readHistory`; users and chats use
    /// `messages.readHistory` (handled internally by grammers).
    pub async fn mark_read(&self, peer_ref: &grammers_session::types::PeerRef) -> Result<()> {
        self.client
            .mark_as_read(*peer_ref)
            .await
            .context("marking chat read")?;
        Ok(())
    }

    /// Tells the server that the user is (or stopped) typing in a chat.
    ///
    /// `typing=true` sends `SendMessageTypingAction`; `false` cancels it.
    /// Errors (flood wait, unknown peer) are ignored: typing is best-effort.
    pub async fn set_typing(
        &self,
        peer_ref: &grammers_session::types::PeerRef,
        typing: bool,
    ) -> Result<()> {
        let action = if typing {
            grammers_client::tl::enums::SendMessageAction::SendMessageTypingAction
        } else {
            grammers_client::tl::enums::SendMessageAction::SendMessageCancelAction
        };
        self.client
            .invoke(&grammers_client::tl::functions::messages::SetTyping {
                peer: (*peer_ref).into(),
                top_msg_id: None,
                action,
            })
            .await?;
        Ok(())
    }

    /// Edits the text of one of the user's outgoing messages.
    pub async fn edit_message(
        &self,
        peer_ref: &grammers_session::types::PeerRef,
        msg_id: i32,
        text: &str,
    ) -> Result<()> {
        self.client
            .edit_message(
                *peer_ref,
                msg_id,
                grammers_client::message::InputMessage::new().text(text),
            )
            .await
            .context("editing message")?;
        Ok(())
    }

    /// Deletes a message from both ends (all devices).
    pub async fn delete_message(
        &self,
        peer_ref: &grammers_session::types::PeerRef,
        msg_id: i32,
    ) -> Result<()> {
        self.client
            .delete_messages(*peer_ref, &[msg_id])
            .await
            .context("deleting message")?;
        Ok(())
    }

    /// Sends a text message to a chat, optionally as a reply, and optionally
    /// scheduled (`schedule` is the wall-clock instant to send at; if `Some`,
    /// the server holds the message until then instead of delivering it now).
    pub async fn send_message(
        &self,
        peer: &grammers_session::types::PeerRef,
        text: &str,
        reply_to: Option<i32>,
        schedule: Option<std::time::SystemTime>,
    ) -> Result<()> {
        let mut input = grammers_client::message::InputMessage::new()
            .text(text)
            .reply_to(reply_to);
        if let Some(at) = schedule {
            input = input.schedule_date(Some(at));
        }
        self.client
            .send_message(*peer, input)
            .await
            .context("sending message")
            .map(|_| ())
    }

    /// Uploads and sends a file to a chat, classified by its extension:
    /// photos go out compressed, videos/GIFs/audio as documents with the
    /// proper attributes (so the receivers render them as such). Returns the
    /// id of the sent message.
    ///
    /// `on_progress(bytes_sent, total)` fires during the upload (at least
    /// every [`UPLOAD_PROGRESS_STEP`] bytes) so callers can surface progress.
    pub async fn send_media(
        &self,
        peer: &grammers_session::types::PeerRef,
        path: &std::path::Path,
        caption: &str,
        reply_to: Option<i32>,
        on_progress: impl FnMut(u64, u64) + Send + 'static,
    ) -> Result<i32> {
        let uploaded = self.upload_with_progress(path, on_progress).await?;
        let mut input = grammers_client::message::InputMessage::new()
            .text(caption)
            .reply_to(reply_to);
        match media_kind_of_path(path) {
            // Compressed photo.
            Some(MediaKind::Photo { .. }) => input = input.photo(uploaded),
            // Everything else goes as a document, adding media attributes so
            // the UI can classify it when it comes back through updates.
            kind => {
                let attrs = attributes_for(&kind, path);
                let raw = tl::types::InputMediaUploadedDocument {
                    nosound_video: false,
                    force_file: false,
                    spoiler: false,
                    file: uploaded.raw.clone(),
                    thumb: None,
                    mime_type: mime_guess_of_path(path),
                    attributes: attrs,
                    stickers: None,
                    ttl_seconds: None,
                    video_cover: None,
                    video_timestamp: None,
                };
                input = input.media(raw);
            }
        }
        let sent = self
            .client
            .send_message(*peer, input)
            .await
            .context("sending media")?;
        Ok(sent.id())
    }

    /// Forwards one message from a chat into another. Returns the id of the
    /// forwarded copy in the destination.
    pub async fn forward_message(
        &self,
        from_peer: &grammers_session::types::PeerRef,
        msg_id: i32,
        to_peer: &grammers_session::types::PeerRef,
    ) -> Result<Option<i32>> {
        let sent = self
            .client
            .forward_messages(*to_peer, &[msg_id], *from_peer)
            .await
            .context("forwarding message")?;
        Ok(sent.into_iter().flatten().next().map(|m| m.id()))
    }

    /// Reacts to a message with a single emoji. The reaction is toggled
    /// server-side: sending the same emoji twice removes it. `peer` may be any
    /// chat and `msg_id` its message — in a forum topic the message id already
    /// identifies the message within the peer, so reacting by id works there
    /// too (the current grammers layer needs no explicit reply header).
    pub async fn send_reaction(
        &self,
        peer: &grammers_session::types::PeerRef,
        msg_id: i32,
        emoji: &str,
    ) -> Result<()> {
        self.client
            .send_reactions(*peer, msg_id, emoji)
            .await
            .context("sending reaction")?;
        Ok(())
    }

    /// Downloads a message's document into `dir` (`{msg_id}_{name}`), returning
    /// the saved path, or `None` if the message carries no document.
    /// Cached on disk like photos.
    pub async fn download_document(
        &self,
        peer_ref: &grammers_session::types::PeerRef,
        msg_id: i32,
        dir: &std::path::Path,
    ) -> Result<Option<std::path::PathBuf>> {
        std::fs::create_dir_all(dir)?;
        let msgs = self
            .client
            .get_messages_by_id(*peer_ref, &[msg_id])
            .await
            .context("fetching message")?;
        let Some(Some(msg)) = msgs.into_iter().next() else {
            return Ok(None);
        };
        let Some(Media::Document(doc)) = msg.media().clone() else {
            return Ok(None);
        };
        let raw_name = doc.name().unwrap_or("file").to_string();
        // The file name comes off the wire: keep only the last component to
        // avoid path traversal when building the cache path.
        let safe_name = raw_name.rsplit(['/', '\\']).next().unwrap_or("file");
        let cached = dir.join(format!("{msg_id}_{safe_name}"));
        if cached.exists() {
            return Ok(transcode_ogg_for_playback(&cached));
        }
        let mut it = self.client.iter_download(&doc);
        let mut bytes = Vec::new();
        while let Some(chunk) = it.next().await? {
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            return Ok(None);
        }
        let tmp = dir.join(format!("{msg_id}.tmp"));
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &cached)?;
        Ok(transcode_ogg_for_playback(&cached))
    }

    /// Downloads a message's sticker into `dir` (`{msg_id}.webp`), returning
    /// the saved path, or `None` if the message carries no sticker. Cached on
    /// disk by message id like photos/documents.
    pub async fn download_sticker(
        &self,
        peer_ref: &grammers_session::types::PeerRef,
        msg_id: i32,
        dir: &std::path::Path,
    ) -> Result<Option<std::path::PathBuf>> {
        std::fs::create_dir_all(dir)?;
        let cached = dir.join(format!("{msg_id}.webp"));
        if cached.exists() {
            return Ok(Some(cached));
        }
        let msgs = self
            .client
            .get_messages_by_id(*peer_ref, &[msg_id])
            .await
            .context("fetching message")?;
        let Some(Some(msg)) = msgs.into_iter().next() else {
            return Ok(None);
        };
        let Some(Media::Document(doc)) = msg.media().clone() else {
            return Ok(None);
        };
        // Only cache actual stickers here (keeps the sticker cache clean when
        // a caller races with a re-classified document).
        if !matches!(
            media_kind_of_document(&doc),
            Some(MediaKind::Sticker { .. })
        ) {
            return Ok(None);
        }
        let mut it = self.client.iter_download(&doc);
        let mut bytes = Vec::new();
        while let Some(chunk) = it.next().await? {
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            return Ok(None);
        }
        let tmp = dir.join(format!("{msg_id}.tmp"));
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &cached)?;
        Ok(Some(cached))
    }

    /// Sends an existing sticker (by its document id + access hash) to a chat,
    /// optionally as a reply. Returns the id of the sent message.
    ///
    /// The document reference is rebuilt from the picker listing; Telegram
    /// resolves it back to the original sticker (attribute preserved).
    pub async fn send_sticker(
        &self,
        peer: &grammers_session::types::PeerRef,
        doc_id: i64,
        access_hash: i64,
        file_reference: Vec<u8>,
        reply_to: Option<i32>,
    ) -> Result<i32> {
        use grammers_client::message::InputMessage;
        let input_doc = tl::enums::InputDocument::Document(tl::types::InputDocument {
            id: doc_id,
            access_hash,
            file_reference,
        });
        let media = tl::enums::InputMedia::Document(tl::types::InputMediaDocument {
            spoiler: false,
            id: input_doc,
            video_cover: None,
            video_timestamp: None,
            ttl_seconds: None,
            query: None,
        });
        let input = InputMessage::new().reply_to(reply_to).media(media);
        let sent = self
            .client
            .send_message(*peer, input)
            .await
            .context("sending sticker")?;
        Ok(sent.id())
    }

    /// Downloads a sticker picker thumbnail by reference into `dir`,
    /// returning the saved path. Prefers the small server-side thumbnail
    /// (`thumb_size="s"` — always a decodable jpeg/webp); animated formats
    /// (.tgs/.webm) only exist as full documents, which this pipeline cannot
    /// decode, so the thumbnail is the reliable source. Cached on disk.
    pub async fn download_sticker_doc(
        &self,
        doc_id: i64,
        access_hash: i64,
        file_reference: Vec<u8>,
        dir: &std::path::Path,
    ) -> Result<Option<std::path::PathBuf>> {
        std::fs::create_dir_all(dir)?;
        // Cache probe first (both extensions we may have written).
        let cached_webp = dir.join(format!("{doc_id}.webp"));
        let cached_jpg = dir.join(format!("{doc_id}.jpg"));
        if cached_webp.exists() {
            return Ok(Some(cached_webp));
        }
        if cached_jpg.exists() {
            return Ok(Some(cached_jpg));
        }
        let location_for = |thumb: &str| {
            tl::enums::InputFileLocation::InputDocumentFileLocation(
                tl::types::InputDocumentFileLocation {
                    id: doc_id,
                    access_hash,
                    file_reference: file_reference.clone(),
                    thumb_size: thumb.to_string(),
                },
            )
        };
        for size in ["s", ""] {
            let mut it = self.client.iter_download(&RawPhoto(location_for(size)));
            let mut bytes = Vec::new();
            while let Some(chunk) = it.next().await? {
                bytes.extend_from_slice(&chunk);
            }
            if !Self::decodable_image(&bytes) {
                continue;
            }
            let ext = if bytes.starts_with(b"\xff\xd8") {
                "jpg"
            } else {
                "webp"
            };
            let cached = dir.join(format!("{doc_id}.{ext}"));
            let tmp = dir.join(format!("{doc_id}.tmp"));
            std::fs::write(&tmp, &bytes)?;
            std::fs::rename(&tmp, &cached)?;
            return Ok(Some(cached));
        }
        Ok(None)
    }

    /// True when the payload is an image this app can decode (jpeg or webp
    /// raster). Animated sticker payloads (.tgs gzip / .webm container) are
    /// rejected here so callers keep showing placeholders instead of broken
    /// images.
    fn decodable_image(bytes: &[u8]) -> bool {
        bytes.starts_with(b"\xff\xd8")                       // jpeg
            || bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP"
    }

    /// Lists the installed sticker packs with their documents (picker data).
    ///
    /// `messages.getAllStickers` returns only pack metadata; the actual
    /// sticker documents come from one `messages.getStickerSet` call per pack
    /// (capped at [`STICKER_SETS_LIMIT`] packs to bound round-trips). Packs
    /// whose fetch fails are skipped — a partial picker beats an error page.
    pub async fn sticker_sets(&self) -> Result<Vec<StickerSetInfo>> {
        const SETS_LIMIT: usize = 12;
        const DOCS_LIMIT: usize = 60;
        let all = self
            .client
            .invoke(&tl::functions::messages::GetAllStickers { hash: 0 })
            .await
            .context("listing sticker sets")?;
        let sets = match all {
            tl::enums::messages::AllStickers::Stickers(a) => a.sets,
            tl::enums::messages::AllStickers::NotModified => return Ok(Vec::new()),
        };
        let mut out = Vec::new();
        for set_enum in sets.into_iter().take(SETS_LIMIT) {
            let tl::enums::StickerSet::Set(set) = set_enum;
            let req = tl::functions::messages::GetStickerSet {
                stickerset: tl::enums::InputStickerSet::ShortName(
                    tl::types::InputStickerSetShortName {
                        short_name: set.short_name.clone(),
                    },
                ),
                hash: 0,
            };
            let Ok(res) = self.client.invoke(&req).await else {
                continue;
            };
            let tl::enums::messages::StickerSet::Set(s) = res else {
                continue;
            };
            // `s.set` is itself a single-constructor enum wrapper.
            let tl::enums::StickerSet::Set(meta) = s.set;
            let docs: Vec<StickerDoc> = s
                .documents
                .into_iter()
                .take(DOCS_LIMIT)
                .filter_map(|d| match d {
                    tl::enums::Document::Document(raw) => {
                        let alt = raw.attributes.iter().find_map(|a| match a {
                            tl::enums::DocumentAttribute::Sticker(st) => Some(st.alt.clone()),
                            _ => None,
                        })?;
                        Some(StickerDoc {
                            id: raw.id,
                            access_hash: raw.access_hash,
                            alt,
                            file_reference: raw.file_reference.clone(),
                        })
                    }
                    _ => None,
                })
                .collect();
            out.push(StickerSetInfo {
                title: meta.title,
                short_name: meta.short_name,
                count: docs.len(),
                docs,
            });
        }
        Ok(out)
    }

    /// Uploads a file with progress reporting: `on_progress(bytes_sent,
    /// total)` is called at least every [`UPLOAD_PROGRESS_STEP`] bytes.
    ///
    /// grammers' `upload_stream` reads through our counting wrapper, so no
    /// extra buffering happens and the callback rate is proportional to real
    /// network progress.
    async fn upload_with_progress(
        &self,
        path: &std::path::Path,
        on_progress: impl FnMut(u64, u64) + Send + 'static,
    ) -> Result<grammers_client::media::Uploaded> {
        let size = tokio::fs::metadata(path).await?.len();
        let file = tokio::fs::File::open(path)
            .await
            .with_context(|| format!("opening {}", path.display()))?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());
        let mut reader = ProgressReader::new(file, size, on_progress);
        self.client
            .upload_stream(&mut reader, size as usize, name)
            .await
            .map_err(|e| anyhow::anyhow!("upload failed: {e}"))
    }

    /// Creates a basic group chat titled `title`, inviting `users`.
    ///
    /// The caller supplies the resolved [`PeerRef`]s of the initial members
    /// (the network layer keeps them for every known dialog); each one is
    /// turned into an `InputUser` here using the access hash cached in the
    /// session. An empty slice creates the group without members — more can
    /// be invited later.
    ///
    /// Returns the bot-api id of the created chat (parsed from the updates
    /// the server answers with).
    pub async fn create_group(
        &self,
        title: &str,
        users: &[grammers_session::types::PeerRef],
    ) -> Result<i64> {
        let input_users: Vec<tl::enums::InputUser> = users
            .iter()
            .map(|p| {
                tl::enums::InputUser::User(tl::types::InputUser {
                    user_id: p.id.bare_id(),
                    access_hash: p.auth.hash(),
                })
            })
            .collect();
        let res = self
            .client
            .invoke(&tl::functions::messages::CreateChat {
                users: input_users,
                title: title.to_string(),
                ttl_period: None,
            })
            .await
            .context("creating group")?;
        // messages.createChat answers with messages.invitedUsers wrapping the
        // Updates that carry the new chat.
        let updates = match res {
            tl::enums::messages::InvitedUsers::Users(i) => i.updates,
        };
        created_chat_id(updates).context("created chat not found in response")
    }

    /// Creates a channel (`megagroup=false`) or a supergroup
    /// (`megagroup=true`) titled `title`, and returns its bot-api id.
    pub async fn create_channel(&self, title: &str, about: &str, megagroup: bool) -> Result<i64> {
        let res = self
            .client
            .invoke(&tl::functions::channels::CreateChannel {
                broadcast: !megagroup,
                megagroup,
                for_import: false,
                forum: false,
                title: title.to_string(),
                about: about.to_string(),
                geo_point: None,
                address: None,
                ttl_period: None,
            })
            .await
            .context("creating channel")?;
        created_chat_id(res).context("created channel not found in response")
    }

    /// Leaves a chat: channels/supergroups via `channels.leaveChannel`,
    /// basic groups via `messages.deleteChatUser` (with ourselves as user).
    /// grammers' shared helper picks the right request per peer kind.
    pub async fn leave_chat(&self, peer: &grammers_session::types::PeerRef) -> Result<()> {
        self.client
            .delete_dialog(*peer)
            .await
            .context("leaving chat")?;
        Ok(())
    }

    /// Deletes a chat from the account (clears history and leaves it).
    /// Same mechanics as leaving — this is Telegram's "delete dialog".
    pub async fn delete_chat(&self, peer: &grammers_session::types::PeerRef) -> Result<()> {
        self.client
            .delete_dialog(*peer)
            .await
            .context("deleting chat")?;
        Ok(())
    }

    /// Renames a chat: `channels.editTitle` for channels/supergroups,
    /// `messages.editChatTitle` for basic groups.
    pub async fn edit_chat_title(
        &self,
        peer: &grammers_session::types::PeerRef,
        title: &str,
    ) -> Result<()> {
        match peer.id.kind() {
            grammers_session::types::PeerKind::Channel => {
                self.client
                    .invoke(&tl::functions::channels::EditTitle {
                        channel: tl::enums::InputChannel::Channel(tl::types::InputChannel {
                            channel_id: peer.id.bare_id(),
                            access_hash: peer.auth.hash(),
                        }),
                        title: title.to_string(),
                    })
                    .await
                    .context("renaming channel")?;
            }
            grammers_session::types::PeerKind::Chat => {
                self.client
                    .invoke(&tl::functions::messages::EditChatTitle {
                        chat_id: peer.id.bare_id(),
                        title: title.to_string(),
                    })
                    .await
                    .context("renaming chat")?;
            }
            _ => anyhow::bail!("only groups and channels can be renamed"),
        }
        Ok(())
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

    /// Fetches detailed info about a chat for the info side panel.
    ///
    /// Dispatches on the peer kind: `users.getFullUser`,
    /// `channels.getFullChannel` or `messages.getFullChat`. Whatever Telegram
    /// doesn't expose (hidden bio, no username…) stays `None` — a partial
    /// result beats an error here.
    pub async fn get_chat_detail(
        &self,
        peer_ref: &grammers_session::types::PeerRef,
    ) -> Result<ChatDetail> {
        use grammers_session::types::PeerKind;
        let id = peer_ref.id.bot_api_dialog_id();
        match peer_ref.id.kind() {
            PeerKind::User | PeerKind::UserSelf => {
                let input_user = tl::enums::InputUser::User(tl::types::InputUser {
                    user_id: peer_ref.id.bare_id(),
                    access_hash: peer_ref.auth.hash(),
                });
                let mut users = self
                    .client
                    .invoke(&tl::functions::users::GetUsers {
                        id: vec![input_user.clone()],
                    })
                    .await
                    .context("fetching user")?;
                let (title, username, phone, bot) = match users.pop() {
                    Some(tl::enums::User::User(u)) => {
                        let name = [
                            u.first_name.clone().unwrap_or_default(),
                            u.last_name.clone().unwrap_or_default(),
                        ]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ");
                        let username = u
                            .username
                            .clone()
                            .or_else(|| {
                                u.usernames.as_ref().and_then(|v| {
                                    v.iter()
                                        .map(|x| match x {
                                            tl::enums::Username::Username(un) => {
                                                un.username.clone()
                                            }
                                        })
                                        .next()
                                })
                            })
                            .filter(|s| !s.is_empty());
                        (
                            if name.is_empty() {
                                "Unknown".to_string()
                            } else {
                                name
                            },
                            username,
                            u.phone
                                .clone()
                                .filter(|p: &String| !p.is_empty())
                                .map(|p| format!("+{p}")),
                            u.bot,
                        )
                    }
                    _ => ("Unknown".to_string(), None, None, false),
                };
                let full = self
                    .client
                    .invoke(&tl::functions::users::GetFullUser { id: input_user })
                    .await
                    .context("fetching user info")?;
                let about = match full {
                    tl::enums::users::UserFull::Full(f) => match f.full_user {
                        tl::enums::UserFull::Full(u) => u.about,
                    },
                };
                Ok(ChatDetail {
                    id,
                    title,
                    kind: if bot { ChatKind::Bot } else { ChatKind::User },
                    username,
                    bio: about.filter(|s| !s.trim().is_empty()),
                    phone,
                    members_count: None,
                    is_forum: false,
                })
            }
            PeerKind::Channel => {
                let input_channel = tl::enums::InputChannel::Channel(tl::types::InputChannel {
                    channel_id: peer_ref.id.bare_id(),
                    access_hash: peer_ref.auth.hash(),
                });
                // Title + username come off the channel entity; about/count
                // off the full-info call.
                let chats = self
                    .client
                    .invoke(&tl::functions::channels::GetChannels {
                        id: vec![input_channel.clone()],
                    })
                    .await
                    .context("fetching channel")?;
                let mut forum = false;
                let (title, username) = chats
                    .chats()
                    .into_iter()
                    .find_map(|c| match c {
                        tl::enums::Chat::Channel(ch) => {
                            forum = ch.forum;
                            Some((ch.title.clone(), ch.username.clone()))
                        }
                        _ => None,
                    })
                    .unwrap_or(("Unknown".to_string(), None));
                let full = self
                    .client
                    .invoke(&tl::functions::channels::GetFullChannel {
                        channel: input_channel,
                    })
                    .await
                    .context("fetching channel info")?;
                // GetFullChannel returns messages.ChatFull{ full_chat:
                // ChatFull::ChannelFull } — unwrap both layers.
                let (about, members_count) = match full {
                    tl::enums::messages::ChatFull::Full(f) => match f.full_chat {
                        tl::enums::ChatFull::ChannelFull(cf) => (
                            Some(cf.about),
                            cf.participants_count.map(|c| c.max(0) as u32),
                        ),
                        tl::enums::ChatFull::Full(_) => (None, None),
                    },
                };
                Ok(ChatDetail {
                    id,
                    title,
                    kind: ChatKind::Channel,
                    username: username.filter(|s| !s.is_empty()),
                    bio: about.filter(|s| !s.trim().is_empty()),
                    phone: None,
                    members_count,
                    is_forum: forum,
                })
            }
            PeerKind::Chat => {
                let chat_id = peer_ref.id.bare_id();
                let full = self
                    .client
                    .invoke(&tl::functions::messages::GetFullChat { chat_id })
                    .await
                    .context("fetching group info")?;
                // Single-constructor type: the payload is always `Full`.
                let tl::enums::messages::ChatFull::Full(f) = full;
                let tl::enums::ChatFull::Full(chat) = f.full_chat else {
                    anyhow::bail!("unexpected chat payload for a basic group");
                };
                let members_count = match &chat.participants {
                    tl::enums::ChatParticipants::Participants(p) => {
                        Some(p.participants.len().min(u32::MAX as usize) as u32)
                    }
                    _ => None,
                };
                // Basic groups have no username; the title comes from the
                // chat entity (`get_chats`), with a graceful fallback.
                let title = match self
                    .client
                    .invoke(&tl::functions::messages::GetChats { id: vec![chat_id] })
                    .await
                {
                    Ok(chats) => chats
                        .chats()
                        .into_iter()
                        .find_map(|c| match c {
                            tl::enums::Chat::Chat(c) => Some(c.title.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| "Group".to_string()),
                    Err(_) => "Group".to_string(),
                };
                Ok(ChatDetail {
                    id,
                    title,
                    kind: ChatKind::Group,
                    username: None,
                    bio: (!chat.about.trim().is_empty()).then(|| chat.about.clone()),
                    phone: None,
                    members_count,
                    is_forum: false,
                })
            }
        }
    }

    /// Lists up to `limit` members of a group/channel. Private conversations
    /// have no participants; that's an empty list, not an error.
    pub async fn get_participants(
        &self,
        peer_ref: &grammers_session::types::PeerRef,
        limit: usize,
    ) -> Result<Vec<ParticipantRow>> {
        use grammers_client::peer::Role;
        use grammers_session::types::PeerKind;
        if !matches!(peer_ref.id.kind(), PeerKind::Chat | PeerKind::Channel) {
            return Ok(Vec::new());
        }
        let mut it = self.client.iter_participants(*peer_ref);
        let mut out = Vec::new();
        while out.len() < limit {
            let Some(p) = it.next().await.context("listing participants")? else {
                break;
            };
            let role = match &p.role {
                Role::Creator(_) => ParticipantRole::Creator,
                Role::Admin(_) => ParticipantRole::Admin,
                _ => ParticipantRole::Member,
            };
            let name = {
                let n = p.user.full_name();
                if n.is_empty() {
                    format!("User {}", p.user.id().bot_api_dialog_id())
                } else {
                    n
                }
            };
            out.push(ParticipantRow {
                id: p.user.id().bot_api_dialog_id(),
                name,
                username: p.user.username().map(str::to_string),
                role,
            });
        }
        Ok(out)
    }

    /// Removes `user_id` from the chat (kick). The user is resolved through
    /// the session first so the kick carries a usable peer reference.
    pub async fn kick_participant(
        &self,
        peer_chat: &grammers_session::types::PeerRef,
        user_id: i64,
    ) -> Result<()> {
        use grammers_session::types::{PeerAuth, PeerId, PeerRef};
        let user = PeerRef {
            id: PeerId::user(user_id),
            auth: PeerAuth::default(),
        };
        let _ = self.client.resolve_peer(user).await;
        self.client
            .kick_participant(*peer_chat, user)
            .await
            .context("kicking participant")?;
        Ok(())
    }

    /// Mutes (`muted=true`) or unmutes a chat by pushing future-dated
    /// / cleared notification settings to the server.
    pub async fn set_muted(
        &self,
        peer_ref: &grammers_session::types::PeerRef,
        muted: bool,
    ) -> Result<()> {
        // Mute for ~5 years out (Telegram's own "forever" convention);
        // unmute resets the deadline to 0.
        const MUTE_YEARS: i32 = 5 * 365 * 24 * 3600;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i32)
            .unwrap_or(0);
        let mute_until = Some(if muted { now + MUTE_YEARS } else { 0 });
        self.client
            .invoke(&tl::functions::account::UpdateNotifySettings {
                peer: tl::enums::InputNotifyPeer::Peer(tl::types::InputNotifyPeer {
                    peer: (*peer_ref).into(),
                }),
                settings: tl::enums::InputPeerNotifySettings::Settings(
                    tl::types::InputPeerNotifySettings {
                        show_previews: None,
                        silent: None,
                        mute_until,
                        sound: None,
                        stories_muted: None,
                        stories_hide_sender: None,
                        stories_sound: None,
                    },
                ),
            })
            .await
            .context("updating notification settings")?;
        Ok(())
    }

    /// Fetches the signed-in user's own profile: name/username/phone off
    /// `users.getUsers` (self), bio (about) off `users.getFullUser`. Whatever
    /// Telegram hides stays `None` — a partial result beats an error here.
    pub async fn get_me(&self) -> Result<SelfProfile> {
        let mut users = self
            .client
            .invoke(&tl::functions::users::GetUsers {
                id: vec![tl::enums::InputUser::UserSelf],
            })
            .await
            .context("fetching own profile")?;
        let (name, last_name, username, phone) = match users.pop() {
            Some(tl::enums::User::User(u)) => {
                let name = [
                    u.first_name.clone().unwrap_or_default(),
                    u.last_name.clone().unwrap_or_default(),
                ]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
                let last_name = u.last_name.clone().filter(|s| !s.trim().is_empty());
                let username = u
                    .username
                    .clone()
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        u.usernames.as_ref().and_then(|v| {
                            v.iter()
                                .map(|x| match x {
                                    tl::enums::Username::Username(un) => un.username.clone(),
                                })
                                .next()
                        })
                    })
                    .filter(|s| !s.is_empty());
                (
                    name,
                    last_name,
                    username,
                    u.phone
                        .clone()
                        .filter(|p| !p.is_empty())
                        .map(|p| format!("+{p}")),
                )
            }
            _ => ("Unknown".to_string(), None, None, None),
        };
        let bio = match self
            .client
            .invoke(&tl::functions::users::GetFullUser {
                id: tl::enums::InputUser::UserSelf,
            })
            .await
        {
            Ok(full) => match full {
                tl::enums::users::UserFull::Full(f) => match f.full_user {
                    tl::enums::UserFull::Full(u) => u.about,
                },
            },
            Err(_) => None,
        };
        Ok(SelfProfile {
            name,
            last_name,
            username,
            phone,
            bio: bio.filter(|s| !s.trim().is_empty()),
        })
    }

    /// Updates the user's own profile (`None` leaves a field unchanged).
    /// The caller refreshes the panel with a fresh [`Self::get_me`].
    pub async fn update_profile(
        &self,
        first_name: Option<String>,
        last_name: Option<String>,
        about: Option<String>,
    ) -> Result<()> {
        self.client
            .invoke(&tl::functions::account::UpdateProfile {
                first_name,
                last_name,
                about,
            })
            .await
            .context("updating profile")?;
        Ok(())
    }

    /// Lists the account's active sessions (settings panel).
    pub async fn sessions(&self) -> Result<Vec<AuthSession>> {
        let res = self
            .client
            .invoke(&tl::functions::account::GetAuthorizations {})
            .await
            .context("listing sessions")?;
        let list = match res {
            tl::enums::account::Authorizations::Authorizations(a) => a.authorizations,
        };
        Ok(list
            .into_iter()
            .map(|a| match a {
                tl::enums::Authorization::Authorization(a) => AuthSession {
                    device: a.device_model,
                    platform: if a.platform.is_empty() {
                        a.system_version
                    } else {
                        a.platform
                    },
                    country: a.country,
                    current: a.current,
                    hash: a.hash,
                },
            })
            .collect())
    }

    /// Terminates another session of the account by its authorization hash.
    pub async fn revoke_session(&self, hash: i64) -> Result<()> {
        let ok = self
            .client
            .invoke(&tl::functions::account::ResetAuthorization { hash })
            .await
            .context("revoking session")?;
        if !ok {
            anyhow::bail!("session {hash} could not be revoked");
        }
        Ok(())
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
            grammers_session::types::PeerKind::Chat => self
                .client
                .invoke(&tl::functions::messages::GetChats {
                    id: vec![peer_ref.id.bare_id()],
                })
                .await
                .context("fetching chat")?,
            grammers_session::types::PeerKind::Channel => {
                let input_channel = tl::enums::InputChannel::Channel(tl::types::InputChannel {
                    channel_id: peer_ref.id.bare_id(),
                    access_hash: peer_ref.auth.hash(),
                });
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

/// Extracts the bot-api id of the chat a creation call created: the server
/// answers with `Updates` whose `chats` vector holds exactly the new
/// group/channel.
fn created_chat_id(updates: tl::enums::Updates) -> Option<i64> {
    let tl::enums::Updates::Updates(u) = updates else {
        return None;
    };
    u.chats.into_iter().find_map(|chat| match chat {
        tl::enums::Chat::Chat(c) => {
            Some(grammers_session::types::PeerId::chat(c.id).bot_api_dialog_id())
        }
        tl::enums::Chat::Channel(c) => {
            Some(grammers_session::types::PeerId::channel(c.id).bot_api_dialog_id())
        }
        _ => None,
    })
}

/// Downloadable wrapper over a raw Telegram photo location.
struct RawPhoto(tl::enums::InputFileLocation);

impl Downloadable for RawPhoto {
    fn to_raw_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        Some(self.0.clone())
    }
}

/// The forum-topic / thread root a message belongs to: the `top_msg_id` of its
/// reply header. Grammers only exposes the direct reply target
/// (`reply_to_message_id`), which for an in-thread reply to *another* post
/// differs from the thread root. Topic views must use this to keep everyone's
/// messages in the thread.
fn reply_top_msg_id(msg: &grammers_client::message::Message) -> Option<i32> {
    match &msg.raw {
        tl::enums::Message::Message(tl::types::Message {
            reply_to: Some(tl::enums::MessageReplyHeader::Header(header)),
            ..
        }) => header.reply_to_top_id,
        _ => None,
    }
}

/// The reactions received on a message, flattened to *plain emoji* ones
/// (custom-emoji, paid and other non-renderable reactions are skipped: they
/// cannot be drawn as a single glyph). `chosen_order` marks a reaction the
/// current account gave itself.
fn reactions_of(msg: &grammers_client::message::Message) -> Vec<ReactionInfo> {
    let tl::enums::Message::Message(tl::types::Message {
        reactions: Some(tl::enums::MessageReactions::Reactions(reactions)),
        ..
    }) = &msg.raw
    else {
        return Vec::new();
    };
    reactions
        .results
        .iter()
        .filter_map(|r| match r {
            tl::enums::ReactionCount::Count(c) => match &c.reaction {
                tl::enums::Reaction::Emoji(e) => Some(ReactionInfo {
                    emoji: e.emoticon.clone(),
                    count: c.count as u32,
                    mine: c.chosen_order.is_some(),
                }),
                _ => None,
            },
        })
        .collect()
}

/// Extracts `code` (inline) and `pre` (block) formatting entities from a
/// message's format entities, as byte-offset spans usable to slice `text`.
/// Emoji/link/strike/etc. are ignored — only code preview spans matter here.
fn code_entities(entities: Option<&Vec<tl::enums::MessageEntity>>, text: &str) -> Vec<CodeEntity> {
    let Some(entities) = entities else {
        return Vec::new();
    };
    entities
        .iter()
        .filter_map(|e| match e {
            tl::enums::MessageEntity::Code(raw) => {
                let (s, end) = uv16_span_to_bytes(
                    text,
                    raw.offset.max(0) as usize,
                    raw.length.max(0) as usize,
                )?;
                Some(CodeEntity {
                    start: s,
                    end,
                    block: None,
                })
            }
            tl::enums::MessageEntity::Pre(raw) => {
                let (s, end) = uv16_span_to_bytes(
                    text,
                    raw.offset.max(0) as usize,
                    raw.length.max(0) as usize,
                )?;
                Some(CodeEntity {
                    start: s,
                    end,
                    block: Some(raw.language.clone()),
                })
            }
            _ => None,
        })
        .collect()
}

/// Extract the `code`/`pre` formatting spans of a message as byte-offset
/// spans (usable both by `message_info` and by live-update paths in the UI).
pub fn code_entities_of(msg: &grammers_client::message::Message) -> Vec<CodeEntity> {
    code_entities(msg.fmt_entities(), msg.text())
}

/// Maps a grammers `Message` to the display model shared by history and
/// search results (reply/forward/media extracted the same way everywhere).
fn message_info(msg: &grammers_client::message::Message) -> MessageInfo {
    // Sender name only matters in group-ish chats; private chats show one
    // side of the conversation so the name is redundant.
    let is_private = matches!(
        msg.peer_id().kind(),
        grammers_session::types::PeerKind::User | grammers_session::types::PeerKind::UserSelf
    );
    let sender = msg.sender();
    MessageInfo {
        id: msg.id(),
        text: msg.text().to_string(),
        code: code_entities_of(msg),
        date: msg.date().timestamp() as i32,
        out: msg.outgoing(),
        media: media_kind(msg.media().as_ref()),
        reactions: reactions_of(msg),
        reply_to: msg.reply_to_message_id(),
        reply_to_top: reply_top_msg_id(msg),
        forwarded: msg.forward_header().as_ref().and_then(forward_info),
        sender_name: if is_private {
            None
        } else {
            sender.and_then(|p| p.name()).map(str::to_string)
        },
        sender_id: if is_private {
            None
        } else {
            msg.sender_id().map(|id| id.bot_api_dialog_id())
        },
        pinned: msg.pinned(),
    }
}

/// Media kind (for layout) of a message attachment.
pub fn media_kind(media: Option<&Media>) -> Option<MediaKind> {
    match media? {
        Media::Photo(photo) => {
            let thumb = pick_photo_size(photo)?;
            Some(MediaKind::Photo {
                width: thumb.0,
                height: thumb.1,
            })
        }
        Media::Document(doc) => media_kind_of_document(doc),
        _ => None,
    }
}

/// Classifies a grammers `Document` (video / gif / audio / voice) from its
/// `DocumentAttribute`s, falling back to a plain file.
pub fn media_kind_of_document(doc: &grammers_client::media::Document) -> Option<MediaKind> {
    let name = doc.name().unwrap_or_default().to_string();
    let size = doc.size().map(|s| s as i64).unwrap_or(0);
    use grammers_client::tl::enums::Document as D;
    let attributes = match doc.raw.document.as_ref() {
        Some(D::Document(d)) => d.attributes.clone(),
        _ => return Some(MediaKind::Document { name, size }),
    };
    use grammers_client::tl::enums::DocumentAttribute as Attr;
    for attr in &attributes {
        match attr {
            // Sticker check MUST come first: animated stickers carry both a
            // sticker and an Animated attribute, and they stay stickers.
            Attr::Sticker(s) => {
                return Some(MediaKind::Sticker {
                    name,
                    size,
                    alt: s.alt.clone(),
                })
            }
            Attr::Video(v) => {
                return Some(MediaKind::Video {
                    name,
                    size,
                    duration: v.duration,
                })
            }
            Attr::Animated => return Some(MediaKind::Gif { name, size }),
            Attr::Audio(a) => {
                return Some(MediaKind::Audio {
                    name,
                    size,
                    voice: a.voice,
                    duration: a.duration as f64,
                })
            }
            _ => {}
        }
    }
    Some(MediaKind::Document { name, size })
}

/// True when the path looks like an image that Telegram can compress into a
/// photo (by extension — no content sniffing).
/// Transcodes an Ogg file to WAV next to it (same stem) so rodio can play
/// Telegram voice notes: they are Opus-in-Ogg and neither rodio nor
/// symphonia ship an Opus decoder, while ffmpeg handles them everywhere.
/// Returns the WAV path on success, the original path otherwise (a missing
/// system ffmpeg degrades to a logged playback failure, never an error).
fn transcode_ogg_for_playback(cached: &std::path::Path) -> Option<std::path::PathBuf> {
    let is_ogg = std::fs::read(cached)
        .ok()
        .is_some_and(|b| b.starts_with(b"OggS"));
    if !is_ogg {
        return Some(cached.to_path_buf());
    }
    let wav = cached.with_extension("wav");
    if wav.exists() {
        return Some(wav);
    }
    let out = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(cached)
        .arg(&wav)
        .output();
    match out {
        Ok(o) if o.status.success() => Some(wav),
        other => {
            // Keep serving the ORIGINAL ogg path: playback falls back to the
            // system player (which decodes Opus natively) instead of losing
            // the download entirely.
            eprintln!(
                "voice transcode failed for {}: {:?}",
                cached.display(),
                other
                    .map(|o| o.stderr)
                    .map(|e| String::from_utf8_lossy(&e).into_owned())
            );
            Some(cached.to_path_buf())
        }
    }
}

pub fn is_image(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp")
    )
}

/// Returns the [`MediaKind`] a local file should be uploaded as, by extension.
pub fn media_kind_of_path(path: &std::path::Path) -> Option<MediaKind> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let size = std::fs::metadata(path)
        .ok()
        .map(|m| m.len() as i64)
        .unwrap_or(0);
    let kind = match ext.as_str() {
        "jpg" | "jpeg" | "png" | "webp" | "bmp" => {
            return Some(MediaKind::Photo {
                width: 0,
                height: 0,
            })
        }
        "gif" => MediaKind::Gif { name, size },
        "mp4" | "webm" | "mkv" | "mov" | "m4v" => MediaKind::Video {
            name,
            size,
            duration: 0.0,
        },
        "ogg" | "oga" | "opus" | "m4a" | "mp3" | "wav" | "flac" => MediaKind::Audio {
            name,
            size,
            voice: false,
            duration: 0.0,
        },
        _ => return None,
    };
    Some(kind)
}

/// Document attributes to attach when uploading a non-photo file, so the
/// server + receivers classify it as video / GIF / audio.
fn attributes_for(
    kind: &Option<MediaKind>,
    path: &std::path::Path,
) -> Vec<grammers_client::tl::enums::DocumentAttribute> {
    use grammers_client::tl::enums::DocumentAttribute as Attr;
    use grammers_client::tl::types::{
        DocumentAttributeAudio, DocumentAttributeFilename, DocumentAttributeVideo,
    };

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    match kind {
        Some(MediaKind::Video { duration, .. }) => vec![
            Attr::Video(DocumentAttributeVideo {
                round_message: false,
                supports_streaming: true,
                nosound: false,
                duration: *duration,
                w: 640,
                h: 480,
                preload_prefix_size: None,
                video_start_ts: None,
                video_codec: None,
            }),
            Attr::Filename(DocumentAttributeFilename { file_name }),
        ],
        Some(MediaKind::Gif { .. }) => vec![
            Attr::Animated,
            Attr::Filename(DocumentAttributeFilename { file_name }),
        ],
        Some(MediaKind::Audio { voice, .. }) => vec![
            Attr::Audio(DocumentAttributeAudio {
                voice: *voice,
                duration: 0,
                title: None,
                performer: None,
                waveform: None,
            }),
            Attr::Filename(DocumentAttributeFilename { file_name }),
        ],
        // Plain document (unknown extension, "other files").
        _ => vec![Attr::Filename(DocumentAttributeFilename { file_name })],
    }
}

/// MIME type guess by extension (lightweight, no content sniffing).
fn mime_guess_of_path(path: &std::path::Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("mp4") => "video/mp4".into(),
        Some("webm") => "video/webm".into(),
        Some("mkv") => "video/x-matroska".into(),
        Some("mov") => "video/quicktime".into(),
        Some("ogg" | "opus") => "audio/ogg".into(),
        Some("oga") => "audio/ogg".into(),
        Some("m4a") => "audio/mp4".into(),
        Some("mp3") => "audio/mpeg".into(),
        Some("wav") => "audio/wav".into(),
        Some("flac") => "audio/flac".into(),
        _ => "application/octet-stream".into(),
    }
}

/// Extracts the forward origin from a raw forward header: the originating
/// chat id when resolvable, otherwise the anonymous `from_name`.
fn forward_info(header: &tl::enums::MessageFwdHeader) -> Option<ForwardInfo> {
    let tl::enums::MessageFwdHeader::Header(h) = header;
    let chat_id = h
        .from_id
        .as_ref()
        .map(|peer| grammers_session::types::PeerId::from(peer.clone()).bot_api_dialog_id());
    let name = h.from_name.clone();
    if chat_id.is_none() && name.is_none() {
        return None;
    }
    Some(ForwardInfo { chat_id, name })
}

/// Minimum byte delta between two progress callbacks (256 KiB): keeps the UI
/// feed quiet without visibly stepping the progress bar.
const UPLOAD_PROGRESS_STEP: u64 = 256 * 1024;

/// An [`tokio::io::AsyncRead`] wrapper that counts bytes handed to the
/// uploader and reports progress through a callback.
struct ProgressReader<R> {
    inner: R,
    sent: u64,
    total: u64,
    last_reported: u64,
    on_progress: Box<dyn FnMut(u64, u64) + Send>,
}

impl<R> ProgressReader<R> {
    fn new(inner: R, total: u64, on_progress: impl FnMut(u64, u64) + Send + 'static) -> Self {
        Self {
            inner,
            sent: 0,
            total,
            last_reported: 0,
            on_progress: Box::new(on_progress),
        }
    }
}

impl<R: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for ProgressReader<R> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        match std::pin::Pin::new(&mut self.inner).poll_read(cx, buf) {
            std::task::Poll::Ready(result) => {
                let read = (buf.filled().len() - before) as u64;
                if read > 0 {
                    self.sent += read;
                    let sent = self.sent;
                    let total = self.total;
                    if sent - self.last_reported >= UPLOAD_PROGRESS_STEP || sent >= total {
                        self.last_reported = sent;
                        (self.on_progress)(sent, total);
                    }
                }
                std::task::Poll::Ready(result)
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
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
