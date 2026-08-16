//! Data model for the UI: chat, message, etc.

use grammers_session::types::{PeerId, PeerRef};

/// Summary of a chat shown in the list.
#[derive(Debug, Clone)]
pub struct ChatInfo {
    pub id: PeerId,
    pub title: String,
    pub last_message: Option<String>,
    /// Unix timestamp of the last message, if any.
    pub last_date: Option<i32>,
    pub unread_count: i32,
    /// Peer reference, needed to reload messages.
    pub peer_ref: PeerRef,
}

/// A message from a chat (history).
#[derive(Debug, Clone)]
pub struct MessageInfo {
    pub id: i32,
    pub text: String,
    /// Unix timestamp of the message.
    pub date: i32,
    /// True if the message was sent by us.
    pub out: bool,
}