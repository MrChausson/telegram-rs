//! UI state (screens, clicks, network messages), testable without a window.

use crate::bridge::{Request, UiMessage};
use crate::chatlist::ChatList;
use crate::messages::{MessageList, MsgRow};
use crate::renderer::{INPUTBAR_H, MESSAGES_BOTTOM_GAP, TOPBAR_H};
use crate::text::TextRenderer;

/// Usable height of the messages area (logical units).
fn messages_viewport(h: f32) -> f32 {
    (h - TOPBAR_H - INPUTBAR_H - MESSAGES_BOTTOM_GAP).max(0.0)
}

/// Current screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    List,
    Chat { id: i64, loading: bool },
}

/// Pure UI state, driven by window/network events.
pub struct UiState {
    pub list: ChatList,
    pub messages: MessageList,
    pub screen: Screen,
    pub status: String,
    /// Text typed in the composer (chat view).
    pub input: String,
    /// True if a received message requires scrolling back to the bottom.
    needs_scroll_bottom: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

impl UiState {
    pub fn new() -> Self {
        Self {
            list: ChatList::new(),
            messages: MessageList::new(),
            screen: Screen::List,
            status: "Connecting…".to_string(),
            input: String::new(),
            needs_scroll_bottom: false,
        }
    }

    /// Returns the "scroll to bottom" flag if it was raised.
    pub fn take_scroll_bottom(&mut self) -> bool {
        std::mem::take(&mut self.needs_scroll_bottom)
    }

    pub fn chat_title(&self) -> String {
        match &self.screen {
            Screen::Chat { id, .. } => self
                .list
                .rows
                .iter()
                .find(|r| r.id == *id)
                .map(|r| r.title.clone())
                .unwrap_or_default(),
            Screen::List => String::new(),
        }
    }

    /// Handles a message from the network.
    pub fn on_message(&mut self, msg: UiMessage) {
        match msg {
            UiMessage::Dialogs(rows) => {
                self.list.rows = rows;
                self.status = if self.list.rows.is_empty() {
                    "No chats".to_string()
                } else {
                    format!("{} chats", self.list.rows.len())
                };
            }
            UiMessage::Messages { id, title, rows } => {
                if let Screen::Chat { id: current, .. } = &self.screen {
                    if *current == id {
                        let prev_len = self.messages.rows.len();
                        self.messages.rows = rows;
                        if let Screen::Chat { loading, .. } = &mut self.screen {
                            *loading = false;
                        }
                        let _ = title;
                        self.status.clear();
                        // Scroll to the bottom on open, or when a new message
                        // arrived (not for a plain edit).
                        if prev_len == 0 || self.messages.rows.len() > prev_len {
                            self.needs_scroll_bottom = true;
                        }
                    }
                }
            }
            UiMessage::NewMessage { chat_id, id, text, out } => {
                // Open chat? Merge the message (dedupe the optimistic local
                // send, which is tagged id=0).
                if let Screen::Chat { id: opened, .. } = &self.screen {
                    if *opened == chat_id {
                        let rows = &mut self.messages.rows;
                        if let Some(m) = rows.iter_mut().find(|m| m.id == id) {
                            m.text = text;
                            m.out = out;
                        } else if let Some(i) = rows.iter().position(|m| m.id == 0 && m.text == text) {
                            let m = &mut rows[i];
                            m.id = id;
                            m.out = out;
                        } else {
                            rows.push(MsgRow { id, text, out });
                            self.needs_scroll_bottom = true;
                        }
                        return;
                    }
                }
                // Otherwise: update the list row (preview + unread only for
                // incoming messages).
                if let Some(row) = self.list.rows.iter_mut().find(|r| r.id == chat_id) {
                    row.subtitle = text;
                    if !out {
                        row.unread += 1;
                    }
                }
            }
            UiMessage::MessageEdited { chat_id, id, text } => {
                // Open chat? Update the matching message's text.
                if let Screen::Chat { id: open, .. } = &self.screen {
                    if *open == chat_id {
                        if let Some(msg) = self.messages.rows.iter_mut().find(|m| m.id == id) {
                            msg.text = text;
                        }
                        return;
                    }
                }
                // Otherwise: update the list preview.
                if let Some(row) = self.list.rows.iter_mut().find(|r| r.id == chat_id) {
                    row.subtitle = text;
                }
            }
            UiMessage::MessageDeleted { ids } => {
                // Remove deleted messages from the open chat (the push does
                // not always carry the chat id; filter by message id).
                if matches!(self.screen, Screen::Chat { .. }) {
                    self.messages
                        .rows
                        .retain(|m| !ids.contains(&m.id));
                }
            }
            UiMessage::Error(msg) => self.status = msg,
        }
    }

    /// Mouse click: returns the request to send to the network (or `None`).
    pub fn click(&mut self, x: f32, y: f32, _width: f32) -> Option<Request> {
        match &self.screen {
            Screen::List => {
                let id = self.list.row_at(y)?;
                self.enter_chat(id)
            }
            Screen::Chat { .. } => {
                if (x as f32) < 120.0 && (y as f32) < TOPBAR_H {
                    self.screen = Screen::List;
                }
                None
            }
        }
    }

    /// Opens a chat directly by id (programmed click / test).
    pub fn enter_chat(&mut self, id: i64) -> Option<Request> {
        self.screen = Screen::Chat { id, loading: true };
        self.messages.rows = Vec::new();
        self.status = "Loading…".to_string();
        Some(Request::OpenChat { id })
    }

    /// Mouse wheel.
    pub fn scroll(&mut self, dy: f32, w: f32, h: f32, text: &TextRenderer) {
        match &self.screen {
            Screen::List => self.list.scroll_by(dy, h),
            Screen::Chat { .. } => {
                let viewport = messages_viewport(h);
                let content = self.messages.content_height(text, w);
                self.messages.scroll_by(dy, viewport, content);
            }
        }
    }

    /// Appends typed text (only in the chat view).
    pub fn push_text(&mut self, s: &str) {
        if matches!(self.screen, Screen::Chat { .. }) {
            self.input.push_str(s);
        }
    }

    /// Removes the last typed character.
    pub fn backspace(&mut self) {
        if matches!(self.screen, Screen::Chat { .. }) {
            self.input.pop();
        }
    }

    /// Sends the typed text (Enter): adds the message locally and returns the
    /// network request. Clears the field when something was sent.
    pub fn enter(&mut self) -> Option<Request> {
        let (id, text) = match &self.screen {
            Screen::Chat { id, .. } => (*id, self.input.trim().to_string()),
            Screen::List => return None,
        };
        if text.is_empty() {
            return None;
        }
        self.input.clear();
        // Optimistic local send: the id is unknown (incoming updates will
        // provide it).
        self.messages
            .rows
            .push(MsgRow { id: 0, text: text.clone(), out: true });
        Some(Request::SendMessage { id, text })
    }

    /// Forces the messages to scroll to the bottom (after a send).
    pub fn scroll_messages_to_bottom(&mut self, text: &TextRenderer, w: f32, h: f32) {
        if matches!(self.screen, Screen::Chat { .. }) {
            let viewport = messages_viewport(h);
            let content = self.messages.content_height(text, w);
            self.messages.set_scroll_bottom(content, viewport);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chatlist::ChatRow;
    use crate::messages::MsgRow;

    fn rows() -> Vec<ChatRow> {
        vec![
            ChatRow {
                id: 1,
                title: "Alpha".into(),
                subtitle: "hi".into(),
                unread: 0,
            },
            ChatRow {
                id: 2,
                title: "Beta".into(),
                subtitle: "hey".into(),
                unread: 2,
            },
            ChatRow {
                id: 3,
                title: "Gamma".into(),
                subtitle: "hello".into(),
                unread: 0,
            },
        ]
    }

    #[test]
    fn clicking_a_chat_opens_it_and_sends_the_request() {
        let mut state = UiState::new();
        state.on_message(UiMessage::Dialogs(rows()));

        // Row 1 (index 1): y between 64 and 128.
        let req = state.click(50.0, 70.0, 400.0);

        assert!(matches!(
            req,
            Some(Request::OpenChat { id: 2 })
        ));
        assert!(matches!(
            state.screen,
            Screen::Chat { id: 2, loading: true }
        ));
        assert_eq!(state.status, "Loading…");
    }

    #[test]
    fn clicking_the_back_bar_returns_to_the_list() {
        let mut state = UiState::new();
        state.on_message(UiMessage::Dialogs(rows()));
        state.click(50.0, 70.0, 400.0);

        state.click(10.0, 10.0, 400.0);
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn clicking_outside_rows_does_nothing() {
        let mut state = UiState::new();
        state.on_message(UiMessage::Dialogs(rows()));

        let req = state.click(50.0, 1000.0, 400.0);
        assert!(req.is_none());
        assert_eq!(state.screen, Screen::List);
    }

    #[test]
    fn received_messages_fill_the_open_chat() {
        let mut state = UiState::new();
        state.on_message(UiMessage::Dialogs(rows()));
        state.click(50.0, 70.0, 400.0);

        state.on_message(UiMessage::Messages {
            id: 2,
            title: "Beta".into(),
            rows: vec![MsgRow {
                id: 99,
                text: "first".into(),
                out: false,
            }],
        });

        assert_eq!(state.messages.rows.len(), 1);
        assert!(matches!(
            state.screen,
            Screen::Chat { id: 2, loading: false }
        ));
    }

    #[test]
    fn messages_from_another_chat_are_ignored() {
        let mut state = UiState::new();
        state.on_message(UiMessage::Dialogs(rows()));
        state.click(50.0, 70.0, 400.0); // opens id 2

        state.on_message(UiMessage::Messages {
            id: 3,
            title: "Gamma".into(),
            rows: vec![MsgRow {
                id: 101,
                text: "other".into(),
                out: true,
            }],
        });

        assert!(state.messages.rows.is_empty());
    }

    #[test]
    fn edited_message_updates_the_text_in_the_open_chat() {
        let mut state = UiState::new();
        open_chat(&mut state); // loads "already here" (id 100)
        state.take_scroll_bottom();

        state.on_message(UiMessage::MessageEdited {
            chat_id: 2,
            id: 100,
            text: "already here (edited)".into(),
        });

        assert_eq!(state.messages.rows[0].text, "already here (edited)");
    }

    #[test]
    fn phone_sent_message_is_displayed() {
        // A message sent from the phone arrives with out=true: it must be
        // displayed in the open chat.
        let mut state = UiState::new();
        open_chat(&mut state);
        state.take_scroll_bottom();
        let before = state.messages.rows.len();

        state.on_message(UiMessage::NewMessage {
            chat_id: 2,
            id: 300,
            text: "sent from phone".into(),
            out: true,
        });

        assert_eq!(state.messages.rows.len(), before + 1);
        assert!(state.messages.rows.last().unwrap().out);
        assert!(state.take_scroll_bottom());
    }

    #[test]
    fn optimistic_send_is_deduplicated_by_the_update() {
        // enter() adds an optimistic message (id=0); the pushed update for the
        // same message (same text) must merge instead of adding a duplicate.
        let mut state = UiState::new();
        open_chat(&mut state);
        state.push_text("hello");
        let req = state.enter().unwrap();
        assert!(matches!(req, Request::SendMessage { .. }));
        let count = state.messages.rows.len();

        state.on_message(UiMessage::NewMessage {
            chat_id: 2,
            id: 777,
            text: "hello".into(),
            out: true,
        });

        assert_eq!(state.messages.rows.len(), count, "no duplicate");
        let last = state.messages.rows.last().unwrap();
        assert_eq!(last.id, 777);
    }

    #[test]
    fn deleted_message_is_removed_from_the_chat() {
        let mut state = UiState::new();
        open_chat(&mut state); // "already here" (id 100)
        state.on_message(UiMessage::MessageDeleted { ids: vec![100] });
        assert!(state.messages.rows.is_empty());
    }

    #[test]
    fn error_is_remembered_as_status() {
        let mut state = UiState::new();
        state.on_message(UiMessage::Error("boom".into()));
        assert_eq!(state.status, "boom");
    }

    fn open_chat(state: &mut UiState) {
        state.on_message(UiMessage::Dialogs(rows()));
        state.click(50.0, 70.0, 400.0); // opens id 2
        state.on_message(UiMessage::Messages {
            id: 2,
            title: "Beta".into(),
            rows: vec![MsgRow {
                id: 100,
                text: "already here".into(),
                out: false,
            }],
        });
    }

    #[test]
    fn typing_and_backspace() {
        let mut state = UiState::new();
        open_chat(&mut state);

        state.push_text("salut ");
        state.push_text("toi");
        assert_eq!(state.input, "salut toi");

        state.backspace();
        assert_eq!(state.input, "salut to");
    }

    #[test]
    fn typing_is_ignored_outside_the_chat() {
        let mut state = UiState::new();
        state.on_message(UiMessage::Dialogs(rows()));
        state.push_text("abc");
        assert_eq!(state.input, "");
    }

    #[test]
    fn enter_sends_and_clears_the_field() {
        let mut state = UiState::new();
        open_chat(&mut state);
        state.push_text("coucou");

        let req = state.enter();

        assert!(matches!(
            req,
            Some(Request::SendMessage { id: 2, ref text }) if text == "coucou"
        ));
        assert_eq!(state.input, "");
        // The message was added locally (optimistic).
        assert_eq!(state.messages.rows.len(), 2);
        assert!(state.messages.rows.last().unwrap().out);
    }

    #[test]
    fn empty_enter_does_nothing() {
        let mut state = UiState::new();
        open_chat(&mut state);

        assert!(state.enter().is_none());
        assert_eq!(state.messages.rows.len(), 1);
    }

    #[test]
    fn new_message_in_open_chat_is_appended() {
        let mut state = UiState::new();
        open_chat(&mut state); // opens id 2

        state.on_message(UiMessage::NewMessage {
            chat_id: 2,
            id: 55,
            text: "coucou en direct".into(),
            out: false,
        });

        assert_eq!(state.messages.rows.len(), 2);
        assert!(state.messages.rows.last().unwrap().out == false);
        assert!(state.take_scroll_bottom());
    }

    #[test]
    fn new_message_in_another_chat_updates_the_list() {
        let mut state = UiState::new();
        open_chat(&mut state); // opens id 2, chat 1 is not open
        state.take_scroll_bottom(); // consume the open flag

        state.on_message(UiMessage::NewMessage {
            chat_id: 1,
            id: 56,
            text: "for Alpha".into(),
            out: false,
        });

        let alpha = state.list.rows.iter().find(|r| r.id == 1).unwrap();
        assert_eq!(alpha.subtitle, "for Alpha");
        assert_eq!(alpha.unread, 1);
        // The open chat is not modified.
        assert_eq!(state.messages.rows.len(), 1);
        assert!(!state.take_scroll_bottom());
    }

    #[test]
    fn new_message_updates_the_list_in_list_view() {
        let mut state = UiState::new();
        state.on_message(UiMessage::Dialogs(rows()));
        state.on_message(UiMessage::NewMessage {
            chat_id: 3,
            id: 57,
            text: "Gamma parle".into(),
            out: false,
        });
        let gamma = state.list.rows.iter().find(|r| r.id == 3).unwrap();
        assert_eq!(gamma.unread, 1);
    }
}