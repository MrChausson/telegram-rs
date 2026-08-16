//! Types exchanged between the window (winit) and the network runtime (tokio).
//! The UI only knows `i64` ids; the network maps them to grammers `PeerRef`s.

use crate::chatlist::ChatRow;
use crate::messages::MsgRow;

/// Request sent by the UI to the network.
#[derive(Debug, Clone)]
pub enum Request {
    /// Opens a chat: loads its history.
    OpenChat { id: i64 },
    /// Sends a text message to a chat.
    SendMessage { id: i64, text: String },
    /// Downloads a message's photo thumbnail into the local cache.
    DownloadPhoto { chat_id: i64, msg_id: i32 },
    /// Downloads a chat's profile photo thumbnail.
    DownloadAvatar { chat_id: i64 },
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
    /// A profile photo thumbnail was downloaded (path = option).
    AvatarReady { chat_id: i64, path: Option<String> },
    /// Some messages were deleted live (in the open chat).
    MessageDeleted { ids: Vec<i32> },
    /// Error to display (status).
    Error(String),
}