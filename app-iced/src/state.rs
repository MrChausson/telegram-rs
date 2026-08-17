//! Pure, testable application state (Model-View-Update). No Iced widget types
//! here: `update` turns `Message`s into state changes + `Request`s, exactly
//! like the custom `ui` crate's `UiState`, so it can be unit tested headlessly.

use crate::bridge::{ChatRow, MsgRow, Request, UiMessage};

/// Sign-in flow step (only used while unauthenticated).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginStep {
    Phone,
    Code,
    Password,
}

/// Open context menu over a message (right-click / long-press).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextMenu {
    /// Index of the message row the menu is over.
    pub row: usize,
}

/// Application state.
#[derive(Debug)]
pub struct State {
    /// Sender for requests to the network runtime.
    pub req_tx: tokio::sync::mpsc::UnboundedSender<Request>,

    pub dialogs: Vec<ChatRow>,
    pub messages: Vec<MsgRow>,
    pub open_chat: Option<i64>,
    pub chat_title: String,
    pub status: String,
    pub login_error: bool,

    /// True once the account is signed in (chat UI active).
    pub authenticated: bool,
    /// The peer of the open chat is typing.
    pub typing: bool,

    /// Open context menu, if any.
    pub context_menu: Option<ContextMenu>,
    /// Message id being edited; its old text lives in `composer`.
    pub editing: Option<i32>,

    pub login_step: LoginStep,
    pub login_input: String,
    /// Composer text (fresh message, or the edited text).
    pub composer: String,
    /// Full-screen photo viewer path, if any.
    pub viewer: Option<String>,
    /// Open the first chat once the list arrives (test/demo convenience).
    pub auto_open_first: bool,
}

impl State {
    pub fn new(req_tx: tokio::sync::mpsc::UnboundedSender<Request>) -> Self {
        Self {
            req_tx,
            dialogs: Vec::new(),
            messages: Vec::new(),
            open_chat: None,
            chat_title: String::new(),
            status: String::new(),
            login_error: false,
            authenticated: false,
            typing: false,
            context_menu: None,
            editing: None,
            login_step: LoginStep::Phone,
            login_input: String::new(),
            composer: String::new(),
            viewer: None,
            auto_open_first: false,
        }
    }

    /// Sets the "open first chat once the list is loaded" convenience flag.
    pub fn with_auto_open_first(mut self, on: bool) -> Self {
        self.auto_open_first = on;
        self
    }

    /// Applies an incoming network message.
    pub fn on_message(&mut self, msg: UiMessage) {
        match msg {
            UiMessage::Dialogs(rows) => {
                self.dialogs = rows;
                // A valid session already existed (restart): the chat list is
                // the sign that the account is authenticated.
                self.authenticated = true;
            }
            UiMessage::Messages { id, title, rows } => {
                if self.open_chat == Some(id) {
                    self.chat_title = title;
                    self.messages = rows;
                    self.editing = None;
                    self.context_menu = None;
                }
            }
            UiMessage::NewMessage { chat_id, id, text, date, out, photo } => {
                self.messages.push(MsgRow {
                    id,
                    text,
                    date,
                    out,
                    photo,
                    photo_path: None,
                    read: false,
                });
                if !out && self.open_chat == Some(chat_id) {
                    self.typing = false;
                }
            }
            UiMessage::MessageEdited { chat_id, id, text, .. } => {
                if self.open_chat == Some(chat_id) {
                    for m in &mut self.messages {
                        if m.id == id {
                            m.text = text;
                            break;
                        }
                    }
                }
            }
            UiMessage::MessageDeleted { ids } => {
                self.messages.retain(|m| !ids.contains(&m.id));
                if self.editing.is_some_and(|e| ids.contains(&e)) {
                    self.editing = None;
                }
                self.context_menu = None;
            }
            UiMessage::PhotoReady { chat_id, msg_id, path } => {
                if self.open_chat == Some(chat_id) {
                    for m in &mut self.messages {
                        if m.id == msg_id {
                            m.photo_path = path;
                            break;
                        }
                    }
                }
            }
            UiMessage::ChatRead { id } => {
                if self.open_chat == Some(id) {
                    for m in &mut self.messages {
                        if m.out {
                            m.read = true;
                        }
                    }
                }
            }
            UiMessage::OutboxRead { chat_id, max_id } => {
                if self.open_chat == Some(chat_id) {
                    for m in &mut self.messages {
                        if m.out && m.id <= max_id {
                            m.read = true;
                        }
                    }
                }
            }
            UiMessage::UnreadCount { chat_id, count } => {
                for d in &mut self.dialogs {
                    if d.id == chat_id {
                        d.unread = count;
                    }
                }
            }
            UiMessage::PeerTyping { chat_id, typing } => {
                if self.open_chat == Some(chat_id) {
                    self.typing = typing;
                }
            }
            UiMessage::AvatarReady { chat_id, path } => {
                for d in &mut self.dialogs {
                    if d.id == chat_id {
                        d.avatar_path = path;
                        break;
                    }
                }
            }
            UiMessage::LoginCodeRequired => self.login_step = LoginStep::Code,
            UiMessage::LoginPasswordRequired { hint } => {
                self.login_step = LoginStep::Password;
                if !hint.is_empty() {
                    self.status = format!("2FA password (hint: {hint})");
                }
            }
            UiMessage::LoginOk { name } => {
                self.authenticated = true;
                self.status = format!("Welcome, {name}");
            }
            UiMessage::Error(e) => {
                self.status = e;
                self.login_error = true;
            }
        }
    }

    /// A message item clicked (left). If a context menu is open, the click
    /// hits the menu instead of the message behind it.
    pub fn click(&mut self, row: usize) {
        if self.context_menu.is_some() {
            self.dismiss_menu();
            return;
        }
        if let Some(path) = self.photo_at(row) {
            self.viewer = Some(path);
        }
    }

    /// Right-click over a message row: open the context menu (outgoing only,
    /// matching the original client).
    pub fn open_context(&mut self, row: usize) {
        let Some(m) = self.messages.get(row) else { return };
        if !m.out {
            return;
        }
        self.context_menu = Some(ContextMenu { row });
    }

    pub fn dismiss_menu(&mut self) {
        self.context_menu = None;
    }

    /// Click on the context menu's "Modifier" item.
    pub fn context_edit(&mut self) {
        if let Some(menu) = self.context_menu.take() {
            if let Some(m) = self.messages.get(menu.row) {
                self.editing = Some(m.id);
                self.composer = m.text.clone();
            }
        }
    }

    /// Click on the context menu's "Supprimer" item.
    pub fn context_delete(&mut self) {
        let Some(menu) = self.context_menu.take() else {
            return;
        };
        let Some(m) = self.messages.get(menu.row).cloned() else {
            return;
        };
        let _ = self.req_tx.send(Request::DeleteMessage {
            id: self.open_chat.unwrap_or(0),
            msg_id: m.id,
        });
        self.messages.retain(|x| x.id != m.id);
        if self.editing == Some(m.id) {
            self.editing = None;
        }
    }

    /// Submit the composer: send (or edit) the current text.
    pub fn submit(&mut self) {
        let text = self.composer.trim().to_string();
        if text.is_empty() {
            return;
        }
        if let Some(msg_id) = self.editing.take() {
            let _ = self.req_tx.send(Request::EditMessage {
                id: self.open_chat.unwrap_or(0),
                msg_id,
                text: text.clone(),
            });
            for m in &mut self.messages {
                if m.id == msg_id {
                    m.text = text.clone();
                }
            }
        } else if let Some(id) = self.open_chat {
            let _ = self.req_tx.send(Request::SendMessage { id, text });
        }
        self.composer.clear();
        self.typing = false;
    }

    /// Open a chat.
    pub fn open_chat(&mut self, id: i64) {
        self.open_chat = Some(id);
        self.messages.clear();
        self.chat_title.clear();
        self.editing = None;
        self.context_menu = None;
        self.composer.clear();
        let _ = self.req_tx.send(Request::OpenChat { id });
        let _ = self.req_tx.send(Request::MarkRead { id });
    }

    /// Path of the photo attached to row, if any and already downloaded.
    fn photo_at(&self, row: usize) -> Option<String> {
        let m = self.messages.get(row)?;
        if m.photo.is_some() {
            m.photo_path.clone()
        } else {
            None
        }
    }

    pub fn back(&mut self) {
        self.viewer = None;
        self.context_menu = None;
        self.editing = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_state() -> (State, tokio::sync::mpsc::UnboundedReceiver<Request>) {
        let (req_tx, req_rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
        let mut state = State::new(req_tx);
        // Signed in with an open chat.
        state.authenticated = true;
        state.open_chat = Some(42);
        state.messages = vec![
            MsgRow {
                id: 1,
                text: "incoming".into(),
                date: 100,
                out: false,
                photo: None,
                photo_path: None,
                read: false,
            },
            MsgRow {
                id: 2,
                text: "mine".into(),
                date: 200,
                out: true,
                photo: None,
                photo_path: None,
                read: true,
            },
        ];
        (state, req_rx)
    }

    /// Drains and returns everything the state sent.
    fn drain(req_rx: &mut tokio::sync::mpsc::UnboundedReceiver<Request>) -> Vec<Request> {
        std::iter::from_fn(|| req_rx.try_recv().ok()).collect()
    }

    #[test]
    fn demo_signed_in_state_is_ready() {
        let (state, _) = demo_state();
        assert!(state.authenticated);
        assert_eq!(state.open_chat, Some(42));
        assert_eq!(state.messages.len(), 2);
    }

    #[test]
    fn left_click_outgoing_opens_edit() {
        let (mut state, _) = demo_state();
        state.open_context(1); // row 1 is outgoing ("mine")
        assert!(state.context_menu.is_some());
        state.context_edit();
        assert_eq!(state.editing, Some(2));
        assert_eq!(state.composer, "mine");
        assert!(state.context_menu.is_none());
    }

    #[test]
    fn left_click_incoming_does_not_open_menu() {
        let (mut state, _) = demo_state();
        state.open_context(0); // row 0 is incoming
        assert!(state.context_menu.is_none());
    }

    #[test]
    fn delete_sends_request_and_removes_row() {
        let (mut state, mut req_rx) = demo_state();
        state.open_context(1);
        state.context_delete();
        assert!(state.context_menu.is_none());
        assert!(!state.messages.iter().any(|m| m.id == 2));
        assert_eq!(state.messages.len(), 1);
        let reqs = drain(&mut req_rx);
        assert!(matches!(
            reqs.last(),
            Some(Request::DeleteMessage { id: 42, msg_id: 2 })
        ));
    }

    #[test]
    fn clicking_message_opens_viewer_when_photo_ready() {
        let (mut state, _) = demo_state();
        state.messages[0].photo = Some((640, 480));
        state.messages[0].photo_path = Some("/tmp/pic.jpg".into());
        state.click(0);
        assert_eq!(state.viewer.as_deref(), Some("/tmp/pic.jpg"));
    }

    #[test]
    fn click_on_open_menu_dismisses_instead_of_opening_viewer() {
        let (mut state, _) = demo_state();
        state.messages[0].photo = Some((640, 480));
        state.messages[0].photo_path = Some("/tmp/pic.jpg".into());
        state.open_context(1);
        state.click(0); // menu is open: dismiss, don't open the viewer
        assert!(state.context_menu.is_none());
        assert!(state.viewer.is_none());
    }

    #[test]
    fn edit_flows_back_into_the_row_and_sends_request() {
        let (mut state, mut req_rx) = demo_state();
        state.editing = Some(2);
        state.composer = "edited".into();
        state.submit();
        assert_eq!(state.messages[1].text, "edited");
        assert!(state.editing.is_none());
        let reqs = drain(&mut req_rx);
        assert!(matches!(
            reqs.last(),
            Some(Request::EditMessage { id: 42, msg_id: 2, text }) if text == "edited"
        ));
    }

    #[test]
    fn new_message_appends_and_clears_typing() {
        let (mut state, _) = demo_state();
        state.typing = true;
        state.on_message(UiMessage::NewMessage {
            chat_id: 42,
            id: 3,
            text: "hey".into(),
            date: 300,
            out: false,
            photo: None,
        });
        assert_eq!(state.messages.len(), 3);
        assert!(!state.typing);
    }

    #[test]
    fn dialogs_from_a_valid_session_authenticate_the_app() {
        let (mut state, _) = demo_state();
        state.authenticated = false;
        state.on_message(UiMessage::Dialogs(vec![ChatRow {
            id: 42,
            title: "Chat".into(),
            subtitle: String::new(),
            date: 0,
            unread: 0,
            avatar_path: None,
        }]));
        assert!(state.authenticated, "existing session must skip the login screen");
    }
}
