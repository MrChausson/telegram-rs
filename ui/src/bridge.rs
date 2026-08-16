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
    NewMessage { chat_id: i64, id: i32, text: String, out: bool },
    /// An existing message was edited live.
    MessageEdited { chat_id: i64, id: i32, text: String },
    /// Some messages were deleted live (in the open chat).
    MessageDeleted { ids: Vec<i32> },
    /// Error to display (status).
    Error(String),
}