//! Pure, testable application state (Model-View-Update). No Iced widget types
//! here: `update` turns `Message`s into state changes + `Request`s, exactly
//! like the custom `ui` crate's `UiState`, so it can be unit tested headlessly.

use crate::bridge::{
    ChatDetail, ChatRow, DocMeta, MsgRow, MyProfile, ParticipantRow, Request, SearchHit,
    SessionInfo, StickerMeta, StickerSetBridge, UiMessage,
};

// ---------------------------------------------------------------------------
// Theme (light/dark) — Lot 3a
// ---------------------------------------------------------------------------

/// Selected UI color scheme. Defined in [`crate::theme`] (next to the
/// palettes) and re-exported here: the settings panel toggles
/// `Message::ToggleTheme` → `State::toggle_theme`.
pub use crate::theme::ThemeMode;

/// Reads the persisted theme mode from `path`; a missing/unreadable/garbage
/// marker falls back to dark.
fn read_theme_mode_at(path: &std::path::Path) -> ThemeMode {
    std::fs::read_to_string(path)
        .map(|content| ThemeMode::parse(&content))
        .unwrap_or_default()
}

/// Persists `m` at `path` (`"dark"`/`"light"`). Best-effort: a read-only or
/// missing data dir must never crash the app.
fn write_theme_mode_at(path: &std::path::Path, m: ThemeMode) {
    let _ = std::fs::write(path, m.as_str());
}

/// Default location of the `theme-mode` file inside the app data dir.
fn theme_mode_path() -> std::path::PathBuf {
    crate::network::data_dir().join("theme-mode")
}

/// One-line preview of a message for list rows / reply snippets: the text,
/// or a media placeholder when there is none.
pub fn preview_text(
    text: &str,
    photo: &Option<(u32, u32)>,
    doc: &Option<DocMeta>,
    sticker: &Option<StickerMeta>,
) -> String {
    if !text.is_empty() {
        return text.to_string();
    }
    if let Some(sticker) = sticker {
        let alt = sticker.alt.trim();
        if alt.is_empty() {
            return "🖼 Sticker".to_string();
        }
        return format!("{alt} Sticker");
    }
    if let Some(doc) = doc {
        let name = if doc.name.is_empty() { "File" } else { &doc.name };
        return format!("📄 {name}");
    }
    if photo.is_some() {
        return "📷 Photo".to_string();
    }
    String::new()
}

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Last-open-chat persistence
// ---------------------------------------------------------------------------

/// Reads the persisted last-open chat id from `path` (a single i64 line).
fn read_last_chat_at(path: &std::path::Path) -> Option<i64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Persists (`Some`) or clears (`None`) the last-open chat id at `path`.
/// Best-effort: a read-only or missing data dir must never crash the app.
fn write_last_chat_at(path: &std::path::Path, id: Option<i64>) {
    let _ = match id {
        Some(id) => std::fs::write(path, id.to_string()),
        None => std::fs::remove_file(path),
    };
}

/// Default location of the `last-chat` marker inside the app data dir.
fn last_chat_path() -> std::path::PathBuf {
    crate::network::data_dir().join("last-chat")
}

// ---------------------------------------------------------------------------
// Emoji picker (composer): panel flag + recently used emojis
// ---------------------------------------------------------------------------

/// Maximum number of recently-used emojis kept (and persisted).
pub const EMOJI_RECENTS_MAX: usize = 24;

/// Reads persisted recents from `path`: one emoji per line.
fn read_emoji_recents_at(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|content| {
            content
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Persists `recents` at `path` (one emoji per line). Best-effort.
fn write_emoji_recents_at(path: &std::path::Path, recents: &[String]) {
    let _ = std::fs::write(path, recents.join("\n"));
}

/// Default location of the `emoji-recents` file inside the app data dir.
fn emoji_recents_path() -> std::path::PathBuf {
    crate::network::data_dir().join("emoji-recents")
}

/// Sign-in flow step (only used while unauthenticated).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginStep {
    Phone,
    Code,
    Password,
}

/// Progress of the QR sign-in pane (runs beside the phone `LoginStep` flow;
/// the two are independent and [`LoginStep`] is never touched by QR logic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QrStage {
    /// Nothing requested yet (pane just switched in / after a failure).
    Idle,
    /// A QR image is shown; waiting for the phone to scan it.
    AwaitingScan,
    /// The phone scanned the code; the server is confirming this device.
    ScanConfirmed,
}

// ---------------------------------------------------------------------------
// Notifications preference persistence ("on"/"off" file in the data dir)
// ---------------------------------------------------------------------------

/// Default location of the `notifications` file inside the app data dir.
fn notifications_path() -> std::path::PathBuf {
    crate::network::data_dir().join("notifications")
}

/// Reads the persisted notifications flag from `path` (`"off"` = disabled,
/// anything else — including a missing file — = enabled).
fn read_notifications_at(path: &std::path::Path) -> bool {
    std::fs::read_to_string(path)
        .map(|c| c.trim() != "off")
        .unwrap_or(true)
}

/// Persists the notifications flag at `path`. Best-effort.
fn write_notifications_at(path: &std::path::Path, on: bool) {
    let _ = std::fs::write(path, if on { "on" } else { "off" });
}

/// Open context menu over a message (right-click / long-press).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextMenu {
    /// Index of the message row the menu is over.
    pub row: usize,
}

/// What the creation modal is currently creating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateKind {
    Group,
    Channel,
}

/// Destructive chat action awaiting user confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmKind {
    /// Leave the group/channel.
    Leave,
    /// Delete the chat from the account.
    Delete,
}

/// Destructive member-admin action awaiting its inline Yes/No confirmation
/// (info panel member rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminConfirm {
    /// Ban the member forever (cannot rejoin).
    Ban,
    /// Remove the member from the group (may rejoin).
    Remove,
}

/// One admin action offered by a member row's context menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminAction {
    /// Grant every admin right.
    Promote,
    /// Revoke every admin right.
    Demote,
    /// Ban the member forever.
    Ban,
    /// Remove the member (kick only).
    Remove,
}

/// Menu items for a member row, as data (pure; unit-tested): the owner and
/// yourself get no menu at all, an admin offers Demote+Ban, a plain member
/// offers Promote+Ban+Remove.
pub fn admin_menu_items(role: crate::bridge::ParticipantRole, is_self: bool) -> Vec<AdminAction> {
    if is_self || role == crate::bridge::ParticipantRole::Creator {
        return Vec::new();
    }
    match role {
        crate::bridge::ParticipantRole::Admin => vec![AdminAction::Demote, AdminAction::Ban],
        _ => vec![AdminAction::Promote, AdminAction::Ban, AdminAction::Remove],
    }
}

/// Which tab of the settings panel is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    /// Own profile (identity + edit + notifications + theme).
    Profile,
    /// Media cache size + clear action.
    Storage,
    /// Active account sessions.
    Sessions,
}

/// Where the search UI is currently searching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// Search across all chats (opened from the list header).
    Global,
    /// Search inside the open chat only.
    InChat,
}

/// The composer is replying to a message: `(msg_id, preview snippet)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyTarget {
    pub msg_id: i32,
    /// Short preview of the replied-to text (or a media placeholder).
    pub snippet: String,
}

/// Cached virtualization metrics for the open chat's message list, so a scroll
/// frame doesn't re-estimate row heights / rebuild offset tables for the whole
/// history. Rebuilt by `messages_list` only when invalidated (see
/// [`State::invalidate_layout`]) or when the pane size changes.
#[derive(Debug)]
pub(crate) struct MsgLayoutCache {
    /// Pane width the metrics were computed for.
    pub(crate) pane_w: f32,
    /// Viewport height used for the pinned-to-bottom spacer.
    pub(crate) view_h: f32,
    /// `State::layout_epoch` this cache was built from.
    pub(crate) epoch: u64,
    /// Per-row estimated heights.
    pub(crate) heights: Vec<f32>,
    /// Cumulative top offset of each row (content coordinates, starting at
    /// `view_h` for the bottom-anchoring spacer).
    pub(crate) tops: Vec<f32>,
    /// Total content height.
    pub(crate) total: f32,
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
    /// True between login completing and the dialog list arriving; the view
    /// shows a "Loading chats…" spinner instead of the frozen QR page.
    pub connecting: bool,
    /// The peer of the open chat is typing.
    pub typing: bool,

    /// Open context menu, if any.
    pub context_menu: Option<ContextMenu>,
    /// Message id being edited; its old text lives in `composer`.
    pub editing: Option<i32>,

    // -----------------------------------------------------------------
    // Group/channel creation + chat management
    // -----------------------------------------------------------------
    /// The header "+" picker menu (New Group / New Channel) is open.
    pub create_menu_open: bool,
    /// Open creation modal, if any.
    pub create_dialog: Option<CreateKind>,
    /// Title typed in the creation modal.
    pub create_title: String,
    /// Description typed for a channel.
    pub create_about: String,
    /// Checkable contacts shown when creating a group: `(id, name, checked)`,
    /// seeded from the known dialogs.
    pub member_pick: Vec<(i64, String, bool)>,
    /// Right-click mini menu over a chat-list row (id of that chat).
    pub row_menu: Option<i64>,
    /// Destructive action pending confirmation: (kind, chat id).
    pub confirm_leave: Option<(ConfirmKind, i64)>,
    /// Message the composer is replying to, if any.
    pub reply_target: Option<ReplyTarget>,
    /// Index of the message being forwarded (chat-picker overlay open).
    pub forward_pick: Option<usize>,
    /// A downloaded document the user asked to open (consumed by the shell
    /// layer, which launches the system opener).
    pub open_file: Option<String>,

    /// Active search UI, if any: global or in-chat.
    pub search_mode: Option<SearchMode>,
    /// Live query of the search field.
    pub search_query: String,
    /// Latest search hits (from `UiMessage::SearchResults`).
    pub search_hits: Vec<SearchHit>,
    /// True while a search result may still arrive (keeps "…" placeholder).
    pub search_pending: bool,
    /// Target content Y to scroll the message list to (a search jump), consumed
    /// by the shell after the next view build.
    pub scroll_target: Option<f32>,
    /// Id of a message to jump to once it appears in `messages` (used when the
    /// search hit already lives in the loaded history).
    pending_jump_id: Option<i32>,

    /// Voice note currently playing (or paused): (chat_id, msg_id, path).
    pub playing_voice: Option<(i64, i32, String)>,
    /// True while the voice note is actively playing (not paused).
    pub voice_playing: bool,
    /// Seconds elapsed on the current voice note (drives the progress bar).
    pub voice_elapsed: f32,
    /// Known duration of the current note (0.0 when unknown) — slider range.
    pub voice_duration: f32,
    /// Progress is SIMULATED (wall clock) because the payload went to the
    /// system player (Opus fallback): we cannot poll its position.
    pub voice_sim: bool,
    /// When the simulated playback started (for pause-free progression).
    voice_sim_started: Option<std::time::Instant>,
    /// The voice note we just stopped (so a paused→resume doesn't restart).
    pub voice_paused_path: Option<String>,

    /// Monotonic token source for optimistic media sends (ties upload
    /// progress events back to their row).
    next_media_token: u64,

    /// Shared tray flags (open/quit) polled by the shell.
    pub tray_actions: Arc<crate::tray::TrayActions>,

    pub login_step: LoginStep,
    pub login_input: String,

    // QR sign-in pane (`login_qr_selected` picks the screen; everything else
    // is reset on toggle/complete so a retry always starts clean).
    /// True when the sign-in screen shows the QR pane instead of the phone form.
    pub login_qr_selected: bool,
    /// QR sign-in progress.
    pub qr_stage: QrStage,
    /// On-disk PNG of the current login QR code, once generated.
    pub qr_png_path: Option<String>,
    /// Last QR sign-in failure (shown under the card; `None` = no failure).
    pub qr_error: Option<String>,

    /// Composer text (fresh message, or the edited text).
    pub composer: String,
    /// Full-screen photo viewer path, if any.
    pub viewer: Option<String>,
    /// Open the first chat once the list arrives (test/demo convenience).
    pub auto_open_first: bool,
    /// Persist/restore UI state (last open chat). Off in `--demo` so QA runs
    /// never write into the real data dir.
    pub persist_ui: bool,
    /// Chat to re-open when the dialog list arrives (read from `last-chat`
    /// at boot). Consumed on first `Dialogs`, falls back to `auto_open_first`.
    pub initial_chat: Option<i64>,
    /// True once the message list should be scrolled to the bottom (open chat,
    /// new/outgoing message). Cleared by the view after use.
    pub scroll_to_bottom: bool,

    /// Id of the open chat's pinned message (`None` = nothing pinned). Drives
    /// the banner under the chat header; a click jumps to the message.
    pub pinned_id: Option<i32>,

    /// Right-hand info panel is open (chat details + members).
    pub info_open: bool,
    /// Latest fetched detail of the open chat (`None` until it arrives).
    pub chat_info: Option<ChatDetail>,
    /// Members of the open group/channel (info panel).
    pub participants: Vec<ParticipantRow>,
    /// Member id awaiting kick confirmation (inline Yes/No on its row).
    pub kick_confirm: Option<i64>,
    /// Member id whose admin context menu is open (right-click on a row).
    pub admin_menu: Option<i64>,
    /// Destructive admin action pending confirmation: (kind, member id).
    pub admin_confirm: Option<(AdminConfirm, i64)>,
    /// Bot-api id of the signed-in user (`MemberSelfId`); admin actions
    /// never apply to yourself.
    pub admin_self_id: Option<i64>,
    /// Local mute flag of the open chat (optimistic; drives the button label).
    pub muted: bool,

    // -----------------------------------------------------------------
    // Forum topics (chips bar of forum supergroups; namespaced `topic_`)
    // -----------------------------------------------------------------
    /// Topics of the open forum chat (`UiMessage::Topics`); empty until the
    /// first `GetTopics` answer (non-forum chats always stay empty).
    pub topic_topics: Vec<crate::bridge::TopicRow>,
    /// True when the open chat is a forum (supergroup with topics enabled).
    pub topic_is_forum: bool,
    /// Selected topic's root message id (`None` = the "All messages" chip).
    pub topic_selected: Option<i32>,
    /// The inline create-topic field (title input) is open.
    pub topic_creating: bool,
    /// Title input of the inline create-topic field.
    pub topic_title: String,
    /// FULL history of the open chat; `messages` is the topic-filtered view
    /// of it (identical when no topic is selected). Slice-1 trade-off: thread
    /// filtering runs over the already-loaded history only — a topic whose
    /// older messages were never fetched (beyond `MESSAGE_LIMIT`) shows a
    /// partial thread; fetch-on-demand via `messages.getReplies` is future
    /// work.
    pub(crate) topic_all_messages: Vec<MsgRow>,

    // -----------------------------------------------------------------
    // Theme (light/dark)
    // -----------------------------------------------------------------
    /// Current color scheme. Flipped by [`Self::toggle_theme`]; applied to
    /// the live palette and persisted (with `persist_ui`) there too.
    pub theme_mode: ThemeMode,

    // -----------------------------------------------------------------
    // Emoji picker (composer)
    // -----------------------------------------------------------------
    /// The emoji picker panel above the composer is open.
    pub emoji_panel_open: bool,
    /// Recently used emojis, most recent first (capped at
    /// [`EMOJI_RECENTS_MAX`]), persisted in the data dir.
    pub emoji_recents: Vec<String>,

    // Stickers (picker + rendering)
    // -----------------------------------------------------------------
    /// The sticker picker panel (floating above the composer) is open.
    pub sticker_picker_open: bool,
    /// Installed sticker packs (`UiMessage::StickerSets`). Global, kept
    /// across chats; fetched on first picker open.
    pub sticker_sets: Vec<StickerSetBridge>,
    /// Downloaded picker thumbnails keyed by document id.
    pub sticker_thumbs: HashMap<i64, String>,

    // -----------------------------------------------------------------
    // Settings panel (global overlay, NOT reset when switching chats)
    // -----------------------------------------------------------------
    /// The right-hand settings sheet is open.
    pub settings_open: bool,
    /// Which settings tab is displayed.
    pub settings_tab: SettingsTab,
    /// The signed-in user's own profile (`None` until `GetMe` answers).
    pub my_profile: Option<MyProfile>,
    /// First-name input of the profile edit form.
    pub edit_name: String,
    /// Last-name input of the profile edit form.
    pub edit_last_name: String,
    /// Bio input of the profile edit form.
    pub edit_bio: String,
    /// Active account sessions (`UiMessage::Sessions`).
    pub sessions: Vec<SessionInfo>,
    /// Media cache size in bytes (`Some` once measured).
    pub cache_bytes: Option<u64>,
    /// The "Clear cache" destructive action awaits inline confirmation.
    pub confirm_clear_cache: bool,
    /// The "Log out" destructive action awaits inline confirmation.
    pub confirm_logout: bool,

    /// Shared notifications preference (flipped by the panel, consulted by
    /// the network runtime before raising a desktop notification).
    pub notify_pref: Arc<crate::network::NotifyPref>,
    /// Local mirror of the shared flag (drives the On/Off label).
    pub notifications_on: bool,

    /// Absolute Y offset of the message list's viewport (content coordinates),
    /// fed by the scrollable's `on_scroll`. Drives virtualization: only the
    /// rows overlapping `[offset, offset + viewport]` are built each frame.
    pub scroll_offset: f32,
    /// Absolute Y offset of the dialog (chat) list's viewport, fed by its
    /// `on_scroll`. Drives the dialog-list virtualization (the left pane).
    pub dialog_scroll_offset: f32,

    /// Virtualization metrics (`heights`/`tops`/…) for the open chat's message
    /// list, cached across frames. `messages_list` recomputes it only when the
    /// messages or the context menu changed ([`Self::invalidate_layout`]) or
    /// the pane was resized — otherwise a scroll frame costs O(visible rows)
    /// instead of O(all messages) (the 2k+ message chats that made scrolling
    /// lag).
    pub(crate) layout_cache: Mutex<Option<MsgLayoutCache>>,
    /// Bumped on any change that affects row heights or offsets (`messages`
    /// content, context menu). The view compares it to the cached one.
    pub(crate) layout_epoch: u64,

    /// Pre-ellipsized list-pane labels, aligned 1:1 with `dialogs`:
    /// `(title_short, subtitle_short)`. `list_pane` borrows these as `&str`
    /// each frame instead of re-running `ellipsize` (which allocates a new
    /// String) on every dialog row on every redraw. Rebuilt when `dialogs` is
    /// replaced, and the touched row re-ellipsized when a subtitle changes.
    pub dialog_short: Vec<(String, String)>,

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
            connecting: false,
            typing: false,
            context_menu: None,
            editing: None,
            create_menu_open: false,
            create_dialog: None,
            create_title: String::new(),
            create_about: String::new(),
            member_pick: Vec::new(),
            row_menu: None,
            confirm_leave: None,
            reply_target: None,
            forward_pick: None,
            open_file: None,
            search_mode: None,
            search_query: String::new(),
            search_hits: Vec::new(),
            search_pending: false,
            scroll_target: None,
            pending_jump_id: None,
            playing_voice: None,
            voice_playing: false,
            voice_elapsed: 0.0,
            voice_duration: 0.0,
            voice_sim: false,
            voice_sim_started: None,
            voice_paused_path: None,
            next_media_token: 1,
            tray_actions: Arc::new(crate::tray::TrayActions::default()),
            login_step: LoginStep::Phone,
            login_input: String::new(),
            login_qr_selected: false,
            qr_stage: QrStage::Idle,
            qr_png_path: None,
            qr_error: None,
            composer: String::new(),
            viewer: None,
            auto_open_first: false,
            persist_ui: false,
            initial_chat: None,
            scroll_to_bottom: false,
            pinned_id: None,
            info_open: false,
            chat_info: None,
            participants: Vec::new(),
            kick_confirm: None,
            admin_menu: None,
            admin_confirm: None,
            admin_self_id: None,
            muted: false,
            topic_topics: Vec::new(),
            topic_is_forum: false,
            topic_selected: None,
            topic_creating: false,
            topic_title: String::new(),
            topic_all_messages: Vec::new(),
            theme_mode: ThemeMode::Dark,
            emoji_panel_open: false,
            emoji_recents: Vec::new(),
            sticker_picker_open: false,
            sticker_sets: Vec::new(),
            sticker_thumbs: HashMap::new(),
            settings_open: false,
            settings_tab: SettingsTab::Profile,
            my_profile: None,
            edit_name: String::new(),
            edit_last_name: String::new(),
            edit_bio: String::new(),
            sessions: Vec::new(),
            cache_bytes: None,
            confirm_clear_cache: false,
            confirm_logout: false,
            notify_pref: Arc::new(crate::network::NotifyPref::default()),
            notifications_on: true,
            scroll_offset: 0.0,
            dialog_scroll_offset: 0.0,
            layout_cache: Mutex::new(None),
            layout_epoch: 0,
            dialog_short: Vec::new(),
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

    /// Enables persisting/restoring the last open chat (off in `--demo`).
    /// Also restores the emoji recents and the theme mode (same data dir,
    /// same flag).
    pub fn with_persist_ui(mut self, on: bool) -> Self {
        self.persist_ui = on;
        self.initial_chat = read_last_chat_at(&last_chat_path());
        if on {
            self.emoji_recents = read_emoji_recents_at(&emoji_recents_path());
            self.notifications_on = read_notifications_at(&notifications_path());
            use std::sync::atomic::Ordering;
            self.notify_pref
                .0
                .store(self.notifications_on, Ordering::SeqCst);
            // Theme: restore from disk AND apply to the live palette so the
            // very first frame already uses it.
            let m = read_theme_mode_at(&theme_mode_path());
            self.theme_mode = m;
            crate::theme::set_mode(m);
        }
        self
    }

    /// Flips dark ⇄ light, applies the new palette (the next rendered frame
    /// picks it up) and persists the choice when `persist_ui` is on.
    pub fn toggle_theme(&mut self) {
        self.theme_mode = match self.theme_mode {
            ThemeMode::Dark => ThemeMode::Light,
            ThemeMode::Light => ThemeMode::Dark,
        };
        crate::theme::set_mode(self.theme_mode);
        if self.persist_ui {
            write_theme_mode_at(&theme_mode_path(), self.theme_mode);
        }
    }

    /// Enables the `--perf` FPS overlay.
    pub fn with_perf(mut self, on: bool) -> Self {
        self.perf_show = on;
        self
    }

    /// `--scroll-perf=SECS`: self-drive a synthetic fling for end-to-end
    /// scroll-rate measurement (see [`Self::advance_scroll_sim`]). The FPS
    /// overlay only turns on for an actual run, not for the default 0.
    pub fn with_scroll_perf(mut self, secs: f32) -> Self {
        self.scroll_perf_dur = secs;
        self.perf_show = secs > 0.0;
        self
    }

    /// `--continuous`: redraw continuously (asks for a frame every 4 ms) so the
    /// compositor always gets a fresh frame, defeating any event-only throttling.
    pub fn with_continuous(mut self, on: bool) -> Self {
        self.continuous = on;
        self
    }

/// Marks the message-list layout cache as stale. Called on any mutation
    /// that changes row heights/offsets (`messages` content or the context
    /// menu); the view rebuilds the metrics lazily on the next frame.
    pub fn invalidate_layout(&mut self) {
        self.layout_epoch = self.layout_epoch.wrapping_add(1);
    }

    /// Re-ellipsizes the list-pane labels of one chat (after a preview text
    /// changed). `dialog_short` keeps `(title_short, subtitle_short)` aligned
    /// with `dialogs` so the view can borrow the strings each frame.
    fn refresh_dialog_short(&mut self, chat_id: i64) {
        let pos = self.dialogs.iter().position(|d| d.id == chat_id);
        let Some(pos) = pos else { return };
        let Some(d) = self.dialogs.get(pos) else { return };
        let short = (crate::ellipsize(&d.title, 15), crate::ellipsize(&d.subtitle, 20));
        if self.dialog_short.len() <= pos {
            self.dialog_short.resize(pos + 1, (String::new(), String::new()));
        }
        self.dialog_short[pos] = short;
    }

    /// Records the message list's absolute scroll offset (from `on_scroll`).
    pub fn on_scrolled(&mut self, y: f32) {
        self.scroll_offset = y;
        self.perf_scroll_events += 1;
        self.sample_frame_time();
    }

    /// Records the dialog list's absolute scroll offset (from `on_scroll`)
    /// for the dialog-list virtualization.
    pub fn on_dialog_scrolled(&mut self, y: f32) {
        self.dialog_scroll_offset = y;
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
            UiMessage::Connecting => self.connecting = true,
            UiMessage::Dialogs(rows) => {
                self.dialogs = rows;
                // Pre-ellipsize every list label once (see `dialog_short`), so
                // the view can borrow them per frame instead of allocating.
                self.dialog_short = self
                    .dialogs
                    .iter()
                    .map(|d| {
                        (
                            crate::ellipsize(&d.title, 15),
                            crate::ellipsize(&d.subtitle, 20),
                        )
                    })
                    .collect();
                // A valid session already existed (restart): the chat list is
                // the sign that the account is authenticated.
                self.authenticated = true;
            }
            UiMessage::Messages { id, title, rows } => {
                if self.open_chat == Some(id) {
                    let prev_len = self.messages.len();
                    self.chat_title = title;
                    // The latest pinned row in the payload seeds the banner;
                    // the authoritative `PinnedMessage` (fetched right after)
                    // overrides it.
                    self.pinned_id = rows.iter().rev().find(|m| m.pinned).map(|m| m.id);
                    // `messages` is the topic-filtered view of the history.
                    self.topic_all_messages = rows;
                    self.messages =
                        Self::topic_view(&self.topic_all_messages, self.topic_selected);
                    self.loading = false;
                    self.editing = None;
                    self.context_menu = None;
                    // Scroll to the bottom on open, or when a new message
                    // arrived (not for a plain edit/refresh).
                    if prev_len == 0 || self.messages.len() > prev_len {
                        self.scroll_to_bottom = true;
                    }
                    self.resolve_pending_jump();
                    self.invalidate_layout();
                }
            }
            UiMessage::NewMessage {
                chat_id,
                id,
                text,
                date,
                out,
                photo,
                doc,
                sticker,
                reply_to,
                forwarded_from,
                sender_name,
                sender_id,
            } => {
                // Open chat? Merge the message (dedupe the optimistic local
                // send, which is tagged id=0) into the full history, then
                // mirror it into the topic-filtered view when it belongs.
                if self.open_chat == Some(chat_id) {
                    self.loading = false;
                    // Merge into the full history first; the topic-filtered
                    // view then mirrors the stored row when it belongs to
                    // the selected thread.
                    let row = MsgRow {
                        id,
                        text,
                        date,
                        out,
                        photo,
                        doc,
                        sticker,
                        reply_to,
                        forwarded_from,
                        sender_name,
                        sender_id,
                        ..MsgRow::text(0, "", 0, false)
                    };
                    Self::merge_new_message(&mut self.topic_all_messages, row);
                    let stored = self
                        .topic_all_messages
                        .iter()
                        .find(|m| m.id == id)
                        .cloned();
                    let in_view = self.topic_selected.is_none_or(|root| {
                        stored.as_ref().is_some_and(|m| Self::topic_in_thread(m, root))
                    });
                    if in_view {
                        if let Some(m) = stored {
                            Self::merge_new_message(&mut self.messages, m);
                        }
                    }
                    // Stickers stream their image separately: ask for any
                    // sticker row still missing its file (new arrivals and
                    // just-merged optimistic sends alike).
                    let missing = self
                        .topic_all_messages
                        .iter()
                        .filter(|m| m.id > 0 && m.sticker.is_some() && m.sticker_path.is_none())
                        .map(|m| m.id)
                        .collect::<Vec<_>>();
                    for mid in missing {
                        let _ = self.req_tx.send(Request::DownloadSticker {
                            chat_id,
                            msg_id: mid,
                        });
                    }
                    self.scroll_to_bottom = true;
                    if !out {
                        self.typing = false;
                    }
                    self.invalidate_layout();
                    return;
                }
                // Otherwise: update the list row (preview + unread only for
                // incoming messages).
                if let Some(row) = self.dialogs.iter_mut().find(|r| r.id == chat_id) {
                    row.subtitle = preview_text(&text, &photo, &doc, &sticker);
                    row.date = date;
                    if !out {
                        row.unread += 1;
                    }
                    self.refresh_dialog_short(chat_id);
                }
            }
            UiMessage::MessageEdited { chat_id, id, text, date } => {
                if self.open_chat == Some(chat_id) {
                    for m in &mut self.messages {
                        if m.id == id {
                            m.text = text.clone();
                            m.date = date;
                            break;
                        }
                    }
                    for m in &mut self.topic_all_messages {
                        if m.id == id {
                            m.text = text;
                            m.date = date;
                            break;
                        }
                    }
                    self.invalidate_layout();
                } else if let Some(row) = self.dialogs.iter_mut().find(|r| r.id == chat_id) {
                    row.subtitle = text;
                    self.refresh_dialog_short(chat_id);
                }
            }
            UiMessage::MessageDeleted { ids } => {
                self.messages.retain(|m| !ids.contains(&m.id));
                self.topic_all_messages.retain(|m| !ids.contains(&m.id));
                if self.editing.is_some_and(|e| ids.contains(&e)) {
                    self.editing = None;
                }
                if self.pinned_id.is_some_and(|p| ids.contains(&p)) {
                    self.pinned_id = None;
                }
                self.context_menu = None;
                self.invalidate_layout();
            }
            UiMessage::PinnedMessage { chat_id, msg_id } => {
                if self.open_chat == Some(chat_id) {
                    self.pinned_id = msg_id;
                    // Sync the per-row flags with the authoritative pin state.
                    for m in &mut self.messages {
                        let is_pin = msg_id == Some(m.id);
                        if m.pinned != is_pin {
                            m.pinned = is_pin;
                        }
                    }
                    self.invalidate_layout();
                }
            }
            UiMessage::ChatCreated { id } => {
                // The refreshed dialog list arrived first; jump into the new
                // chat right away.
                self.create_menu_open = false;
                self.open_chat(id);
            }
            UiMessage::ChatGone { id } => {
                if self.open_chat == Some(id) {
                    self.close_chat();
                }
                if self.row_menu == Some(id) {
                    self.row_menu = None;
                }
                if self.confirm_leave.map(|(_, cid)| cid) == Some(id) {
                    self.confirm_leave = None;
                }
            }
            UiMessage::PhotoReady { chat_id, msg_id, path } => {
                if self.open_chat == Some(chat_id) {
                    for m in &mut self.messages {
                        if m.id == msg_id {
                            m.photo_path = path;
                            break;
                        }
                    }
                    self.invalidate_layout();
                }
            }
            UiMessage::DocReady { chat_id, msg_id, path } => {
                if self.open_chat == Some(chat_id) {
                    let downloaded = path.clone();
                    for m in &mut self.messages {
                        if m.id == msg_id {
                            m.doc_path = downloaded.clone();
                            break;
                        }
                    }
                    if let Some(p) = path {
                        self.doc_downloaded(chat_id, msg_id, &p);
                    }
                    self.invalidate_layout();
                }
            }
            UiMessage::StickerPathReady { chat_id, msg_id, path } => {
                if self.open_chat == Some(chat_id) {
                    for m in &mut self.messages {
                        if m.id == msg_id {
                            m.sticker_path = path;
                            break;
                        }
                    }
                    self.invalidate_layout();
                }
            }
            UiMessage::StickerThumbReady { doc_id, path } => {
                if let Some(p) = path {
                    self.sticker_thumbs.insert(doc_id, p);
                }
            }
            UiMessage::StickerSets(sets) => {
                // Only refresh when the picker still wants them (stale
                // responses after a close are dropped).
                if self.sticker_picker_open || self.sticker_sets.is_empty() {
                    self.sticker_sets = sets;
                }
            }
            UiMessage::UploadProgress { chat_id, token, progress } => {                if self.open_chat != Some(chat_id) {
                    return;
                }
                for m in &mut self.messages {
                    if m.upload_token == Some(token) {
                        m.uploading = Some(progress.clamp(0.0, 1.0));
                        break;
                    }
                }
            }
            UiMessage::UploadDone { chat_id, token } => {
                if self.open_chat != Some(chat_id) {
                    return;
                }
                // Upload finished: hold at 100% until the server echo
                // replaces the row (dedup clears `uploading`).
                for m in &mut self.messages {
                    if m.upload_token == Some(token) && m.uploading.is_some() {
                        m.uploading = Some(1.0);
                        break;
                    }
                }
            }
            UiMessage::SearchResults { id, query, hits } => {
                // Guard races: only apply if the search UI is still open and
                // matches the response's mode/query.
                let mode_matches = match (&self.search_mode, id) {
                    (Some(SearchMode::Global), None) => true,
                    (Some(SearchMode::InChat), Some(chat_id)) => self.open_chat == Some(chat_id),
                    _ => false,
                };
                self.search_pending = false;
                if mode_matches && self.search_query == query {
                    self.search_hits = hits;
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
            UiMessage::ChatInfo(detail) => {
                // Stale guard: only apply while that chat is still open.
                if self.open_chat == Some(detail.id) {
                    self.chat_info = Some(detail);
                }
            }
            UiMessage::Participants(rows) => {
                // The payload carries no chat id; members are only requested
                // while the panel is open on the current chat.
                if self.info_open {
                    if let Some(k) = self.kick_confirm {
                        if !rows.iter().any(|p| p.id == k) {
                            self.kick_confirm = None;
                        }
                    }
                    self.participants = rows;
                }
            }
            UiMessage::ParticipantKicked { user_id } => {
                self.participants.retain(|p| p.id != user_id);
                if self.kick_confirm == Some(user_id) {
                    self.kick_confirm = None;
                }
            }
            UiMessage::MemberUpdated { chat_id, user_id } => {
                // An admin action landed server-side: refresh the member
                // list of the open chat.
                if self.open_chat == Some(chat_id) && self.info_open {
                    if self.admin_confirm.is_some_and(|(_, u)| u == user_id) {
                        self.admin_confirm = None;
                    }
                    if self.admin_menu == Some(user_id) {
                        self.admin_menu = None;
                    }
                    let _ = self.req_tx.send(Request::GetParticipants { id: chat_id });
                }
            }
            UiMessage::MemberSelfId { id } => self.admin_self_id = Some(id),
            UiMessage::Topics { id, is_forum, topics } => {
                if self.open_chat == Some(id) {
                    self.topic_is_forum = is_forum;
                    self.topic_topics = topics;
                }
            }
            UiMessage::TopicCreated { chat_id, topic } => {
                if self.open_chat == Some(chat_id) {
                    // `Topics` usually carried the refreshed list already;
                    // append defensively (idempotent).
                    if !self.topic_topics.iter().any(|t| t.id == topic.id) {
                        self.topic_topics.push(topic.clone());
                    }
                    self.topic_creating = false;
                    self.topic_title.clear();
                    // Jump into the fresh thread so posting continues there.
                    self.topic_select(Some(topic.root_msg_id));
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
                self.qr_cancelled();
                self.login_qr_selected = false;
                self.status = if self.dialogs.is_empty() {
                    format!("Signed in as {name}")
                } else {
                    format!("Signed in as {name} — {} chats", self.dialogs.len())
                };
            }
            UiMessage::QrCodeReady { path } => self.qr_code_ready(path),
            UiMessage::QrScanConfirmed => self.qr_scan_confirmed(),
            UiMessage::QrLoginFailed { error } => self.qr_failed(error),
            UiMessage::MyProfile(p) => {
                // Seed the edit inputs only while they're untouched, so a
                // profile echo never clobbers text being typed.
                let untouched = self.edit_name.is_empty()
                    && self.edit_last_name.is_empty()
                    && self.edit_bio.is_empty();
                if untouched {
                    // First name = display name with the last-name suffix
                    // removed (multi-word last names stay intact).
                    let first = match &p.last_name {
                        Some(last) => p
                            .name
                            .strip_suffix(last.as_str())
                            .map(|s| s.trim_end())
                            .unwrap_or(p.name.trim_end()),
                        None => p.name.trim_end(),
                    };
                    self.edit_name = first.to_string();
                    self.edit_last_name = p.last_name.clone().unwrap_or_default();
                    self.edit_bio = p.bio.clone().unwrap_or_default();
                }
                self.my_profile = Some(p);
            }
            UiMessage::Sessions(list) => {
                // Stale guard: only apply while the panel still wants them.
                if self.settings_open || self.sessions.is_empty() {
                    self.sessions = list;
                }
            }
            UiMessage::SessionRevoked { hash } => {
                self.sessions.retain(|s| s.hash != hash);
            }
            UiMessage::CacheCleared { bytes } => {
                self.cache_bytes = Some(bytes);
                self.confirm_clear_cache = false;
            }
            UiMessage::LoggedOut => self.reset_to_login(),
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
        let Some(m) = self.messages.get(row) else {
            return;
        };
        let msg: Option<(i32, Option<crate::bridge::DocKind>, Option<String>)> = Some((
            m.id,
            m.doc.as_ref().map(|d| d.kind),
            m.doc_path.clone(),
        ));
        let photo_viewer = m.photo_path.clone();
        // Voice note: toggling playback needs the path; the doc is downloaded
        // on first click (see the doc branch below).
        if let Some((msg_id, Some(crate::bridge::DocKind::Audio { voice: true }), Some(path))) = msg
        {
            let chat_id = self.open_chat.unwrap_or(0);
            self.voice_click(chat_id, msg_id, &path);
            return;
        }
        if let Some((msg_id, Some(kind), path)) = msg {
            match path {
                Some(path) => {
                    // Non-voice documents open with the system opener.
                    if !matches!(kind, crate::bridge::DocKind::Audio { voice: true }) {
                        self.open_file = Some(path);
                    }
                }
                None => {
                    let _ = self.req_tx.send(Request::DownloadDoc {
                        chat_id: self.open_chat.unwrap_or(0),
                        msg_id,
                    });
                }
            }
            return;
        }
        // Photo attached, and the row isn't a doc.
        if let Some(path) = photo_viewer {
            if self
                .messages
                .get(row)
                .is_some_and(|m| m.doc.is_none())
            {
                self.viewer = Some(path);
            }
        }
    }

    /// A voice-note bubble was clicked: play (download handled earlier),
    /// pause, resume or stop the currently-playing note. Returns the path to
    /// play as a shell-side action only when playback should start fresh
    /// (the click on a non-cached voice note triggers a download instead).
    pub fn voice_click(&mut self, chat_id: i64, msg_id: i32, path: &str) {
        if self.playing_voice.as_ref() == Some(&(chat_id, msg_id, path.to_string())) {
            // Simulated playback (external player): click stops it.
            if self.voice_sim {
                self.playing_voice = None;
                self.voice_playing = false;
                self.voice_sim = false;
                self.voice_sim_started = None;
                self.voice_elapsed = 0.0;
                return;
            }
            // Same note: toggle pause/play.
            if crate::audio::is_active() {
                if self.voice_playing {
                    crate::audio::pause();
                } else {
                    crate::audio::resume();
                }
                self.voice_playing = !self.voice_playing;
            } else {
                // Finished on its own; restart.
                self.playing_voice = Some((chat_id, msg_id, path.to_string()));
                self.voice_elapsed = 0.0;
                self.voice_playing = crate::audio::play(path);
            }
            return;
        }
        // Different note (or none): switch.
        self.playing_voice = Some((chat_id, msg_id, path.to_string()));
        self.voice_elapsed = 0.0;
        self.voice_duration = self.note_duration(chat_id, msg_id);
        self.voice_playing = crate::audio::play(path);
        self.fallback_to_system_player_if_undecodable(path);
    }

    /// Duration of the note (chat_id, msg_id) per its message card, if known.
    fn note_duration(&self, chat_id: i64, msg_id: i32) -> f32 {
        self.messages
            .iter()
            .find(|m| m.id == msg_id && self.open_chat == Some(chat_id))
            .and_then(|m| m.doc.as_ref().and_then(|d| d.duration))
            .unwrap_or(0.0) as f32
    }

    /// Inline playback failed and the payload is Ogg (Opus voice note):
    /// hand it to the system player instead of doing nothing. Telegram
    /// voices are Opus-in-Ogg; inline playback needs the ffmpeg transcode
    /// performed at download time (missing/broken ffmpeg degrades here).
    /// Detection is by magic bytes — wire names are often extension-less.
    fn fallback_to_system_player_if_undecodable(&mut self, path: &str) {
        let is_ogg = std::fs::read(path)
            .map(|b| b.starts_with(b"OggS"))
            .unwrap_or(false);
        if !self.voice_playing && is_ogg {
            self.open_file = Some(path.to_string());
            self.status = "Opening with the system player…".to_string();
            // Simulate the progress bar over the known duration: we cannot
            // poll an external process. A second click stops the simulation.
            self.voice_duration = match self.playing_voice.as_ref() {
                Some((cid, mid, _)) => self.note_duration(*cid, *mid),
                None => 0.0,
            };
            self.voice_sim = true;
            self.voice_playing = true;
            self.voice_sim_started = Some(std::time::Instant::now());
            self.voice_elapsed = 0.0;
        }
    }

    /// Resolves a `DocReady` for a voice note into playable state (and starts
    /// playback automatically, matching Telegram: click a voice note → it
    /// downloads + plays).
    fn doc_downloaded(&mut self, chat_id: i64, msg_id: i32, path: &str) {
        let is_voice = self
            .messages
            .iter()
            .find(|m| m.id == msg_id)
            .is_some_and(|m| {
                m.doc
                    .as_ref()
                    .is_some_and(|d| matches!(d.kind, crate::bridge::DocKind::Audio { voice: true }))
            });
        if !is_voice {
            return;
        }
        // The voice note was just downloaded: start playing it immediately.
        self.playing_voice = Some((chat_id, msg_id, path.to_string()));
        self.voice_elapsed = 0.0;
        self.voice_playing = crate::audio::play(path);
        self.fallback_to_system_player_if_undecodable(path);
    }

    /// Right-click over a message row: open the context menu (any message;
    /// edit stays restricted to outgoing rows).
    pub fn open_context(&mut self, row: usize) {
        if self.messages.get(row).is_none() {
            return;
        }
        self.context_menu = Some(ContextMenu { row });
        self.invalidate_layout();
    }

    /// True when the context menu's target can be edited (own text messages).
    pub fn context_can_edit(&self) -> bool {
        self.context_menu
            .and_then(|c| self.messages.get(c.row))
            .is_some_and(|m| m.out && m.doc.is_none() && m.sticker.is_none())
    }

    /// Dismisses the context menu (a click outside, or opening another one).
    /// Also closes the info panel's member admin menu (same dismissal
    /// contract: click-outside).
    pub fn dismiss_menu(&mut self) {
        self.admin_menu = None;
        if self.context_menu.is_none() {
            return;
        }
        self.context_menu = None;
        self.invalidate_layout();
    }

    /// Escape: closes the topmost overlay — sticker picker, kick
    /// confirmation, info panel, creation modal, chat-row menu, leave
    /// confirmation — then the context menu, cancels editing or the reply.
    pub fn escape(&mut self) {
        // The search UI is active: it hides the chat pane, so Escape (and the
        // back handler) close it before touching the message rows.
        if self.search_open() {
            self.close_search();
            return;
        }
        // The inline create-topic field (forum chips bar) closes first:
        // it is the innermost input of the conversation body.
        if self.topic_creating {
            self.topic_cancel_create();
            return;
        }
        // An armed destructive confirmation is cancelled first.
        if self.confirm_logout {
            self.cancel_logout();
            return;
        }
        if self.sticker_picker_open {
            self.close_sticker_picker();
            return;
        }
        if self.kick_confirm.take().is_some() {
            return; // only cancel the confirmation
        }
        if self.admin_confirm.take().is_some() {
            return; // only cancel the admin confirmation
        }
        if self.admin_menu.take().is_some() {
            return; // only close the admin menu
        }
        if self.info_open {
            self.close_info();
            return;
        }
        if self.create_dialog.take().is_some() {
            self.create_title.clear();
            self.create_about.clear();
            self.member_pick.clear();
            self.invalidate_layout();
            return;
        }
        if self.row_menu.take().is_some() {
            return;
        }
        if self.confirm_leave.take().is_some() {
            return;
        }
        // Emoji picker closes before the context menu (topmost-composer-first).
        if self.emoji_panel_open {
            self.close_emoji_panel();
            return;
        }
        // Settings sheet closes before the context menu (global overlay).
        if self.settings_open {
            self.close_settings();
            return;
        }
        self.context_menu = None;
        if self.editing.is_some() {
            self.editing = None;
            self.composer.clear();
        }
        if self.reply_target.take().is_some() {
            // Reply cancelled; keep the typed text.
        }
        if self.forward_pick.take().is_some() {
            return; // overlay closed; skip a redundant layout invalidation
        }
        self.invalidate_layout();
    }

    // -----------------------------------------------------------------
    // Emoji picker (composer)
    // -----------------------------------------------------------------

    /// Shows/hides the emoji panel above the composer.
    pub fn toggle_emoji_panel(&mut self) {
        self.emoji_panel_open = !self.emoji_panel_open;
        if self.emoji_panel_open {
            // One composer panel at a time.
            self.sticker_picker_open = false;
        }
    }

    /// Hides the emoji panel (Escape or click outside).
    pub fn close_emoji_panel(&mut self) {
        self.emoji_panel_open = false;
    }

    /// An emoji was picked in the panel: append it to the composer text and
    /// move it to the front of the recents (deduplicated, capped), persisting
    /// them when UI persistence is on. The composer keeps its focus; sending
    /// stays an explicit action.
    pub fn pick_emoji(&mut self, emoji: String) {
        if emoji.is_empty() {
            return;
        }
        self.composer.push_str(&emoji);
        self.emoji_recents.retain(|e| *e != emoji);
        self.emoji_recents.insert(0, emoji);
        self.emoji_recents.truncate(EMOJI_RECENTS_MAX);
        if self.persist_ui {
            write_emoji_recents_at(&emoji_recents_path(), &self.emoji_recents);
        }
    }

    /// Click on the context menu's "Reply" item.
    pub fn context_reply(&mut self) {
        let Some(menu) = self.context_menu.take() else {
            return;
        };
        let Some(m) = self.messages.get(menu.row) else {
            return;
        };
        let snippet = preview_text(&m.text, &m.photo, &m.doc, &m.sticker);
        self.reply_target = Some(ReplyTarget {
            msg_id: m.id,
            snippet,
        });
        // Replying is a composer state change, not a row-height change.
    }

    /// Click on the context menu's "Forward" item: opens the chat picker.
    pub fn context_forward(&mut self) {
        let Some(menu) = self.context_menu.take() else {
            return;
        };
        if self.messages.get(menu.row).is_none() {
            return;
        }
        self.forward_pick = Some(menu.row);
        self.invalidate_layout();
    }

    /// Confirms a forward into `to_chat` (chat-picker selection).
    pub fn forward_to(&mut self, to_chat: i64) {
        let Some(row) = self.forward_pick.take() else {
            return;
        };
        let from_chat = match self.open_chat {
            Some(id) => id,
            None => return,
        };
        if let Some(m) = self.messages.get(row) {
            let _ = self.req_tx.send(Request::ForwardMessage {
                from_chat,
                msg_id: m.id,
                to_chat,
            });
        }
        self.invalidate_layout();
    }

    /// Click on the context menu's "Edit" item.
    pub fn context_edit(&mut self) {
        if let Some(menu) = self.context_menu.take() {
            self.invalidate_layout();
            if let Some(m) = self.messages.get(menu.row) {
                self.editing = Some(m.id);
                self.composer = m.text.clone();
            }
        }
    }

    /// Click on the context menu's "Copy" item: returns the copied text (or
    /// `None`), which the caller writes to the system clipboard.
    pub fn context_copy(&mut self) -> Option<String> {
        let menu = self.context_menu.take()?;
        self.invalidate_layout();
        let m = self.messages.get(menu.row)?;
        let text = if m.text.is_empty() {
            // Photo-only message: nothing meaningful to copy.
            return None;
        } else {
            m.text.clone()
        };
        Some(text)
    }

    /// Click on the context menu's "Delete" item.
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
        if self.pinned_id == Some(m.id) {
            self.pinned_id = None;
        }
        self.invalidate_layout();
    }

    /// Click on the context menu's "Pin"/"Unpin" item.
    pub fn context_pin(&mut self) {
        let Some(menu) = self.context_menu.take() else {
            return;
        };
        let Some(m) = self.messages.get(menu.row).cloned() else {
            return;
        };
        let pin = !m.pinned;
        let _ = self.req_tx.send(Request::PinMessage {
            id: self.open_chat.unwrap_or(0),
            msg_id: m.id,
            pin,
        });
        // Optimistic: the authoritative `PinnedMessage` echo confirms.
        self.pinned_id = pin.then_some(m.id);
        for x in &mut self.messages {
            if x.id == m.id {
                x.pinned = pin;
            } else if pin {
                // Only one banner at a time (Telegram's latest-pin behaviour).
                x.pinned = false;
            }
        }
        self.invalidate_layout();
    }

    /// Whether the context-menu row is currently pinned (label switch).
    pub fn context_row_pinned(&self) -> bool {
        self.context_menu
            .and_then(|c| self.messages.get(c.row))
            .is_some_and(|m| m.pinned)
    }

    /// Click on the pinned banner: scroll to the pinned message.
    pub fn jump_to_pinned(&mut self) {
        let Some(pinned) = self.pinned_id else {
            return;
        };
        self.context_menu = None;
        if let Some(y) = self.message_top_of(pinned) {
            self.scroll_to_bottom = false;
            self.scroll_target = Some(y);
        } else {
            // Cache cold or message outside the loaded window: retry once the
            // layout is rebuilt (same mechanism as search jumps).
            self.pending_jump_id = Some(pinned);
            self.invalidate_layout();
        }
    }

    // -------------------------------------------------------------------
    // Group/channel creation + chat management
    // -------------------------------------------------------------------

    /// Toggles the header "+" picker menu (New Group / New Channel).
    pub fn toggle_create_menu(&mut self) {
        self.create_menu_open = !self.create_menu_open;
        self.row_menu = None;
    }

    /// Opens the creation modal pre-filled with the known dialogs as the
    /// checkable member list.
    pub fn open_create(&mut self, kind: CreateKind) {
        self.create_menu_open = false;
        self.create_dialog = Some(kind);
        self.create_title.clear();
        self.create_about.clear();
        // Contacts: every other known dialog (users and chats alike),
        // unchecked by default.
        self.member_pick = self
            .dialogs
            .iter()
            .map(|d| (d.id, d.title.clone(), false))
            .collect();
    }

    /// Closes the creation modal and drops its draft input.
    pub fn cancel_create(&mut self) {
        if self.create_dialog.take().is_none() {
            return;
        }
        self.create_title.clear();
        self.create_about.clear();
        self.member_pick.clear();
    }

    /// Toggles a contact's checkbox in the member picker.
    pub fn toggle_member(&mut self, idx: usize) {
        if let Some(entry) = self.member_pick.get_mut(idx) {
            entry.2 = !entry.2;
        }
    }

    /// Submits the creation modal: sends the create request (groups carry the
    /// checked members) and closes the modal. Empty titles are ignored.
    pub fn submit_create(&mut self) {
        let Some(kind) = self.create_dialog else { return };
        let title = self.create_title.trim().to_string();
        if title.is_empty() {
            return;
        }
        let about = self.create_about.trim().to_string();
        let members: Vec<i64> = self
            .member_pick
            .iter()
            .filter(|(_, _, on)| *on)
            .map(|(id, _, _)| *id)
            .collect();
        let _ = self.req_tx.send(Request::CreateChannel {
            title,
            about,
            megagroup: kind == CreateKind::Group,
            members,
        });
        self.cancel_create();
    }

    /// Right-click over a chat-list row: open its Leave/Delete mini menu.
    pub fn open_row_menu(&mut self, chat_id: i64) {
        self.row_menu = Some(chat_id);
        self.create_menu_open = false;
    }

    /// Dismisses the chat-row mini menu (click elsewhere).
    pub fn dismiss_row_menu(&mut self) {
        self.row_menu = None;
    }

    /// Asks for confirmation before leaving/deleting a chat (closes the row
    /// menu that raised it).
    pub fn ask_confirm(&mut self, kind: ConfirmKind, chat_id: i64) {
        self.row_menu = None;
        self.confirm_leave = Some((kind, chat_id));
    }

    /// Cancels the pending confirmation.
    pub fn cancel_confirm(&mut self) {
        self.confirm_leave = None;
    }

    /// Confirms the pending leave/delete: sends the request and closes the
    /// dialog. The list itself refreshes when the network answers with
    /// `Dialogs` + `ChatGone`.
    pub fn confirm_yes(&mut self) {
        let Some((kind, id)) = self.confirm_leave.take() else { return };
        let req = match kind {
            ConfirmKind::Leave => Request::LeaveChat { id },
            ConfirmKind::Delete => Request::DeleteChat { id },
        };
        let _ = self.req_tx.send(req);
    }

    // Info panel (chat details + members)
    // -------------------------------------------------------------------

    /// Opens the right-hand info panel and fetches this chat's details +
    /// member list. Re-opening on the same chat refreshes both.
    pub fn open_info(&mut self) {
        self.info_open = true;
        self.chat_info = None;
        self.participants.clear();
        self.kick_confirm = None;
        self.admin_menu = None;
        self.admin_confirm = None;
        if let Some(id) = self.open_chat {
            let _ = self.req_tx.send(Request::GetChatInfo { id });
            let _ = self.req_tx.send(Request::GetParticipants { id });
        }
    }

    /// Closes the info panel (✕ button, header click or Escape).
    pub fn close_info(&mut self) {
        self.info_open = false;
        self.kick_confirm = None;
        self.admin_menu = None;
        self.admin_confirm = None;
    }

    /// The Mute button: flips the local flag and pushes it server-side.
    pub fn toggle_mute(&mut self) {
        let Some(id) = self.open_chat else { return };
        self.muted = !self.muted;
        let _ = self.req_tx.send(Request::SetMuted { id, muted: self.muted });
    }

    /// "Remove" was clicked on a member row: arm the inline confirmation.
    pub fn kick(&mut self, user_id: i64) {
        if self.participants.iter().any(|p| p.id == user_id) {
            self.kick_confirm = Some(user_id);
        }
    }

    /// Confirms a pending kick: sends the request; the authoritative
    /// `ParticipantKicked` echo removes the row.
    pub fn kick_confirmed(&mut self) {
        let Some(user_id) = self.kick_confirm.take() else {
            return;
        };
        let Some(id) = self.open_chat else {
            return;
        };
        let _ = self.req_tx.send(Request::KickParticipant { id, user_id });
    }

    // Admin tools (member rows of the info panel)
    // -------------------------------------------------------------------

    /// Right-click on a member row: opens its admin menu — unless the row
    /// is the group owner or yourself (no menu items there).
    pub fn admin_open_menu(&mut self, user_id: i64) {
        let Some(p) = self.participants.iter().find(|p| p.id == user_id) else {
            return;
        };
        let is_self = self.admin_self_id == Some(user_id);
        if admin_menu_items(p.role, is_self).is_empty() {
            return;
        }
        self.admin_menu = Some(user_id);
        self.admin_confirm = None;
        self.kick_confirm = None;
    }

    /// Applies a menu action: promote/demote fire immediately; the
    /// destructive ban/remove arm the inline Yes/No confirmation instead.
    pub fn admin_action(&mut self, user_id: i64, action: AdminAction) {
        match action {
            AdminAction::Promote | AdminAction::Demote => {
                self.admin_menu = None;
                let Some(id) = self.open_chat else {
                    return;
                };
                let req = match action {
                    AdminAction::Promote => Request::AdminPromote { id, user_id },
                    _ => Request::AdminDemote { id, user_id },
                };
                let _ = self.req_tx.send(req);
            }
            AdminAction::Ban | AdminAction::Remove => {
                if self.participants.iter().any(|p| p.id == user_id) {
                    let kind = if action == AdminAction::Ban {
                        AdminConfirm::Ban
                    } else {
                        AdminConfirm::Remove
                    };
                    self.admin_confirm = Some((kind, user_id));
                }
                self.admin_menu = None;
            }
        }
    }

    /// Confirms the pending ban/remove: sends the request; the
    /// `MemberUpdated` echo refreshes the member list.
    pub fn admin_confirmed(&mut self) {
        let Some((kind, user_id)) = self.admin_confirm.take() else {
            return;
        };
        let Some(id) = self.open_chat else {
            return;
        };
        let _ = self.req_tx.send(match kind {
            AdminConfirm::Ban => Request::AdminBan { id, user_id, kick_only: false },
            AdminConfirm::Remove => Request::AdminBan { id, user_id, kick_only: true },
        });
    }

    // -----------------------------------------------------------------
    // Forum topics (chips bar of forum supergroups; namespaced `topic_`)
    // -----------------------------------------------------------------

    /// Thread membership, slice-1 style: the root row itself, or any message
    /// whose reply header anchors it to the topic root. Server-side, every
    /// message of a thread carries `reply_to_msg_id == topic id` (in-thread
    /// replies add `reply_to_top_id` on top), and `MsgRow.reply_to` mirrors
    /// exactly that field.
    fn topic_in_thread(m: &MsgRow, root: i32) -> bool {
        m.id == root || m.reply_to == Some(root)
    }

    /// The visible subset of the history for a topic selection (`None` =
    /// everything).
    fn topic_view(all: &[MsgRow], selected: Option<i32>) -> Vec<MsgRow> {
        match selected {
            None => all.to_vec(),
            Some(root) => all
                .iter()
                .filter(|m| Self::topic_in_thread(m, root))
                .cloned()
                .collect(),
        }
    }

    /// Merges an incoming message (as a fresh [`MsgRow`], paths unset) into
    /// a row store: updates a known id, dedupes the optimistic `id == 0`
    /// send, or appends.
    fn merge_new_message(rows: &mut Vec<MsgRow>, row: MsgRow) {
        if let Some(m) = rows.iter_mut().find(|m| m.id == row.id) {
            m.text = row.text;
            m.out = row.out;
            m.photo = row.photo;
            m.doc = row.doc;
            m.sticker = row.sticker;
            m.reply_to = row.reply_to;
            m.forwarded_from = row.forwarded_from;
            m.uploading = None;
        } else if let Some(i) = rows.iter().position(|m| {
            m.id == 0
                && (m.text == row.text || (m.uploading.is_some() && row.out))
        }) {
            let m = &mut rows[i];
            let optimistic = std::mem::replace(m, row);
            m.photo = optimistic.photo.or(m.photo.take());
            m.doc = optimistic.doc.or(m.doc.take());
            m.sticker = optimistic.sticker.or(m.sticker.take());
            m.photo_path = optimistic.photo_path;
            m.doc_path = optimistic.doc_path;
            m.sticker_path = optimistic.sticker_path;
            m.uploading = None;
        } else {
            rows.push(row);
        }
    }

    /// Chips-bar visibility: forum chats with a loaded topic list only
    /// (non-forum chats never get a bar).
    pub fn topic_bar_visible(&self) -> bool {
        self.topic_is_forum && !self.topic_topics.is_empty()
    }

    /// Rebuilds `messages` from the full history for the current selection.
    fn apply_topic_filter(&mut self) {
        self.messages = Self::topic_view(&self.topic_all_messages, self.topic_selected);
        self.invalidate_layout();
    }

    /// A chip was picked (`None` = the "All messages" chip).
    pub fn topic_select(&mut self, root: Option<i32>) {
        if self.topic_selected == root {
            return;
        }
        self.topic_selected = root;
        self.apply_topic_filter();
        self.scroll_to_bottom = true;
    }

    /// Opens the inline create-topic field (the "+" chip).
    pub fn topic_open_create(&mut self) {
        self.topic_creating = true;
        self.topic_title.clear();
    }

    /// Closes the inline create-topic field (Escape / ✕).
    pub fn topic_cancel_create(&mut self) {
        self.topic_creating = false;
        self.topic_title.clear();
    }

    /// Submits the create-topic field: non-empty titles only (validation);
    /// the field closes when the server echo arrives (`TopicCreated`).
    pub fn topic_submit_create(&mut self) {
        let title = self.topic_title.trim().to_string();
        if title.is_empty() {
            return;
        }
        if let Some(id) = self.open_chat {
            let _ = self.req_tx.send(Request::CreateTopic { id, title });
        }
        self.topic_title.clear();
    }

    /// Drops every conversation-scoped topic bit (chat switch / logout).
    fn topic_reset(&mut self) {
        self.topic_topics.clear();
        self.topic_is_forum = false;
        self.topic_selected = None;
        self.topic_creating = false;
        self.topic_title.clear();
        self.topic_all_messages.clear();
    }

    /// The @username line was clicked: returns the text to copy.
    pub fn copy_username(&mut self) -> Option<String> {
        let username = self.chat_info.as_ref()?.username.as_ref()?;
        Some(format!("@{username}"))
    }

    // -----------------------------------------------------------------
    // Settings panel (global overlay; NOT reset when switching chats)
    // -----------------------------------------------------------------

    /// Opens the settings sheet: fetches the own profile (once) and the
    /// active sessions, and measures the media cache on disk.
    pub fn open_settings(&mut self) {
        self.settings_open = true;
        self.settings_tab = SettingsTab::Profile;
        self.confirm_clear_cache = false;
        if self.my_profile.is_none() {
            let _ = self.req_tx.send(Request::GetMe);
        }
        let _ = self.req_tx.send(Request::GetSessions);
        self.cache_bytes = Some(crate::network::cache_bytes_on_disk());
    }

    /// Closes the settings sheet (✕ or Escape).
    pub fn close_settings(&mut self) {
        self.settings_open = false;
        self.confirm_clear_cache = false;
    }

    /// The header gear button: toggle the sheet.
    pub fn toggle_settings(&mut self) {
        if self.settings_open {
            self.close_settings();
        } else {
            self.open_settings();
        }
    }

    /// Switches the active settings tab.
    pub fn set_settings_tab(&mut self, tab: SettingsTab) {
        self.settings_tab = tab;
    }

    /// Submits the profile edit form (First name / Last name / Bio). Telegram
    /// requires a non-empty first name; the refreshed profile arrives as a
    /// `MyProfile` echo. An emptied last name clears it server-side.
    pub fn submit_profile_edit(&mut self) {
        let first = self.edit_name.trim();
        if first.is_empty() {
            return;
        }
        let last = self.edit_last_name.trim().to_string();
        let bio = self.edit_bio.trim().to_string();
        let _ = self.req_tx.send(Request::UpdateProfile {
            first_name: Some(first.to_string()),
            last_name: Some(last),
            bio: Some(bio),
        });
    }

    /// "Clear cache" pressed: arms the inline Yes/No confirmation.
    pub fn ask_clear_cache(&mut self) {
        self.confirm_clear_cache = true;
    }

    /// Cancels the pending cache-clear confirmation.
    pub fn cancel_clear_cache(&mut self) {
        self.confirm_clear_cache = false;
    }

    /// Confirms the cache wipe: sends `ClearCache`; the `CacheCleared` echo
    /// refreshes the displayed size.
    pub fn confirm_clear_cache_now(&mut self) {
        if !self.confirm_clear_cache {
            return;
        }
        self.confirm_clear_cache = false;
        let _ = self.req_tx.send(Request::ClearCache);
    }

    /// "Log out" pressed: arms the inline Yes/No confirmation.
    pub fn ask_logout(&mut self) {
        self.confirm_logout = true;
    }

    /// Cancels the pending logout confirmation.
    pub fn cancel_logout(&mut self) {
        self.confirm_logout = false;
    }

    /// Confirms the logout: sends `LogOut` once; the authoritative
    /// `LoggedOut` echo performs the UI reset ([`Self::reset_to_login`]).
    pub fn confirm_logout_now(&mut self) {
        if !self.confirm_logout {
            return;
        }
        self.confirm_logout = false;
        let _ = self.req_tx.send(Request::LogOut);
    }

    /// Handles `UiMessage::LoggedOut`: drops every conversation-scoped bit of
    /// state and goes back to the sign-in screen. Theme and emoji recents are
    /// kept on purpose.
    pub fn reset_to_login(&mut self) {
        // Sign-in flow.
        self.authenticated = false;
        self.connecting = false;
        self.login_step = LoginStep::Phone;
        self.login_input.clear();
        self.login_error = false;
        self.status.clear();
        // Conversations.
        self.dialogs.clear();
        self.dialog_short.clear();
        self.messages.clear();
        self.open_chat = None;
        self.chat_title.clear();
        self.loading = false;
        self.typing = false;
        self.scroll_offset = 0.0;
        self.dialog_scroll_offset = 0.0;
        self.scroll_to_bottom = false;
        // Overlays + one-shot interactions.
        self.context_menu = None;
        self.editing = None;
        self.composer.clear();
        self.viewer = None;
        self.reply_target = None;
        self.forward_pick = None;
        self.info_open = false;
        self.chat_info = None;
        self.participants.clear();
        self.kick_confirm = None;
        self.admin_menu = None;
        self.admin_confirm = None;
        self.muted = false;
        self.topic_reset();
        self.pinned_id = None;
        self.create_menu_open = false;
        self.create_dialog = None;
        self.create_title.clear();
        self.create_about.clear();
        self.member_pick.clear();
        self.row_menu = None;
        self.confirm_leave = None;
        self.confirm_clear_cache = false;
        self.sticker_picker_open = false;
        self.emoji_panel_open = false;
        self.open_file = None;
        self.playing_voice = None;
        self.voice_playing = false;
        // Search UI.
        self.search_mode = None;
        self.search_query.clear();
        self.search_hits.clear();
        self.search_pending = false;
        self.scroll_target = None;
        // Settings panel (profile/sessions inputs).
        self.settings_open = false;
        self.settings_tab = SettingsTab::Profile;
        self.my_profile = None;
        self.edit_name.clear();
        self.edit_last_name.clear();
        self.edit_bio.clear();
        self.sessions.clear();
        self.cache_bytes = None;
        self.confirm_logout = false;
        // The cached message-list metrics are stale now.
        self.invalidate_layout();
    }

    /// "Terminate" on a session row: the authoritative `SessionRevoked` echo
    /// removes the row locally.
    pub fn revoke_session(&mut self, hash: i64) {
        let _ = self.req_tx.send(Request::RevokeSession { hash });
    }

    /// The notifications On/Off button: flips the shared flag consulted by
    /// the network runtime and persists it when UI persistence is on.
    pub fn toggle_notifications(&mut self) {
        use std::sync::atomic::Ordering;
        self.notifications_on = !self.notifications_on;
        self.notify_pref
            .0
            .store(self.notifications_on, Ordering::SeqCst);
        if self.persist_ui {
            write_notifications_at(&notifications_path(), self.notifications_on);
        }
    }

    // -----------------------------------------------------------------
    // Stickers (picker + sending)
    // -----------------------------------------------------------------

    /// Toggles the sticker picker panel. Opening it with no packs loaded yet
    /// fires a single `GetStickerSets` request.
    pub fn toggle_sticker_picker(&mut self) {
        self.sticker_picker_open = !self.sticker_picker_open;
        if self.sticker_picker_open {
            // One composer panel at a time.
            self.emoji_panel_open = false;
            if self.sticker_sets.is_empty() {
                let _ = self.req_tx.send(Request::GetStickerSets);
            }
        }
    }

    /// Closes the sticker picker panel.
    pub fn close_sticker_picker(&mut self) {
        self.sticker_picker_open = false;
    }

    /// A picker sticker was clicked: sends it optimistically to the open chat
    /// and closes the panel. The server echo merges into the local row; the
    /// image streams in via `StickerPathReady`.
    pub fn send_sticker(&mut self, set_idx: usize, doc_idx: usize) {
        let Some(id) = self.open_chat else { return };
        let Some((doc_id, access_hash, alt)) =
            self.sticker_sets.get(set_idx).and_then(|s| s.docs.get(doc_idx)).cloned()
        else {
            return;
        };
        // Optimistic local row (id = 0, merged by the echo).
        self.messages.push(MsgRow {
            sticker: Some(StickerMeta { alt }),
            ..MsgRow::text(0, String::new(), 0, true)
        });
        let _ = self.req_tx.send(Request::SendSticker { id, doc_id, access_hash });
        self.sticker_picker_open = false;
        self.scroll_to_bottom = true;
        self.invalidate_layout();
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
            let reply_to = self.reply_target.take().map(|r| r.msg_id);
            // Optimistic local send: the id and date are unknown (incoming
            // updates will provide them). Kept in the full history and in
            // the topic-filtered view alike.
            let row = MsgRow {
                reply_to,
                ..MsgRow::text(0, text.clone(), 0, true)
            };
            self.topic_all_messages.push(row.clone());
            self.messages.push(row);
            match self.topic_selected {
                // Plain post into the selected topic thread. Replies to a
                // specific message keep the plain path: the server lands a
                // reply in the thread of its target.
                Some(root) if reply_to.is_none() => {
                    let _ = self
                        .req_tx
                        .send(Request::SendTopicMessage { id, text, topic_root: root });
                }
                _ => {
                    let _ = self.req_tx.send(Request::SendMessage { id, text, reply_to });
                }
            }
            self.typing = false;
        }
        self.composer.clear();
        self.scroll_to_bottom = true;
        self.invalidate_layout();
    }

    // -----------------------------------------------------------------
    // QR sign-in pane (namespaced `qr_`; see `QrStage`)
    // -----------------------------------------------------------------

    /// Switches the sign-in screen between the phone form and the QR pane.
    pub fn toggle_login_screen(&mut self) {
        self.login_qr_selected = !self.login_qr_selected;
        if !self.login_qr_selected {
            self.qr_cancelled();
        }
        self.login_error = false;
    }

    /// A QR session was requested; clear stale data and wait for the code.
    pub fn qr_started(&mut self) {
        self.qr_stage = QrStage::AwaitingScan;
        self.qr_png_path = None;
        self.qr_error = None;
    }

    /// A fresh QR PNG is on disk: show it.
    pub fn qr_code_ready(&mut self, path: String) {
        self.qr_png_path = Some(path);
        self.qr_stage = QrStage::AwaitingScan;
        self.qr_error = None;
    }

    /// The phone scanned the code; waiting for server-side confirmation.
    pub fn qr_scan_confirmed(&mut self) {
        self.qr_stage = QrStage::ScanConfirmed;
    }

    /// The QR session failed: drop the code and surface the error.
    pub fn qr_failed(&mut self, error: String) {
        self.qr_stage = QrStage::Idle;
        self.qr_png_path = None;
        self.qr_error = Some(error);
    }

    /// Stops the QR pane without a failure (back to the phone screen).
    /// State cleanup only; sending `Request::QrLoginCancel` is the caller's job.
    pub fn qr_cancelled(&mut self) {
        self.qr_stage = QrStage::Idle;
        self.qr_png_path = None;
        self.qr_error = None;
    }

    /// Sends a picked file to the open chat: optimistic media row with a
    /// live upload progress, then the server echo replaces it.
    pub fn send_media(&mut self, path: String) {
        let Some(id) = self.open_chat else { return };
        let is_photo = crate::looks_like_image(&path);
        let caption = std::mem::take(&mut self.composer);
        let reply_to = self.reply_target.take().map(|r| r.msg_id);
        let token = self.next_media_token;
        self.next_media_token += 1;
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.messages.push(MsgRow {
            doc: (!is_photo).then_some(crate::bridge::DocMeta {
                name: name.clone(),
                size: 0,
                kind: crate::bridge::DocKind::Audio { voice: false },
                duration: None,
            }),
            photo: is_photo.then_some((0, 0)),
            uploading: Some(0.0),
            upload_token: Some(token),
            reply_to,
            ..MsgRow::text(0, caption.clone(), 0, true)
        });
        let _ = self.req_tx.send(Request::SendMedia {
            id,
            path,
            caption,
            is_photo,
            reply_to,
            token,
        });
        self.scroll_to_bottom = true;
        self.invalidate_layout();
    }

/// Periodic tick (500 ms) while a voice note plays: advance the progress bar
    /// and clear the state once the audio thread reports the file finished.
    pub fn on_voice_tick(&mut self) {
        if self.playing_voice.is_none() {
            return;
        }
        // If we're the active note, poll the audio engine for completion.
        if crate::audio::is_active() {
            if self.voice_playing {
                self.voice_elapsed = crate::audio::elapsed_secs();
            }
            if crate::audio::finished() {
                self.playing_voice = None;
                self.voice_playing = false;
                self.voice_elapsed = 0.0;
                self.voice_sim = false;
                self.voice_sim_started = None;
                crate::audio::stop();
            }
        } else if self.voice_sim && self.voice_playing {
            // External player: advance a simulated cursor; end at duration.
            if let Some(started) = self.voice_sim_started {
                self.voice_elapsed = started.elapsed().as_secs_f32();
                if self.voice_duration > 0.0 && self.voice_elapsed >= self.voice_duration {
                    self.playing_voice = None;
                    self.voice_playing = false;
                    self.voice_elapsed = 0.0;
                    self.voice_sim = false;
                    self.voice_sim_started = None;
                }
            }
        }
    }

    /// Seek within the current voice note: `secs` from the start, clamped to
    /// [0, duration]. Live for inline playback; rewinds the simulation clock
    /// when progress is externally played.
    pub fn voice_seek(&mut self, secs: f32) {
        let max = if self.voice_duration > 0.0 { self.voice_duration } else { secs };
        let t = secs.clamp(0.0, max);
        if crate::audio::is_active() {
            crate::audio::seek_to(t);
        }
        self.voice_elapsed = t;
        if self.voice_sim {
            self.voice_sim_started =
                Some(std::time::Instant::now() - std::time::Duration::from_secs_f32(t));
        }
    }

    /// Open a chat.
    pub fn open_chat(&mut self, id: i64) {
        crate::audio::stop();
        self.playing_voice = None;
        self.voice_playing = false;
        self.voice_elapsed = 0.0;
        self.voice_sim = false;
        self.voice_sim_started = None;
        self.open_chat = Some(id);
        self.messages.clear();
        self.loading = true;
        self.chat_title.clear();
        self.editing = None;
        self.context_menu = None;
        self.reply_target = None;
        self.forward_pick = None;
        self.pending_jump_id = None;
        self.pinned_id = None;
        self.composer.clear();
        self.info_open = false;
        self.chat_info = None;
        self.participants.clear();
        self.kick_confirm = None;
        self.admin_menu = None;
        self.admin_confirm = None;
        self.muted = false;
        self.topic_reset();
        self.emoji_panel_open = false;
        self.sticker_picker_open = false;
        if self.persist_ui {
            write_last_chat_at(&last_chat_path(), Some(id));
        }
        let _ = self.req_tx.send(Request::OpenChat { id });
        let _ = self.req_tx.send(Request::MarkRead { id });
        // Forum chats answer with their topic list; non-forum chats get an
        // explicit empty answer so stale chips never linger.
        let _ = self.req_tx.send(Request::GetTopics { id });
        self.invalidate_layout();
    }

    /// Back to the chat list: closes the open chat and clears the
    /// conversation-scoped state (also forgets the `last-chat` marker).
    pub fn close_chat(&mut self) {
        self.open_chat = None;
        self.messages.clear();
        self.editing = None;
        self.context_menu = None;
        self.pinned_id = None;
        self.info_open = false;
        self.chat_info = None;
        self.participants.clear();
        self.kick_confirm = None;
        self.admin_menu = None;
        self.admin_confirm = None;
        self.muted = false;
        self.topic_reset();
        self.emoji_panel_open = false;
        self.sticker_picker_open = false;
        if self.persist_ui {
            write_last_chat_at(&last_chat_path(), None);
        }
        self.invalidate_layout();
    }

    /// Path of the photo attached to row, if any and already downloaded.
    #[allow(dead_code)]
    fn photo_at(&self, row: usize) -> Option<String> {
        let m = self.messages.get(row)?;
        if m.photo.is_some() {
            m.photo_path.clone()
        } else {
            None
        }
    }

    // -------------------------------------------------------------------
    // Search (global + in-chat)
    // -------------------------------------------------------------------

    /// Opens the search UI (`mode` decides where it queries).
    pub fn open_search(&mut self, mode: SearchMode) {
        // Keep the query if it was already typed; start fresh otherwise.
        self.search_mode = Some(mode);
        self.search_hits.clear();
        self.search_pending = false;
    }

    /// Closes the search UI (Escape, ✕, or after jumping to a result).
    pub fn close_search(&mut self) {
        self.search_mode = None;
        self.search_query.clear();
        self.search_hits.clear();
        self.search_pending = false;
        self.pending_jump_id = None;
    }

    /// The search field changed: update the query and re-run (the network
    /// layer throttles identical queries).
    pub fn search_changed(&mut self, query: String) {
        self.search_query = query.clone();
        let q = query.trim();
        if q.is_empty() {
            self.search_hits.clear();
            self.search_pending = false;
            return;
        }
        self.search_pending = true;
        let id = match self.search_mode {
            Some(SearchMode::Global) => None,
            Some(SearchMode::InChat) => self.open_chat,
            None => return,
        };
        let _ = self.req_tx.send(Request::Search { id, query: q.to_string() });
    }

    /// A result was clicked: jump to the message, or open its chat.
    pub fn click_search_hit(&mut self, idx: usize) {
        let Some(hit) = self.search_hits.get(idx).cloned() else {
            return;
        };
        // Jumping only works when the hit is the open chat's loaded history.
        if self.search_mode == Some(SearchMode::InChat)
            && self.open_chat == Some(hit.chat_id)
        {
            if let Some(y) = self.message_top_of(hit.row.id) {
                self.scroll_to_bottom = false;
                self.scroll_target = Some(y);
            } else {
                // Not in the loaded window (or cache cold): head to bottom.
                self.scroll_to_bottom = true;
            }
            self.close_search();
            self.invalidate_layout();
            return;
        }
        // Different chat: open it; if the message is not in the (capped)
        // history the scroll naturally lands at the bottom.
        self.pending_jump_id = Some(hit.row.id);
        self.open_chat(hit.chat_id);
        self.close_search();
    }

    /// True when the search UI is open.
    pub fn search_open(&self) -> bool {
        self.search_mode.is_some()
    }

    /// Content Y of `msg_id` in the open chat, per the cached layout, or None
    /// if it isn't in the loaded history (or the cache was never built).
    fn message_top_of(&self, msg_id: i32) -> Option<f32> {
        let idx = self.messages.iter().position(|m| m.id == msg_id)?;
        let cache = self
            .layout_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let c = cache.as_ref()?;
        if c.epoch != self.layout_epoch {
            return None;
        }
        let y = c.tops.get(idx).copied()?;
        // Keep a little margin above the target so it isn't glued to the top.
        Some((y - 24.0).max(0.0))
    }

    /// Consumes and returns a pending scroll target (the shell turns it into a
    /// scrollable scroll). Also resolves `pending_jump_id` once the history
    /// arrived.
    pub fn take_scroll_target(&mut self) -> Option<f32> {
        self.scroll_target.take()
    }

    /// After `open_chat`, `Messages` arrives: try to resolve a pending jump.
    fn resolve_pending_jump(&mut self) {
        let Some(id) = self.pending_jump_id.take() else {
            return;
        };
        if let Some(y) = self.message_top_of(id) {
            self.scroll_to_bottom = false;
            self.scroll_target = Some(y);
        }
    }

    pub fn back(&mut self) {
        self.viewer = None;
        if self.context_menu.take().is_some() {
            self.invalidate_layout();
        }
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
                ..MsgRow::text(1, "incoming", 100, false)
            },
            MsgRow {
                ..MsgRow::text(2, "mine", 200, true)
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
    fn pinned_message_banner_arrives_and_clears() {
        let (mut state, _) = demo_state();
        assert_eq!(state.pinned_id, None);

        // The network reports a pin in the open chat.
        state.on_message(UiMessage::PinnedMessage { chat_id: 42, msg_id: Some(1) });
        assert_eq!(state.pinned_id, Some(1));
        // Row flag synced.
        assert!(state.messages.iter().find(|m| m.id == 1).unwrap().pinned);

        // Unpin clears the banner and the row flag.
        state.on_message(UiMessage::PinnedMessage { chat_id: 42, msg_id: None });
        assert_eq!(state.pinned_id, None);
        assert!(!state.messages.iter().find(|m| m.id == 1).unwrap().pinned);

        // A pin for another chat must not touch this one's banner.
        state.on_message(UiMessage::PinnedMessage { chat_id: 7, msg_id: Some(2) });
        assert_eq!(state.pinned_id, None);
    }

    #[test]
    fn context_pin_sends_request_and_toggles() {
        let (mut state, mut req_rx) = demo_state();
        // Right-click row 0 (msg id 1) and pin it.
        state.open_context(0);
        assert!(!state.context_row_pinned());
        state.context_pin();
        let reqs = drain(&mut req_rx);
        assert!(
            reqs.iter().any(|r| matches!(r, Request::PinMessage { id: 42, msg_id: 1, pin: true })),
            "expected PinMessage request, got {reqs:?}"
        );
        assert_eq!(state.pinned_id, Some(1));
        assert!(state.context_menu.is_none());

        // Pinning again (row is now pinned) sends an unpin.
        state.open_context(0);
        assert!(state.context_row_pinned());
        state.context_pin();
        let reqs = drain(&mut req_rx);
        assert!(
            reqs.iter().any(|r| matches!(r, Request::PinMessage { id: 42, msg_id: 1, pin: false })),
            "expected unpin request, got {reqs:?}"
        );
        assert_eq!(state.pinned_id, None);
    }

    #[test]
    fn deleting_the_pinned_message_clears_the_banner() {
        let (mut state, _) = demo_state();
        state.messages[0].pinned = true;
        state.pinned_id = Some(1);
        // Delete via the context menu on row 0.
        state.open_context(0);
        state.context_delete();
        assert_eq!(state.pinned_id, None);
        assert!(state.messages.iter().all(|m| m.id != 1));
    }

    #[test]
    fn opening_another_chat_resets_the_banner() {
        let (mut state, _) = demo_state();
        state.pinned_id = Some(1);
        state.open_chat(43);
        assert_eq!(state.pinned_id, None);
        // History rows carrying `pinned` seed the banner until the
        // authoritative PinnedMessage arrives.
        state.on_message(UiMessage::Messages {
            id: 43,
            title: "G".into(),
            rows: vec![MsgRow { pinned: true, ..MsgRow::text(5, "topic", 10, false) }],
        });
        assert_eq!(state.pinned_id, Some(5));
    }

    #[test]
    fn last_chat_marker_roundtrip() {
        let dir = std::env::temp_dir().join(format!("telegram-rs-lastchat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("last-chat");

        assert_eq!(read_last_chat_at(&path), None);
        write_last_chat_at(&path, Some(987654321));
        assert_eq!(read_last_chat_at(&path), Some(987654321));
        write_last_chat_at(&path, None);
        assert_eq!(read_last_chat_at(&path), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dialogs_reopen_last_chat_when_present() {
        let (mut state, mut req_rx) = demo_state();
        state.authenticated = true;
        state.open_chat = None;
        state.initial_chat = Some(7);
        state.on_message(UiMessage::Dialogs(vec![
            ChatRow { id: 1, title: "A".into(), subtitle: String::new(), date: 0, unread: 0, avatar_path: None },
            ChatRow { id: 7, title: "B".into(), subtitle: String::new(), date: 0, unread: 0, avatar_path: None },
        ]));
        // The restore logic lives in the shell's `update`, so simulate the
        // fallback it applies when the persisted chat is in the list.
        let wanted = state
            .initial_chat
            .take()
            .filter(|id| state.dialogs.iter().any(|d| d.id == *id));
        assert_eq!(wanted, Some(7));
        state.open_chat(7);
        assert_eq!(state.open_chat, Some(7));
        assert!(drain(&mut req_rx).iter().any(|r| matches!(r, Request::OpenChat { id: 7 })));
    }

    #[test]
    fn stale_last_chat_falls_back_to_first() {
        let (mut state, _) = demo_state();
        state.open_chat = None;
        state.initial_chat = Some(999); // not in the list anymore
        state.on_message(UiMessage::Dialogs(vec![
            ChatRow { id: 1, title: "A".into(), subtitle: String::new(), date: 0, unread: 0, avatar_path: None },
            ChatRow { id: 2, title: "B".into(), subtitle: String::new(), date: 0, unread: 0, avatar_path: None },
        ]));
        let wanted = state
            .initial_chat
            .take()
            .filter(|id| state.dialogs.iter().any(|d| d.id == *id));
        assert_eq!(wanted, None); // shell will fall back to auto_open_first
    }

    #[test]
    fn close_chat_clears_state() {
        let (mut state, mut req_rx) = demo_state();
        state.persist_ui = false; // no fs writes in unit tests
        state.close_chat();
        assert_eq!(state.open_chat, None);
        assert!(state.messages.is_empty());
        assert!(state.editing.is_none());
        assert!(state.context_menu.is_none());
        drain(&mut req_rx);
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
    fn right_click_incoming_opens_menu_without_edit() {
        let (mut state, _) = demo_state();
        state.open_context(0); // row 0 is incoming
        // The menu opens for any message now (reply/forward), but editing
        // stays restricted to outgoing rows.
        assert!(state.context_menu.is_some());
        assert!(!state.context_can_edit());
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
            doc: None,
            reply_to: None,
            forwarded_from: None,
            sender_name: None,
            sender_id: None,
            sticker: None,
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
            doc: None,
            reply_to: None,
            forwarded_from: None,
            sender_name: None,
            sender_id: None,
            sticker: None,
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
            doc: None,
            reply_to: None,
            forwarded_from: None,
            sender_name: None,
            sender_id: None,
            sticker: None,
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
                ..MsgRow::text(55, "mine", 0, true)
            });
        state.messages.push(MsgRow {
                ..MsgRow::text(56, "mine too", 0, true)
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
            doc: None,
            reply_to: None,
            forwarded_from: None,
            sender_name: None,
            sender_id: None,
            sticker: None,
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
    fn reply_armed_then_sent_with_the_message() {
        let (mut state, mut req_rx) = demo_state();
        state.open_context(0); // incoming row
        state.context_reply();
        assert!(state.reply_target.is_some(), "reply target armed");
        state.composer = "ma réponse".into();
        state.submit();

        // The request carries the reply id and the optimistic row quotes it.
        let reqs = drain(&mut req_rx);
        assert!(matches!(
            reqs.last(),
            Some(Request::SendMessage { id: _, reply_to: Some(1), text }) if text == "ma réponse"
        ));
        let last = state.messages.last().unwrap();
        assert_eq!(last.reply_to, Some(1));
        // The reply is consumed after the send.
        assert!(state.reply_target.is_none());
    }

    #[test]
    fn escape_cancels_reply_but_keeps_text() {
        let (mut state, _) = demo_state();
        state.open_context(1);
        state.context_reply();
        state.composer = "brouillon".into();
        state.escape();
        assert!(state.reply_target.is_none());
        assert_eq!(state.composer, "brouillon");
    }

    #[test]
    fn qr_toggle_switches_screen_and_cleans_up_on_leave() {
        let (mut state, _) = demo_state();
        // Entering the QR pane: screen flips, then the UI layer starts a
        // session (qr_started mirrors lib.rs's update arm).
        state.toggle_login_screen();
        assert!(state.login_qr_selected);
        state.qr_started();
        assert_eq!(state.qr_stage, QrStage::AwaitingScan);

        state.on_message(UiMessage::QrCodeReady { path: "/tmp/q.png".into() });
        assert_eq!(state.qr_png_path.as_deref(), Some("/tmp/q.png"));

        // Leaving via the switcher stops the poller with no error shown.
        state.toggle_login_screen();
        assert!(!state.login_qr_selected);
        assert_eq!(state.qr_stage, QrStage::Idle);
        assert!(state.qr_png_path.is_none());
    }

    #[test]
    fn qr_code_ready_scan_confirmed_then_login_ok_resets() {
        let (mut state, _) = demo_state();
        state.login_qr_selected = true;
        state.qr_started();
        state.on_message(UiMessage::QrCodeReady { path: "/tmp/q.png".into() });
        assert_eq!(state.qr_png_path.as_deref(), Some("/tmp/q.png"));

        state.on_message(UiMessage::QrScanConfirmed);
        assert_eq!(state.qr_stage, QrStage::ScanConfirmed);

        state.on_message(UiMessage::LoginOk { name: "QR Demo".into() });
        assert!(state.authenticated);
        assert!(!state.login_qr_selected, "back on phone pane next login");
        assert_eq!(state.qr_stage, QrStage::Idle);
        assert!(state.qr_png_path.is_none());
    }

    #[test]
    fn qr_failure_clears_the_code_and_shows_the_error() {
        let (mut state, _) = demo_state();
        state.login_qr_selected = true;
        state.qr_started();
        state.on_message(UiMessage::QrCodeReady { path: "/tmp/q.png".into() });
        state.on_message(UiMessage::QrLoginFailed {
            error: "QR sign-in failed".into(),
        });
        assert_eq!(state.qr_stage, QrStage::Idle);
        assert!(state.qr_png_path.is_none());
        assert_eq!(state.qr_error.as_deref(), Some("QR sign-in failed"));
        // The phone LoginStep flow is untouched by everything QR.
        assert_eq!(state.login_step, LoginStep::Phone);
    }

    #[test]
    fn forward_picks_a_chat_and_sends_one_request() {
        let (mut state, mut req_rx) = demo_state();
        state.dialogs = vec![ChatRow {
            id: 7,
            title: "Cible".into(),
            subtitle: String::new(),
            date: 0,
            unread: 0,
            avatar_path: None,
        }];
        state.open_context(1); // forward my own message (id 2)
        state.context_forward();
        assert_eq!(state.forward_pick, Some(1));

        state.forward_to(7);
        let reqs = drain(&mut req_rx);
        assert!(matches!(
            reqs.last(),
            Some(Request::ForwardMessage { from_chat: 42, msg_id: 2, to_chat: 7 })
        ));
        assert!(state.forward_pick.is_none(), "picker closes after send");
    }

    #[test]
    fn media_send_pushes_optistic_row_and_tracks_progress() {
        let (mut state, mut req_rx) = demo_state();
        state.send_media("/tmp/vacances.png".into());

        // Optimistic photo row with a fresh upload progress.
        let last = state.messages.last().unwrap();
        assert_eq!(last.id, 0);
        assert_eq!(last.photo, Some((0, 0)));
        assert_eq!(last.uploading, Some(0.0));
        assert!(last.upload_token.is_some());

        let token = last.upload_token.unwrap();
        let reqs = drain(&mut req_rx);
        match reqs.last() {
            Some(Request::SendMedia { id: 42, path, is_photo, token: t, .. }) => {
                assert_eq!(path, "/tmp/vacances.png");
                assert!(is_photo, "png must be detected as a photo");
                assert_eq!(*t, token);
            }
            other => panic!("expected SendMedia, got {other:?}"),
        }

        // Progress updates flow into the same row.
        state.on_message(UiMessage::UploadProgress { chat_id: 42, token, progress: 0.5 });
        assert_eq!(state.messages.last().unwrap().uploading, Some(0.5));

        // The server echo replaces the optimistic row.
        state.on_message(UiMessage::NewMessage {
            chat_id: 42,
            id: 777,
            text: String::new(),
            date: 500,
            out: true,
            photo: Some((640, 480)),
            doc: None,
            reply_to: None,
            forwarded_from: None,
            sender_name: None,
            sender_id: None,
            sticker: None,
        });
        assert_eq!(state.messages.len(), 3, "echo merges, no duplicate");
        let merged = state.messages.last().unwrap();
        assert_eq!(merged.id, 777);
        assert_eq!(merged.uploading, None);
    }

    #[test]
    fn document_send_is_not_flagged_as_photo() {
        let (mut state, mut req_rx) = demo_state();
        state.send_media("/tmp/rapport.pdf".into());
        let last = state.messages.last().unwrap();
        assert!(last.doc.is_some());
        assert!(last.photo.is_none());

        let reqs = drain(&mut req_rx);
        assert!(matches!(
            reqs.last(),
            Some(Request::SendMedia { is_photo: false, .. })
        ));
    }

    #[test]
    fn opening_search_sends_requests_and_applies_results() {
        let (mut state, mut req_rx) = demo_state();
        state.open_search(SearchMode::InChat);
        assert!(state.search_open());
        state.search_changed("weekend".into());

        let reqs = drain(&mut req_rx);
        assert!(matches!(
            reqs.last(),
            Some(Request::Search { id: Some(42), query }) if query == "weekend"
        ));

        // A matching result lands in the list.
        state.on_message(UiMessage::SearchResults {
            id: Some(42),
            query: "weekend".into(),
            hits: vec![SearchHit {
                chat_id: 42,
                chat_title: "Camille".into(),
                row: MsgRow::text(77, "le weekend 😎", 500, false),
            }],
        });
        assert_eq!(state.search_hits.len(), 1);
        assert_eq!(state.search_hits[0].row.id, 77);
    }

    #[test]
    fn search_results_for_a_stale_mode_are_ignored() {
        let (mut state, _) = demo_state();
        // Query survived a close: results from an old search must not repopulate.
        state.open_search(SearchMode::Global);
        state.search_changed("foo".into());
        state.close_search();
        assert!(!state.search_open());
        assert!(state.search_query.is_empty());

        state.on_message(UiMessage::SearchResults {
            id: Some(42),
            query: "foo".into(),
            hits: vec![],
        });
        assert!(state.search_hits.is_empty());
    }

    #[test]
    fn in_chat_hit_jumps_to_the_message() {
        let (mut state, _) = demo_state();
        // Ensure the layout cache is populated (the view builds it lazily).
        crate::messages_list(&state, 800.0, 600.0);

        state.open_search(SearchMode::InChat);
        state.on_message(UiMessage::SearchResults {
            id: Some(42),
            query: "".into(),
            hits: vec![SearchHit {
                chat_id: 42,
                chat_title: "Camille".into(),
                row: MsgRow::text(1, "incoming", 100, false),
            }],
        });
        state.click_search_hit(0);
        assert!(!state.search_open(), "jumping closes the search UI");
        // Row 1 of the demo history (id 1) → its cached top ≈ view_h (0-pad).
        assert!(state.scroll_target.unwrap_or(-1.0) >= 0.0);
    }

    #[test]
    fn global_hit_opens_the_target_chat() {
        let (mut state, mut req_rx) = demo_state();
        state.open_search(SearchMode::Global);
        state.on_message(UiMessage::SearchResults {
            id: None,
            query: "".into(),
            hits: vec![SearchHit {
                chat_id: 7,
                chat_title: "Autre".into(),
                row: MsgRow::text(9, "ping", 400, false),
            }],
        });
        state.click_search_hit(0);
        // Left the search UI and opened the other chat (OpenChat + MarkRead).
        assert!(!state.search_open());
        let reqs = drain(&mut req_rx);
        assert!(matches!(reqs.first(), Some(Request::OpenChat { id: 7 })));
    }

    #[test]
    fn escape_closes_search_before_context_menu() {
        let (mut state, _) = demo_state();
        state.open_search(SearchMode::Global);
        state.escape();
        assert!(!state.search_open());
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
        assert_eq!(
            reqs.len(),
            3,
            "open + mark-read + get-topics are sent to the network"
        );
        assert!(matches!(reqs[0], Request::OpenChat { id: 7 }));
        assert!(matches!(reqs[1], Request::MarkRead { id: 7 }));
        assert!(matches!(reqs[2], Request::GetTopics { id: 7 }));
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
                ..MsgRow::text(1, "hi", 5, false)
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
                ..MsgRow::text(1, "hi", 5, true)
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

    fn seed_dialogs(state: &mut State) {
        state.on_message(UiMessage::Dialogs(vec![
            ChatRow { id: 1001, title: "Camille".into(), subtitle: String::new(), date: 0, unread: 0, avatar_path: None },
            ChatRow { id: 1002, title: "Rust Group".into(), subtitle: String::new(), date: 0, unread: 0, avatar_path: None },
            ChatRow { id: 1003, title: "Landscape Channel".into(), subtitle: String::new(), date: 0, unread: 0, avatar_path: None },
        ]));
    }

    #[test]
    fn create_modal_opens_with_contacts_and_cancels() {
        let (mut state, mut req_rx) = demo_state();
        seed_dialogs(&mut state);

        state.toggle_create_menu();
        assert!(state.create_menu_open, "+ picker menu opens");
        state.open_create(CreateKind::Group);
        assert_eq!(state.create_dialog, Some(CreateKind::Group));
        assert!(!state.create_menu_open, "picker closes when the modal opens");
        // Contacts seeded from the known dialogs, all unchecked.
        assert_eq!(state.member_pick.len(), 3);
        assert!(state.member_pick.iter().all(|(_, _, on)| !on));

        // Escape cancels and clears the draft.
        state.create_title = "draft".into();
        state.escape();
        assert!(state.create_dialog.is_none());
        assert!(state.create_title.is_empty());
        assert!(state.member_pick.is_empty());
        drain(&mut req_rx);
    }

    #[test]
    fn submit_channel_sends_request_and_closes() {
        let (mut state, mut req_rx) = demo_state();
        state.open_create(CreateKind::Channel);
        state.create_title = "  My Channel  ".into();
        state.create_about = "about it".into();
        state.submit_create();

        assert!(state.create_dialog.is_none(), "modal closes after submit");
        let reqs = drain(&mut req_rx);
        assert!(
            matches!(
                reqs.last(),
                Some(Request::CreateChannel { title, about, megagroup: false, members })
                    if title == "My Channel" && about == "about it" && members.is_empty()
            ),
            "expected CreateChannel request, got {reqs:?}"
        );

        // Empty titles are not submitted.
        state.open_create(CreateKind::Channel);
        state.submit_create();
        assert!(state.create_dialog.is_some());
        assert!(drain(&mut req_rx).is_empty());
    }

    #[test]
    fn submit_group_carries_checked_members() {
        let (mut state, mut req_rx) = demo_state();
        seed_dialogs(&mut state);
        state.open_create(CreateKind::Group);
        state.toggle_member(0);
        state.toggle_member(2);
        state.create_title = "Team".into();
        state.submit_create();

        let reqs = drain(&mut req_rx);
        match reqs.last() {
            Some(Request::CreateChannel { megagroup: true, members, .. }) => {
                assert_eq!(members, &[1001, 1003]);
            }
            other => panic!("expected group CreateChannel, got {other:?}"),
        }
    }

    fn group_detail(id: i64) -> ChatDetail {
        ChatDetail {
            id,
            title: "Rust Group".into(),
            kind: crate::bridge::ChatKind::Group,
            username: None,
            bio: Some("Weekly reviews".into()),
            phone: None,
            members_count: Some(3),
            is_forum: false,
        }
    }

    #[test]
    fn chat_created_opens_the_new_chat() {
        let (mut state, mut req_rx) = demo_state();
        state.on_message(UiMessage::ChatCreated { id: 1006 });
        assert_eq!(state.open_chat, Some(1006), "the new chat opens right away");
        assert!(state.loading);
        let reqs = drain(&mut req_rx);
        assert!(matches!(reqs.first(), Some(Request::OpenChat { id: 1006 })));
    }

    #[test]
    fn chat_gone_closes_only_that_chat() {
        let (mut state, _) = demo_state();
        // Gone chat is not the open one: nothing changes.
        state.on_message(UiMessage::ChatGone { id: 7 });
        assert_eq!(state.open_chat, Some(42));

        state.on_message(UiMessage::ChatGone { id: 42 });
        assert_eq!(state.open_chat, None, "the open chat closes");
        assert!(state.messages.is_empty());
    }

    #[test]
    fn leave_and_delete_go_through_confirmation() {
        let (mut state, mut req_rx) = demo_state();
        state.open_row_menu(42);
        assert_eq!(state.row_menu, Some(42));

        state.ask_confirm(ConfirmKind::Leave, 42);
        assert!(state.row_menu.is_none(), "row menu closes");
        assert_eq!(state.confirm_leave, Some((ConfirmKind::Leave, 42)));

        // Cancel clears without sending anything.
        state.cancel_confirm();
        assert!(state.confirm_leave.is_none());
        assert!(drain(&mut req_rx).is_empty());

        // Confirming a delete sends DeleteChat.
        state.ask_confirm(ConfirmKind::Delete, 42);
        state.confirm_yes();
        let reqs = drain(&mut req_rx);
        assert!(matches!(reqs.last(), Some(Request::DeleteChat { id: 42 })));
        assert!(state.confirm_leave.is_none());
    }

    #[test]
    fn escape_closes_row_menu_before_other_overlays() {
        let (mut state, _) = demo_state();
        state.row_menu = Some(42);
        state.editing = Some(2);
        state.escape();
        assert!(state.row_menu.is_none());
        assert_eq!(state.editing, Some(2), "deeper overlays are untouched");
    }

    #[test]
    fn info_panel_opens_and_requests_details() {
        let (mut state, mut req_rx) = demo_state();
        assert!(!state.info_open);
        state.open_info();
        assert!(state.info_open, "panel open");
        let reqs = drain(&mut req_rx);
        assert!(
            reqs.iter().any(|r| matches!(r, Request::GetChatInfo { id: 42 })),
            "expected GetChatInfo, got {reqs:?}"
        );
        assert!(
            reqs.iter().any(|r| matches!(r, Request::GetParticipants { id: 42 })),
            "expected GetParticipants, got {reqs:?}"
        );
    }

    #[test]
    fn chat_info_populates_panel_and_stale_is_ignored() {
        let (mut state, _) = demo_state();
        state.open_info();
        state.on_message(UiMessage::ChatInfo(group_detail(42)));
        assert_eq!(state.chat_info.as_ref().map(|d| d.id), Some(42));
        // A response for another chat must not overwrite the panel.
        state.on_message(UiMessage::ChatInfo(group_detail(99)));
        assert_eq!(state.chat_info.as_ref().map(|d| d.id), Some(42));
    }

    #[test]
    fn participants_kick_flow_with_inline_confirmation() {
        let (mut state, mut req_rx) = demo_state();
        state.open_info();
        state.on_message(UiMessage::Participants(vec![
            crate::bridge::ParticipantRow {
                id: 2001,
                name: "Camille".into(),
                username: None,
                role: crate::bridge::ParticipantRole::Creator,
            },
            crate::bridge::ParticipantRow {
                id: 2002,
                name: "Léo".into(),
                username: None,
                role: crate::bridge::ParticipantRole::Member,
            },
        ]));
        assert_eq!(state.participants.len(), 2);

        // Click "remove" → arms the confirmation, no request yet.
        state.kick(2002);
        assert_eq!(state.kick_confirm, Some(2002));
        drain(&mut req_rx);

        // Confirm → request sent; row stays until the echo.
        state.kick_confirmed();
        let reqs = drain(&mut req_rx);
        assert!(
            reqs.iter().any(|r| matches!(r, Request::KickParticipant { id: 42, user_id: 2002 })),
            "expected KickParticipant, got {reqs:?}"
        );
        assert_eq!(state.participants.len(), 2);

        // Echo removes the row and clears the confirmation.
        state.on_message(UiMessage::ParticipantKicked { user_id: 2002 });
        assert_eq!(state.participants.len(), 1);
        assert_eq!(state.kick_confirm, None);

        // Kicking an unknown member must not arm anything.
        state.kick(9999);
        assert_eq!(state.kick_confirm, None);
    }

    fn part(id: i64, role: crate::bridge::ParticipantRole) -> crate::bridge::ParticipantRow {
        crate::bridge::ParticipantRow {
            id,
            name: format!("Member {id}"),
            username: None,
            role,
        }
    }

    #[test]
    fn admin_menu_items_follow_the_role() {
        use crate::bridge::ParticipantRole;
        // Plain members: promote + ban + remove.
        assert_eq!(
            admin_menu_items(ParticipantRole::Member, false),
            vec![AdminAction::Promote, AdminAction::Ban, AdminAction::Remove]
        );
        // Admins: demote + ban.
        assert_eq!(
            admin_menu_items(ParticipantRole::Admin, false),
            vec![AdminAction::Demote, AdminAction::Ban]
        );
        // The owner and yourself: no menu at all.
        assert!(admin_menu_items(ParticipantRole::Creator, false).is_empty());
        assert!(admin_menu_items(ParticipantRole::Member, true).is_empty());
        assert!(admin_menu_items(ParticipantRole::Admin, true).is_empty());
    }

    #[test]
    fn admin_menu_hidden_on_owner_and_self() {
        let (mut state, _) = demo_state();
        state.open_info();
        state.on_message(UiMessage::Participants(vec![
            part(2001, crate::bridge::ParticipantRole::Creator),
            part(2002, crate::bridge::ParticipantRole::Member),
            part(2003, crate::bridge::ParticipantRole::Admin),
        ]));

        state.admin_open_menu(2001);
        assert_eq!(state.admin_menu, None, "the owner gets no menu");

        state.admin_open_menu(2003);
        assert_eq!(state.admin_menu, Some(2003));
        state.admin_menu = None;

        // Yourself: no menu even as a plain member.
        state.admin_self_id = Some(2002);
        state.admin_open_menu(2002);
        assert_eq!(state.admin_menu, None, "you get no menu on yourself");

        state.admin_self_id = None;
        state.admin_open_menu(2002);
        assert_eq!(state.admin_menu, Some(2002));
    }

    #[test]
    fn admin_promote_and_demote_fire_immediately() {
        let (mut state, mut req_rx) = demo_state();
        state.open_info();
        state.on_message(UiMessage::Participants(vec![
            part(2002, crate::bridge::ParticipantRole::Member),
            part(2003, crate::bridge::ParticipantRole::Admin),
        ]));

        state.admin_open_menu(2002);
        state.admin_action(2002, AdminAction::Promote);
        assert!(state.admin_menu.is_none(), "menu closes");
        assert!(
            drain(&mut req_rx).iter().any(|r| matches!(r, Request::AdminPromote { id: 42, user_id: 2002 })),
            "expected AdminPromote"
        );

        state.admin_open_menu(2003);
        state.admin_action(2003, AdminAction::Demote);
        assert!(
            drain(&mut req_rx).iter().any(|r| matches!(r, Request::AdminDemote { id: 42, user_id: 2003 })),
            "expected AdminDemote"
        );
    }

    #[test]
    fn admin_ban_and_remove_go_through_inline_confirmation() {
        let (mut state, mut req_rx) = demo_state();
        state.open_info();
        state.on_message(UiMessage::Participants(vec![
            part(2002, crate::bridge::ParticipantRole::Member),
        ]));
        drain(&mut req_rx);

        // Ban arms the confirmation; nothing is sent yet.
        state.admin_open_menu(2002);
        state.admin_action(2002, AdminAction::Ban);
        assert_eq!(state.admin_confirm, Some((AdminConfirm::Ban, 2002)));
        assert!(drain(&mut req_rx).is_empty());

        // Escape cancels without sending.
        state.escape();
        assert_eq!(state.admin_confirm, None);
        assert!(drain(&mut req_rx).is_empty());

        // Yes on a remove sends kick_only=true.
        state.admin_open_menu(2002);
        state.admin_action(2002, AdminAction::Remove);
        assert_eq!(state.admin_confirm, Some((AdminConfirm::Remove, 2002)));
        state.admin_confirmed();
        assert!(
            drain(&mut req_rx).iter().any(|r| matches!(r, Request::AdminBan { id: 42, user_id: 2002, kick_only: true })),
            "expected AdminBan kick_only=true"
        );
        assert_eq!(state.admin_confirm, None);

        // Yes on a ban sends kick_only=false.
        state.admin_open_menu(2002);
        state.admin_action(2002, AdminAction::Ban);
        state.admin_confirmed();
        assert!(
            drain(&mut req_rx).iter().any(|r| matches!(r, Request::AdminBan { id: 42, user_id: 2002, kick_only: false })),
            "expected AdminBan kick_only=false"
        );
    }

    #[test]
    fn member_updated_refreshes_the_member_list() {
        let (mut state, mut req_rx) = demo_state();
        state.open_info();
        drain(&mut req_rx);

        state.on_message(UiMessage::MemberUpdated { chat_id: 42, user_id: 2002 });
        assert!(
            drain(&mut req_rx).iter().any(|r| matches!(r, Request::GetParticipants { id: 42 })),
            "expected the member list to be re-requested"
        );

        // Another chat's update must not touch this panel.
        state.on_message(UiMessage::MemberUpdated { chat_id: 99, user_id: 2002 });
        assert!(drain(&mut req_rx).is_empty());
    }

    #[test]
    fn escape_closes_admin_menu_before_the_info_panel() {
        let (mut state, _) = demo_state();
        state.open_info();
        state.on_message(UiMessage::Participants(vec![
            part(2002, crate::bridge::ParticipantRole::Member),
        ]));
        state.admin_open_menu(2002);

        state.escape();
        assert_eq!(state.admin_menu, None);
        assert!(state.info_open, "only the menu closes first");

        state.escape();
        assert!(!state.info_open);
    }

    #[test]
    fn member_self_id_hides_admin_actions_on_yourself() {
        let (mut state, _) = demo_state();
        assert_eq!(state.admin_self_id, None);
        state.on_message(UiMessage::MemberSelfId { id: 2002 });
        assert_eq!(state.admin_self_id, Some(2002));
    }

    #[test]
    fn toggle_mute_flips_and_sends() {
        let (mut state, mut req_rx) = demo_state();
        assert!(!state.muted);
        state.toggle_mute();
        assert!(state.muted);
        state.toggle_mute();
        assert!(!state.muted);
        let reqs = drain(&mut req_rx);
        assert_eq!(
            reqs.iter().filter(|r| matches!(r, Request::SetMuted { id: 42, .. })).count(),
            2
        );
    }

    #[test]
    fn switching_chats_resets_the_info_panel() {
        let (mut state, _) = demo_state();
        state.open_info();
        state.on_message(UiMessage::ChatInfo(group_detail(42)));
        state.on_message(UiMessage::Participants(vec![crate::bridge::ParticipantRow {
            id: 2001,
            name: "Camille".into(),
            username: None,
            role: crate::bridge::ParticipantRole::Creator,
        }]));
        state.kick(2001);
        assert!(state.info_open && !state.participants.is_empty());
        assert!(!state.muted);

        state.toggle_mute();
        state.muted = true;
        state.open_chat(43);
        assert!(!state.info_open, "panel closed on chat switch");
        assert!(state.chat_info.is_none(), "stale detail dropped");
        assert!(state.participants.is_empty(), "stale members dropped");
        assert_eq!(state.kick_confirm, None);
        assert!(!state.muted, "mute flag is per-chat");
        assert_eq!(state.pinned_id, None);
    }

    #[test]
    fn escape_closes_kick_confirm_then_info_panel() {
        let (mut state, _) = demo_state();
        state.open_info();
        state.on_message(UiMessage::Participants(vec![crate::bridge::ParticipantRow {
            id: 2001,
            name: "Camille".into(),
            username: None,
            role: crate::bridge::ParticipantRole::Creator,
        }]));
        state.kick(2001);

        // First Escape cancels only the inline confirmation.
        state.escape();
        assert_eq!(state.kick_confirm, None);
        assert!(state.info_open, "panel must stay open");

        // Second Escape closes the panel itself.
        state.escape();
        assert!(!state.info_open);
    }

    #[test]
    fn close_chat_clears_the_info_panel() {
        let (mut state, _) = demo_state();
        state.persist_ui = false;
        state.open_info();
        state.on_message(UiMessage::ChatInfo(group_detail(42)));
        state.close_chat();
        assert!(!state.info_open);
        assert!(state.chat_info.is_none());
        assert!(state.participants.is_empty());
        assert!(state.kick_confirm.is_none());
    }

    #[test]
    fn copy_username_returns_at_prefixed_handle() {
        let (mut state, _) = demo_state();
        state.open_info();
        assert_eq!(state.copy_username(), None, "no info yet");
        state.on_message(UiMessage::ChatInfo(ChatDetail {
            id: 42,
            title: "Camille".into(),
            kind: crate::bridge::ChatKind::User,
            username: Some("camille_dev".into()),
            bio: None,
            phone: None,
            members_count: None,
            is_forum: false,
        }));
        assert_eq!(state.copy_username().as_deref(), Some("@camille_dev"));
    }

    #[test]
    fn emoji_panel_toggles_and_escape_closes_it_first() {
        let (mut state, _) = demo_state();
        assert!(!state.emoji_panel_open);
        state.toggle_emoji_panel();
        assert!(state.emoji_panel_open);
        state.toggle_emoji_panel();
        assert!(!state.emoji_panel_open);

        // Escape closes the panel BEFORE the context menu.
        state.toggle_emoji_panel();
        state.open_context(0);
        state.escape();
        assert!(!state.emoji_panel_open, "panel must close first");
        assert!(state.context_menu.is_some(), "menu must stay open");
        state.escape();
        assert!(state.context_menu.is_none());
    }

    #[test]
    fn emoji_pick_appends_to_composer_and_dedups_recents() {
        let (mut state, _) = demo_state();
        assert!(state.emoji_recents.is_empty());

        state.composer.push_str("hi ");
        state.pick_emoji("😀".into());
        state.pick_emoji("🎉".into());
        assert_eq!(state.composer, "hi 😀🎉");
        assert_eq!(state.emoji_recents, vec!["🎉".to_string(), "😀".to_string()]);

        // Picking an existing emoji moves it to the front instead of duping.
        state.pick_emoji("😀".into());
        assert_eq!(state.emoji_recents, vec!["😀".to_string(), "🎉".to_string()]);
    }

    #[test]
    fn emoji_recents_are_capped_at_max() {
        let (mut state, _) = demo_state();
        for i in 0..30 {
            state.pick_emoji(format!("e{i}"));
        }
        assert_eq!(state.emoji_recents.len(), EMOJI_RECENTS_MAX);
        // Most recent first: the last picked emoji leads, the oldest fell off.
        assert_eq!(state.emoji_recents[0], "e29");
        assert_eq!(state.emoji_recents[EMOJI_RECENTS_MAX - 1], "e6");
    }

    #[test]
    fn emoji_recents_persistence_roundtrip() {
        let dir = std::env::temp_dir().join(format!("telegram-rs-emoji-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("emoji-recents");

        assert!(read_emoji_recents_at(&path).is_empty());
        write_emoji_recents_at(&path, &["🎉".into(), "😀".into()]);
        assert_eq!(read_emoji_recents_at(&path), vec!["🎉".to_string(), "😀".to_string()]);
        // A missing/corrupt file must never crash: empty list fallback.
        std::fs::write(&path, "").unwrap();
        assert!(read_emoji_recents_at(&path).is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn theme_defaults_to_dark_and_toggle_flips_palette() {
        let _guard = crate::theme::mode_test_guard();
        let (mut state, _) = demo_state();
        assert_eq!(state.theme_mode, ThemeMode::Dark);
        assert_eq!(crate::theme::LIST_BG(), (23, 33, 43));

        state.toggle_theme();
        assert_eq!(state.theme_mode, ThemeMode::Light);
        // The live palette follows immediately (the next frame re-reads it).
        assert_eq!(crate::theme::LIST_BG(), (255, 255, 255));
        assert_eq!(crate::theme::TEXT_PRIMARY(), (0, 0, 0));
        assert_eq!(crate::theme::BUBBLE_SENT(), (238, 255, 222));

        state.toggle_theme();
        assert_eq!(state.theme_mode, ThemeMode::Dark);
        assert_eq!(crate::theme::LIST_BG(), (23, 33, 43));
    }

    #[test]
    fn theme_mode_persistence_roundtrip() {
        let dir = std::env::temp_dir().join(format!("telegram-rs-theme-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("theme-mode");

        // Missing file → dark.
        assert_eq!(read_theme_mode_at(&path), ThemeMode::Dark);
        write_theme_mode_at(&path, ThemeMode::Light);
        assert_eq!(read_theme_mode_at(&path), ThemeMode::Light);
        write_theme_mode_at(&path, ThemeMode::Dark);
        assert_eq!(read_theme_mode_at(&path), ThemeMode::Dark);
        // Corrupt marker must never crash: dark fallback.
        std::fs::write(&path, "neon-pink").unwrap();
        assert_eq!(read_theme_mode_at(&path), ThemeMode::Dark);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sticker_picker_opens_and_requests_sets_once() {
        let (mut state, mut req_rx) = demo_state();
        assert!(!state.sticker_picker_open);
        // First open: the panel shows and the packs are requested.
        state.toggle_sticker_picker();
        assert!(state.sticker_picker_open);
        let reqs = drain(&mut req_rx);
        assert!(
            matches!(reqs.last(), Some(Request::GetStickerSets)),
            "expected GetStickerSets, got {reqs:?}"
        );
        // Re-open (close + open) with packs still empty: requested once more
        // only after a fresh toggle — but once sets arrived, never again.
        state.escape();
        assert!(!state.sticker_picker_open, "Escape closes the picker");
        state.sticker_sets = vec![StickerSetBridge {
            title: "Happy Blocks".into(),
            short_name: "happy_blocks".into(),
            docs: vec![(900_000_000, 42, "🎉".to_string())],
        }];
        state.toggle_sticker_picker();
        let reqs = drain(&mut req_rx);
        assert!(
            !reqs.iter().any(|r| matches!(r, Request::GetStickerSets)),
            "packs must not be re-fetched when already loaded"
        );
        // Chat switch closes the picker but keeps the global packs.
        state.open_chat(43);
        assert!(!state.sticker_picker_open);
        assert_eq!(state.sticker_sets.len(), 1, "sets are global");
    }

    #[test]
    fn sticker_sets_arrive_and_pick_sends_request() {
        let (mut state, mut req_rx) = demo_state();
        state.toggle_sticker_picker();
        let _ = req_rx.try_recv();
        state.on_message(UiMessage::StickerSets(vec![StickerSetBridge {
            title: "Happy Blocks".into(),
            short_name: "happy_blocks".into(),
            docs: vec![
                (900_000_001, 11, "🎉".to_string()),
                (900_000_002, 22, "⭐".to_string()),
            ],
        }]));
        assert_eq!(state.sticker_sets.len(), 1);

        state.send_sticker(0, 1);
        let reqs = drain(&mut req_rx);
        assert!(
            matches!(
                reqs.last(),
                Some(Request::SendSticker { id: 42, doc_id: 900_000_002, access_hash: 22 })
            ),
            "expected SendSticker with the picked doc, got {reqs:?}"
        );
        assert!(!state.sticker_picker_open, "picker closes on send");
        // Optimistic local row (id 0) carrying the sticker meta.
        let last = state.messages.last().expect("optimistic row");
        assert_eq!(last.id, 0);
        assert_eq!(last.sticker.as_ref().map(|s| s.alt.as_str()), Some("⭐"));

        // The echo merges into the optimistic row and keeps the sticker.
        state.on_message(UiMessage::NewMessage {
            chat_id: 42,
            id: 777,
            text: String::new(),
            date: 500,
            out: true,
            photo: None,
            doc: None,
            sticker: Some(StickerMeta { alt: "⭐".into() }),
            reply_to: None,
            forwarded_from: None,
            sender_name: None,
            sender_id: None,
        });
        assert!(
            !state.messages.iter().any(|m| m.id == 0),
            "optimistic row merged"
        );
        let merged = state.messages.iter().find(|m| m.id == 777).expect("echo row");
        assert_eq!(merged.sticker.as_ref().map(|s| s.alt.as_str()), Some("⭐"));
        // The merge arms the image download for the merged row.
        let reqs = drain(&mut req_rx);
        assert!(
            matches!(
                reqs.last(),
                Some(Request::DownloadSticker { chat_id: 42, msg_id: 777 })
            ),
            "expected DownloadSticker for the merged row, got {reqs:?}"
        );

        // The image path lands on the right row.
        state.on_message(UiMessage::StickerPathReady {
            chat_id: 42,
            msg_id: 777,
            path: Some("/tmp/sticker.webp".into()),
        });
        let with_path = state.messages.iter().find(|m| m.id == 777).unwrap();
        assert_eq!(with_path.sticker_path.as_deref(), Some("/tmp/sticker.webp"));
    }

    #[test]
    fn incoming_sticker_updates_dialog_preview_and_row() {
        let (mut state, _) = demo_state();
        state.on_message(UiMessage::NewMessage {
            chat_id: 42,
            id: 900,
            text: String::new(),
            date: 600,
            out: false,
            photo: None,
            doc: None,
            sticker: Some(StickerMeta { alt: "🎉".into() }),
            reply_to: None,
            forwarded_from: None,
            sender_name: None,
            sender_id: None,
        });
        let row = state.messages.iter().find(|m| m.id == 900).expect("row pushed");
        assert_eq!(row.sticker.as_ref().map(|s| s.alt.as_str()), Some("🎉"));
        assert!(row.doc.is_none(), "stickers are not document cards");
        assert!(preview_text("", &None, &None, &row.sticker).contains("🎉"));
    }

    // -----------------------------------------------------------------
    // Settings panel
    // -----------------------------------------------------------------

    #[test]
    fn settings_open_close_requests_profile_and_sessions() {
        let (mut state, mut req_rx) = demo_state();
        state.open_settings();
        assert!(state.settings_open);
        assert!(state.cache_bytes.is_some(), "cache measured on open");
        let reqs = drain(&mut req_rx);
        assert!(reqs.iter().any(|r| matches!(r, Request::GetMe)));
        assert!(reqs.iter().any(|r| matches!(r, Request::GetSessions)));
        state.close_settings();
        assert!(!state.settings_open);
    }

    #[test]
    fn settings_survives_chat_switches() {
        let (mut state, _) = demo_state();
        state.open_settings();
        state.open_chat(43);
        assert!(state.settings_open, "settings is global, not chat-scoped");
    }

    #[test]
    fn settings_tabs_switch() {
        let (mut state, _) = demo_state();
        assert_eq!(state.settings_tab, SettingsTab::Profile);
        state.set_settings_tab(SettingsTab::Storage);
        assert_eq!(state.settings_tab, SettingsTab::Storage);
        state.set_settings_tab(SettingsTab::Sessions);
        assert_eq!(state.settings_tab, SettingsTab::Sessions);
    }

    #[test]
    fn profile_echo_seeds_edit_inputs_once() {
        let (mut state, _) = demo_state();
        state.on_message(UiMessage::MyProfile(MyProfile {
            name: "Demo User".into(),
            last_name: Some("User".into()),
            username: Some("demo_user".into()),
            phone: Some("+33612345678".into()),
            bio: Some("Hello".into()),
        }));
        assert_eq!(state.edit_name, "Demo");
        assert_eq!(state.edit_last_name, "User");
        assert_eq!(state.edit_bio, "Hello");
        // A later echo must NOT clobber typed text.
        state.edit_name = "Typed".into();
        state.on_message(UiMessage::MyProfile(MyProfile {
            name: "Demo User".into(),
            last_name: Some("User".into()),
            username: None,
            phone: None,
            bio: None,
        }));
        assert_eq!(state.edit_name, "Typed");
        assert_eq!(
            state.my_profile.as_ref().unwrap().name,
            "Demo User"
        );
    }

    #[test]
    fn profile_update_request_shape() {
        let (mut state, mut req_rx) = demo_state();
        state.edit_name = "  Alice  ".into();
        state.edit_last_name = "  Smith  ".into();
        state.edit_bio = "  Rust dev  ".into();
        state.submit_profile_edit();
        let reqs = drain(&mut req_rx);
        assert!(
            reqs.iter().any(|r| matches!(
                r,
                Request::UpdateProfile {
                    first_name: Some(f),
                    last_name: Some(l),
                    bio: Some(b),
                } if f == "Alice" && l == "Smith" && b == "Rust dev"
            )),
            "expected trimmed UpdateProfile request, got {reqs:?}"
        );
        // Empty first name never submits.
        state.edit_name = "   ".into();
        state.submit_profile_edit();
        assert!(drain(&mut req_rx).is_empty());
    }

    #[test]
    fn sessions_arrive_and_revoke_removes_row() {
        let (mut state, mut req_rx) = demo_state();
        state.open_settings();
        drain(&mut req_rx);
        state.on_message(UiMessage::Sessions(vec![
            SessionInfo {
                device: "This device".into(),
                platform: "Linux".into(),
                country: "France".into(),
                current: true,
                hash: 111,
            },
            SessionInfo {
                device: "iPhone".into(),
                platform: "iOS".into(),
                country: "France".into(),
                current: false,
                hash: 222,
            },
        ]));
        assert_eq!(state.sessions.len(), 2);
        state.revoke_session(222);
        assert!(drain(&mut req_rx)
            .iter()
            .any(|r| matches!(r, Request::RevokeSession { hash: 222 })));
        // Optimistic removal only via the authoritative echo.
        assert_eq!(state.sessions.len(), 2);
        state.on_message(UiMessage::SessionRevoked { hash: 222 });
        assert_eq!(state.sessions.len(), 1);
        assert_eq!(state.sessions[0].hash, 111);
    }

    #[test]
    fn clear_cache_confirmation_flow() {
        let (mut state, mut req_rx) = demo_state();
        state.ask_clear_cache();
        assert!(state.confirm_clear_cache);
        // Cancel path sends nothing.
        state.cancel_clear_cache();
        assert!(drain(&mut req_rx).is_empty());
        state.ask_clear_cache();
        state.confirm_clear_cache_now();
        let reqs = drain(&mut req_rx);
        assert!(reqs.iter().any(|r| matches!(r, Request::ClearCache)));
        assert!(!state.confirm_clear_cache);
        // The echo refreshes the size and keeps the confirmation down.
        state.on_message(UiMessage::CacheCleared { bytes: 0 });
        assert_eq!(state.cache_bytes, Some(0));
        assert!(!state.confirm_clear_cache);
        // Confirm without arming does nothing.
        state.confirm_clear_cache_now();
        assert!(drain(&mut req_rx).is_empty());
    }

    #[test]
    fn logout_confirmation_flow() {
        let (mut state, mut req_rx) = demo_state();
        state.ask_logout();
        assert!(state.confirm_logout);
        // Cancel path sends nothing.
        state.cancel_logout();
        assert!(drain(&mut req_rx).is_empty());
        state.ask_logout();
        state.confirm_logout_now();
        let reqs = drain(&mut req_rx);
        assert!(reqs.iter().any(|r| matches!(r, Request::LogOut)));
        assert!(!state.confirm_logout);
        // Confirm without arming does nothing.
        state.confirm_logout_now();
        assert!(drain(&mut req_rx).is_empty());
    }

    #[test]
    fn logged_out_resets_to_sign_in() {
        let (mut state, _) = demo_state();
        state.on_message(UiMessage::Dialogs(vec![ChatRow {
            id: 42,
            title: "Camille".into(),
            subtitle: "Hello!".into(),
            date: 100,
            unread: 1,
            avatar_path: None,
        }]));
        state.on_message(UiMessage::Messages {
            id: 42,
            title: "Camille".into(),
            rows: vec![MsgRow::text(1, "incoming", 100, false)],
        });
        state.my_profile = Some(MyProfile {
            name: "Demo User".into(),
            ..MyProfile::default()
        });
        state.sessions.push(SessionInfo {
            device: "This device".into(),
            platform: "Linux".into(),
            country: "France".into(),
            current: true,
            hash: 111,
        });
        state.settings_open = true;
        state.settings_tab = SettingsTab::Sessions;
        state.edit_name = "Demo".into();
        state.search_mode = Some(SearchMode::Global);
        state.search_query = "rust".into();
        state.reply_target = Some(ReplyTarget { msg_id: 1, snippet: "s".into() });
        state.scroll_offset = 250.0;
        let epoch_before = state.layout_epoch;

        state.on_message(UiMessage::LoggedOut);

        assert!(!state.authenticated);
        assert_eq!(state.login_step, LoginStep::Phone);
        assert!(state.login_input.is_empty());
        assert!(state.dialogs.is_empty());
        assert!(state.messages.is_empty());
        assert_eq!(state.open_chat, None);
        assert!(state.chat_title.is_empty());
        assert!(!state.loading);
        assert!(state.dialog_short.is_empty());
        assert_eq!(state.scroll_offset, 0.0);
        assert_eq!(state.dialog_scroll_offset, 0.0);
        assert!(state.context_menu.is_none());
        assert!(state.composer.is_empty());
        assert!(state.viewer.is_none());
        assert!(state.reply_target.is_none());
        assert!(!state.info_open);
        assert!(state.chat_info.is_none());
        assert!(state.participants.is_empty());
        assert!(state.search_mode.is_none());
        assert!(state.search_query.is_empty());
        assert!(state.search_hits.is_empty());
        assert!(state.my_profile.is_none());
        assert!(state.sessions.is_empty());
        assert!(!state.settings_open);
        assert_eq!(state.settings_tab, SettingsTab::Profile);
        assert!(state.edit_name.is_empty());
        assert_ne!(state.layout_epoch, epoch_before, "layout cache invalidated");
        // Kept on purpose.
        let theme_before = state.theme_mode;
        state.on_message(UiMessage::LoggedOut);
        assert_eq!(state.theme_mode, theme_before, "theme survives logout");
    }

    #[test]
    fn notifications_flip_updates_shared_flag_and_persists() {
        use std::sync::atomic::Ordering;
        let dir = std::env::temp_dir().join(format!("telegram-rs-notif-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notifications");

        assert!(read_notifications_at(&path), "missing file = enabled");
        write_notifications_at(&path, false);
        assert!(!read_notifications_at(&path));
        write_notifications_at(&path, true);
        assert!(read_notifications_at(&path));

        let (mut state, _) = demo_state();
        assert!(state.notifications_on);
        state.toggle_notifications();
        assert!(!state.notifications_on);
        assert!(
            !state.notify_pref.0.load(Ordering::SeqCst),
            "shared flag flipped"
        );
        state.toggle_notifications();
        assert!(state.notify_pref.0.load(Ordering::SeqCst));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn escape_closes_settings_before_context_menu() {
        let (mut state, _) = demo_state();
        state.open_settings();
        state.context_menu = Some(ContextMenu { row: 0 });
        state.escape();
        assert!(!state.settings_open);
        assert!(
            state.context_menu.is_some(),
            "settings closes first; menu untouched"
        );
        state.escape();
        assert!(state.context_menu.is_none());
    }

    // -----------------------------------------------------------------
    // Forum topics (chips bar of forum supergroups)
    // -----------------------------------------------------------------

    /// A forum chat (the demo Rust Group, id 1002): topics loaded, mixed
    /// history with a canned "Announcements" thread (root 20) and one
    /// general row.
    fn forum_state() -> (State, tokio::sync::mpsc::UnboundedReceiver<Request>) {
        let (req_tx, req_rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
        let mut state = State::new(req_tx);
        state.authenticated = true;
        state.open_chat = Some(1002);
        state.topic_is_forum = true;
        state.topic_topics = vec![
            crate::bridge::TopicRow {
                id: 20,
                root_msg_id: 20,
                title: "Announcements".into(),
                icon_color: 0,
            },
            crate::bridge::TopicRow {
                id: 30,
                root_msg_id: 30,
                title: "PR review".into(),
                icon_color: 1,
            },
        ];
        state.topic_all_messages = vec![
            MsgRow::text(20, "topic root", 100, false),
            MsgRow {
                reply_to: Some(20),
                ..MsgRow::text(21, "in the thread", 110, false)
            },
            MsgRow::text(50, "general row", 120, false),
        ];
        state.messages =
            State::topic_view(&state.topic_all_messages, state.topic_selected);
        (state, req_rx)
    }

    #[test]
    fn topic_chips_visible_only_for_loaded_forum_topics() {
        let (mut state, _) = forum_state();
        assert!(state.topic_bar_visible());

        // A plain (non-forum) chat: no flag, no bar — even the topics
        // payload alone would not turn it on.
        state.topic_is_forum = false;
        assert!(!state.topic_bar_visible());
        state.topic_is_forum = true;

        // Topics not loaded yet: no bar (no empty strip while loading).
        state.topic_topics.clear();
        assert!(!state.topic_bar_visible());
    }

    #[test]
    fn topic_selection_filters_the_visible_list() {
        let (mut state, _) = forum_state();
        assert_eq!(state.messages.len(), 3, "All messages shows everything");

        // Selecting a topic keeps only the thread (root + anchored rows).
        state.topic_select(Some(20));
        assert_eq!(
            state.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![20, 21]
        );
        // The full history is untouched.
        assert_eq!(state.topic_all_messages.len(), 3);

        // Back to "All messages".
        state.topic_select(None);
        assert_eq!(state.messages.len(), 3);

        // A topic without loaded rows renders an empty view (slice-1
        // client-side filter; older messages may not be cached).
        state.topic_select(Some(30));
        assert!(state.messages.is_empty());
    }

    #[test]
    fn topic_create_flow_validates_and_echoes() {
        let (mut state, mut req_rx) = forum_state();
        state.topic_open_create();
        assert!(state.topic_creating);

        // Empty title: rejected, no request, field stays open.
        state.topic_submit_create();
        assert!(drain(&mut req_rx).is_empty());
        assert!(state.topic_creating, "invalid input keeps the field open");

        // Whitespace is trimmed before the request goes out.
        state.topic_title = "  Release  ".into();
        state.topic_submit_create();
        let reqs = drain(&mut req_rx);
        assert!(
            reqs.iter()
                .any(|r| matches!(r, Request::CreateTopic { id: 1002, title }
                    if title.as_str() == "Release")),
            "expected CreateTopic, got {reqs:?}"
        );

        // Server echo: appended, selected, field closed.
        state.on_message(UiMessage::TopicCreated {
            chat_id: 1002,
            topic: crate::bridge::TopicRow {
                id: 60,
                root_msg_id: 60,
                title: "Release".into(),
                icon_color: 3,
            },
        });
        assert_eq!(state.topic_topics.len(), 3);
        assert_eq!(state.topic_selected, Some(60));
        assert!(!state.topic_creating);
        assert!(state.topic_title.is_empty());
    }

    #[test]
    fn topic_composer_routes_plain_posts_into_the_thread() {
        let (mut state, mut req_rx) = forum_state();
        state.topic_select(Some(20));
        state.composer = "hello thread".into();
        state.submit();
        let reqs = drain(&mut req_rx);
        assert!(
            reqs.iter().any(|r| matches!(r,
                Request::SendTopicMessage { id: 1002, topic_root: 20, .. })),
            "expected SendTopicMessage, got {reqs:?}"
        );
        // The optimistic row landed in the view AND the full history.
        assert!(state
            .messages
            .iter()
            .any(|m| m.id == 0 && m.text == "hello thread"));
        assert!(state.topic_all_messages.iter().any(|m| m.id == 0));

        // The server echo merges the optimistic row in BOTH stores and
        // anchors it to the thread (reply header = topic root).
        state.on_message(UiMessage::NewMessage {
            chat_id: 1002,
            id: 70,
            text: "hello thread".into(),
            date: 130,
            out: true,
            photo: None,
            doc: None,
            sticker: None,
            reply_to: Some(20),
            forwarded_from: None,
            sender_name: None,
            sender_id: None,
        });
        assert!(state.messages.iter().all(|m| m.id != 0), "row deduped");
        assert!(state.topic_all_messages.iter().all(|m| m.id != 0));
        assert_eq!(
            state.messages.iter().find(|m| m.id == 70).map(|m| m.reply_to),
            Some(Some(20))
        );

        // No selection: the plain send path (no topic_root).
        state.topic_select(None);
        state.composer = "general".into();
        state.submit();
        let reqs = drain(&mut req_rx);
        assert!(
            reqs.iter()
                .any(|r| matches!(r, Request::SendMessage { id: 1002, .. }))
        );
        assert!(
            !reqs.iter()
                .any(|r| matches!(r, Request::SendTopicMessage { .. }))
        );
    }

    #[test]
    fn topic_state_resets_on_chat_switch() {
        let (mut state, mut req_rx) = forum_state();
        state.topic_select(Some(20));
        state.topic_open_create();
        state.topic_title = "half typed".into();

        state.open_chat(1004);
        assert_eq!(state.topic_selected, None);
        assert!(!state.topic_is_forum);
        assert!(state.topic_topics.is_empty());
        assert!(!state.topic_creating);
        assert!(state.topic_title.is_empty());
        assert!(state.topic_all_messages.is_empty());
        let reqs = drain(&mut req_rx);
        assert!(
            reqs.iter()
                .any(|r| matches!(r, Request::GetTopics { id: 1004 })),
            "chat switch requests the new chat's topics, got {reqs:?}"
        );
    }

    #[test]
    fn topic_new_message_outside_the_selection_stays_out_of_the_view() {
        let (mut state, _) = forum_state();
        state.topic_select(Some(20));
        state.on_message(UiMessage::NewMessage {
            chat_id: 1002,
            id: 80,
            text: "general arrival".into(),
            date: 140,
            out: false,
            photo: None,
            doc: None,
            sticker: None,
            reply_to: None,
            forwarded_from: None,
            sender_name: None,
            sender_id: None,
        });
        // Recorded in the full history, kept out of the filtered view.
        assert!(state.topic_all_messages.iter().any(|m| m.id == 80));
        assert!(state.messages.iter().all(|m| m.id != 80));

        // Selecting its topic would surface it (thread rows only).
        state.topic_select(None);
        assert!(state.messages.iter().any(|m| m.id == 80));
    }

    #[test]
    fn escape_closes_the_topic_create_field_before_other_overlays() {
        let (mut state, _) = forum_state();
        state.topic_open_create();
        state.info_open = true; // another overlay is up too
        state.escape();
        assert!(!state.topic_creating, "the create field closes first");
        assert!(state.info_open, "only the topic field closed");
        state.escape();
        assert!(!state.info_open);
    }
}
