//! Pure, testable application state (Model-View-Update). No Iced widget types
//! here: `update` turns `Message`s into state changes + `Request`s, exactly
//! like the custom `ui` crate's `UiState`, so it can be unit tested headlessly.

use crate::bridge::{ChatRow, MsgRow, Request, UiMessage};

use std::io::Write;

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

    /// True while the open chat's history hasn't arrived yet (avoids showing
    /// "No messages yet" during the fetch).
    pub loading: bool,

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
    /// True once the message list should be scrolled to the bottom (open chat,
    /// new/outgoing message). Cleared by the view after use.
    pub scroll_to_bottom: bool,

    /// Absolute Y offset of the message list's viewport (content coordinates),
    /// fed by the scrollable's `on_scroll`. Drives virtualization: only the
    /// rows overlapping `[offset, offset + viewport]` are built each frame.
    pub scroll_offset: f32,

    /// `--perf`: draw the FPS overlay in the top-right corner.
    pub perf_show: bool,
    /// `--continuous`: request a redraw every 4 ms (constant render loop).
    pub continuous: bool,
    /// Samples of recent frame times (ms) used by the overlay.
    perf_frames: std::collections::VecDeque<f32>,
    /// Instant of the previous cadence sample.
    perf_last: std::time::Instant,
    /// Scroll events since the last `TG_PERF_LOG` sample (event-delivery probe).
    perf_scroll_events: u64,
    /// Wall-clock of the previous cadence tick (for renders/sec).
    perf_tick_last: std::time::Instant,
    /// `--scroll-perf=SECS`: simulate a self-driven fling (real update→view→
    /// present turns) for end-to-end scroll-rate measurement. Seconds left.
    pub scroll_perf_dur: f32,
    /// Elapsed simulated scroll time (ms).
    /// Wall-clock start of the current scroll-perf run (for renders/sec).
    perf_wall0: std::time::Instant,
    perf_sim_time: f32,
    /// Ping-pong phase for the synthetic scroll offset.
    perf_sim_phase: f32,
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
            loading: false,
            authenticated: false,
            typing: false,
            context_menu: None,
            editing: None,
            login_step: LoginStep::Phone,
            login_input: String::new(),
            composer: String::new(),
            viewer: None,
            auto_open_first: false,
            scroll_to_bottom: false,
            scroll_offset: 0.0,
            perf_show: false,
            continuous: false,
            perf_frames: std::collections::VecDeque::new(),
            perf_last: std::time::Instant::now(),
            perf_scroll_events: 0,
            perf_tick_last: std::time::Instant::now(),
            scroll_perf_dur: 0.0,
            perf_sim_time: 0.0,
            perf_wall0: std::time::Instant::now(),
            perf_sim_phase: 0.0,
        }
    }

    /// Sets the "open first chat once the list is loaded" convenience flag.
    pub fn with_auto_open_first(mut self, on: bool) -> Self {
        self.auto_open_first = on;
        self
    }

    /// Enables the `--perf` FPS overlay.
    pub fn with_perf(mut self, on: bool) -> Self {
        self.perf_show = on;
        self
    }

    /// `--scroll-perf=SECS`: self-drive a synthetic fling for end-to-end
    /// scroll-rate measurement (see [`Self::advance_scroll_sim`]).
    pub fn with_scroll_perf(mut self, secs: f32) -> Self {
        self.scroll_perf_dur = secs;
        self.perf_show = true;
        self
    }

    /// `--continuous`: redraw continuously (asks for a frame every 4 ms) so the
    /// compositor always gets a fresh frame, defeating any event-only throttling.
    pub fn with_continuous(mut self, on: bool) -> Self {
        self.continuous = on;
        self
    }

    /// Records the message list's absolute scroll offset (from `on_scroll`).
    pub fn on_scrolled(&mut self, y: f32) {
        self.scroll_offset = y;
        self.perf_scroll_events += 1;
        self.sample_frame_time();
    }

    /// Periodic tick (only active under `--perf`): updates the FPS estimate.
    pub fn on_perf_tick(&mut self) {
        self.sample_frame_time();
        // Persistent cadence log (undocumented, used by the perf harness):
        // `--perf` with `TG_PERF_LOG` writes "fps=<n> ms=<avg> renders=/s"
        // every ~500 ms. `fps` is the scroll-event update rate; `renders`
        // is the ACTUAL redraw/present rate (this is what you see).
        if let Ok(path) = std::env::var("TG_PERF_LOG") {
            let n = self.perf_frames.len();
            let events = self.perf_scroll_events;
            self.perf_scroll_events = 0;
            let renders = crate::rendered_since();
            let now = std::time::Instant::now();
            let span = now.duration_since(self.perf_tick_last).as_secs_f32() * 1000.0;
            self.perf_tick_last = now;
            let span = span.max(1.0);
            if n >= 2 {
                let sum: f32 = self.perf_frames.iter().sum();
                let avg = sum / n as f32;
                let rps = renders as f32 * (1000.0 / span);
                let line = format!(
                    "fps={:.0} ms={avg:.2} events={events} renders={renders} renders_s={rps:.0}\n",
                    1000.0 / avg
                );
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = f.write_all(line.as_bytes());
                }
            }
        }
    }

    /// Current FPS estimate from the recent frame-time samples (0 when idle).
    pub fn fps(&self) -> u32 {
        let n = self.perf_frames.len();
        if n == 0 {
            return 0;
        }
        let sum: f32 = self.perf_frames.iter().sum();
        (1000.0 / (sum / n as f32)).round() as u32
    }

    fn sample_frame_time(&mut self) {
        let now = std::time::Instant::now();
        let dt = now.duration_since(self.perf_last).as_secs_f32() * 1000.0;
        self.perf_last = now;
        // Ignore gaps that are idle pauses (the 500 ms PerfTick fires when
        // there is no input), so the average reflects the render cadence while
        // scrolling, not pauses — but still catch genuinely slow frames.
        if dt > 0.0 && dt < 400.0 {
            self.perf_frames.push_back(dt);
            if self.perf_frames.len() > 120 {
                self.perf_frames.pop_front();
            }
        }
    }

    /// One synthetic fling step (only under `--scroll-perf`): brings up the
    /// virtual window like a real scroll tick and, when the time budget runs
    /// out, exits so the caller can read `TG_PERF_LOG`.
    pub fn advance_scroll_sim(&mut self) {
        const STEP_MS: f32 = 16.0;
        let max = (self.messages.len() * 70) as f32;
        let cycle = max * 2.0;
        self.perf_sim_phase = (self.perf_sim_phase + 46.0) % cycle;
        let y = if self.perf_sim_phase <= max {
            self.perf_sim_phase
        } else {
            cycle - self.perf_sim_phase
        };
        self.on_scrolled(y);
        self.perf_sim_time += STEP_MS;
        if self.perf_sim_time >= self.scroll_perf_dur * 1000.0 {
            // Write a final line via the cadence log, then exit cleanly.
            self.write_perf_log();
            std::process::exit(0);
        }
    }

    fn write_perf_log(&self) {
        use std::io::Write;
        if let Ok(path) = std::env::var("TG_PERF_LOG") {
            let n = self.perf_frames.len();
            if n >= 4 {
                let sum: f32 = self.perf_frames.iter().sum();
                let avg = sum / n as f32;
                let renders = crate::rendered_since();
                let span = self.perf_wall0.elapsed().as_secs_f32() * 1000.0;
                let span = span.max(1.0);
                let rps = renders as f32 * (1000.0 / span);
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = f.write_all(
                        format!(
                            "FINAL fps={:.0} ms={avg:.2} renders={renders} renders_s={rps:.0} n={n}\n",
                            1000.0 / avg
                        )
                        .as_bytes(),
                    );
                }
            }
        }
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
                    let prev_len = self.messages.len();
                    self.chat_title = title;
                    self.messages = rows;
                    self.loading = false;
                    self.editing = None;
                    self.context_menu = None;
                    // Scroll to the bottom on open, or when a new message
                    // arrived (not for a plain edit/refresh).
                    if prev_len == 0 || self.messages.len() > prev_len {
                        self.scroll_to_bottom = true;
                    }
                }
            }
            UiMessage::NewMessage { chat_id, id, text, date, out, photo } => {
                // Open chat? Merge the message (dedupe the optimistic local
                // send, which is tagged id=0).
                if self.open_chat == Some(chat_id) {
                    self.loading = false;
                    let rows = &mut self.messages;
                    if let Some(m) = rows.iter_mut().find(|m| m.id == id) {
                        m.text = text;
                        m.out = out;
                    } else if let Some(i) =
                        rows.iter().position(|m| m.id == 0 && m.text == text)
                    {
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
                    }
                    self.scroll_to_bottom = true;
                    if !out {
                        self.typing = false;
                    }
                    return;
                }
                // Otherwise: update the list row (preview + unread only for
                // incoming messages).
                if let Some(row) = self.dialogs.iter_mut().find(|r| r.id == chat_id) {
                    row.subtitle = text;
                    row.date = date;
                    if !out {
                        row.unread += 1;
                    }
                }
            }
            UiMessage::MessageEdited { chat_id, id, text, date } => {
                if self.open_chat == Some(chat_id) {
                    for m in &mut self.messages {
                        if m.id == id {
                            m.text = text;
                            m.date = date;
                            break;
                        }
                    }
                } else if let Some(row) = self.dialogs.iter_mut().find(|r| r.id == chat_id) {
                    row.subtitle = text;
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
                if let Some(row) = self.dialogs.iter_mut().find(|r| r.id == id) {
                    row.unread = 0;
                }
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
                // Only sync the count when it goes down or another device
                // read the chat; a locally-received message already bumped it.
                if let Some(row) = self.dialogs.iter_mut().find(|r| r.id == chat_id) {
                    if count < row.unread {
                        row.unread = count;
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
            UiMessage::LoginCodeRequired => {
                self.login_step = LoginStep::Code;
                self.login_input.clear();
                self.login_error = false;
                self.status = "Enter the code received by Telegram".to_string();
            }
            UiMessage::LoginPasswordRequired { hint } => {
                self.login_step = LoginStep::Password;
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
                self.status = if self.dialogs.is_empty() {
                    format!("Signed in as {name}")
                } else {
                    format!("Signed in as {name} — {} chats", self.dialogs.len())
                };
            }
            UiMessage::Error(e) => {
                self.loading = false;
                if !self.authenticated {
                    self.login_error = true;
                }
                self.status = e;
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

    /// Dismisses the context menu (a click outside, or opening another one).
    pub fn dismiss_menu(&mut self) {
        self.context_menu = None;
    }

    /// Escape: closes the context menu and cancels editing.
    pub fn escape(&mut self) {
        self.context_menu = None;
        if self.editing.is_some() {
            self.editing = None;
            self.composer.clear();
        }
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

    /// Click on the context menu's "Copier" item: returns the copied text (or
    /// `None`), which the caller writes to the system clipboard.
    pub fn context_copy(&mut self) -> Option<String> {
        let menu = self.context_menu.take()?;
        let m = self.messages.get(menu.row)?;
        let text = if m.text.is_empty() {
            // Photo-only message: nothing meaningful to copy.
            return None;
        } else {
            m.text.clone()
        };
        Some(text)
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
            // Optimistic local send: the id and date are unknown (incoming
            // updates will provide them).
            self.messages.push(MsgRow {
                id: 0,
                text: text.clone(),
                date: 0,
                out: true,
                photo: None,
                photo_path: None,
                read: false,
            });
            let _ = self.req_tx.send(Request::SendMessage { id, text });
        }
        self.composer.clear();
        self.typing = false;
        self.scroll_to_bottom = true;
    }

    /// Open a chat.
    pub fn open_chat(&mut self, id: i64) {
        self.open_chat = Some(id);
        self.messages.clear();
        self.loading = true;
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
        assert!(state.scroll_to_bottom, "new message must scroll to bottom");
    }

    #[test]
    fn new_message_from_other_chat_updates_list_not_open_chat() {
        let (mut state, _) = demo_state();
        state.dialogs = vec![ChatRow {
            id: 7,
            title: "Other".into(),
            subtitle: String::new(),
            date: 0,
            unread: 0,
            avatar_path: None,
        }];
        state.on_message(UiMessage::NewMessage {
            chat_id: 7,
            id: 9,
            text: "ping".into(),
            date: 400,
            out: false,
            photo: None,
        });
        // The open chat's rows must be untouched.
        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.dialogs[0].subtitle, "ping");
        assert_eq!(state.dialogs[0].unread, 1);
        assert!(!state.scroll_to_bottom, "other chat's message must not scroll");
    }

    #[test]
    fn optimistic_send_is_deduplicated_by_the_update() {
        let (mut state, _) = demo_state();
        state.composer = "hello".into();
        state.submit();
        // Optimistic local row was added.
        assert_eq!(state.messages.len(), 3);
        assert_eq!(state.messages.last().unwrap().id, 0);

        state.on_message(UiMessage::NewMessage {
            chat_id: 42,
            id: 777,
            text: "hello".into(),
            date: 0,
            out: true,
            photo: None,
        });
        // The update merged with the optimistic row: no duplicate.
        assert_eq!(state.messages.len(), 3);
        assert_eq!(state.messages.last().unwrap().id, 777);
    }

    #[test]
    fn mark_read_clears_the_list_badge() {
        let (mut state, _) = demo_state();
        state.dialogs = vec![ChatRow {
            id: 42,
            title: "Camille".into(),
            subtitle: String::new(),
            date: 0,
            unread: 5,
            avatar_path: None,
        }];
        state.on_message(UiMessage::ChatRead { id: 42 });
        assert_eq!(state.dialogs[0].unread, 0);
        assert!(state.messages.iter().all(|m| !m.out || m.read));
    }

    #[test]
    fn outbox_read_marks_only_the_open_chat_sent_messages() {
        let (mut state, _) = demo_state();
        state.messages.push(MsgRow {
            id: 55,
            text: "mine".into(),
            date: 0,
            out: true,
            photo: None,
            photo_path: None,
            read: false,
        });
        state.messages.push(MsgRow {
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
            chat_id: 42,
            max_id: 55,
        });
        assert!(state.messages.iter().find(|m| m.id == 55).unwrap().read);
        assert!(!state.messages.iter().find(|m| m.id == 56).unwrap().read);

        // A stop for a non-open chat must not touch the open chat's messages.
        state.on_message(UiMessage::OutboxRead {
            chat_id: 7,
            max_id: i32::MAX,
        });
        assert!(!state.messages.iter().find(|m| m.id == 56).unwrap().read);
    }

    #[test]
    fn unread_count_sync_only_lowers_the_badge() {
        let (mut state, _) = demo_state();
        state.dialogs = vec![ChatRow {
            id: 42,
            title: "Camille".into(),
            subtitle: String::new(),
            date: 0,
            unread: 3,
            avatar_path: None,
        }];
        // Higher count from another device is ignored (a local message already
        // bumped it), but a lower one syncs down.
        state.on_message(UiMessage::UnreadCount {
            chat_id: 42,
            count: 10,
        });
        assert_eq!(state.dialogs[0].unread, 3);
        state.on_message(UiMessage::UnreadCount {
            chat_id: 42,
            count: 1,
        });
        assert_eq!(state.dialogs[0].unread, 1);
    }

    #[test]
    fn new_message_in_list_view_updates_the_list() {
        let (mut state, _) = demo_state();
        state.open_chat = None;
        state.dialogs = vec![ChatRow {
            id: 7,
            title: "Other".into(),
            subtitle: String::new(),
            date: 0,
            unread: 0,
            avatar_path: None,
        }];
        state.on_message(UiMessage::NewMessage {
            chat_id: 7,
            id: 9,
            text: "ping".into(),
            date: 400,
            out: false,
            photo: None,
        });
        assert_eq!(state.dialogs[0].subtitle, "ping");
        assert_eq!(state.dialogs[0].unread, 1);
        assert!(!state.scroll_to_bottom, "list view must not scroll messages");
    }

    #[test]
    fn escape_cancels_editing() {
        let (mut state, _) = demo_state();
        state.open_context(1);
        state.context_edit();
        assert_eq!(state.editing, Some(2));
        assert_eq!(state.composer, "mine");
        state.escape();
        assert!(state.editing.is_none());
        assert!(state.composer.is_empty());
        assert!(state.context_menu.is_none());
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

    #[test]
    fn opening_a_chat_clears_messages_and_is_loading() {
        let (mut state, mut req_rx) = demo_state();
        state.open_chat(7);
        // The good, so a regression like "click a chat → stuck on 'No
        // messages yet'" is caught: the chat is flagged as loading.
        assert_eq!(state.open_chat, Some(7));
        assert!(state.messages.is_empty());
        assert!(state.loading, "opening a chat must set the loading flag");
        let reqs = drain(&mut req_rx);
        assert_eq!(reqs.len(), 2, "open + mark-read are sent to the network");
        assert!(matches!(reqs[0], Request::OpenChat { id: 7 }));
        assert!(matches!(reqs[1], Request::MarkRead { id: 7 }));
    }

    #[test]
    fn messages_reply_clears_loading_and_populates_rows() {
        let (mut state, _) = demo_state();
        state.open_chat(7);
        assert!(state.loading);
        state.on_message(UiMessage::Messages {
            id: 7,
            title: "Camille".into(),
            rows: vec![MsgRow {
                id: 1,
                text: "hi".into(),
                date: 5,
                out: false,
                photo: None,
                photo_path: None,
                read: false,
            }],
        });
        assert!(!state.loading, "loading stops once history arrives");
        assert_eq!(state.messages.len(), 1);
        assert!(state.scroll_to_bottom, "first history loads scroll to bottom");
        assert_eq!(state.chat_title, "Camille");
    }

    #[test]
    fn messages_for_a_different_chat_do_not_clear_loading() {
        let (mut state, _) = demo_state();
        state.open_chat(7);
        state.on_message(UiMessage::Messages {
            id: 8,
            title: "Other".into(),
            rows: vec![],
        });
        assert!(state.loading, "a stale response must not clear loading");
        assert!(state.messages.is_empty());
    }

    #[test]
    fn loading_is_not_set_after_history_arrives() {
        let (mut state, _) = demo_state();
        state.open_chat(7);
        state.on_message(UiMessage::Messages {
            id: 7,
            title: "Camille".into(),
            rows: vec![MsgRow {
                id: 1,
                text: "hi".into(),
                date: 5,
                out: true,
                photo: None,
                photo_path: None,
                read: false,
            }],
        });
        // A refresh with a changed signature also clears loading (idempotent).
        state.on_message(UiMessage::Messages {
            id: 7,
            title: "Camille".into(),
            rows: vec![],
        });
        assert!(!state.loading);
        assert!(state.messages.is_empty());
    }
}
