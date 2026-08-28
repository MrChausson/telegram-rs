//! Types exchanged between the Iced UI and the network runtime (tokio).
//! Duplicated from the custom `ui` crate so `app-iced` is fully independent
//! of the winit/softbuffer renderer it is meant to replace.

/// A single chat row (list pane).
#[derive(Debug, Clone)]
pub struct ChatRow {
    pub id: i64,
    pub title: String,
    pub subtitle: String,
    /// Unix timestamp of the last message (0 if none).
    pub date: i32,
    pub unread: i32,
    /// On-disk path of the profile photo thumbnail, once ready.
    pub avatar_path: Option<String>,
}

/// Metadata of a document attached to a message.
#[derive(Debug, Clone, PartialEq)]
pub struct DocMeta {
    /// Original file name (may be empty for unnamed uploads).
    pub name: String,
    /// Size in bytes (0 when unknown).
    pub size: i64,
    /// How to treat the document: plain file, video, gif, audio/voice.
    pub kind: DocKind,
    /// Duration in seconds (videos/audio), if known.
    pub duration: Option<f64>,
}

/// Sticker attachment of a message (rendered frameless, no bubble).
#[derive(Debug, Clone, PartialEq)]
pub struct StickerMeta {
    /// Emoji associated with the sticker.
    pub alt: String,
}

/// One sticker inside a picker set: `(doc_id, access_hash, alt)`.
pub type StickerDocRef = (i64, i64, String);

/// A sticker pack for the picker panel.
#[derive(Debug, Clone)]
pub struct StickerSetBridge {
    pub title: String,
    pub short_name: String,
    pub docs: Vec<StickerDocRef>,
}

/// The kind of document attachment, mirrored from the UI so `app-iced`
/// doesn't depend on the tg types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    File,
    /// A video message (document + video attributes).
    Video,
    /// An animated GIF.
    Gif,
    /// An audio file or voice note.
    Audio { voice: bool },
}

/// A displayed message.
#[derive(Debug, Clone)]
pub struct MsgRow {
    pub id: i32,
    pub text: String,
    /// Unix timestamp of the message.
    pub date: i32,
    /// True if the message was sent by us.
    pub out: bool,
    /// Dimensions of an attached photo, if any.
    pub photo: Option<(u32, u32)>,
    /// On-disk path of the downloaded photo thumbnail, once ready.
    pub photo_path: Option<String>,
    /// Document attachment (name + size), if any.
    pub doc: Option<DocMeta>,
    /// On-disk path of the downloaded document, once fetched.
    pub doc_path: Option<String>,
    /// Sticker attachment (rendered without a bubble), if any.
    pub sticker: Option<StickerMeta>,
    /// On-disk path of the downloaded sticker image, once fetched.
    pub sticker_path: Option<String>,
    /// Id of the message this one replies to, if any.
    pub reply_to: Option<i32>,
    /// Display name of the forward origin, if the message was forwarded
    /// (resolved against the dialog list by the network layer).
    pub forwarded_from: Option<String>,
    /// Upload progress of an optimistic local media send (0.0..=1.0).
    /// `None` on server-known messages.
    pub uploading: Option<f32>,
    /// Token tying this optimistic row to its `SendMedia` request (progress
    /// events come back tagged with it). `None` on server-known messages.
    pub upload_token: Option<u64>,
    /// True once the (outgoing) message was read by the other party.
    pub read: bool,
    /// Display name of the sender in group chats (`None` in private chats).
    pub sender_name: Option<String>,
    /// Bot-API id of the sender, when known (drives per-sender colors).
    pub sender_id: Option<i64>,
    /// True when the message is currently pinned in its chat.
    pub pinned: bool,
}

impl MsgRow {
    /// A plain text row (test/demo convenience).
    pub fn text(id: i32, text: impl Into<String>, date: i32, out: bool) -> Self {
        Self {
            id,
            text: text.into(),
            date,
            out,
            photo: None,
            photo_path: None,
            doc: None,
            doc_path: None,
            sticker: None,
            sticker_path: None,
            reply_to: None,
            forwarded_from: None,
            uploading: None,
            upload_token: None,
            read: false,
            sender_name: None,
            sender_id: None,
            pinned: false,
        }
    }
}

/// A single search result (in-chat or global).
#[derive(Debug, Clone)]
pub struct SearchHit {
    /// Chat the message lives in.
    pub chat_id: i64,
    /// Resolved display name of that chat.
    pub chat_title: String,
    /// The message itself (media/reply/forward included).
    pub row: MsgRow,
}

/// Kind of chat, mirrored from the `tg` crate; drives the info panel layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatKind {
    User,
    Group,
    Channel,
    Bot,
}

/// Detailed info about a chat (right-hand info panel). Duplicated from the
/// `tg` model like `ChatRow`/`MsgRow` so `app-iced` stays independent of the
/// core types.
#[derive(Debug, Clone)]
pub struct ChatDetail {
    pub id: i64,
    pub title: String,
    pub kind: ChatKind,
    pub username: Option<String>,
    pub bio: Option<String>,
    pub phone: Option<String>,
    pub members_count: Option<u32>,
    /// True for supergroups/channels with forums (topics) enabled.
    pub is_forum: bool,
}

/// Role of a member inside a group/channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticipantRole {
    Creator,
    Admin,
    Member,
}

/// One member of a group/channel (info panel's member list).
#[derive(Debug, Clone)]
pub struct ParticipantRow {
    pub id: i64,
    pub name: String,
    pub username: Option<String>,
    pub role: ParticipantRole,
}

/// A forum topic of a supergroup (chips bar of forum chats). Duplicated from
/// the `tg` model like the other bridge types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicRow {
    /// Server-side topic id (same value as `root_msg_id`).
    pub id: i64,
    /// Id of the topic's root message — the thread anchor messages reply to.
    pub root_msg_id: i32,
    pub title: String,
    /// Icon color index (Telegram's topic palette).
    pub icon_color: i32,
}

/// The signed-in user's own profile (settings panel).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MyProfile {
    /// Display name ("First Last").
    pub name: String,
    /// Last name (`None` when unset).
    pub last_name: Option<String>,
    /// @username without the sigil (`None` when unset).
    pub username: Option<String>,
    /// Phone in international format (`+336…`), `None` when hidden.
    pub phone: Option<String>,
    /// Self-description ("bio"), `None` when unset.
    pub bio: Option<String>,
}

/// One active session of the account (settings panel > Sessions).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionInfo {
    pub device: String,
    pub platform: String,
    pub country: String,
    /// True for the device this client runs on.
    pub current: bool,
    /// Server-side id used to revoke this authorization.
    pub hash: i64,
}

/// Request sent by the UI to the network.
#[derive(Debug, Clone)]
pub enum Request {
    /// Opens a chat: loads its history.
    OpenChat { id: i64 },
    /// Marks all messages in a chat as read (server side).
    MarkRead { id: i64 },
    /// Notifies the server that the user is (or stopped) typing in a chat.
    Typing { id: i64, typing: bool },
    /// Sends a text message to a chat, optionally as a reply.
    SendMessage { id: i64, text: String, reply_to: Option<i32> },
    /// Uploads and sends a file to a chat (`is_photo` picks compressed-photo
    /// vs document), optionally with a caption and as a reply. `token` ties
    /// upload progress events back to the optimistic row.
    SendMedia {
        id: i64,
        path: String,
        caption: String,
        is_photo: bool,
        reply_to: Option<i32>,
        token: u64,
    },
    /// Forwards a message from one chat into another.
    ForwardMessage { from_chat: i64, msg_id: i32, to_chat: i64 },
    /// Edits an outgoing message (text only).
    EditMessage { id: i64, msg_id: i32, text: String },
    /// Deletes one of the user's messages (from all devices).
    DeleteMessage { id: i64, msg_id: i32 },
    /// Pins (`pin: true`) or unpins a message in a chat.
    PinMessage { id: i64, msg_id: i32, pin: bool },
    /// Downloads a chat's profile photo thumbnail.
    DownloadAvatar { chat_id: i64 },
    /// Creates a group (`megagroup=true`) or channel. `members` carries the
    /// bot-api ids of the initial members (groups only; empty = create
    /// without inviting anyone).
    CreateChannel {
        title: String,
        about: String,
        megagroup: bool,
        members: Vec<i64>,
    },
    /// Leaves a group/channel (membership ends, dialog disappears).
    LeaveChat { id: i64 },
    /// Deletes a chat from the account (leaves it and clears history).
    DeleteChat { id: i64 },
    /// Renames a group/channel.
    EditChatTitle { id: i64, title: String },
    /// Fetches the info-panel details of a chat.
    GetChatInfo { id: i64 },
    /// Mutes (`muted=true`) or unmutes a chat server-side.
    SetMuted { id: i64, muted: bool },
    /// Lists the members of a group/channel (info panel).
    GetParticipants { id: i64 },
    /// Removes a member from the open group/channel.
    KickParticipant { id: i64, user_id: i64 },
    /// Downloads a message's document into the cache.
    DownloadDoc { chat_id: i64, msg_id: i32 },
    /// Downloads a message's sticker image into the cache.
    DownloadSticker { chat_id: i64, msg_id: i32 },
    /// Sends an existing sticker (by document reference) to a chat.
    SendSticker { id: i64, doc_id: i64, access_hash: i64 },
    /// Lists the installed sticker packs (picker panel).
    GetStickerSets,
    /// Searches messages (`id: None` = global, `Some` = that chat). The
    /// network layer throttles re-runs so typing doesn't flood MTProto.
    Search { id: Option<i64>, query: String },
    /// Login step 1: request the SMS/call verification code for a phone.
    LoginPhone { phone: String },
    /// Login step 2: submit the received code.
    LoginCode { code: String },
    /// Login step 3 (2FA): submit the account password.
    LoginPassword { password: String },
    /// Login (QR): start the export/poll login-token session.
    QrLoginStart,
    /// Login (QR): stop any running token-polling task.
    QrLoginCancel,
    /// Fetches the signed-in user's profile (settings panel).
    GetMe,
    /// Updates the user's own profile (`None` leaves a field unchanged).
    UpdateProfile {
        first_name: Option<String>,
        last_name: Option<String>,
        bio: Option<String>,
    },
    /// Lists the account's active sessions (settings panel).
    GetSessions,
    /// Terminates another session of the account.
    RevokeSession { hash: i64 },
    /// Wipes the on-disk media cache (never the session or UI state).
    ClearCache,
    /// Logs the account out (`auth.logOut`) and purges the local session.
    LogOut,
    /// Grants every admin right to a member of the open group/channel.
    AdminPromote { id: i64, user_id: i64 },
    /// Revokes every admin right from a member of the open group/channel.
    AdminDemote { id: i64, user_id: i64 },
    /// Bans a member forever (`kick_only=false`) or just removes them from
    /// the group, letting them rejoin (`kick_only=true`).
    AdminBan { id: i64, user_id: i64, kick_only: bool },
    /// Loads the forum topics of a chat (empty list for non-forum chats).
    GetTopics { id: i64 },
    /// Creates a forum topic titled `title` in the chat.
    CreateTopic { id: i64, title: String },
    /// Sends a text message into the topic thread anchored by `topic_root`.
    SendTopicMessage { id: i64, text: String, topic_root: i32 },
}

/// Message sent by the network to the UI.
#[derive(Debug, Clone)]
pub enum UiMessage {
    /// The chat list is ready.
    Dialogs(Vec<ChatRow>),
    /// A chat's history was loaded.
    Messages {
        id: i64,
        title: String,
        rows: Vec<MsgRow>,
    },
    /// A new message was received live (incoming, or sent from another
    /// device of the same account).
    NewMessage {
        chat_id: i64,
        id: i32,
        text: String,
        date: i32,
        out: bool,
        photo: Option<(u32, u32)>,
        doc: Option<DocMeta>,
        /// Sticker attachment (emoji only; the image streams in via
        /// `StickerPathReady`).
        sticker: Option<StickerMeta>,
        reply_to: Option<i32>,
        forwarded_from: Option<String>,
        sender_name: Option<String>,
        sender_id: Option<i64>,
    },
    /// An existing message was edited live.
    MessageEdited {
        chat_id: i64,
        id: i32,
        text: String,
        date: i32,
    },
    /// A photo thumbnail was downloaded (path = on-disk location).
    PhotoReady {
        chat_id: i64,
        msg_id: i32,
        path: Option<String>,
    },
    /// A document was downloaded (path = on-disk location).
    DocReady {
        chat_id: i64,
        msg_id: i32,
        path: Option<String>,
    },
    /// A sticker image was downloaded (path = on-disk location, `None` when
    /// the fetch failed or the sticker can't be rendered).
    StickerPathReady {
        chat_id: i64,
        msg_id: i32,
        path: Option<String>,
    },
    /// A picker thumbnail was downloaded (`doc_id` keys it; path = location).
    StickerThumbReady { doc_id: i64, path: Option<String> },
    /// The installed sticker packs arrived (picker panel data).
    StickerSets(Vec<StickerSetBridge>),
    /// Search results for `query`; `id` mirrors the `Request::Search` that
    /// produced them (`None` = global results).
    SearchResults {
        id: Option<i64>,
        query: String,
        hits: Vec<SearchHit>,
    },
    /// Progress of an optimistic media upload (token = the request's token,
    /// progress = 0.0..=1.0).
    UploadProgress { chat_id: i64, token: u64, progress: f32 },
    /// The media upload finished; the server echo will replace the row.
    UploadDone { chat_id: i64, token: u64 },
    /// A chat was marked read (server-side), so the local badge can clear.
    ChatRead { id: i64 },
    /// Another device read a chat: sync its unread badge.
    UnreadCount { chat_id: i64, count: i32 },
    /// The other party read our outgoing messages up to `max_id`; mark them
    /// as read (double check).
    OutboxRead { chat_id: i64, max_id: i32 },
    /// A peer is typing in a chat (`typing=true`) or stopped (`false`).
    PeerTyping { chat_id: i64, typing: bool },
    /// A profile photo thumbnail was downloaded (path = option).
    AvatarReady { chat_id: i64, path: Option<String> },
    /// Detailed info of a chat arrived (info panel).
    ChatInfo(ChatDetail),
    /// The member list of the open group/channel arrived.
    Participants(Vec<ParticipantRow>),
    /// A member was removed from the open chat server-side.
    ParticipantKicked { user_id: i64 },
    /// Some messages were deleted live (in the open chat).
    MessageDeleted { ids: Vec<i32> },
    /// The open chat's pinned message changed (`None` = no pin left).
    PinnedMessage { chat_id: i64, msg_id: Option<i32> },
    /// A group/channel was just created (id = the new chat); the refreshed
    /// dialog list arrives first.
    ChatCreated { id: i64 },
    /// A chat was left/deleted (id = the gone chat).
    ChatGone { id: i64 },
    /// The server acknowledged the phone number: ask the user for the code.
    LoginCodeRequired,
    /// The account has a 2FA password: ask for it (hint = if any).
    LoginPasswordRequired { hint: String },
    /// Sign-in completed: the account is ready to use.
    LoginOk { name: String },
    /// Login (QR): a scannable QR image is ready on disk.
    QrCodeReady { path: String },
    /// Login (QR): the phone scanned the code; sign-in is being confirmed.
    QrScanConfirmed,
    /// Login (QR): the token session failed (message shown on the QR pane).
    QrLoginFailed { error: String },
    /// The signed-in user's own profile (settings panel).
    MyProfile(MyProfile),
    /// The account's active sessions arrived (settings panel).
    Sessions(Vec<SessionInfo>),
    /// A session was terminated server-side (`hash` = the revoked one).
    SessionRevoked { hash: i64 },
    /// The media cache was wiped; `bytes` = the remaining cache size.
    CacheCleared { bytes: u64 },
    /// The session was logged out: the UI resets to the sign-in screen.
    LoggedOut,
    /// A member of the open chat changed server-side (promoted, demoted,
    /// banned or removed); the UI re-requests the participant list.
    MemberUpdated { chat_id: i64, user_id: i64 },
    /// Bot-api id of the signed-in user (so admin actions can hide on
    /// yourself). Sent by the network layer when the member list is served.
    MemberSelfId { id: i64 },
    /// The forum topics of the open chat arrived (`is_forum: false` means the
    /// chat is not a forum and `topics` is empty).
    Topics {
        id: i64,
        is_forum: bool,
        topics: Vec<TopicRow>,
    },
    /// A forum topic was created server-side; the refreshed topic list was
    /// sent right before it (`Topics`).
    TopicCreated { chat_id: i64, topic: TopicRow },
    /// Error to display (status).
    Error(String),
}
