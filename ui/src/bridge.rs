//! Types exchanged between the window (winit) and the network runtime (tokio).
//! The UI only knows `i64` ids; the network maps them to grammers `PeerRef`s.

use crate::chatlist::ChatRow;
use crate::messages::MsgRow;

/// Request sent by the UI to the network.
#[derive(Debug, Clone)]
pub enum Request {
    /// Opens a chat: loads its history.
    OpenChat { id: i64 },
    /// Marks all messages in a chat as read (server side).
    MarkRead { id: i64 },
    /// Tells the server the user started (`typing=true`) or stopped typing.
    Typing { id: i64, typing: bool },
    /// Sends a text message to a chat.
    SendMessage { id: i64, text: String },
    /// Edits an outgoing message (text only).
    EditMessage { id: i64, msg_id: i32, text: String },
    /// Deletes one of the user's messages (from all devices).
    DeleteMessage { id: i64, msg_id: i32 },
    /// Downloads a message's photo thumbnail into the local cache.
    DownloadPhoto { chat_id: i64, msg_id: i32 },
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
    NewMessage { chat_id: i64, id: i32, text: String, date: i32, out: bool, photo: Option<(u32, u32)> },
    /// An existing message was edited live.
    MessageEdited { chat_id: i64, id: i32, text: String, date: i32 },
    /// A photo thumbnail was downloaded (path = on-disk location).
    PhotoReady { chat_id: i64, msg_id: i32, path: Option<String> },
    /// A chat was marked read (server-side), so the local badge can clear.
    ChatRead { id: i64 },
    /// Another device read a chat: sync its unread badge.
    UnreadCount { chat_id: i64, count: i32 },
    /// The other party read our outgoing messages up to `max_id`; mark them
    /// as read (double check).
    OutboxRead { chat_id: i64, max_id: i32 },
    /// A peer is typing in a chat (`typing=true`) or stopped (`false`).
    /// Shown as a "typing…" status in the header of the open chat.
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