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
#[derive(Debug, Clone, PartialEq)]
pub enum MediaKind {
    Photo { width: u32, height: u32 },
    /// A generic file (document): original file name and byte size.
    Document { name: String, size: i64 },
    /// A video message (document + video attributes).
    Video { name: String, size: i64, duration: f64 },
    /// An animated GIF (document + animated attribute).
    Gif { name: String, size: i64 },
    /// An audio file or voice note (document + audio attributes).
    Audio { name: String, size: i64, voice: bool, duration: f64 },
    /// A sticker (document + sticker attribute): associated emoji, byte size
    /// and the original file name (usually empty for stickers).
    Sticker { name: String, size: i64, alt: String },
}

/// One sticker document inside a set (`messages.getAllStickers` listing).
#[derive(Debug, Clone)]
pub struct StickerDoc {
    pub id: i64,
    pub access_hash: i64,
    /// Emoji the sticker stands for.
    pub alt: String,
    /// File reference needed to re-send the document reliably.
    pub file_reference: Vec<u8>,
}

/// A sticker pack title + its documents, for the picker panel.
#[derive(Debug, Clone)]
pub struct StickerSetInfo {
    pub title: String,
    pub short_name: String,
    pub count: usize,
    pub docs: Vec<StickerDoc>,
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
    /// Display name of the sender in group chats (`None` in private chats
    /// where the name is redundant, or when unresolvable).
    pub sender_name: Option<String>,
    /// Bot-API id of the sender, when known (drives per-sender colors).
    pub sender_id: Option<i64>,
    /// True when the message is currently pinned in its chat.
    pub pinned: bool,
}

/// A global search hit: the message plus the id of the chat it lives in
/// (resolvable to a title by the caller, which knows the dialog list).
#[derive(Debug, Clone)]
pub struct GlobalHit {
    pub peer_id: i64,
    pub msg: MessageInfo,
}

/// Kind of a chat (drives what the info panel shows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatKind {
    /// A private conversation with a human.
    User,
    /// A basic (legacy) group chat.
    Group,
    /// A channel or megagroup.
    Channel,
    /// A bot account.
    Bot,
}

/// Detailed info about a chat, shown in the right-hand info panel.
/// Every field except `id`/`title`/`kind` is optional: Telegram hides what
/// the peer didn't publish.
#[derive(Debug, Clone)]
pub struct ChatDetail {
    /// Bot-API style id (same space as the dialog list ids).
    pub id: i64,
    pub title: String,
    pub kind: ChatKind,
    pub username: Option<String>,
    pub bio: Option<String>,
    pub phone: Option<String>,
    pub members_count: Option<u32>,
}

/// Role of a member inside a group/channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticipantRole {
    Creator,
    Admin,
    Member,
}

/// One member row of the participants list.
#[derive(Debug, Clone)]
pub struct ParticipantRow {
    /// Bot-API style user id.
    pub id: i64,
    pub name: String,
    pub username: Option<String>,
    pub role: ParticipantRole,
}
