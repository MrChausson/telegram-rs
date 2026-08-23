//! Data model for the UI: chat, message, media, etc.

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

/// Kind of media attached to a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaKind {
    Photo { width: u32, height: u32 },
    /// A generic file (document): original file name and byte size.
    Document { name: String, size: i64 },
}

/// Origin of a forwarded message, as much as the forward header exposes:
/// the originating chat id (resolvable against the dialog list) or, for
/// users who hid their profile, a plain name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardInfo {
    pub chat_id: Option<i64>,
    pub name: Option<String>,
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
    /// Media attached to the message, if any.
    pub media: Option<MediaKind>,
    /// Id of the message this one replies to, if any.
    pub reply_to: Option<i32>,
    /// Forward header, if the message was forwarded from somewhere.
    pub forwarded: Option<ForwardInfo>,
}

/// A global search hit: the message plus the id of the chat it lives in
/// (resolvable to a title by the caller, which knows the dialog list).
#[derive(Debug, Clone)]
pub struct GlobalHit {
    pub peer_id: i64,
    pub msg: MessageInfo,
}
