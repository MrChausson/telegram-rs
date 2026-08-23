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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocMeta {
    /// Original file name (may be empty for unnamed uploads).
    pub name: String,
    /// Size in bytes (0 when unknown).
    pub size: i64,
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
            reply_to: None,
            forwarded_from: None,
            uploading: None,
            upload_token: None,
            read: false,
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
    /// Downloads a chat's profile photo thumbnail.
    DownloadAvatar { chat_id: i64 },
    /// Downloads a message's document into the cache.
    DownloadDoc { chat_id: i64, msg_id: i32 },
    /// Searches messages (`id: None` = global, `Some` = that chat). The
    /// network layer throttles re-runs so typing doesn't flood MTProto.
    Search { id: Option<i64>, query: String },
    /// Login step 1: request the SMS/call verification code for a phone.
    LoginPhone { phone: String },
    /// Login step 2: submit the received code.
    LoginCode { code: String },
    /// Login step 3 (2FA): submit the account password.
    LoginPassword { password: String },
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
        reply_to: Option<i32>,
        forwarded_from: Option<String>,
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
    /// Some messages were deleted live (in the open chat).
    MessageDeleted { ids: Vec<i32> },
    /// The server acknowledged the phone number: ask the user for the code.
    LoginCodeRequired,
    /// The account has a 2FA password: ask for it (hint = if any).
    LoginPasswordRequired { hint: String },
    /// Sign-in completed: the account is ready to use.
    LoginOk { name: String },
    /// Error to display (status).
    Error(String),
}
