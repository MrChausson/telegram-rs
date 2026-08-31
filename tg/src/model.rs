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
    Photo {
        width: u32,
        height: u32,
    },
    /// A generic file (document): original file name and byte size.
    Document {
        name: String,
        size: i64,
    },
    /// A video message (document + video attributes).
    Video {
        name: String,
        size: i64,
        duration: f64,
    },
    /// An animated GIF (document + animated attribute).
    Gif {
        name: String,
        size: i64,
    },
    /// An audio file or voice note (document + audio attributes).
    Audio {
        name: String,
        size: i64,
        voice: bool,
        duration: f64,
    },
    /// A sticker (document + sticker attribute): associated emoji, byte size
    /// and the original file name (usually empty for stickers).
    Sticker {
        name: String,
        size: i64,
        alt: String,
    },
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

/// A reaction received on a message, as a renderable emoji chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionInfo {
    pub emoji: String,
    pub count: u32,
    /// True when this reaction was given by the current account (so a toggle
    /// can be offered later).
    pub mine: bool,
}
/// A `code` / `pre` formatting entity of a message, precomputed to byte
/// offsets so the UI can slice the UTF-8 `text` directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeEntity {
    /// Byte range `[start, end)` into the message text.
    pub start: usize,
    pub end: usize,
    /// `Some(language)` for a `pre` (block) entity, `None` for inline `code`.
    pub block: Option<String>,
}

/// Telegram entity offsets/lengths are expressed in UTF-16 code units, but
/// the message text we render is a UTF-8 `&str`. Convert a code-unit span
/// into a byte span. Returns `None` when the span is out of bounds.
pub fn uv16_span_to_bytes(
    text: &str,
    uv16_offset: usize,
    uv16_len: usize,
) -> Option<(usize, usize)> {
    fn uv16_to_byte(text: &str, target: usize) -> Option<usize> {
        let mut units = 0usize;
        for (bi, ch) in text.char_indices() {
            if units == target {
                return Some(bi);
            }
            units += ch.len_utf16();
        }
        (units == target).then_some(text.len())
    }
    let start = uv16_to_byte(text, uv16_offset)?;
    let end = uv16_to_byte(text, uv16_offset + uv16_len)?;
    (start <= end).then_some((start, end))
}

/// A message from a chat (history).
#[derive(Debug, Clone)]
pub struct MessageInfo {
    pub id: i32,
    pub text: String,
    /// `code`/`pre` formatting spans (byte offsets into `text`), if any.
    pub code: Vec<CodeEntity>,
    /// Unix timestamp of the message.
    pub date: i32,
    /// True if the message was sent by us.
    pub out: bool,
    /// Media attached to the message, if any.
    pub media: Option<MediaKind>,
    /// Reactions received on the message, as emoji chips (most frequent
    /// first, as Telegram orders the server-side list).
    pub reactions: Vec<ReactionInfo>,
    /// Id of the message this one replies to, if any.
    pub reply_to: Option<i32>,
    /// Id of the topic/thread root a message belongs to, when it lives inside
    /// a forum topic (`reply_to_top_id`). In a thread, top-level posts carry
    /// `reply_to_msg_id == topic id` while *in-thread* replies point their
    /// `reply_to` at the message they answer and only record the root here —
    /// so a topic filter must consult this field, or it drops everyone's
    /// nested replies.
    pub reply_to_top: Option<i32>,
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
    /// True for supergroups/channels with forums (topics) enabled
    /// (`Channel.forum` flag).
    pub is_forum: bool,
}

/// A forum topic of a supergroup (drives the chips bar of forum chats).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicInfo {
    /// Server-side topic id — the same value as `root_msg_id` (Telegram keys
    /// a topic by its root service message id).
    pub id: i64,
    /// Id of the topic's root message (the thread anchor; messages of the
    /// thread carry a reply header pointing at it).
    pub root_msg_id: i32,
    /// Topic title as set at creation.
    pub title: String,
    /// Icon color index (Telegram's topic palette, 0-6).
    pub icon_color: i32,
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

/// The signed-in user's own profile (settings panel).
#[derive(Debug, Clone, Default)]
pub struct SelfProfile {
    /// Display name ("First Last").
    pub name: String,
    /// Last name (`None` when unset).
    pub last_name: Option<String>,
    pub username: Option<String>,
    pub phone: Option<String>,
    pub bio: Option<String>,
}

/// One active authorization (session) of the account.
#[derive(Debug, Clone)]
pub struct AuthSession {
    pub device: String,
    pub platform: String,
    pub country: String,
    /// True for the device making the query.
    pub current: bool,
    /// Server-side id used by `account.resetAuthorization`.
    pub hash: i64,
}
