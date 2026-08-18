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
    /// True once the (outgoing) message was read by the other party.
    pub read: bool,
}

/// Request sent by the UI to the network.
#[derive(Debug, Clone)]
pub enum Request {
    /// Opens a chat: loads its history.
    OpenChat { id: i64 },
    /// Marks all messages in a chat as read (server side).
    MarkRead { id: i64 },
    /// Sends a text message to a chat.
    SendMessage { id: i64, text: String },
    /// Edits an outgoing message (text only).
    EditMessage { id: i64, msg_id: i32, text: String },
    /// Deletes one of the user's messages (from all devices).
    DeleteMessage { id: i64, msg_id: i32 },
    /// Downloads a chat's profile photo thumbnail.
    DownloadAvatar { chat_id: i64 },
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
