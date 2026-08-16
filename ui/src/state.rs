//! UI state (screens, clicks, network messages), testable without a window.

use crate::bridge::{Request, UiMessage};
use crate::chatlist::ChatList;
use crate::messages::{MessageList, MsgRow};
use std::collections::HashMap;

use crate::theme::{self, layout};
use crate::text::TextRenderer;

/// Usable height of the messages area (logical units).
fn messages_viewport(h: f32) -> f32 {
    (h - layout::CHAT_HEADER_H - layout::INPUT_H - layout::MESSAGES_BOTTOM_GAP).max(0.0)
}

/// Screen states / overlay for the photo viewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// No conversation selected: the right pane shows a placeholder.
    Idle,
    /// A conversation is open.
    Chat { id: i64, loading: bool },
}

/// Step of the in-app sign-in flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginStep {
    /// Ask for the phone number.
    Phone,
    /// The code has been sent: ask for it (possibly with retry count).
    Code,
    /// The account is protected by a 2FA password.
    Password { hint: String },
}

impl Default for LoginStep {
    fn default() -> Self {
        LoginStep::Phone
    }
}

/// Pure UI state, driven by window/network events.
pub struct UiState {
    pub list: ChatList,
    pub messages: MessageList,
    pub screen: Screen,
    pub status: String,
    /// True once the account is signed in (chat UI active).
    pub authenticated: bool,
    /// Current step of the sign-in flow (only used while unauthenticated).
    pub login_step: LoginStep,
    /// Text typed in the sign-in field (phone / code / password).
    pub login_input: String,
    /// True when the last login status is an error (rendered in red).
    pub login_error: bool,
    /// Text typed in the composer (chat view).
    pub input: String,
    /// On-disk path of a photo shown full-screen (overlay), if any.
    pub viewer: Option<String>,
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
            screen: Screen::Idle,
            status: "Connecting…".to_string(),
            authenticated: false,
            login_step: LoginStep::Phone,
            login_input: String::new(),
            login_error: false,
            input: String::new(),
            viewer: None,
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
            Screen::Idle => String::new(),
        }
    }

    /// Handles a message from the network.
    pub fn on_message(&mut self, msg: UiMessage) {
        match msg {
            UiMessage::Dialogs(rows) => {
                self.list.rows = rows;
                // A valid session already existed (restart): the chat list is
                // the sign that the account is authenticated.
                self.authenticated = true;
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
                        // Keep already-downloaded photo thumbnails across
                        // refreshes (the network list does not carry them).
                        let paths: HashMap<i32, String> = self
                            .messages
                            .rows
                            .iter()
                            .filter_map(|r| r.photo_path.clone().map(|p| (r.id, p)))
                            .collect();
                        self.messages.rows = rows
                            .into_iter()
                            .map(|mut r| {
                                if r.photo_path.is_none() {
                                    if let Some(p) = paths.get(&r.id) {
                                        r.photo_path = Some(p.clone());
                                    }
                                }
                                r
                            })
                            .collect();
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
            UiMessage::NewMessage { chat_id, id, text, date, out, photo } => {
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
                            rows.push(MsgRow {
                                id,
                                text,
                                date,
                                out,
                                photo,
                                photo_path: None,
                            });
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
            UiMessage::MessageEdited { chat_id, id, text, date } => {
                // Open chat? Update the matching message's text.
                if let Screen::Chat { id: open, .. } = &self.screen {
                    if *open == chat_id {
                        if let Some(msg) = self.messages.rows.iter_mut().find(|m| m.id == id) {
                            msg.text = text;
                            msg.date = date;
                        }
                        return;
                    }
                }
                // Otherwise: update the list preview.
                if let Some(row) = self.list.rows.iter_mut().find(|r| r.id == chat_id) {
                    row.subtitle = text;
                }
            }
            UiMessage::AvatarReady { chat_id, path } => {
                if let Some(row) = self.list.rows.iter_mut().find(|r| r.id == chat_id) {
                    row.avatar_path = path;
                }
            }
            UiMessage::PhotoReady { chat_id, msg_id, path } => {
                if let Screen::Chat { id: open, .. } = &self.screen {
                    if *open == chat_id {
                        if let Some(m) = self.messages.rows.iter_mut().find(|m| m.id == msg_id) {
                            m.photo_path = path;
                        }
                    }
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
            UiMessage::LoginCodeRequired => {
                self.login_step = LoginStep::Code;
                self.login_input.clear();
                self.login_error = false;
                self.status = "Enter the code received by Telegram".to_string();
            }
            UiMessage::LoginPasswordRequired { hint } => {
                self.login_step = LoginStep::Password { hint: hint.clone() };
                self.login_input.clear();
                self.login_error = false;
                self.status = if hint.is_empty() {
                    "Enter your two-step verification password".to_string()
                } else {
                    format!("Two-step verification (hint: {hint})")
                };
            }
            UiMessage::LoginOk { name } => {
                self.authenticated = true;
                self.login_input.clear();
                self.login_error = false;
                self.status = if self.list.rows.is_empty() {
                    format!("Signed in as {name}")
                } else {
                    format!("Signed in as {name} — {} chats", self.list.rows.len())
                };
            }
            UiMessage::Error(msg) => {
                if !self.authenticated {
                    self.login_error = true;
                }
                self.status = msg
            }
        }
    }

    /// Mouse click (logical coordinates): routes to the left list pane or the
    /// right conversation pane, returning the request to send (or `None`).
    pub fn click(&mut self, x: f32, y: f32, width: f32, height: f32) -> Option<Request> {
        if !self.authenticated {
            return self.on_login(x, y, width, height);
        }
        if x < theme::layout::LIST_W {
            let row_y = (y - theme::layout::LIST_HEADER_H).max(0.0);
            let id = self.list.row_at(row_y)?;
            return self.enter_chat(id);
        }
        // Right pane: the back arrow (top-left of the chat header) clears
        // the selection.
        if let Screen::Chat { .. } = &self.screen {
            if (x - theme::layout::LIST_W) < 96.0 && (y as f32) < theme::layout::CHAT_HEADER_H {
                self.screen = Screen::Idle;
            }
        }
        None
    }

    /// Clicks on the sign-in screen: primary button resumes the flow, the
    /// back arrow reopens the phone step.
    fn on_login(&mut self, x: f32, y: f32, width: f32, height: f32) -> Option<Request> {
        let layout = theme::login_layout(width, height);
        if theme::LoginLayout::contains(layout.button, x, y) {
            return self.login_continue();
        }
        if self.login_step != LoginStep::Phone
            && theme::LoginLayout::contains(layout.back, x, y)
        {
            self.login_back();
        }
        None
    }

    /// Submits the current login field (phone / code / password).
    fn login_continue(&mut self) -> Option<Request> {
        match self.login_step {
            LoginStep::Phone => {
                let phone = self.login_input.trim().to_string();
                if phone.is_empty() {
                    return None;
                }
                self.login_input = phone.clone();
                self.status = "Sending code…".to_string();
                Some(Request::LoginPhone { phone })
            }
            LoginStep::Code => {
                let code = self.login_input.trim().to_string();
                if code.is_empty() {
                    return None;
                }
                self.login_input.clear();
                self.status = "Signing in…".to_string();
                Some(Request::LoginCode { code })
            }
            LoginStep::Password { .. } => {
                let password = self.login_input.clone();
                if password.is_empty() {
                    return None;
                }
                self.login_input.clear();
                self.status = "Signing in…".to_string();
                Some(Request::LoginPassword { password })
            }
        }
    }

    /// Goes back one login step (2FA → code → phone).
    pub fn login_back(&mut self) {
        self.login_step = match &self.login_step {
            LoginStep::Password { hint: _ } => LoginStep::Code,
            _ => LoginStep::Phone,
        };
        self.login_input.clear();
        self.status.clear();
    }

    /// Opens a chat directly by id (programmed click / test).
    pub fn enter_chat(&mut self, id: i64) -> Option<Request> {
        self.screen = Screen::Chat { id, loading: true };
        self.messages.rows = Vec::new();
        self.status = "Loading…".to_string();
        Some(Request::OpenChat { id })
    }

    /// Insights whether a click/touch position is inside the left pane.
    pub fn is_left_pane(&self, x: f32) -> bool {
        x < theme::layout::LIST_W
    }

    /// Path of the photo under a click in the conversation pane (logical
    /// coordinates), if any.
    pub fn photo_at(&self, _x: f32, y: f32) -> Option<String> {
        if !matches!(self.screen, Screen::Chat { .. }) {
            return None;
        }
        let tr = crate::text::TextRenderer::new();
        let rows = &self.messages.rows;
        let mut top = -self.messages.scroll;
        for row in rows {
            let row_h = self.messages.row_height(&tr, row, 700.0);
            if y >= top && y < top + row_h {
                if row.photo.is_some() {
                    return row.photo_path.clone();
                }
                return None;
            }
            top += row_h;
        }
        None
    }

    /// Opens the photo viewer overlay for `path`.
    pub fn open_viewer(&mut self, path: String) {
        self.viewer = Some(path);
    }

    /// Closes the photo viewer overlay, if open.
    pub fn close_viewer(&mut self) {
        self.viewer = None;
    }

    /// Mouse wheel: the list scrolls when the cursor is over the left pane,
    /// the conversation otherwise (logical coordinates).
    pub fn scroll(&mut self, dy: f32, x: f32, w: f32, h: f32, text: &TextRenderer) {
        if x < theme::layout::LIST_W {
            self.list.scroll_by(dy, h);
            return;
        }
        if let Screen::Chat { .. } = &self.screen {
            let viewport = messages_viewport(h);
            let content = self.messages.content_height(text, w);
            self.messages.scroll_by(dy, viewport, content);
        }
    }

    /// Appends typed text (login field while unauthenticated, composer in
    /// the chat view).
    pub fn push_text(&mut self, s: &str) {
        if !self.authenticated {
            self.login_input.push_str(s);
        } else if matches!(self.screen, Screen::Chat { .. }) {
            self.input.push_str(s);
        }
    }

    /// Removes the last typed character.
    pub fn backspace(&mut self) {
        if !self.authenticated {
            self.login_input.pop();
        } else if matches!(self.screen, Screen::Chat { .. }) {
            self.input.pop();
        }
    }

    /// Sends the typed text (Enter): adds the message locally and returns the
    /// network request. Clears the field when something was sent. While
    /// unauthenticated, continues the sign-in flow instead.
    pub fn enter(&mut self) -> Option<Request> {
        if !self.authenticated {
            return self.login_continue();
        }
        let (id, text) = match &self.screen {
            Screen::Chat { id, .. } => (*id, self.input.trim().to_string()),
            Screen::Idle => return None,
        };
        if text.is_empty() {
            return None;
        }
        self.input.clear();
        // Optimistic local send: the id and date are unknown (incoming
        // updates will provide them).
        self.messages
            .rows
            .push(MsgRow {
                id: 0,
                text: text.clone(),
                date: 0,
                out: true,
                photo: None,
                photo_path: None,
            });
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
                date: 0,
                unread: 0,
                avatar_path: None,
            },
            ChatRow {
                id: 2,
                title: "Beta".into(),
                subtitle: "hey".into(),
                date: 0,
                unread: 2,
                avatar_path: None,
            },
            ChatRow {
                id: 3,
                title: "Gamma".into(),
                subtitle: "hello".into(),
                date: 0,
                unread: 0,
                avatar_path: None,
            },
        ]
    }

    /// A state past the sign-in screen (chat view active).
    fn ready_state() -> UiState {
        let mut s = UiState::new();
        s.authenticated = true;
        s
    }

    #[test]
    fn login_types_go_to_the_login_field() {
        let mut state = UiState::new();
        state.push_text("+336");
        assert_eq!(state.login_input, "+336");
        state.backspace();
        assert_eq!(state.login_input, "+33");
        assert_eq!(state.input, "");
    }

    #[test]
    fn login_phone_enter_sends_login_request() {
        let mut state = UiState::new();
        state.push_text("+33612345678");
        let req = state.enter().unwrap();
        assert!(matches!(req, Request::LoginPhone { ref phone } if phone == "+33612345678"));
        assert_eq!(state.status, "Sending code…");
        assert!(!state.authenticated);
    }

    #[test]
    fn empty_login_is_ignored() {
        let mut state = UiState::new();
        assert!(state.enter().is_none());
    }

    #[test]
    fn code_required_moves_to_code_step() {
        let mut state = UiState::new();
        state.push_text("+33612345678");
        state.enter();
        state.on_message(UiMessage::LoginCodeRequired);
        assert_eq!(state.login_step, LoginStep::Code);

        state.push_text("12345");
        let req = state.enter().unwrap();
        assert!(matches!(req, Request::LoginCode { .. }));
        let Request::LoginCode { code } = req else {
            unreachable!()
        };
        assert_eq!(code, "12345");
        assert_eq!(state.login_input, "");
    }

    #[test]
    fn password_required_moves_to_password_step() {
        let mut state = UiState::new();
        state.on_message(UiMessage::LoginPasswordRequired {
            hint: "pet".into(),
        });
        assert_eq!(
            state.login_step,
            LoginStep::Password { hint: "pet".into() }
        );
        assert_eq!(state.status, "Two-step verification (hint: pet)");
        state.push_text("secret");
        let req = state.enter().unwrap();
        assert!(matches!(req, Request::LoginPassword { .. }));
        assert!(!state.authenticated);
    }

    #[test]
    fn login_ok_signs_in_and_clears_input() {
        let mut state = UiState::new();
        state.push_text("+33612345678");
        state.enter();
        state.on_message(UiMessage::LoginOk {
            name: "Hanni".into(),
        });
        assert!(state.authenticated);
        assert_eq!(state.login_input, "");
        assert_eq!(state.status, "Signed in as Hanni");
    }

    #[test]
    fn login_error_keeps_the_step_for_retry() {
        let mut state = UiState::new();
        state.on_message(UiMessage::LoginCodeRequired);
        state.on_message(UiMessage::Error("Invalid code".into()));
        assert_eq!(state.login_step, LoginStep::Code);
        assert_eq!(state.status, "Invalid code");
    }

    #[test]
    fn login_back_button_returns_to_phone() {
        let mut state = UiState::new();
        state.on_message(UiMessage::LoginCodeRequired);
        state.login_back();
        assert_eq!(state.login_step, LoginStep::Phone);
        state.on_message(UiMessage::LoginPasswordRequired { hint: "".into() });
        state.login_back();
        assert_eq!(state.login_step, LoginStep::Code);
    }

    #[test]
    fn login_click_on_button_continues() {
        let mut state = UiState::new();
        state.push_text("+33");
        let layout = crate::theme::login_layout(980.0, 720.0);
        let (bx, by, bw, bh) = layout.button;
        let req = state.click(bx + bw / 2.0, by + bh / 2.0, 980.0, 720.0);
        assert!(matches!(req, Some(Request::LoginPhone { .. })));
    }

    #[test]
    fn clicking_a_chat_opens_it_and_sends_the_request() {
        let mut state = ready_state();
        state.on_message(UiMessage::Dialogs(rows()));

        // Row 1 (index 1): y between 108 (header) and 172.
        let req = state.click(50.0, 150.0, 400.0, 720.0);

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
        let mut state = ready_state();
        state.on_message(UiMessage::Dialogs(rows()));
        state.click(50.0, 150.0, 400.0, 720.0);

        state.click(310.0, 10.0, 400.0, 720.0);
        assert_eq!(state.screen, Screen::Idle);
    }

    #[test]
    fn clicking_outside_rows_does_nothing() {
        let mut state = ready_state();
        state.on_message(UiMessage::Dialogs(rows()));

        let req = state.click(150.0, 1000.0, 400.0, 720.0);
        assert!(req.is_none());
        assert_eq!(state.screen, Screen::Idle);
    }

    #[test]
    fn received_messages_fill_the_open_chat() {
        let mut state = ready_state();
        state.on_message(UiMessage::Dialogs(rows()));
        state.click(50.0, 150.0, 400.0, 720.0);

        state.on_message(UiMessage::Messages {
            id: 2,
            title: "Beta".into(),
            rows: vec![MsgRow {
                id: 99,
                text: "first".into(),
            date: 0,
                out: false,
            photo: None,
            photo_path: None,
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
        let mut state = ready_state();
        state.on_message(UiMessage::Dialogs(rows()));
        state.click(50.0, 150.0, 400.0, 720.0); // opens id 2

        state.on_message(UiMessage::Messages {
            id: 3,
            title: "Gamma".into(),
            rows: vec![MsgRow {
                id: 101,
                text: "other".into(),
            date: 0,
                out: true,
            photo: None,
            photo_path: None,
            }],
        });

        assert!(state.messages.rows.is_empty());
    }

    #[test]
    fn edited_message_updates_the_text_in_the_open_chat() {
        let mut state = ready_state();
        open_chat(&mut state); // loads "already here" (id 100)
        state.take_scroll_bottom();

        state.on_message(UiMessage::MessageEdited {
            chat_id: 2,
            id: 100,
            text: "already here (edited)".into(),
            date: 0,
        });

        assert_eq!(state.messages.rows[0].text, "already here (edited)");
    }

    #[test]
    fn phone_sent_message_is_displayed() {
        // A message sent from the phone arrives with out=true: it must be
        // displayed in the open chat.
        let mut state = ready_state();
        open_chat(&mut state);
        state.take_scroll_bottom();
        let before = state.messages.rows.len();

        state.on_message(UiMessage::NewMessage {
            chat_id: 2,
            id: 300,
            text: "sent from phone".into(),
            date: 0,
            out: true,
            photo: None,
        });

        assert_eq!(state.messages.rows.len(), before + 1);
        assert!(state.messages.rows.last().unwrap().out);
        assert!(state.take_scroll_bottom());
    }

    #[test]
    fn optimistic_send_is_deduplicated_by_the_update() {
        // enter() adds an optimistic message (id=0); the pushed update for the
        // same message (same text) must merge instead of adding a duplicate.
        let mut state = ready_state();
        open_chat(&mut state);
        state.push_text("hello");
        let req = state.enter().unwrap();
        assert!(matches!(req, Request::SendMessage { .. }));
        let count = state.messages.rows.len();

        state.on_message(UiMessage::NewMessage {
            chat_id: 2,
            id: 777,
            text: "hello".into(),
            date: 0,
            out: true,
            photo: None,
        });

        assert_eq!(state.messages.rows.len(), count, "no duplicate");
        let last = state.messages.rows.last().unwrap();
        assert_eq!(last.id, 777);
    }

    #[test]
    fn deleted_message_is_removed_from_the_chat() {
        let mut state = ready_state();
        open_chat(&mut state); // "already here" (id 100)
        state.on_message(UiMessage::MessageDeleted { ids: vec![100] });
        assert!(state.messages.rows.is_empty());
    }

    #[test]
    fn error_is_remembered_as_status() {
        let mut state = ready_state();
        state.on_message(UiMessage::Error("boom".into()));
        assert_eq!(state.status, "boom");
    }

    fn open_chat(state: &mut UiState) {
        state.on_message(UiMessage::Dialogs(rows()));
        state.click(50.0, 150.0, 400.0, 720.0); // opens id 2
        state.on_message(UiMessage::Messages {
            id: 2,
            title: "Beta".into(),
            rows: vec![MsgRow {
                id: 100,
                text: "already here".into(),
            date: 0,
                out: false,
            photo: None,
            photo_path: None,
            }],
        });
    }

    #[test]
    fn typing_and_backspace() {
        let mut state = ready_state();
        open_chat(&mut state);

        state.push_text("salut ");
        state.push_text("toi");
        assert_eq!(state.input, "salut toi");

        state.backspace();
        assert_eq!(state.input, "salut to");
    }

    #[test]
    fn typing_is_ignored_outside_the_chat() {
        let mut state = ready_state();
        state.on_message(UiMessage::Dialogs(rows()));
        state.push_text("abc");
        assert_eq!(state.input, "");
    }

    #[test]
    fn enter_sends_and_clears_the_field() {
        let mut state = ready_state();
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
        let mut state = ready_state();
        open_chat(&mut state);

        assert!(state.enter().is_none());
        assert_eq!(state.messages.rows.len(), 1);
    }

    #[test]
    fn new_message_in_open_chat_is_appended() {
        let mut state = ready_state();
        open_chat(&mut state); // opens id 2

        state.on_message(UiMessage::NewMessage {
            chat_id: 2,
            id: 55,
            text: "coucou en direct".into(),
            date: 0,
            out: false,
            photo: None,
        });

        assert_eq!(state.messages.rows.len(), 2);
        assert!(state.messages.rows.last().unwrap().out == false);
        assert!(state.take_scroll_bottom());
    }

    #[test]
    fn new_message_in_another_chat_updates_the_list() {
        let mut state = ready_state();
        open_chat(&mut state); // opens id 2, chat 1 is not open
        state.take_scroll_bottom(); // consume the open flag

        state.on_message(UiMessage::NewMessage {
            chat_id: 1,
            id: 56,
            text: "for Alpha".into(),
            date: 0,
            out: false,
            photo: None,
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
        let mut state = ready_state();
        state.on_message(UiMessage::Dialogs(rows()));
        state.on_message(UiMessage::NewMessage {
            chat_id: 3,
            id: 57,
            text: "Gamma parle".into(),
            date: 0,
            out: false,
            photo: None,
        });
        let gamma = state.list.rows.iter().find(|r| r.id == 3).unwrap();
        assert_eq!(gamma.unread, 1);
    }
}