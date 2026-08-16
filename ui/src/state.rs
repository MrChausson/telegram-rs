//! UI state (screens, clicks, network messages), testable without a window.

use crate::bridge::{Request, UiMessage};
use crate::chatlist::ChatList;
use crate::messages::{Anchor, MessageList, MsgRow, Selection};
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

/// A message the user right-clicked (context menu affordance) — its row
/// index into the open chat's rows and its logical (pane-relative) position.
#[derive(Debug, Clone, Copy)]
pub struct ContextMenu {
    pub row: usize,
    pub y: f32,
}

/// Pure UI state, driven by window/network events.
pub struct UiState {
    pub list: ChatList,
    pub messages: MessageList,
    pub screen: Screen,
    pub status: String,
    /// True once the account is signed in (chat UI active).
    pub authenticated: bool,
    /// Chat id currently showing a "typing…" status (the open chat's peer
    /// is typing), if any.
    pub typing: Option<i64>,
    /// Right-click context menu over the open chat's messages, if open.
    pub context_menu: Option<ContextMenu>,
    /// Message id being edited (its old text is in `input`); `None` when
    /// typing fresh text.
    pub editing: Option<i32>,
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
    /// Text selection in the messages pane, if any.
    selection: Option<Selection>,
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
            typing: None,
            context_menu: None,
            editing: None,
            login_step: LoginStep::Phone,
            login_input: String::new(),
            login_error: false,
            input: String::new(),
            viewer: None,
            needs_scroll_bottom: false,
            selection: None,
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
                                read: false,
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
            UiMessage::ChatRead { id } => {
                if let Some(row) = self.list.rows.iter_mut().find(|r| r.id == id) {
                    row.unread = 0;
                }
            }
            UiMessage::UnreadCount { chat_id, count } => {
                // Only sync the count when it goes down or another device
                // read the chat; a locally-received message already bumped it.
                if let Some(row) = self.list.rows.iter_mut().find(|r| r.id == chat_id) {
                    if count < row.unread {
                        row.unread = count;
                    }
                }
            }
            UiMessage::PeerTyping { chat_id, typing } => {
                // Track the open chat's peer typing status (header shows it).
                if typing {
                    self.typing = Some(chat_id);
                } else if self.typing == Some(chat_id) {
                    self.typing = None;
                }
            }
            UiMessage::OutboxRead { chat_id, max_id } => {
                // Only the open chat's bubbles show ticks; ignore others.
                if let Screen::Chat { id: open, .. } = &self.screen {
                    if *open == chat_id {
                        for m in self.messages.rows.iter_mut() {
                            if m.out && m.id <= max_id {
                                m.read = true;
                            }
                        }
                    }
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
                self.context_menu = None;
                self.editing = None;
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

    /// Closes the context menu and cancels any pending message edit.
    pub fn clear_context(&mut self) {
        self.context_menu = None;
        self.editing = None;
    }

    /// Opens a chat directly by id (programmed click / test).
    pub fn enter_chat(&mut self, id: i64) -> Option<Request> {
        self.screen = Screen::Chat { id, loading: true };
        self.messages.rows = Vec::new();
        self.status = "Loading…".to_string();
        self.clear_context();
        Some(Request::OpenChat { id })
    }

    /// Insights whether a click/touch position is inside the left pane.
    pub fn is_left_pane(&self, x: f32) -> bool {
        x < theme::layout::LIST_W
    }

    /// Path of the photo under a click in the conversation pane (logical
    /// window coordinates), if any.
    pub fn photo_at(&self, x: f32, y: f32, width: f32) -> Option<String> {
        if !matches!(self.screen, Screen::Chat { .. }) || x < theme::layout::LIST_W {
            return None;
        }
        let tr = crate::text::TextRenderer::new();
        let rows = &self.messages.rows;
        let pane_w = (width - theme::layout::LIST_W).max(0.0);
        let py = y - theme::layout::CHAT_HEADER_H;
        let mut top = -self.messages.scroll;
        for row in rows {
            let row_h = self.messages.row_height(&tr, row, pane_w);
            if py >= top && py < top + row_h {
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

    /// Opens the right-click context menu for a message in the open chat.
    /// Only outgoing messages offer edit/delete; incoming messages close any
    /// open menu. `x`/`y` are logical window coordinates; returns `true`
    /// when a menu is now shown.
    pub fn open_context(&mut self, x: f32, y: f32, width: f32, text: &TextRenderer) -> bool {
        if !matches!(self.screen, Screen::Chat { .. }) || x < theme::layout::LIST_W {
            self.context_menu = None;
            return false;
        }
        let pane_w = (width - theme::layout::LIST_W).max(0.0);
        let py = y - theme::layout::CHAT_HEADER_H;
        let Some(row) = self.messages.row_at(text, py, pane_w) else {
            self.context_menu = None;
            return false;
        };
        let out = self.messages.rows.get(row).map(|m| m.out).unwrap_or(false);
        if !out {
            self.context_menu = None;
            return false;
        }
        self.context_menu = Some(ContextMenu { row, y: py });
        self.clear_selection();
        true
    }

    /// Logical size of the context menu, as drawn. Only used by the hit-test
    /// and the renderer (which also scale by `ui_scale`).
    pub fn context_menu_size(&self) -> (f32, f32, f32, f32) {
        let w = theme::layout::CONTEXT_W;
        let h = theme::layout::CONTEXT_ITEM_H;
        let (x, y) = match &self.context_menu {
            Some(m) => (
                10.0,
                (m.y - h - 6.0).max(4.0),
            ),
            None => (0.0, 0.0),
        };
        (x, y, w, h * 2.0)
    }

    /// Handles a click in the open-chat pane: routes to the context menu
    /// when one is open (start editing / delete / dismiss), otherwise
    /// nothing. Returns a network request to send, if any.
    pub fn context_click(&mut self, x: f32, y: f32) -> (bool, Option<Request>) {
        let Some(_) = self.context_menu else {
            return (false, None);
        };
        let (mx, my, mw, mh) = self.context_menu_size();
        let inside = x >= theme::layout::LIST_W + mx
            && x <= theme::layout::LIST_W + mx + mw
            && y >= my
            && y <= my + mh;
        if !inside {
            self.context_menu = None;
            return (true, None);
        }
        let item_h = theme::layout::CONTEXT_ITEM_H;
        let edit_hit = y < my + item_h;
        let chat_id = match &self.screen {
            Screen::Chat { id, .. } => *id,
            Screen::Idle => 0,
        };
        let msg_id = match &self.context_menu {
            Some(m) => self.messages.rows.get(m.row).map(|r| r.id).unwrap_or(0),
            None => 0,
        };
        let req = if edit_hit {
            // Edit: prefill the composer with the message's current text.
            if let Some(text) = self.messages.rows.iter().find(|r| r.id == msg_id) {
                self.editing = Some(msg_id);
                self.input = text.text.clone();
            }
            None
        } else {
            // Delete: remove the row locally right away (the server echoes
            // a MessageDeleted update that is now a no-op).
            self.messages.rows.retain(|r| r.id != msg_id);
            Some(Request::DeleteMessage {
                id: chat_id,
                msg_id,
            })
        };
        self.context_menu = None;
        (true, req)
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
            let pane_w = (w - theme::layout::LIST_W).max(0.0);
            let content = self.messages.content_height(text, pane_w);
            self.messages.scroll_by(dy, viewport, content);
        }
    }

    /// Appends typed text (login field while unauthenticated, composer in
    /// the chat view). Returns `true` the first time text appears in the
    /// composer, so the window can notify the server of typing.
    pub fn push_text(&mut self, s: &str) -> bool {
        if !self.authenticated {
            self.login_input.push_str(s);
            return false;
        }
        if matches!(self.screen, Screen::Chat { .. }) {
            let was_empty = self.input.is_empty();
            self.input.push_str(s);
            was_empty && s.chars().any(|c| !c.is_control())
        } else {
            false
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
        if let Some(msg_id) = self.editing.take() {
            // Editing an outgoing message: update it locally (the server
            // echoes the edit back through an update), then request the edit.
            if let Some(m) = self.messages.rows.iter_mut().find(|m| m.id == msg_id) {
                m.text = text.clone();
            }
            return Some(Request::EditMessage {
                id,
                msg_id,
                text,
            });
        }
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
                read: false,
            });
        Some(Request::SendMessage { id, text })
    }

    /// Forces the messages to scroll to the bottom (after a send).
    pub fn scroll_messages_to_bottom(&mut self, text: &TextRenderer, w: f32, h: f32) {
        if matches!(self.screen, Screen::Chat { .. }) {
            let viewport = messages_viewport(h);
            let pane_w = (w - theme::layout::LIST_W).max(0.0);
            let content = self.messages.content_height(text, pane_w);
            self.messages.set_scroll_bottom(content, viewport);
        }
    }

    /// Starts a text selection at a logical window point. Returns `true` when
    /// the selection actually started (the point landed on a message).
    pub fn begin_selection(&mut self, text: &TextRenderer, x: f32, y: f32, width: f32) -> bool {
        let Some(a) = self.selection_anchor(text, x, y, width) else {
            return false;
        };
        self.selection = Some(Selection { start: a, end: a });
        true
    }

    /// Extends the ongoing selection to a logical window point.
    pub fn update_selection(&mut self, text: &TextRenderer, x: f32, y: f32, width: f32) {
        if self.selection.is_none() {
            return;
        }
        if let Some(a) = self.selection_anchor(text, x, y, width) {
            if let Some(sel) = &mut self.selection {
                sel.end = a;
            }
        }
    }

    /// The active text selection, if any.
    pub fn selection(&self) -> Option<&Selection> {
        self.selection.as_ref()
    }

    /// Clears the current text selection, if any.
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// The selected text, if a non-empty selection exists.
    pub fn selection_text(&self) -> Option<String> {
        let sel = self.selection?;
        let out = self.messages.selected_text(sel);
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    /// Text to copy on Ctrl+C: the selection, else the message under the
    /// cursor, else the active input field.
    pub fn copy_source(&self, text: &TextRenderer, x: f32, y: f32, width: f32) -> Option<String> {
        if let Some(sel) = self.selection_text() {
            return Some(sel);
        }
        if let Some(a) = self.selection_anchor(text, x, y, width) {
            let row = self.messages.rows.get(a.row)?;
            if !row.text.is_empty() {
                return Some(row.text.clone());
            }
        }
        if !self.authenticated {
            if !self.login_input.is_empty() {
                return Some(self.login_input.clone());
            }
        } else if !self.input.is_empty() {
            return Some(self.input.clone());
        }
        None
    }

    /// Resolves a logical window point to a character anchor in the messages
    /// pane (converting to pane-local coordinates), or `None` outside it.
    fn selection_anchor(&self, text: &TextRenderer, x: f32, y: f32, width: f32) -> Option<Anchor> {
        if !matches!(self.screen, Screen::Chat { .. }) {
            return None;
        }
        let pane_w = (width - theme::layout::LIST_W).max(0.0);
        if x < theme::layout::LIST_W || y < theme::layout::CHAT_HEADER_H {
            return None;
        }
        self.messages.anchor_at(text, x - theme::layout::LIST_W, y - theme::layout::CHAT_HEADER_H, pane_w)
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
            read: false,
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
            read: false,
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
            read: false,
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

    #[test]
    fn chat_read_clears_the_unread_badge() {
        let mut state = ready_state();
        state.on_message(UiMessage::Dialogs(rows()));
        state.on_message(UiMessage::ChatRead { id: 2 });
        assert_eq!(
            state.list.rows.iter().find(|r| r.id == 2).unwrap().unread,
            0
        );
    }

    #[test]
    fn context_edit_prefills_the_composer() {
        let mut state = ready_state();
        open_chat(&mut state); // opens id 2 with one incoming row (id 100)
        state.messages.rows.clear();
        state.messages.rows.push(MsgRow {
            id: 77,
            text: "ma phrase".into(),
            date: 0,
            out: true,
            photo: None,
            photo_path: None,
            read: false,
        });
        state.take_scroll_bottom();

        let text = text();
        // Right-click anywhere in the (only) outgoing message pane row.
        let ok = state.open_context(400.0, theme::layout::CHAT_HEADER_H + 25.0, 400.0, &text);
        assert!(ok, "menu opens on outgoing message");
        assert!(state.context_menu.is_some());

        // Click the "Modifier" item (top half of the menu).
        let (mx, my, mw, _) = state.context_menu_size();
        let (handled, req) = state.context_click(
            theme::layout::LIST_W + mx + mw / 2.0,
            my + 10.0,
        );
        assert!(handled);
        assert!(req.is_none());
        assert_eq!(state.editing, Some(77));
        assert_eq!(state.input, "ma phrase");

        // Enter now issues an edit, not a send.
        state.push_text(" !");
        let req = state.enter().unwrap();
        assert!(
            matches!(
                req,
                Request::EditMessage { msg_id: 77, ref text, .. } if text == "ma phrase !"
            ),
            "expected EditMessage, got {req:?}"
        );
        assert_eq!(state.editing, None);
        assert_eq!(
            state.messages.rows.iter().find(|m| m.id == 77).unwrap().text,
            "ma phrase !"
        );
    }

    #[test]
    fn context_delete_removes_the_row_and_requests_the_network() {
        let mut state = ready_state();
        open_chat(&mut state);
        state.messages.rows.clear();
        state.messages.rows.push(MsgRow {
            id: 88,
            text: "bye".into(),
            date: 0,
            out: true,
            photo: None,
            photo_path: None,
            read: false,
        });
        state.take_scroll_bottom();

        let text = text();
        let ok = state.open_context(
            theme::layout::LIST_W + 30.0,
            theme::layout::CHAT_HEADER_H + 25.0,
            400.0,
            &text,
        );
        assert!(ok);

        let (mx, my, mw, mh) = state.context_menu_size();
        // Click "Supprimer" (bottom half).
        let (handled, req) = state.context_click(
            theme::layout::LIST_W + mx + mw / 2.0,
            my + mh - 10.0,
        );
        assert!(handled);
        assert!(
            matches!(req, Some(Request::DeleteMessage { msg_id: 88, .. })),
            "expected DeleteMessage, got {req:?}"
        );
        assert!(state.context_menu.is_none());
        assert!(
            !state.messages.rows.iter().any(|m| m.id == 88),
            "deleted row should vanish locally"
        );
    }

    #[test]
    fn outbox_read_marks_only_the_open_chat_sent_messages() {
        let mut state = ready_state();
        open_chat(&mut state); // opens id 2, with one incoming row (id 100)
        state.messages.rows.push(MsgRow {
            id: 55,
            text: "mine".into(),
            date: 0,
            out: true,
            photo: None,
            photo_path: None,
            read: false,
        });
        state.messages.rows.push(MsgRow {
            id: 56,
            text: "mine too".into(),
            date: 0,
            out: true,
            photo: None,
            photo_path: None,
            read: false,
        });

        // Read only up to id 55.
        state.on_message(UiMessage::OutboxRead {
            chat_id: 2,
            max_id: 55,
        });
        let rows = &state.messages.rows;
        let read55 = rows.iter().find(|m| m.id == 55).unwrap().read;
        let read56 = rows.iter().find(|m| m.id == 56).unwrap().read;
        assert!(read55);
        assert!(!read56);

        // A stop for a non-open chat must not touch the open chat's messages.
        state.on_message(UiMessage::OutboxRead {
            chat_id: 3,
            max_id: i32::MAX,
        });
        let read56 = state.messages.rows.iter().find(|m| m.id == 56).unwrap().read;
        assert!(!read56);
    }

    #[test]
    fn typing_tracks_the_open_chat_peer() {
        let mut state = ready_state();
        state.on_message(UiMessage::Dialogs(rows()));
        state.click(50.0, 150.0, 400.0, 720.0); // opens id 2

        assert_eq!(state.typing, None);
        state.on_message(UiMessage::PeerTyping {
            chat_id: 2,
            typing: true,
        });
        assert_eq!(state.typing, Some(2));

        // A stop for a non-open chat must not clear the open chat's status.
        state.on_message(UiMessage::PeerTyping {
            chat_id: 3,
            typing: false,
        });
        assert_eq!(state.typing, Some(2));

        state.on_message(UiMessage::PeerTyping {
            chat_id: 2,
            typing: false,
        });
        assert_eq!(state.typing, None);
    }

    #[test]
    fn unread_count_sync_only_lowers_the_badge() {
        let mut state = ready_state();
        state.on_message(UiMessage::Dialogs(rows()));
        // Another device read chat 2 (badge 2 -> 0).
        state.on_message(UiMessage::UnreadCount {
            chat_id: 2,
            count: 0,
        });
        assert_eq!(state.list.rows.iter().find(|r| r.id == 2).unwrap().unread, 0);
        // A stale/larger count (e.g. a race) never raises the badge.
        state.on_message(UiMessage::UnreadCount {
            chat_id: 2,
            count: 5,
        });
        assert_eq!(state.list.rows.iter().find(|r| r.id == 2).unwrap().unread, 0);
        // Incoming messages still bump it locally afterwards.
        state.on_message(UiMessage::NewMessage {
            chat_id: 2,
            id: 58,
            text: "back".into(),
            date: 0,
            out: false,
            photo: None,
        });
        assert_eq!(state.list.rows.iter().find(|r| r.id == 2).unwrap().unread, 1);
    }

    fn text() -> crate::text::TextRenderer {
        crate::text::TextRenderer::new()
    }

    fn open_chat_long(state: &mut UiState) {
        state.on_message(UiMessage::Dialogs(rows()));
        state.click(50.0, 150.0, 400.0, 720.0); // opens id 2
        let long = "word ".repeat(60);
        let mut msgs = Vec::new();
        for i in 0..30 {
            msgs.push(MsgRow {
                id: 1000 + i,
                text: long.clone(),
                date: 0,
                out: false,
                photo: None,
                photo_path: None,
                read: false,
            });
        }
        state.on_message(UiMessage::Messages {
            id: 2,
            title: "Beta".into(),
            rows: msgs,
        });
    }

    #[test]
    fn sending_in_a_long_chat_reveals_the_new_message() {
        let mut state = ready_state();
        open_chat_long(&mut state);
        state.take_scroll_bottom(); // consume the open flag
        state.messages.scroll = 0.0; // scroll back up first

        state.push_text("salut");
        state.enter();

        let w = 600.0;
        let h = 720.0;
        state.scroll_messages_to_bottom(&text(), w, h);

        let tr = text();
        let pane_w = w - theme::layout::LIST_W;
        let content = state.messages.content_height(&tr, pane_w);
        let last_h = state.messages.row_height(&tr, state.messages.rows.last().unwrap(), pane_w);
        let viewport = messages_viewport(h);
        let view_top = content - last_h - state.messages.scroll;
        assert!(
            view_top >= -0.5 && view_top + last_h <= viewport + 0.5,
            "sent message not fully visible: top={view_top} last_h={last_h} viewport={viewport}"
        );
    }

    #[test]
    fn selection_begin_update_clear_lifecycle() {
        let mut state = ready_state();
        open_chat(&mut state); // one row "already here"
        state.take_scroll_bottom();
        let tr = text();

        // Left pane / above header -> no selection.
        assert!(!state.begin_selection(&tr, 100.0, 80.0, 600.0));
        state.update_selection(&tr, 370.0, 80.0, 600.0);
        assert!(state.selection_text().is_none());

        // Begin + drag over the message text selects a range.
        assert!(state.begin_selection(&tr, 330.0, 80.0, 600.0));
        state.update_selection(&tr, 370.0, 80.0, 600.0);
        let a = state.selection_text().unwrap();
        assert!(a.starts_with("ady"));

        state.clear_selection();
        assert!(state.selection_text().is_none());
    }

    #[test]
    fn copy_source_uses_selection_then_message_then_input() {
        let mut state = ready_state();
        open_chat(&mut state); // one row "already here"
        state.take_scroll_bottom();
        let tr = text();

        state.begin_selection(&tr, 330.0, 80.0, 600.0);
        state.update_selection(&tr, 370.0, 80.0, 600.0);
        let sel = state.copy_source(&tr, 10.0, 10.0, 600.0).unwrap();
        assert!(!sel.is_empty());

        state.clear_selection();
        // Hovering over the message row copies its text.
        let from_msg = state.copy_source(&tr, 300.0, 80.0, 600.0).unwrap();
        assert_eq!(from_msg, "already here");

        // Empty input + no message under the cursor -> nothing to copy.
        assert!(state.copy_source(&tr, 100.0, 10.0, 600.0).is_none());

        // Composer text is the last fallback.
        state.push_text("typed");
        assert_eq!(state.copy_source(&tr, 100.0, 10.0, 600.0).unwrap(), "typed");
    }
}