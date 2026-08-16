//! MTProto client: grammers wrapper (auth, connect, dialogs, messages).

use std::sync::Arc;

use anyhow::{Context, Result};
use grammers_client::client::{LoginToken, PasswordToken, SignInError};
use grammers_client::Client;
use grammers_mtsender::SenderPool;
use grammers_session::updates::UpdatesLike;

use crate::model::{ChatInfo, MessageInfo};
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
            let last_message = dialog.last_message.map(|m| m.text().to_string());
            out.push(ChatInfo {
                id,
                title,
                last_message,
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
                out: msg.outgoing(),
            });
        }
        out.reverse();
        Ok(out)
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