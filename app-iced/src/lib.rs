//! `app-iced`: the same Telegram client, but with the UI rendered by Iced
//! (tiny-skia software backend) instead of the custom winit renderer.
//!
//! Experimental prototype on `experiment/iced`: proves RAM consumption and
//! testability of an Iced-based build before committing to a migration.
//!
//! A library target (`app_iced`) is exposed so the `benches/frame.rs`
//! performance harness can drive the exact same view code headlessly.

pub mod audio;
pub mod bridge;
pub mod emoji;
pub mod icons;
pub mod network;
pub mod qr_png;
pub mod state;
pub mod theme;
pub mod tray;
use std::collections::HashMap;
use std::sync::atomic::Ordering;

use iced::widget::{
    button, column, container, image, mouse_area, rich_text, row, scrollable, text, text_input,
};
use iced::{Alignment, Color, Length, Task};

use bridge::{ChatKind, ChatRow, CodeSpan, MsgRow, Request, UiMessage};
use icons::{icon, Icon};
use state::{LoginStep, QrStage, SearchMode, State};

/// Id of the open chat's message list, used to auto-scroll to the bottom.
const MSG_LIST_ID: &str = "msg-list";
/// Id of the dialog (chat) list, used to feed its scroll offset into the
/// dialog-list virtualization.
const DIALOG_LIST_ID: &str = "dialog-list";

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

type Element<'a> = iced::Element<'a, Message>;

fn rgb(c: (u8, u8, u8)) -> Color {
    Color::from_rgb8(c.0, c.1, c.2)
}

/// True when a picked file looks like an image (compressed-photo send).
pub fn looks_like_image(path: &str) -> bool {
    tg::client::is_image(std::path::Path::new(path))
}

/// Characters that terminate an inline URL (whitespace + closing
/// punctuation). `.`/`:`/`?`/`!` are NOT terminators — they occur inside
/// URLs ("www.", "https://", "?query=1"); trailing ones are stripped from
/// the match instead.
fn is_url_end(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            ',' | ';' | ')' | ']' | '}' | '>' | '<' | '"' | '\'' | '('
        )
}

/// Byte offset of the first inline URL in `s`, if any.
fn find_url(s: &str) -> Option<usize> {
    ["https://", "http://", "www."]
        .iter()
        .filter_map(|p| s.find(p))
        .min()
}

/// Splits `s` into spans, turning http(s)/`www.` URLs into accent-colored,
/// underlined, clickable spans.
///
/// `None` when `s` has no URL: callers keep the plain `text` widget so the
/// rich-text path is only paid for messages that actually contain links
/// (scroll perf: most rows stay on the plain-text hot path).
fn linkify(s: &str) -> Option<Vec<iced::widget::text::Span<'_, String>>> {
    use iced::widget::text::Span;

    let mut spans: Vec<Span<'_, String>> = Vec::new();
    let mut rest = s;
    while let Some(at) = find_url(rest) {
        let (head, from) = rest.split_at(at);
        let url_len = from
            .char_indices()
            .find(|(_, c)| is_url_end(*c))
            .map(|(i, _)| i)
            .unwrap_or(from.len());
        let (matched, _) = from.split_at(url_len);
        // Sentence punctuation glued to the URL ("…voir https://a.io." / ",…")
        // belongs to the text, not the link.
        let stripped = matched
            .trim_end_matches(['.', ',', ':', ';', '!', '?'])
            .len();
        let (url, tail) = from.split_at(stripped);
        if url.len() > 4 || url.starts_with("http") {
            if !head.is_empty() {
                spans.push(Span::new(head));
            }
            spans.push(
                Span::new(url)
                    .link(url.to_string())
                    .color(rgb(theme::ACCENT()))
                    .underline(true),
            );
            rest = tail;
        } else {
            // Bare "www." with nothing after it — plain text, keep scanning.
            spans.push(Span::new(&rest[..at + 4]));
            rest = &rest[at + 4..];
        }
    }
    // Only worth the rich-text widget when an actual link was found.
    if !spans.iter().any(|s| s.link.is_some()) {
        return None;
    }
    if !rest.is_empty() {
        spans.push(Span::new(rest));
    }
    Some(spans)
}

/// One-line list-row text (chat names / previews): emoji runs get the
/// color-emoji font; `None` keeps the cheap plain `text` widget.
///
/// Same rationale as [`body_spans`] but without link handling — preview
/// lines are single-line, pre-ellipsized labels.
fn line_spans(t: &str) -> Option<Vec<iced::widget::text::Span<'_, String>>> {
    use iced::widget::text::Span;

    let runs = crate::emoji::emoji_ranges(t);
    if runs.is_empty() {
        return None;
    }
    let mut spans: Vec<Span<'_, String>> = Vec::new();
    let mut cursor = 0;
    for run in runs {
        if run.start > cursor {
            spans.push(Span::new(&t[cursor..run.start]));
        }
        spans.push(Span::new(&t[run.clone()]).font(emoji_font()));
        cursor = run.end;
    }
    if cursor < t.len() {
        spans.push(Span::new(&t[cursor..]));
    }
    Some(spans)
}

/// Message body / caption spans: `None` when the text is plain (no link, no
/// emoji) so callers keep the cheap `text` widget (scroll-perf: most rows
/// stay on the plain-text hot path).
///
/// Carve `(s, e, kind)` styled sub-ranges out of a flat segment list. Each
/// input segment is either kept whole, split on a `kind` boundary, or replaced
/// (when already a non-default style wins over a later carve).
fn carve_segments(segs: &mut Vec<(usize, usize, u8)>, s: usize, e: usize, kind: u8) {
    if s >= e {
        return;
    }
    let old = std::mem::take(segs);
    let mut out: Vec<(usize, usize, u8)> = Vec::with_capacity(old.len() + 2);
    for (a, b, k) in old {
        if e <= a || s >= b {
            out.push((a, b, k));
            continue;
        }
        if a < s {
            out.push((a, s, k));
        }
        let lo = a.max(s);
        let hi = b.min(e);
        if lo < hi {
            out.push((lo, hi, if k != 0 { k } else { kind }));
        }
        if e < b {
            out.push((e, b, k));
        }
    }
    *segs = out;
}

/// Span builder for a slice of message text. Splits on `code` monospace
/// ranges (byte offsets), color-emoji runs and links, and falls back to
/// `None` (plain rendering) when nothing needs specialised styling.
///
/// `code` ranges are relative byte offsets into `t`; spans overlapping one
/// are rendered in the monospace [`code_font`].
fn body_spans<'a>(
    t: &'a str,
    code: &[(usize, usize)],
) -> Option<Vec<iced::widget::text::Span<'a, String>>> {
    use iced::widget::text::Span;

    let runs = crate::emoji::emoji_ranges(t);
    if runs.is_empty() && code.is_empty() {
        return linkify(t);
    }

    // 0 = default, 1 = monospace code, 2 = color-emoji font.
    let mut segs: Vec<(usize, usize, u8)> = vec![(0, t.len(), 0)];
    for &(s, e) in code {
        carve_segments(&mut segs, s.min(t.len()), e.min(t.len()), 1);
    }
    for run in &runs {
        carve_segments(&mut segs, run.start, run.end, 2);
    }

    let mut spans: Vec<Span<'_, String>> = Vec::new();
    for (a, b, kind) in segs {
        let slice = &t[a..b];
        if slice.is_empty() {
            continue;
        }
        match kind {
            1 => spans.push(Span::new(slice).font(code_font())),
            2 => spans.push(Span::new(slice).font(emoji_font())),
            _ => spans.extend(linkify(slice).unwrap_or_else(|| vec![Span::new(slice)])),
        }
    }
    Some(spans)
}

/// Monospace font for code spans and blocks (resolved by the text engine to a
/// system monospace face on target desktops).
fn code_font() -> iced::Font {
    iced::Font::MONOSPACE
}

/// Message body / caption text: plain `text` when there is no URL and no
/// emoji and no code, clickable/color rich spans otherwise. `pre` blocks are
/// pulled out of the flow and rendered as self-contained monospace cards;
/// inline `code` spans inside the surrounding text are styled monospaced.
/// `WordOrGlyph` wrapping: a word longer than the bubble (a long URL) breaks
/// at glyph level instead of overflowing.
fn message_body<'a>(t: &'a str, code: &'a [CodeSpan]) -> Element<'a> {
    // Split the text around `pre` (block) entities into alternating
    // text / block parts (byte ranges relative to `t`).
    let mut parts: Vec<(std::ops::Range<usize>, bool)> = Vec::new();
    let mut cursor = 0usize;
    for c in code {
        if c.block.is_none() {
            continue;
        }
        if c.start < cursor || c.end <= c.start || c.end > t.len() {
            continue;
        }
        if c.start > cursor {
            parts.push((cursor..c.start, false));
        }
        parts.push((c.start..c.end, true));
        cursor = c.end;
    }
    if cursor < t.len() {
        parts.push((cursor..t.len(), false));
    }

    match parts.as_slice() {
        [] => inline_body(t, local_code(code, &(0..t.len()))),
        [(r, true)] => code_block(lang_of(code, r), &t[r.clone()]),
        [(r, false)] => inline_body(&t[r.clone()], local_code(code, r)),
        _ => {
            let mut col: iced::widget::Column<'a, Message> = column![].spacing(6);
            for (r, is_block) in parts {
                col = if is_block {
                    col.push(code_block(lang_of(code, &r), &t[r.clone()]))
                } else {
                    col.push(inline_body(&t[r.clone()], local_code(code, &r)))
                };
            }
            col.into()
        }
    }
}

/// A text-only message body (possibly with inline `code` spans styled
/// monospace). Plain `text` when nothing needs styling.
fn inline_body<'a>(t: &'a str, code: Vec<(usize, usize)>) -> Element<'a> {
    match body_spans(t, &code) {
        Some(spans) => rich_text(spans)
            .on_link_click(Message::OpenUrl)
            .size(theme::font::MESSAGE)
            .color(rgb(theme::TEXT_PRIMARY()))
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
            .into(),
        None => text(t)
            .size(theme::font::MESSAGE)
            .color(rgb(theme::TEXT_PRIMARY()))
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
            .into(),
    }
}

/// A `pre` (block) code card: dark surface, monospace body and an optional
/// language tag, matching the bubble's corner radius.
fn code_block<'a>(lang: Option<&'a str>, body: &'a str) -> Element<'a> {
    let mut col: iced::widget::Column<'a, Message> = column![].spacing(6);
    if let Some(l) = lang {
        col = col.push(
            text(l)
                .size(theme::font::CODE_TAG)
                .font(code_font())
                .color(rgb(theme::TEXT_SECONDARY())),
        );
    }
    col = col.push(
        text(body)
            .size(theme::font::CODE)
            .font(code_font())
            .color(rgb(theme::TEXT_PRIMARY()))
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
    );
    container(col)
        .width(iced::Length::Fill)
        .padding([8, 10])
        .style(|_| container::Style {
            background: Some(iced::Background::Color(rgb(theme::CODE_BG()))),
            border: iced::Border::default()
                .color(rgb(theme::CODE_BORDER()))
                .width(1)
                .rounded(8),
            ..container::Style::default()
        })
        .into()
}

/// The inline `code` spans (relative to a segment) whose byte range overlaps
/// `range`.
fn local_code(code: &[CodeSpan], range: &std::ops::Range<usize>) -> Vec<(usize, usize)> {
    code.iter()
        .filter(|c| c.block.is_none())
        .filter_map(|c| {
            let s = c.start.max(range.start);
            let e = c.end.min(range.end);
            if s < e {
                Some((s - range.start, e - range.start))
            } else {
                None
            }
        })
        .collect()
}

/// The `pre` language of the block starting at `range.start`, if any.
fn lang_of<'a>(code: &'a [CodeSpan], range: &std::ops::Range<usize>) -> Option<&'a str> {
    code.iter()
        .find(|c| {
            c.block.is_some() && c.start == range.start && c.start < range.end && range.end <= c.end
        })
        .and_then(|c| c.block.as_deref())
        .filter(|l| !l.is_empty())
}

/// UI → application messages.
#[derive(Debug, Clone)]
pub enum Message {
    /// A message from the network runtime.
    Ui(UiMessage),
    /// A chat row was clicked.
    OpenChat(i64),
    /// Go back from the chat to the list.
    BackToChats,
    /// A message row was clicked (left).
    RowClicked(usize),
    /// A message row was right-clicked (open context menu).
    RowContext(usize),
    /// "Edit" pressed in the context menu.
    ContextEdit,
    /// "Copy" pressed in the context menu.
    ContextCopy,
    /// "Delete" pressed in the context menu.
    ContextDelete,
    /// "Reply" pressed in the context menu.
    ContextReply,
    /// "Pin"/"Unpin" pressed in the context menu.
    ContextPin,
    /// The pinned-message banner was clicked: jump to the message.
    PinnedClicked,
    // -----------------------------------------------------------------
    // Group/channel creation + chat management
    // -----------------------------------------------------------------
    /// The list header's "+" button: toggle the New Group/Channel picker.
    OpenCreateMenu,
    /// Picker item: create a group.
    CreateGroup,
    /// Picker item: create a channel.
    CreateChannel,
    /// The creation modal's title field changed.
    CreateTitleChanged(String),
    /// The creation modal's description field changed (channels).
    CreateAboutChanged(String),
    /// A contact checkbox in the member picker was toggled.
    ToggleMember(usize),
    /// The creation modal's "Create" button.
    SubmitCreate,
    /// The creation modal was cancelled (✕ / Escape).
    CancelCreate,
    /// A chat-list row was right-clicked: open its Leave/Delete mini menu.
    RowMenu(i64),
    /// A Leave/Delete confirmation was requested for a chat.
    AskConfirm(state::ConfirmKind, i64),
    /// The confirmation dialog's destructive button was pressed.
    ConfirmYes,
    /// The confirmation dialog was cancelled.
    ConfirmNo,
    /// The chat header (or ℹ️) was clicked: toggle the right info panel.
    ToggleInfo,
    /// The info panel's ✕ was pressed.
    CloseInfo,
    /// The info panel's Mute button was pressed.
    ToggleMute,
    ToggleBlock,
    /// The @username line in the info panel was clicked (copy).
    CopyUsername,
    /// "Remove" was clicked on a member row (arms the inline confirmation).
    KickMember(i64),
    /// The inline kick confirmation was accepted.
    ConfirmKick,
    /// A member row was right-clicked: open its admin context menu.
    MemberMenu(i64),
    /// An admin action was picked in the member context menu.
    AdminAction(state::AdminAction, i64),
    /// "Yes" was pressed on the destructive admin confirmation (ban/remove).
    AdminConfirmYes,
    /// The reply bar's ✕ was pressed (cancel the armed reply).
    CancelReply,
    /// "Forward" pressed in the context menu (opens the chat picker).
    ContextForward,
    /// A destination chat was picked in the forward overlay.
    ForwardTo(i64),
    /// The attach 📎 button was pressed: open a file dialog.
    AttachFile,
    /// The file dialog returned a path (None = cancelled).
    FilePicked(Option<String>),
    /// The sticker button was pressed: toggle the picker panel.
    ToggleStickerPicker,
    /// A sticker thumbnail was clicked in the picker: `(set index, doc index)`.
    StickerPicked(usize, usize),
    /// The sticker picker panel was closed (✕).
    CloseStickerPicker,
    /// The context menu was dismissed.
    DismissMenu,
    /// Clicking elsewhere: dismiss any open overlay (menu and/or strip).
    CloseOverlays,
    /// The context menu React item was pressed: open the reaction strip.
    ContextReact,
    /// An emoji was picked in the reaction strip: send the reaction.
    React(String),
    /// The reaction strip was dismissed without reacting.
    CloseReact,
    /// The strip's "+" button: toggle the expanded emoji picker grid.
    ToggleReactPicker,
    // -----------------------------------------------------------------
    // Settings panel
    // -----------------------------------------------------------------
    /// The list header's gear button: toggle the settings sheet.
    ToggleSettings,
    /// The settings sheet's ✕ was pressed.
    CloseSettings,
    /// A settings tab header was clicked.
    SettingsTab(state::SettingsTab),
    /// The First name field of the profile edit form changed.
    SettingsNameChanged(String),
    /// The Last name field of the profile edit form changed.
    SettingsLastNameChanged(String),
    /// The Bio field of the profile edit form changed.
    SettingsBioChanged(String),
    /// The profile edit form's Save button.
    SaveProfile,
    /// The notifications On/Off button was pressed.
    ToggleNotifications,
    /// The "Clear cache" button was pressed (arms the confirmation).
    AskClearCache,
    /// The inline clear-cache confirmation was cancelled.
    CancelClearCache,
    /// The inline clear-cache confirmation was accepted.
    ConfirmClearCache,
    /// The "Log out" button was pressed (arms the confirmation).
    AskLogout,
    /// The inline logout confirmation was cancelled.
    CancelLogout,
    /// The inline logout confirmation was accepted.
    DoLogout,
    /// "Terminate" was clicked on a session row.
    RevokeSession(i64),
    /// Flip dark ⇄ light (Settings panel). `State::toggle_theme` applies the
    /// palette and persists the choice.
    ToggleTheme,
    /// Escape: close the context menu, cancel editing or close the viewer.
    Escape,
    /// The list header's search icon: global search.
    OpenGlobalSearch,
    /// The conversation header's search icon: in-chat search.
    OpenInChatSearch,
    /// Close the search UI (✕ / Escape).
    CloseSearch,
    /// The search field changed (debounced/throttled by the network layer).
    SearchChanged(String),
    /// A search result row was clicked.
    SearchHitClicked(usize),
    /// A voice note was clicked: play / pause / stop it.
    VoiceClicked {
        chat_id: i64,
        msg_id: i32,
        path: String,
    },
    /// Periodic tick while a voice note is playing (progress + completion).
    VoiceTick,
    /// The voice progress slider was moved: seek to `f32` seconds.
    VoiceSeek(f32),
    /// A link inside a message was clicked: open it in the system browser.
    OpenUrl(String),
    /// Composer text changed.
    ComposerChanged(String),
    /// The composer's smiley button toggled the emoji panel.
    EmojiToggle,
    /// An emoji was picked in the panel: append it to the composer.
    EmojiPicked(String),
    /// A click outside the emoji panel dismissed it.
    EmojiDismiss,
    /// Composer submitted (send / edit).
    Submit,
    /// Viewer close / back.
    CloseViewer,
    /// Login field changed.
    LoginChanged(String),
    /// Login step submitted (phone / code / password).
    LoginSubmit,
    /// Login "back" pressed.
    LoginBack,
    // QR sign-in pane
    /// The [Phone | QR] switcher was pressed (toggles + starts/stops polling).
    ToggleLoginScreen,
    /// Leaving the QR pane ("Use phone number instead"): stop token polling.
    QrCancel,
    /// The message list was scrolled: carries the absolute Y offset of the
    /// scrollable's viewport (content coordinates). Feeds message-list
    /// virtualization so only the visible rows are built/layed-out per frame.
    Scrolled(f32),
    /// The dialog (chat) list was scrolled: feeds dialog-list virtualization.
    DialogScrolled(f32),
    /// Periodic tick (only useful with `--perf`): samples the frame cadence.
    PerfTick,
    /// Continuous-redraw tick (see `--continuous`): only asks for a redraw.
    PerfTickC,
    // -----------------------------------------------------------------
    // Forum topics (chips bar of forum supergroups)
    // -----------------------------------------------------------------
    /// A topic chip was picked (`None` = the "All messages" chip).
    TopicChipPicked(Option<i32>),
    /// The "+" chip opened the inline create-topic field.
    TopicCreateOpen,
    /// The create-topic title field changed.
    TopicCreateTitle(String),
    /// The create-topic field was submitted (Enter / "Add").
    TopicCreateSubmit,
    /// The create-topic field was cancelled (✕).
    TopicCreateCancel,
}

fn boot() -> (State, Task<Message>) {
    let demo = std::env::args().any(|a| a == "--demo");
    let open_first = std::env::args().any(|a| a == "--open-first");
    let big = std::env::args().any(|a| a == "--demo-big");
    let perf = std::env::args().any(|a| a == "--perf");
    let continuous = std::env::args().any(|a| a == "--continuous");
    let scroll_perf = std::env::args()
        .find_map(|a| {
            a.strip_prefix("--scroll-perf=")
                .and_then(|v| v.parse::<f32>().ok())
        })
        .unwrap_or(0.0);
    // Shared notifications preference: the panel flips it, the network
    // runtime consults it (same wiring pattern as the tray flags).
    let notify_pref = std::sync::Arc::new(network::NotifyPref::default());
    let req_tx = network::spawn_network(demo, big, notify_pref.clone());
    // System tray: best-effort (silent no-op without a StatusNotifier host).
    let tray_actions = std::sync::Arc::new(tray::TrayActions::default());
    tray::start(tray_actions.clone());
    let mut state = State::new(req_tx);
    state.tray_actions = tray_actions;
    state.notify_pref = notify_pref;
    let state = state
        .with_auto_open_first(open_first || demo)
        .with_persist_ui(!demo)
        .with_perf(perf)
        .with_continuous(continuous)
        .with_scroll_perf(scroll_perf);
    (state, Task::none())
}

fn update(state: &mut State, msg: Message) -> Task<Message> {
    match msg {
        Message::Ui(ui) => {
            // Once the dialog list arrives: re-open the last-used chat when
            // one was persisted, else fall back to "open the first chat".
            if matches!(ui, UiMessage::Dialogs(_)) {
                state.on_message(ui);
                if state.open_chat.is_none() {
                    let wanted = state
                        .initial_chat
                        .take()
                        .filter(|id| state.dialogs.iter().any(|d| d.id == *id));
                    let target = wanted.or_else(|| {
                        state
                            .auto_open_first
                            .then(|| state.dialogs.first().map(|d| d.id))
                            .flatten()
                    });
                    if let Some(id) = target {
                        state.open_chat(id);
                    }
                }
            } else {
                state.on_message(ui);
            }
        }
        Message::OpenChat(id) => state.open_chat(id),
        Message::BackToChats => state.close_chat(),
        Message::RowClicked(row) => state.click(row),
        Message::RowContext(row) => state.open_context(row),
        Message::ContextEdit => state.context_edit(),
        Message::ContextCopy => {
            if let Some(text) = state.context_copy() {
                return iced::clipboard::write::<Message>(text);
            }
        }
        Message::ContextDelete => state.context_delete(),
        Message::ContextReply => state.context_reply(),
        Message::ContextPin => state.context_pin(),
        Message::PinnedClicked => state.jump_to_pinned(),
        Message::OpenCreateMenu => state.toggle_create_menu(),
        Message::CreateGroup => state.open_create(state::CreateKind::Group),
        Message::CreateChannel => state.open_create(state::CreateKind::Channel),
        Message::CreateTitleChanged(t) => state.create_title = t,
        Message::CreateAboutChanged(a) => state.create_about = a,
        Message::ToggleMember(idx) => state.toggle_member(idx),
        Message::SubmitCreate => state.submit_create(),
        Message::CancelCreate => state.cancel_create(),
        Message::RowMenu(id) => state.open_row_menu(id),
        Message::AskConfirm(kind, id) => state.ask_confirm(kind, id),
        Message::ConfirmYes => state.confirm_yes(),
        Message::ConfirmNo => state.cancel_confirm(),
        Message::ToggleInfo => {
            if state.info_open {
                state.close_info();
            } else {
                state.open_info();
            }
        }
        Message::CloseInfo => state.close_info(),
        Message::ToggleMute => state.toggle_mute(),
        Message::ToggleBlock => state.toggle_block(),
        Message::CopyUsername => {
            if let Some(handle) = state.copy_username() {
                return iced::clipboard::write::<Message>(handle);
            }
        }
        Message::KickMember(user_id) => state.kick(user_id),
        Message::ConfirmKick => state.kick_confirmed(),
        Message::MemberMenu(user_id) => state.admin_open_menu(user_id),
        Message::AdminAction(action, user_id) => state.admin_action(user_id, action),
        Message::AdminConfirmYes => state.admin_confirmed(),
        Message::CancelReply => state.reply_target = None,
        Message::ContextForward => state.context_forward(),
        Message::ForwardTo(chat) => {
            state.forward_to(chat);
            return Task::none();
        }
        Message::AttachFile => {
            let task = iced::Task::future(async {
                let picked = rfd::AsyncFileDialog::new()
                    .set_title("Send a file")
                    .pick_file()
                    .await
                    .map(|f| f.path().to_string_lossy().into_owned());
                Message::FilePicked(picked)
            });
            return task;
        }
        Message::FilePicked(Some(path)) => {
            if std::path::Path::new(&path).exists() {
                state.send_media(path);
            } else {
                state.status = format!("File not found: {path}");
            }
        }
        Message::FilePicked(None) => {}
        Message::ToggleStickerPicker => state.toggle_sticker_picker(),
        Message::StickerPicked(set, doc) => state.send_sticker(set, doc),
        Message::CloseStickerPicker => state.close_sticker_picker(),
        Message::DismissMenu => state.dismiss_menu(),
        Message::CloseOverlays => {
            state.dismiss_menu();
            state.close_react();
        }
        Message::ContextReact => state.context_react(),
        Message::React(emoji) => state.react(&emoji),
        Message::CloseReact => state.close_react(),
        Message::ToggleReactPicker => state.toggle_react_picker(),
        // -----------------------------------------------------------------
        // Settings panel
        // -----------------------------------------------------------------
        Message::ToggleSettings => state.toggle_settings(),
        Message::CloseSettings => state.close_settings(),
        Message::SettingsTab(tab) => state.set_settings_tab(tab),
        Message::SettingsNameChanged(t) => state.edit_name = t,
        Message::SettingsLastNameChanged(t) => state.edit_last_name = t,
        Message::SettingsBioChanged(b) => state.edit_bio = b,
        Message::SaveProfile => state.submit_profile_edit(),
        Message::ToggleNotifications => state.toggle_notifications(),
        Message::AskClearCache => state.ask_clear_cache(),
        Message::CancelClearCache => state.cancel_clear_cache(),
        Message::ConfirmClearCache => state.confirm_clear_cache_now(),
        Message::AskLogout => state.ask_logout(),
        Message::CancelLogout => state.cancel_logout(),
        Message::DoLogout => state.confirm_logout_now(),
        Message::RevokeSession(hash) => state.revoke_session(hash),
        Message::ToggleTheme => state.toggle_theme(),
        Message::Escape => {
            if state.search_open() {
                state.close_search();
            } else if state.viewer.is_some() {
                state.back();
            } else {
                state.escape();
            }
        }
        Message::OpenGlobalSearch => state.open_search(SearchMode::Global),
        Message::OpenInChatSearch => state.open_search(SearchMode::InChat),
        Message::CloseSearch => state.close_search(),
        Message::SearchChanged(text) => state.search_changed(text),
        Message::SearchHitClicked(idx) => state.click_search_hit(idx),
        Message::VoiceClicked {
            chat_id,
            msg_id,
            path,
        } => {
            let cached = state
                .messages
                .iter()
                .find(|m| m.id == msg_id)
                .is_some_and(|m| m.doc_path.is_some());
            if cached {
                state.voice_click(chat_id, msg_id, &path);
            } else {
                // Not downloaded yet: fetch it, then DocReady auto-plays.
                let _ = state.req_tx.send(Request::DownloadDoc { chat_id, msg_id });
            }
        }
        Message::VoiceTick => state.on_voice_tick(),
        Message::VoiceSeek(secs) => state.voice_seek(secs),
        Message::OpenUrl(url) => {
            // Bare "www." links get a scheme so the opener accepts them.
            let url = if url.starts_with("www.") {
                format!("https://{url}")
            } else {
                url
            };
            let _ = std::process::Command::new("xdg-open").arg(url).spawn();
        }
        Message::ComposerChanged(text) => {
            state.composer = text;
            // Notify the server while the user types (best-effort).
            if let Some(id) = state.open_chat {
                let _ = state.req_tx.send(Request::Typing { id, typing: true });
            }
        }
        Message::EmojiToggle => state.toggle_emoji_panel(),
        Message::EmojiPicked(e) => state.pick_emoji(e),
        Message::EmojiDismiss => state.close_emoji_panel(),
        Message::Submit => {
            if let Some(id) = state.open_chat {
                let _ = state.req_tx.send(Request::Typing { id, typing: false });
            }
            state.submit();
        }
        Message::CloseViewer => {
            state.back();
            // Re-sync the list scroll: the widget may have been recreated
            // while the viewer was up (GL drivers stall on the full-screen
            // swap), so pin it back to the position we remember.
            let y = state.scroll_offset.max(0.0);
            use iced::widget::operation::{scroll_to, AbsoluteOffset};
            return scroll_to::<Message>(MSG_LIST_ID, AbsoluteOffset { x: 0.0, y });
        }
        Message::LoginChanged(text) => state.login_input = text,
        Message::LoginSubmit => submit_login(state),
        Message::LoginBack => login_back(state),
        Message::ToggleLoginScreen => {
            state.toggle_login_screen();
            let req = if state.login_qr_selected {
                state.qr_started();
                Request::QrLoginStart
            } else {
                Request::QrLoginCancel
            };
            let _ = state.req_tx.send(req);
        }
        Message::QrCancel => {
            state.qr_cancelled();
            let _ = state.req_tx.send(Request::QrLoginCancel);
        }
        Message::PerfTickC => {
            // Continuous redraw loop: the message alone already scheduled a
            // redraw; also keep the cadence log (renders/sec) honest.
            state.on_perf_tick();
        }
        Message::Scrolled(y) => state.on_scrolled(y),
        Message::DialogScrolled(y) => state.on_dialog_scrolled(y),
        Message::PerfTick => {
            if state.scroll_perf_dur > 0.0 {
                state.advance_scroll_sim();
            } else {
                state.on_perf_tick();
            }
        }
        Message::TopicChipPicked(root) => state.topic_select(root),
        Message::TopicCreateOpen => state.topic_open_create(),
        Message::TopicCreateTitle(t) => state.topic_title = t,
        Message::TopicCreateSubmit => state.topic_submit_create(),
        Message::TopicCreateCancel => state.topic_cancel_create(),
    }
    // A downloaded document was clicked: hand it to the system opener.
    if let Some(path) = state.open_file.take() {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
    // A search jump was armed: scroll the message list to the target.
    {
        use iced::widget::operation::{scroll_to, AbsoluteOffset};
        if let Some(y) = state.take_scroll_target() {
            return scroll_to::<Message>(MSG_LIST_ID, AbsoluteOffset { x: 0.0, y });
        }
    }
    // Tray: consume the open/quit requests from the tray thread.
    if state.tray_actions.open.swap(false, Ordering::SeqCst) {
        // Save the window to show (dealt with by the shell; unsaved is fine).
    }
    if state.tray_actions.quit.swap(false, Ordering::SeqCst) {
        std::process::exit(0);
    }
    if state.scroll_to_bottom {
        state.scroll_to_bottom = false;
        return scroll_to_end_task();
    }
    Task::none()
}

/// Returns a [`Task`] that snaps the message list to its end.
fn scroll_to_end_task() -> Task<Message> {
    use iced::widget::operation;
    operation::snap_to_end::<Message>(MSG_LIST_ID)
}

fn submit_login(state: &mut State) {
    // Empty fields don't submit (mirrors the winit client).
    if state.login_input.trim().is_empty() {
        return;
    }
    let req = match state.login_step {
        LoginStep::Phone => Request::LoginPhone {
            phone: state.login_input.trim().to_string(),
        },
        LoginStep::Code => Request::LoginCode {
            code: state.login_input.trim().to_string(),
        },
        LoginStep::Password => Request::LoginPassword {
            password: state.login_input.clone(),
        },
    };
    let _ = state.req_tx.send(req);
    state.login_input.clear();
    state.login_error = false;
}

fn login_back(state: &mut State) {
    match state.login_step {
        LoginStep::Phone => state.authenticated = false,
        LoginStep::Code => state.login_step = LoginStep::Phone,
        LoginStep::Password => state.login_step = LoginStep::Code,
    }
}

/// Plain counter of `view()` invocations (== redraws the runtime started).
/// The runtime redraws once per `RedrawRequested`, so this tracks the ACTUAL
/// frame-presentation rate, as opposed to the scroll-event cadence the FPS
/// badge averages. Exposed to `state` for the `--perf` log.
static RENDERED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Bump + read the redraw counter (see [`RENDERED`]).
pub fn rendered_since() -> u64 {
    use std::sync::atomic::Ordering;
    RENDERED.swap(0, Ordering::SeqCst)
}

/// Rolling frames-per-second measured from ACTUAL `view()` calls (== presents
/// the runtime performed), so the `--perf` badge shows the real display rate —
/// not the scroll-event cadence, which over-states it during floods.
pub fn rendered_per_second() -> f32 {
    use std::sync::Mutex;
    use std::time::Instant;

    static TIMES: std::sync::OnceLock<Mutex<std::collections::VecDeque<Instant>>> =
        std::sync::OnceLock::new();
    let times = TIMES.get_or_init(|| Mutex::new(std::collections::VecDeque::new()));

    let now = Instant::now();
    if let Ok(mut t) = times.lock() {
        t.push_back(now);
        while t
            .front()
            .is_some_and(|x| now.duration_since(*x).as_secs_f32() > 1.0)
        {
            t.pop_front();
        }
        t.len() as f32
    } else {
        0.0
    }
}

fn view(state: &State) -> Element<'_> {
    use std::sync::atomic::Ordering;
    RENDERED.fetch_add(1, Ordering::SeqCst);
    // Entry into the redraw path: bump the display-fps rolling window.
    let _ = rendered_per_second();
    // NOTE: the viewer is NOT a top-level view swap — on GL/NVIDIA, swapping
    // the whole window content for a full-screen image and back stalls
    // surface presentation (last frame stays on screen). It renders as an
    // overlay layer inside `conversation_pane` instead.
    if !state.authenticated {
        if state.connecting {
            return connecting_view(state);
        }
        return login_view(state);
    }
    if state.search_open() {
        return search_view(state);
    }
    chat_view(state)
}

// ---------------------------------------------------------------------------
// Search views (global + in-chat)
// ---------------------------------------------------------------------------

/// Full-window search UI replacing the chat view while `search_mode` is set:
/// a header with the back ✕ + query field, and the results list below.
fn search_view(state: &State) -> Element<'_> {
    let mode_label = match state.search_mode {
        Some(SearchMode::Global) => "Search all chats…",
        Some(SearchMode::InChat) => "Search this chat…",
        None => "Search…",
    };
    let field = text_input(mode_label, &state.search_query)
        .on_input(Message::SearchChanged)
        .padding(12)
        .style(text_input_style);

    let mut results = column![].spacing(2);
    if state.search_query.trim().is_empty() {
        results = results.push(search_hint("Type a keyword to start searching…"));
    } else if state.search_hits.is_empty() {
        if state.search_pending {
            results = results.push(search_hint("Searching…"));
        } else {
            results = results.push(search_hint("No results"));
        }
    } else {
        let highlighted = &state.search_query;
        for (i, hit) in state.search_hits.iter().enumerate() {
            results = results.push(search_hit_row(hit, highlighted, i));
        }
    }

    let header = container(
        row![
            button(icon(Icon::Back, theme::ICON(), 18.0))
                .on_press(Message::CloseSearch)
                .padding(8)
                .style(flat_button),
            container(field)
                .width(Length::Fill)
                .height(40.0)
                .style(field_rounded),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .height(theme::layout::CHAT_HEADER_H)
    .padding([0, 12])
    .style(header_bg);

    column![
        header,
        container(iced::widget::Space::new())
            .width(Length::Fill)
            .height(1.0)
            .style(divider),
        container(scrollable(results))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(chat_bg),
    ]
    .into()
}

/// A tappable search-result line: avatar, chat title + snippet, timestamp.
fn search_hit_row(hit: &bridge::SearchHit, _query: &str, idx: usize) -> Element<'static> {
    let snippet = state::preview_text(
        &hit.row.text,
        &hit.row.photo,
        &hit.row.doc,
        &hit.row.sticker,
    );
    let title = hit.chat_title.clone();
    let ts = if hit.row.date > 0 {
        theme::cached_time(hit.row.date)
    } else {
        String::new()
    };
    // The avatar is resolved lazily by the caller via the dialog list; here we
    // render a deterministic letter circle to keep the row self-contained.
    let av = avatar_circle(None, &title, 40.0);

    button(
        row![
            av,
            column![
                text(title)
                    .size(theme::font::NAME)
                    .color(rgb(theme::TEXT_PRIMARY()))
                    .wrapping(iced::widget::text::Wrapping::None),
                text(snippet)
                    .size(theme::font::PLACEHOLDER)
                    .color(rgb(theme::TEXT_SECONDARY()))
                    .wrapping(iced::widget::text::Wrapping::None),
            ]
            .spacing(2)
            .width(Length::Fill),
            text(ts)
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::TEXT_SECONDARY())),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .on_press(Message::SearchHitClicked(idx))
    .width(Length::Fill)
    .padding([10, 12])
    .style(move |theme, status| row_style(theme, status, false))
    .into()
}

/// Muted centered line for the search hint/empty/loading states.
fn search_hint(label: &str) -> Element<'static> {
    container(
        text(label.to_string())
            .size(theme::font::MESSAGE)
            .color(rgb(theme::TEXT_SECONDARY())),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .into()
}

// ---------------------------------------------------------------------------
// Viewer (full-screen photo)
// ---------------------------------------------------------------------------

fn viewer_view(state: &State) -> Element<'_> {
    let Some(path) = &state.viewer else {
        return container("").into();
    };
    let close = button(icon(Icon::Back, theme::ICON(), 16.0))
        .on_press(Message::CloseViewer)
        .padding(8)
        .style(flat_button);
    // Opaque layer: covers the conversation pane (sidebar stays visible).
    container(
        column![
            row![close].padding(8).width(Length::Fill),
            container(
                mouse_area(
                    image(image::Handle::from_path(path))
                        .width(Length::Fill)
                        .content_fit(iced::ContentFit::Contain),
                )
                .on_press(Message::CloseViewer),
            )
            .center_x(Length::Fill)
            .width(Length::Fill)
            .height(Length::Fill),
        ]
        .spacing(8)
        .padding(16),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(|_| container::Style {
        background: Some(iced::Background::Color(Color::BLACK)),
        ..container::Style::default()
    })
    .into()
}

// ---------------------------------------------------------------------------
// Login screen (mirrors the custom `draw_login`)
// ---------------------------------------------------------------------------

/// One pill of the [Phone | QR] sign-in switcher; `active` uses the accent.
fn login_segment(label: &'static str, active: bool) -> iced::widget::Button<'static, Message> {
    button(text(label).size(13).color(if active {
        Color::WHITE
    } else {
        rgb(theme::TEXT_SECONDARY())
    }))
    .padding([6, 18])
    .on_press(Message::ToggleLoginScreen)
    .style(move |t, s| {
        let mut st = if active {
            accent_button(t, s)
        } else {
            flat_button(t, s)
        };
        if active {
            st.border = iced::Border {
                radius: iced::border::Radius::new(14.0),
                ..iced::Border::default()
            };
        }
        st
    })
}

/// Shown for the brief window between login completing (or a valid session
/// loading) and the dialog list arriving. Replaces the frozen QR page with
/// a branded "Loading chats…" card so the scanned login visibly reacts
/// instead of sitting on an unchanging QR while `get_dialogs` paginates.
fn connecting_view(_state: &State) -> Element<'_> {
    let logo = container(
        container(text("tg").size(20).color(Color::WHITE))
            .width(76)
            .height(76)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .style(accent_circle),
    )
    .width(76)
    .height(76);

    container(
        column![
            logo,
            text("Loading chats…")
                .size(theme::font::TITLE)
                .color(rgb(theme::TEXT_PRIMARY())),
            text("Almost there—one moment…")
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::TEXT_SECONDARY())),
        ]
        .spacing(18)
        .align_x(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Alignment::Center)
    .align_y(Alignment::Center)
    .into()
}

fn login_view(state: &State) -> Element<'_> {
    let _s = 1.0f32; // logical pixels (Iced handles HiDPI internally)

    // [Phone | QR] segmented switcher, shown on top of both screens.
    let switcher = row![
        login_segment("Phone", !state.login_qr_selected),
        login_segment("QR", state.login_qr_selected),
    ];

    // Phone pane (original form) — untouched by the QR flow.
    let title = match state.login_step {
        LoginStep::Phone => "Sign in to Telegram",
        LoginStep::Code => "Check your phone",
        LoginStep::Password => "Two-step verification",
    };
    let subtitle = match state.login_step {
        LoginStep::Phone => "We will send a code to your phone number.",
        LoginStep::Code => "Enter the code that was sent to your Telegram.",
        LoginStep::Password => "This account requires an additional password.",
    };
    let field_placeholder = match state.login_step {
        LoginStep::Phone => "+33612345678",
        LoginStep::Code => "Code",
        LoginStep::Password => "Password",
    };
    let button_label = match state.login_step {
        LoginStep::Phone => "Continue",
        LoginStep::Code => "Log in",
        LoginStep::Password => "Sign in",
    };

    // Status / error line.
    let status = if state.status.is_empty() {
        None
    } else if state.login_error {
        Some(
            text(&state.status)
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::ERROR())),
        )
    } else {
        Some(
            text(&state.status)
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::TEXT_SECONDARY())),
        )
    };

    let logo = container(
        container(text("tg").size(20).color(Color::WHITE))
            .width(76)
            .height(76)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .style(accent_circle),
    )
    .width(76)
    .height(76);

    // QR pane: fixed-white rounded card so the code stays scannable in both
    // themes; caption/status live outside the card in theme colors.
    let qr_pane: Element<'_> = {
        let card_inner: Element<'_> = if let Some(path) = &state.qr_png_path {
            image(image::Handle::from_path(path))
                .width(260)
                .height(260)
                .content_fit(iced::ContentFit::Contain)
                .into()
        } else {
            // Placeholder keeps the card size stable while rendering/scanning.
            container(horizontal_spacer()).width(260).height(260).into()
        };
        let card = container(card_inner)
            .width(276)
            .height(276)
            .center_x(Length::Fill)
            .center_y(Length::Shrink)
            .style(|_| container::Style {
                background: Some(iced::Background::Color(Color::WHITE)),
                border: iced::Border {
                    radius: iced::border::Radius::new(16.0),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            });
        let qr_status: Element<'_> = if let Some(err) = &state.qr_error {
            text(err).size(13).color(rgb(theme::ERROR())).into()
        } else {
            match state.qr_stage {
                QrStage::ScanConfirmed => text("Device confirmed — finishing sign-in…")
                    .size(13)
                    .color(rgb(theme::TEXT_SECONDARY()))
                    .into(),
                _ if state.qr_png_path.is_some() => text("Point your phone camera at the code")
                    .size(13)
                    .color(rgb(theme::TEXT_SECONDARY()))
                    .into(),
                _ => text("Generating QR code…")
                    .size(13)
                    .color(rgb(theme::TEXT_SECONDARY()))
                    .into(),
            }
        };
        column![
            card,
            text("Open Telegram on your phone:\nSettings ▸ Devices ▸ Link Desktop Device")
                .size(13)
                .color(rgb(theme::TEXT_SECONDARY()))
                .align_x(Alignment::Center),
            qr_status,
            button(
                text("Use phone number instead")
                    .size(13)
                    .color(rgb(theme::ICON()))
            )
            .on_press(Message::QrCancel)
            .padding(8)
            .style(flat_button),
        ]
        .align_x(Alignment::Center)
        .spacing(12)
        .into()
    };

    let body: Element<'_> = if state.login_qr_selected {
        qr_pane
    } else {
        let mut input = text_input(field_placeholder, &state.login_input)
            .on_input(Message::LoginChanged)
            .on_submit(Message::LoginSubmit)
            .width(400)
            .padding(14)
            .style(text_input_style);
        if state.login_step == LoginStep::Password {
            input = input.secure(true);
        }

        let mut card = column![
            logo,
            text(title).size(20),
            text(subtitle).size(13).color(rgb(theme::TEXT_SECONDARY())),
            input,
            button(text(button_label).size(15).color(Color::WHITE))
                .on_press(Message::LoginSubmit)
                .width(400)
                .padding(14)
                .style(accent_button),
        ]
        .align_x(Alignment::Center)
        .spacing(12);

        if let Some(st) = status {
            card = card.push(st);
        }

        let back = if state.login_step != LoginStep::Phone {
            Some(
                button(row![
                    icon(Icon::Back, theme::ICON(), 18.0),
                    text("Back").size(13).color(rgb(theme::ICON())),
                ])
                .on_press(Message::LoginBack)
                .padding(8)
                .style(flat_button),
            )
        } else {
            None
        };

        let mut col = column![card];
        if let Some(b) = back {
            col = col.push(b);
        }
        col.align_x(Alignment::Center).spacing(8).into()
    };

    container(
        column![switcher, body]
            .align_x(Alignment::Center)
            .spacing(16),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .padding(48)
    .into()
}

// ---------------------------------------------------------------------------
// Chat view: list pane + conversation pane
// ---------------------------------------------------------------------------

/// Full chat UI (list pane + conversation pane). `pub` so the `benches/`
/// harness can drive the exact per-frame view headlessly. The creation modal
/// and the leave/delete confirmation float over everything.
pub fn chat_view(state: &State) -> Element<'_> {
    // ONE flat always-present stack holds every overlay (viewer, forward,
    // create, confirm, info): closed overlays contribute an empty spacer so
    // toggling never rewrites the tree shape, and NO stack nests another
    // (nested stacks break GL compositing — dimmed layers render over black).
    let viewer_layer: Element<'_> = if state.viewer.is_some() {
        viewer_view(state)
    } else {
        horizontal_spacer()
    };
    let fwd_layer: Element<'_> = if state.forward_pick.is_some() {
        forward_layer(state)
    } else {
        horizontal_spacer()
    };
    iced::widget::stack![
        row![list_pane(state), conversation_pane(state)],
        viewer_layer,
        fwd_layer,
        create_layer(state),
        confirm_layer(state),
        info_layer(state),
        settings_layer(state),
    ]
    .into()
}

/// Width of the right-hand settings sheet.
const SETTINGS_W: f32 = 340.0;

/// Settings as a floating right sheet (same pattern as [`info_layer`]):
/// opaque card + hairline border, NO translucent scrim (see the note there).
fn settings_layer<'a>(state: &'a State) -> Element<'a> {
    if !state.settings_open {
        return horizontal_spacer();
    }
    let sheet = container(settings_panel(state))
        .width(SETTINGS_W)
        .height(Length::Fill)
        .padding([12.0, 0.0])
        .style(|_| container::Style {
            background: Some(iced::Background::Color(rgb(theme::LIST_BG()))),
            border: iced::Border {
                radius: 0.0.into(),
                width: 1.0,
                color: rgb(theme::MENU_BORDER()),
            },
            ..container::Style::default()
        });
    container(sheet)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::End)
        .align_y(Alignment::Start)
        .into()
}

/// One tab header of the settings panel (selected one highlighted).
fn settings_tab_header<'a>(label: &'a str, tab: state::SettingsTab, active: bool) -> Element<'a> {
    button(text(label).size(theme::font::TIMESTAMP).color(if active {
        Color::WHITE
    } else {
        rgb(theme::TEXT_SECONDARY())
    }))
    .on_press(Message::SettingsTab(tab))
    .padding([6, 12])
    .style(flat_button)
    .into()
}

/// A labelled row inside the settings panel (label left, control right).
fn settings_row<'a>(
    label: &str,
    control: impl Into<Element<'a>>,
) -> iced::widget::Row<'a, Message> {
    row![
        text(label.to_string())
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::TEXT_PRIMARY()))
            .width(Length::Fill),
        control.into(),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
}

/// Thin divider between settings sections.
fn settings_divider() -> Element<'static> {
    container(horizontal_spacer())
        .width(Length::Fill)
        .height(1.0)
        .style(|_| container::Style {
            background: Some(iced::Background::Color(rgb(theme::DIVIDER()))),
            ..container::Style::default()
        })
        .into()
}

/// The settings panel body: tabs on top, then the active tab's content.
fn settings_panel(state: &State) -> Element<'_> {
    let close = button(icon(Icon::Close, theme::ICON(), 16.0))
        .on_press(Message::CloseSettings)
        .padding(6)
        .style(flat_button);

    let tabs = row![
        settings_tab_header(
            "Profile",
            state::SettingsTab::Profile,
            state.settings_tab == state::SettingsTab::Profile
        ),
        settings_tab_header(
            "Storage",
            state::SettingsTab::Storage,
            state.settings_tab == state::SettingsTab::Storage
        ),
        settings_tab_header(
            "Sessions",
            state::SettingsTab::Sessions,
            state.settings_tab == state::SettingsTab::Sessions
        ),
        horizontal_spacer(),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let mut col = column![
        row![
            text("Settings")
                .size(theme::font::NAME)
                .color(Color::WHITE)
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..iced::Font::DEFAULT
                }),
            horizontal_spacer(),
            close,
        ]
        .width(Length::Fill)
        .align_y(Alignment::Center),
        tabs,
        settings_divider(),
    ]
    .spacing(12)
    .padding([12, 14]);

    match state.settings_tab {
        state::SettingsTab::Profile => col = col.push(settings_profile_tab(state)),
        state::SettingsTab::Storage => col = col.push(settings_storage_tab(state)),
        state::SettingsTab::Sessions => col = col.push(settings_sessions_tab(state)),
    }

    scrollable(col.width(SETTINGS_W - 2.0).height(Length::Fill)).into()
}

/// Profile tab: identity card + edit form + notifications + theme rows.
fn settings_profile_tab(state: &State) -> Element<'_> {
    let mut col = column![].spacing(10);

    // Identity block.
    if let Some(p) = &state.my_profile {
        col = col.push(container(avatar_circle(None, &p.name, 64.0)).center_x(Length::Fill));
        col = col.push(
            container(
                text(p.name.clone())
                    .size(theme::font::NAME)
                    .color(Color::WHITE)
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..iced::Font::DEFAULT
                    }),
            )
            .center_x(Length::Fill),
        );
        if let Some(u) = &p.username {
            col = col.push(
                container(
                    text(format!("@{u}"))
                        .size(theme::font::TIMESTAMP)
                        .color(rgb(theme::ACCENT())),
                )
                .center_x(Length::Fill),
            );
        }
        if let Some(ph) = &p.phone {
            col = col.push(
                container(
                    text(ph.clone())
                        .size(theme::font::TIMESTAMP)
                        .color(rgb(theme::TEXT_SECONDARY())),
                )
                .center_x(Length::Fill),
            );
        }
        if let Some(bio) = &p.bio {
            col = col.push(
                container(
                    text(bio.clone())
                        .size(theme::font::TIMESTAMP)
                        .color(rgb(theme::TEXT_SECONDARY())),
                )
                .center_x(Length::Fill),
            );
        }
    } else {
        col = col.push(
            text("Loading profile…")
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::TEXT_SECONDARY())),
        );
    }

    col = col.push(settings_divider());

    // Edit form: First name / Bio / Save.
    col = col.push(
        text("Edit profile")
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::ICON()))
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..iced::Font::DEFAULT
            }),
    );
    col = col.push(
        text_input("First name", &state.edit_name)
            .on_input(Message::SettingsNameChanged)
            .size(theme::font::TIMESTAMP)
            .padding(8)
            .style(text_input_style),
    );
    col = col.push(
        text_input("Last name", &state.edit_last_name)
            .on_input(Message::SettingsLastNameChanged)
            .size(theme::font::TIMESTAMP)
            .padding(8)
            .style(text_input_style),
    );
    col = col.push(
        text_input("Bio", &state.edit_bio)
            .on_input(Message::SettingsBioChanged)
            .size(theme::font::TIMESTAMP)
            .padding(8)
            .style(text_input_style),
    );
    col = col.push(
        button(
            text("Save")
                .size(theme::font::TIMESTAMP)
                .color(Color::WHITE),
        )
        .on_press(Message::SaveProfile)
        .padding([6, 16])
        .style(accent_button),
    );

    col = col.push(settings_divider());

    // Notifications toggle.
    col = col.push(settings_row(
        "Notifications",
        button(
            text(if state.notifications_on { "On" } else { "Off" })
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::ICON())),
        )
        .on_press(Message::ToggleNotifications)
        .padding([4, 12])
        .style(flat_button),
    ));

    // Theme switch (contract with the theme feature).
    col = col.push(settings_row(
        "Theme",
        button(
            text(match state.theme_mode {
                state::ThemeMode::Dark => "Switch to Light",
                state::ThemeMode::Light => "Switch to Dark",
            })
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::ICON())),
        )
        .on_press(Message::ToggleTheme)
        .padding([4, 12])
        .style(flat_button),
    ));

    // Account: log out (inline Yes/No confirmation, Clear-cache pattern).
    let logout_control: Element<'_> = if state.confirm_logout {
        row![
            button(
                text("Yes")
                    .size(theme::font::BADGE)
                    .color(rgb(theme::ERROR()))
            )
            .on_press(Message::DoLogout)
            .padding([3, 10])
            .style(flat_button),
            button(
                text("No")
                    .size(theme::font::BADGE)
                    .color(rgb(theme::ICON()))
            )
            .on_press(Message::CancelLogout)
            .padding([3, 10])
            .style(flat_button),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .into()
    } else {
        button(
            text("Log out")
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::ERROR())),
        )
        .on_press(Message::AskLogout)
        .padding([4, 12])
        .style(flat_button)
        .into()
    };
    col = col.push(settings_divider());
    col = col.push(settings_row("Account", logout_control));

    col.into()
}

/// Storage tab: measured cache size + clear action with inline confirmation
/// (kick_confirm pattern).
fn settings_storage_tab(state: &State) -> Element<'_> {
    let mut col = column![].spacing(10);

    col = col.push(
        text("Storage")
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::ICON()))
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..iced::Font::DEFAULT
            }),
    );

    let size_label = match state.cache_bytes {
        Some(bytes) => fmt_size(bytes as i64),
        None => "…".to_string(),
    };
    col = col.push(settings_row(
        "Cache used",
        text(size_label)
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::TEXT_SECONDARY())),
    ));

    if state.confirm_clear_cache {
        col = col.push(
            row![
                text("Clear all cached media?")
                    .size(theme::font::TIMESTAMP)
                    .color(rgb(theme::ERROR()))
                    .width(Length::Fill),
                button(
                    text("Yes")
                        .size(theme::font::BADGE)
                        .color(rgb(theme::ERROR()))
                )
                .on_press(Message::ConfirmClearCache)
                .padding([3, 10])
                .style(flat_button),
                button(
                    text("No")
                        .size(theme::font::BADGE)
                        .color(rgb(theme::ICON()))
                )
                .on_press(Message::CancelClearCache)
                .padding([3, 10])
                .style(flat_button),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        );
    } else {
        col = col.push(
            button(
                row![
                    icon(Icon::Trash, theme::ICON(), 14.0),
                    text("Clear cache")
                        .size(theme::font::TIMESTAMP)
                        .color(rgb(theme::ICON())),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .on_press(Message::AskClearCache)
            .padding([6, 12])
            .style(flat_button),
        );
    }

    col = col.push(
        text("Photos, documents and stickers you received are cached here. Clearing never signs you out.")
            .size(theme::font::BADGE)
            .color(rgb(theme::TEXT_SECONDARY())),
    );

    col.into()
}

/// Sessions tab: active authorizations; the current device is badged, others
/// offer Terminate.
fn settings_sessions_tab(state: &State) -> Element<'_> {
    let mut col = column![].spacing(8);

    col = col.push(
        text("Active sessions")
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::ICON()))
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..iced::Font::DEFAULT
            }),
    );

    if state.sessions.is_empty() {
        col = col.push(
            text(if state.my_profile.is_some() {
                "No other sessions"
            } else {
                "Loading sessions…"
            })
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::TEXT_SECONDARY())),
        );
        return col.into();
    }

    for s in &state.sessions {
        let trailing: Element<'static> = if s.current {
            container(
                text("This device")
                    .size(theme::font::BADGE)
                    .color(rgb(theme::ACCENT())),
            )
            .padding([2, 6])
            .style(|_| container::Style {
                background: Some(iced::Background::Color(rgb(theme::PERF_BADGE_BG()))),
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                ..container::Style::default()
            })
            .into()
        } else {
            button(
                text("Terminate")
                    .size(theme::font::BADGE)
                    .color(rgb(theme::ERROR())),
            )
            .on_press(Message::RevokeSession(s.hash))
            .padding([3, 8])
            .style(flat_button)
            .into()
        };
        col = col.push(
            row![
                column![
                    text(s.device.clone())
                        .size(theme::font::TIMESTAMP)
                        .color(Color::WHITE),
                    text(format!("{} · {}", s.platform, s.country))
                        .size(theme::font::BADGE)
                        .color(rgb(theme::TEXT_SECONDARY())),
                ]
                .width(Length::Fill),
                trailing,
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .padding([6, 0]),
        );
    }

    col.into()
}

/// Chat info as a floating right sheet over a dimmed app (pattern of the
/// create modal): the conversation stays visible behind it.
fn info_layer<'a>(state: &'a State) -> Element<'a> {
    if !(state.info_open && state.open_chat.is_some()) {
        return horizontal_spacer();
    }
    // NOTE: no translucent scrim here — semi-transparent stack layers
    // composite against the window clear color (black) on wgpu/GL, hiding
    // the app behind them. The sheet carries its own border instead.
    let sheet = container(info_panel(state))
        .width(INFO_W)
        .height(Length::Fill)
        .padding([12.0, 0.0])
        .style(|_| container::Style {
            background: Some(iced::Background::Color(rgb(theme::LIST_BG()))),
            border: iced::Border {
                radius: 0.0.into(),
                width: 1.0,
                color: rgb(theme::MENU_BORDER()),
            },
            ..container::Style::default()
        });
    container(sheet)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::End)
        .align_y(Alignment::Start)
        .into()
}

/// Estimated dialog-row height (logical px): avatar diameter + the button's
/// vertical padding (`[10, 14]` → 10 per side). All rows share this height, so
/// the virtualized slice is exact for equal offsets — no drift, no height
/// cache needed.
const DIALOG_ROW_H: f32 = theme::layout::AVATAR_LIST + 20.0;

/// Left pane: "Chats" header (+ "new group/channel" picker and the chat-row
/// right-click menu as floating overlays) + scrollable, virtualized dialog
/// list.
fn list_pane(state: &State) -> Element<'_> {
    // The scrollable only knows its own height at layout time; `responsive`
    // hands it the viewport height used to pick the visible rows.
    let list = iced::widget::responsive(move |size| dialog_list(state, size.height));

    let header = container(
        row![
            icon(Icon::Logo, theme::ACCENT(), 26.0),
            text("Chats")
                .size(theme::font::TITLE)
                .color(rgb(theme::TEXT_PRIMARY())),
            horizontal_spacer(),
            button(icon(Icon::Plus, theme::ICON(), 18.0))
                .on_press(Message::OpenCreateMenu)
                .padding(7)
                .style(icon_button_style),
            button(icon(Icon::Search, theme::ICON(), 18.0))
                .on_press(Message::OpenGlobalSearch)
                .padding(7)
                .style(icon_button_style),
            button(icon(Icon::Settings, theme::ICON(), 18.0))
                .on_press(Message::ToggleSettings)
                .padding(7)
                .style(icon_button_style),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .height(theme::layout::LIST_HEADER_H)
    .padding([0, 14])
    .align_y(Alignment::Center)
    .style(list_bg);

    let pane: Element<'_> = column![
        header,
        container(list)
            .width(theme::layout::LIST_W)
            .height(Length::Fill)
            .style(list_bg),
    ]
    .width(theme::layout::LIST_W)
    .height(Length::Fill)
    .into();

    // Floating overlays anchored under the header (iced has no absolute
    // positioning; a stack layer with top padding is the closest match).
    if state.create_menu_open {
        iced::widget::stack![pane, create_menu_layer()].into()
    } else if state.row_menu.is_some() {
        iced::widget::stack![pane, row_menu_layer(state)].into()
    } else {
        pane
    }
}

/// The "+" picker menu: New Group / New Channel, floated under the header,
/// right-aligned like a dropdown.
fn create_menu_layer() -> Element<'static> {
    let menu = container(
        column![
            menu_item(Message::CreateGroup, Icon::Compose, "New Group", false),
            menu_item(Message::CreateChannel, Icon::Forward, "New Channel", false),
        ]
        .spacing(2),
    )
    .width(theme::layout::CONTEXT_W)
    .padding(CONTEXT_MENU_PAD)
    .style(menu_bg);

    container(menu)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced::alignment::Horizontal::Right)
        .padding(iced::Padding {
            top: theme::layout::LIST_HEADER_H + 6.0,
            right: 10.0,
            ..Default::default()
        })
        .into()
}

/// The chat-row mini menu (Leave / Delete), floated under the header on the
/// right side of the list pane.
fn row_menu_layer(state: &State) -> Element<'static> {
    let Some(id) = state.row_menu else {
        return horizontal_spacer();
    };
    let menu = container(
        column![
            menu_item(
                Message::AskConfirm(state::ConfirmKind::Leave, id),
                Icon::Forward,
                "Leave",
                false
            ),
            menu_item(
                Message::AskConfirm(state::ConfirmKind::Delete, id),
                Icon::Trash,
                "Delete",
                true
            ),
        ]
        .spacing(2),
    )
    .width(theme::layout::CONTEXT_W)
    .padding(CONTEXT_MENU_PAD)
    .style(menu_bg);

    container(menu)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced::alignment::Horizontal::Right)
        .padding(iced::Padding {
            top: theme::layout::LIST_HEADER_H + 6.0,
            right: 10.0,
            ..Default::default()
        })
        .into()
}

/// Virtualized dialog list: only the rows intersecting the scrollable's
/// viewport (plus [`LIST_OVERSCAN`] on each side) are built per frame. The
/// rows have a uniform height ([`DIALOG_ROW_H`]), so the visible window is
/// `[offset / H, (offset + viewport) / H]` — O(1) instead of building all N
/// dialogs every frame (a 500-chat account used to rebuild every row on every
/// redraw). `pub` for the `benches/` harness.
pub fn dialog_list(state: &State, view_h: f32) -> Element<'_> {
    let n = state.dialogs.len();
    let no_virt = std::env::var("TG_NO_VIRT").is_ok();

    let (start, end) = if no_virt || view_h <= 0.0 || n == 0 {
        (0usize, n)
    } else {
        let offset = state.dialog_scroll_offset.max(0.0);
        let first = (offset / DIALOG_ROW_H).floor() as usize;
        let last = ((offset + view_h) / DIALOG_ROW_H).ceil() as usize + 1;
        // Offsets can overshoot the content (scroll bounce / stale events):
        // clamp so `end >= start` and `start <= n` always hold.
        let start = first.saturating_sub(LIST_OVERSCAN).min(n);
        let end = (last + LIST_OVERSCAN).min(n).max(start);
        (start, end)
    };

    let top_pad = DIALOG_ROW_H * start as f32;
    let bottom_pad = DIALOG_ROW_H * (n.saturating_sub(end)) as f32;

    let mut rows = column![];
    for (i, row) in state
        .dialogs
        .iter()
        .enumerate()
        .skip(start)
        .take(end - start)
    {
        // `state.dialog_short` holds the already-ellipsized labels (kept
        // aligned with `dialogs`), so the rows borrow those strings instead of
        // allocating new ones per frame. The mismatch fallback (freshly-built
        // state, unit tests) borrows the full strings until the list arrives.
        let (title, sub) = state
            .dialog_short
            .get(i)
            .map(|(t, s)| (t.as_str(), s.as_str()))
            .unwrap_or((&row.title, &row.subtitle));
        rows = rows.push(chat_row_button(
            row,
            state.open_chat == Some(row.id),
            title,
            sub,
        ));
    }

    scrollable(column![
        iced::widget::Space::new().height(top_pad),
        rows,
        iced::widget::Space::new().height(bottom_pad),
    ])
    .id(DIALOG_LIST_ID)
    .on_scroll(|viewport: iced::widget::scrollable::Viewport| {
        Message::DialogScrolled(viewport.absolute_offset().y)
    })
    .height(Length::Fill)
    .width(Length::Fill)
    .into()
}

fn chat_row_button<'a>(
    row: &'a ChatRow,
    selected: bool,
    title: &'a str,
    sub: &'a str,
) -> Element<'a> {
    let avatar = avatar_circle(
        row.avatar_path.as_deref(),
        &row.title,
        theme::layout::AVATAR_LIST,
    );

    let unread = row.unread > 0;
    // Matching the winit client: names and previews stay on one line (miss of
    // a built-in ellipsis in iced is handled by `ellipsize` + no wrapping).
    // `title`/`sub` are the pre-ellipsized strings from `State::dialog_short`.
    // Emoji runs go through the color-emoji font (rich spans); plain labels
    // keep the cheap text widget.
    let name: Element<'_> = match line_spans(title) {
        Some(spans) => rich_text(spans)
            .size(theme::font::NAME)
            .color(rgb(theme::TEXT_PRIMARY()))
            .wrapping(iced::widget::text::Wrapping::None)
            .width(Length::Fill)
            .into(),
        None => text(title)
            .size(theme::font::NAME)
            .color(rgb(theme::TEXT_PRIMARY()))
            .wrapping(iced::widget::text::Wrapping::None)
            .width(Length::Fill)
            .into(),
    };
    let sub_text: Element<'_> = match line_spans(sub) {
        Some(spans) => rich_text(spans)
            .size(theme::font::MESSAGE)
            .color(rgb(theme::TEXT_SECONDARY()))
            .wrapping(iced::widget::text::Wrapping::None)
            .width(Length::Fill)
            .into(),
        None => text(sub)
            .size(theme::font::MESSAGE)
            .color(rgb(theme::TEXT_SECONDARY()))
            .wrapping(iced::widget::text::Wrapping::None)
            .width(Length::Fill)
            .into(),
    };

    // The right meta column (timestamp + unread badge) gets a RESERVED
    // fixed width so long previews can never run under it.
    let meta_w = 52.0f32;
    let ts: Element<'_> = if row.date > 0 {
        text(theme::cached_time(row.date))
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::TEXT_SECONDARY()))
            .into()
    } else {
        horizontal_spacer()
    };

    let badge: Element<'_> = if unread {
        container(
            text(row.unread)
                .size(theme::font::BADGE)
                .color(Color::WHITE),
        )
        .padding([2, 7])
        .style(badge_circle)
        .into()
    } else {
        container(iced::widget::Space::new().height(18.0)).into()
    };

    let row_button = button(
        row![
            avatar,
            column![name, sub_text].spacing(2).width(Length::Fill),
            column![ts, badge]
                .spacing(4)
                .width(Length::Fixed(meta_w))
                .align_x(Alignment::End),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .on_press(Message::OpenChat(row.id))
    .width(Length::Fill)
    .padding([10, 14])
    .style(move |theme, status| row_style(theme, status, selected));

    // Right-click opens the row's Leave/Delete mini menu; left clicks stay
    // on the inner button.
    mouse_area(row_button)
        .on_right_press(Message::RowMenu(row.id))
        .into()
}

/// Right pane: chat header + messages + composer (+ context menu overlay),
/// plus the forward chat-picker overlay when armed.
fn conversation_pane(state: &State) -> Element<'_> {
    let open = state.open_chat;
    let chat = state.dialogs.iter().find(|d| Some(d.id) == open);
    // Borrow instead of cloning: these can be hundreds of bytes and the pane
    // rebuilds every frame.
    let title: std::borrow::Cow<'_, str> = if !state.chat_title.is_empty() {
        std::borrow::Cow::Borrowed(&state.chat_title)
    } else {
        match chat {
            Some(c) => std::borrow::Cow::Borrowed(&c.title),
            None => std::borrow::Cow::Borrowed(""),
        }
    };
    let avatar_path = chat.and_then(|c| c.avatar_path.as_deref());

    let header = chat_header(
        &title,
        avatar_path,
        state.typing,
        if state.perf_show {
            Some(format!("{} FPS", rendered_per_second().round()))
        } else {
            None
        },
    );

    // Pinned-message banner (under the header): icon + label + snippet.
    let pinned_banner: Option<Element<'_>> = state.pinned_id.and_then(|pid| {
        let m = state.messages.iter().find(|m| m.id == pid)?;
        Some(pinned_banner(m))
    });

    let body: Element<'_> = if state.messages.is_empty() {
        let msg = if state.open_chat.is_some() {
            if state.loading {
                "Loading…".to_string()
            } else if state.status.is_empty() {
                "No messages yet".to_string()
            } else {
                state.status.clone()
            }
        } else {
            "Select a chat to start messaging".to_string()
        };
        container(
            text(msg)
                .size(theme::font::MESSAGE)
                .color(rgb(theme::TEXT_SECONDARY())),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
    } else {
        container(
            iced::widget::responsive(move |size| messages_list(state, size.width, size.height))
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(chat_bg)
        .into()
    };

    // Composer pickers float over the conversation area while open: clicks
    // outside them land on the backdrop layer and close the panel; scrolling
    // still reaches the underlying list (the backdrop only captures buttons),
    // and the composer below the stack stays fully interactive. Pickers float
    // over the message list only (never header or composer).
    //
    // The stack wrapper exists EVERY frame, not only while a picker is up:
    // mounting it conditionally re-parents the scrollable below (different
    // widget-tree shape) which resets its scroll position to 0 — with the
    // bottom-anchored list that blanked every message as soon as a picker
    // opened ("the picker hides the chat").
    let mut stack = iced::widget::Stack::with_children(vec![body]);
    if state.sticker_picker_open {
        stack = stack.push(
            mouse_area(
                iced::widget::Space::new()
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::CloseStickerPicker),
        );
        stack = stack.push(sticker_picker_card(state));
    } else if state.emoji_panel_open {
        stack = stack.push(
            mouse_area(
                iced::widget::Space::new()
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::EmojiDismiss),
        );
        stack = stack.push(emoji_panel_floated(state));
    }
    let body: Element<'_> = stack.into();

    let composer = composer_bar(state);

    let pane = column![
        header,
        container(iced::widget::Space::new())
            .width(Length::Fill)
            .height(1.0)
            .style(divider),
    ]
    .width(Length::Fill);

    // Banner slots between header divider and body.
    let mut top: iced::widget::Column<'_, Message> = column![];
    if let Some(b) = pinned_banner {
        top = top.push(b);
    }
    // Topic chips bar of forum chats: sits in the body zone, right between
    // the pinned-banner zone and the message list.
    if state.topic_bar_visible() {
        top = top.push(topic_chips_bar(state));
    }

    let pane = pane
        .push(top)
        .push(body)
        .push(composer)
        .height(Length::Fill);

    pane.into()
}

/// Width of the right-hand chat-info side panel.
const INFO_W: f32 = 300.0;

/// Right-hand panel with the open chat's details (avatar, username, bio…),
/// quick actions (mute, search) and the member list of groups/channels.
fn info_panel(state: &State) -> Element<'_> {
    let close = button(icon(Icon::Close, theme::ICON(), 16.0))
        .on_press(Message::CloseInfo)
        .padding(6)
        .style(flat_button);

    let mut col = column![row![
        text("Chat info")
            .size(theme::font::NAME)
            .color(rgb(theme::TEXT_PRIMARY()))
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..iced::Font::DEFAULT
            }),
        horizontal_spacer(),
        close,
    ]
    .width(Length::Fill)
    .align_y(Alignment::Center)]
    .spacing(12)
    .padding([12, 14]);

    // Identity block: avatar + title + subtitle.
    let dialog = state.dialogs.iter().find(|d| Some(d.id) == state.open_chat);
    let avatar_path = dialog.and_then(|d| d.avatar_path.as_deref());
    let detail = state.chat_info.as_ref();
    let title = detail
        .map(|d| d.title.clone())
        .unwrap_or_else(|| state.chat_title.clone());

    col = col.push(
        column![
            container(avatar_circle(avatar_path, &title, 72.0)).center_x(Length::Fill),
            container(
                text(title)
                    .size(theme::font::NAME)
                    .color(rgb(theme::TEXT_PRIMARY()))
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..iced::Font::DEFAULT
                    })
                    .wrapping(iced::widget::text::Wrapping::None),
            )
            .center_x(Length::Fill),
            container(
                text(info_subtitle(state))
                    .size(theme::font::TIMESTAMP)
                    .color(rgb(theme::TEXT_SECONDARY())),
            )
            .center_x(Length::Fill),
        ]
        .spacing(6)
        .align_x(Alignment::Center),
    );

    // @username (click to copy), bio, phone.
    if let Some(d) = detail {
        if let Some(username) = &d.username {
            col = col.push(
                button(
                    row![
                        icon(Icon::Info, theme::ICON(), 14.0),
                        text(format!("@{username}"))
                            .size(theme::font::TIMESTAMP)
                            .color(rgb(theme::ACCENT())),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .on_press(Message::CopyUsername)
                .padding([4, 8])
                .style(flat_button),
            );
        }
        if let Some(bio) = &d.bio {
            col = col.push(
                text(bio.clone())
                    .size(theme::font::TIMESTAMP)
                    .color(rgb(theme::TEXT_SECONDARY())),
            );
        }
        if let Some(phone) = &d.phone {
            col = col.push(
                text(phone.clone())
                    .size(theme::font::TIMESTAMP)
                    .color(rgb(theme::TEXT_SECONDARY())),
            );
        }
    }

    // Quick actions: mute toggle + in-chat search (+ block for private users).
    let is_user = matches!(detail.map(|d| d.kind), Some(ChatKind::User));
    let mut quick_actions: Vec<Element<'_>> = vec![
        button(
            text(if state.muted { "Unmute" } else { "Mute" })
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::ICON())),
        )
        .on_press(Message::ToggleMute)
        .padding([6, 12])
        .style(flat_button)
        .into(),
    ];
    if is_user {
        quick_actions.push(
            button(
                text(if state.blocked { "Unblock" } else { "Block" })
                    .size(theme::font::TIMESTAMP)
                    .color(rgb(theme::ICON())),
            )
            .on_press(Message::ToggleBlock)
            .padding([6, 12])
            .style(flat_button)
            .into(),
        );
    }
    quick_actions.push(
        button(
            row![
                icon(Icon::Search, theme::ICON(), 14.0),
                text("Search in chat")
                    .size(theme::font::TIMESTAMP)
                    .color(rgb(theme::ICON())),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .on_press(Message::OpenInChatSearch)
        .padding([6, 12])
        .style(flat_button)
        .into(),
    );
    col = col.push(row(quick_actions).spacing(8));

    // Members section (groups/channels only).
    let show_members = matches!(
        detail.map(|d| d.kind),
        Some(ChatKind::Group) | Some(ChatKind::Channel)
    ) || (!state.participants.is_empty() && detail.is_none());
    if show_members {
        col = col.push(
            text(format!("Members ({})", state.participants.len()))
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::ICON()))
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..iced::Font::DEFAULT
                }),
        );
        for p in &state.participants {
            col = col.push(member_row(state, p));
        }
        if state.participants.is_empty() {
            col = col.push(
                text("No members to show")
                    .size(theme::font::TIMESTAMP)
                    .color(rgb(theme::TEXT_SECONDARY())),
            );
        }
    }

    container(scrollable(col))
        .width(INFO_W)
        .height(Length::Fill)
        .style(list_bg)
        .into()
}

/// One-line summary under the panel's avatar: members count / presence.
fn info_subtitle(state: &State) -> String {
    match state.chat_info.as_ref() {
        Some(d) => match d.kind {
            ChatKind::User => "online".to_string(),
            ChatKind::Bot => "bot".to_string(),
            ChatKind::Group | ChatKind::Channel => match d.members_count {
                Some(n) => format!("{n} members"),
                None => String::new(),
            },
        },
        None => String::new(),
    }
}

/// A member row: letter avatar, name (+ role badge), a remove action that
/// flips into an inline Yes/No confirmation when armed, and a right-click
/// admin menu (promote/demote/ban/remove) rendered inline under the row.
/// The owner and yourself get neither the ✕ nor the menu.
fn member_row(state: &State, p: &bridge::ParticipantRow) -> Element<'static> {
    let name = p.name.clone();
    let is_self = state.admin_self_id == Some(p.id);
    let untouchable = is_self || p.role == bridge::ParticipantRole::Creator;
    let kick_confirming = state.kick_confirm == Some(p.id);
    let admin_confirm = state.admin_confirm.is_some_and(|(_, u)| u == p.id);
    let menu_open = state.admin_menu == Some(p.id);

    let trailing: Element<'static> = if kick_confirming {
        row![
            button(
                text("Remove")
                    .size(theme::font::BADGE)
                    .color(rgb(theme::ERROR()))
            )
            .on_press(Message::ConfirmKick)
            .padding([3, 8])
            .style(flat_button),
            button(
                text("Cancel")
                    .size(theme::font::BADGE)
                    .color(rgb(theme::ICON()))
            )
            .on_press(Message::Escape)
            .padding([3, 8])
            .style(flat_button),
        ]
        .spacing(4)
        .into()
    } else if admin_confirm {
        row![
            button(
                text("Yes")
                    .size(theme::font::BADGE)
                    .color(rgb(theme::ERROR()))
            )
            .on_press(Message::AdminConfirmYes)
            .padding([3, 8])
            .style(flat_button),
            button(
                text("No")
                    .size(theme::font::BADGE)
                    .color(rgb(theme::ICON()))
            )
            .on_press(Message::Escape)
            .padding([3, 8])
            .style(flat_button),
        ]
        .spacing(4)
        .into()
    } else if untouchable {
        horizontal_spacer()
    } else {
        button(icon(Icon::Close, theme::ICON(), 14.0))
            .on_press(Message::KickMember(p.id))
            .padding(5)
            .style(flat_button)
            .into()
    };

    let row_button = button(
        row![
            avatar_circle(None, &name, 34.0),
            column![text(name)
                .size(theme::font::MESSAGE)
                .color(rgb(theme::TEXT_PRIMARY()))
                .wrapping(iced::widget::text::Wrapping::None),]
            .width(Length::Fill),
            role_badge(p.role),
            trailing,
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([4, 0])
    .style(|_theme, status| row_style(_theme, status, false));
    // An open admin menu makes the row left-click a click-outside dismissal;
    // right-click (re)opens the menu, mirroring the chat-list rows.
    let row_button = if state.admin_menu.is_some() {
        row_button.on_press(Message::CloseOverlays)
    } else {
        row_button
    };

    let menu: Element<'static> = if menu_open {
        let mut items = iced::widget::column![].padding(4).spacing(2);
        for action in state::admin_menu_items(p.role, is_self) {
            let (ic, label, destructive) = admin_menu_item_meta(action);
            items = items.push(menu_item(
                Message::AdminAction(action, p.id),
                ic,
                label,
                destructive,
            ));
        }
        container(items)
            .width(Length::Fill)
            .style(|_| container::Style {
                background: Some(iced::Background::Color(rgb(theme::MENU_BG()))),
                border: iced::Border {
                    radius: 8.0.into(),
                    color: rgb(theme::MENU_BORDER()),
                    width: 1.0,
                },
                ..container::Style::default()
            })
            .into()
    } else {
        horizontal_spacer()
    };

    mouse_area(column![row_button, menu].width(Length::Fill).spacing(2))
        .on_right_press(Message::MemberMenu(p.id))
        .into()
}

/// Rendering metadata of one admin menu action (icon + English label +
/// destructive styling), resolved from the state-level menu-items data.
fn admin_menu_item_meta(action: state::AdminAction) -> (Icon, &'static str, bool) {
    match action {
        state::AdminAction::Promote => (Icon::Plus, "Promote to admin", false),
        state::AdminAction::Demote => (Icon::Close, "Demote admin", false),
        state::AdminAction::Ban => (Icon::Trash, "Ban member", true),
        state::AdminAction::Remove => (Icon::Close, "Remove from group", true),
    }
}

/// Small role chip next to a member's name ("Owner"/"Admin"; none for plain
/// members).
fn role_badge(role: bridge::ParticipantRole) -> Element<'static> {
    let (label, color) = match role {
        bridge::ParticipantRole::Creator => ("Owner", theme::ACCENT()),
        bridge::ParticipantRole::Admin => ("Admin", theme::ICON()),
        bridge::ParticipantRole::Member => return horizontal_spacer(),
    };
    container(text(label).size(theme::font::BADGE).color(rgb(color)))
        .padding([2, 6])
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(rgb(theme::PERF_BADGE_BG()))),
            border: iced::Border {
                radius: 6.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// Size of a rendered sticker in a conversation (logical px).
const STICKER_SIZE: f32 = 180.0;
/// Picker panel geometry (floating above the composer).
const STICKER_PICKER_W: f32 = 360.0;
const STICKER_PICKER_H: f32 = 420.0;
/// Thumb cell size inside the picker grid (4 columns).
const STICKER_THUMB: f32 = 64.0;

/// Floating sticker picker anchored above the composer (left side): the
/// installed packs, each with its title and a 4-column thumbnail grid.
/// Clicking a thumbnail sends the sticker and closes the panel.
fn sticker_picker_card<'a>(state: &'a State) -> Element<'a> {
    let close = button(icon(Icon::Close, theme::ICON(), 14.0))
        .on_press(Message::CloseStickerPicker)
        .padding(6)
        .style(flat_button);

    let mut body: iced::widget::Column<'a, Message> = column![].spacing(10);
    if state.sticker_sets.is_empty() {
        body = body.push(
            container(
                text("Loading packs…")
                    .size(theme::font::MESSAGE)
                    .color(rgb(theme::TEXT_SECONDARY())),
            )
            .width(Length::Fill)
            .padding(24),
        );
    }
    for (si, set) in state.sticker_sets.iter().enumerate() {
        body = body.push(
            text(format!("{} ({})", set.title, set.docs.len()))
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::ICON()))
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..iced::Font::DEFAULT
                }),
        );
        for (row_i, chunk) in set.docs.chunks(4).enumerate() {
            let mut cells: iced::widget::Row<'a, Message> = row![].spacing(6);
            for (col_i, (doc_id, _, _alt)) in chunk.iter().enumerate() {
                let di = row_i * 4 + col_i;
                let cell: Element<'a> = match state.sticker_thumbs.get(doc_id) {
                    Some(path) => button(
                        container(
                            image(image::Handle::from_path(path))
                                .width(Length::Fixed(STICKER_THUMB))
                                .height(Length::Fixed(STICKER_THUMB))
                                .content_fit(iced::ContentFit::Contain),
                        )
                        .width(STICKER_THUMB)
                        .height(STICKER_THUMB)
                        .style(|_| container::Style {
                            background: Some(iced::Background::Color(rgb(theme::INPUT_FILL()))),
                            border: iced::Border {
                                radius: 12.0.into(),
                                ..Default::default()
                            },
                            ..container::Style::default()
                        }),
                    )
                    .on_press(Message::StickerPicked(si, di))
                    .padding(2)
                    .style(flat_button)
                    .into(),
                    None => container(icon(Icon::Sticker, theme::DIVIDER(), 22.0))
                        .width(STICKER_THUMB)
                        .height(STICKER_THUMB)
                        .align_x(iced::alignment::Horizontal::Center)
                        .align_y(iced::alignment::Vertical::Center)
                        .style(|_| container::Style {
                            background: Some(iced::Background::Color(rgb(theme::INPUT_FILL()))),
                            border: iced::Border {
                                radius: 12.0.into(),
                                ..Default::default()
                            },
                            ..container::Style::default()
                        })
                        .into(),
                };
                cells = cells.push(cell);
            }
            body = body.push(cells);
        }
    }

    let card = container(
        column![
            row![
                text("Stickers")
                    .size(theme::font::NAME)
                    .color(rgb(theme::TEXT_PRIMARY()))
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..iced::Font::DEFAULT
                    }),
                horizontal_spacer(),
                close,
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            scrollable(body).height(Length::Fill),
        ]
        .spacing(10),
    )
    .width(STICKER_PICKER_W)
    .height(STICKER_PICKER_H)
    .padding(14)
    .style(menu_bg);

    container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Start)
        .align_y(Alignment::End)
        .padding([0.0, 12.0])
        .into()
}

/// Full-pane modal listing the chats as forward destinations. Rendered on
/// top of the conversation pane when a "Forward" is armed; Escape or the
/// header ✕ cancels.
fn forward_layer<'a>(state: &'a State) -> Element<'a> {
    let mut rows = column![].spacing(2);
    for d in &state.dialogs {
        rows = rows.push(
            button(
                row![
                    avatar_circle(
                        d.avatar_path.as_deref(),
                        &d.title,
                        theme::layout::AVATAR_LIST - 12.0
                    ),
                    text(&d.title)
                        .size(theme::font::MESSAGE)
                        .color(rgb(theme::TEXT_PRIMARY()))
                        .wrapping(iced::widget::text::Wrapping::None),
                    horizontal_spacer(),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            )
            .on_press(Message::ForwardTo(d.id))
            .width(Length::Fill)
            .padding([6, 10])
            .style(|_theme, status| row_style(_theme, status, false)),
        );
    }

    let card = container(
        column![
            row![
                icon(Icon::Forward, theme::ACCENT(), 16.0),
                text("Forward to…")
                    .size(theme::font::NAME)
                    .color(rgb(theme::TEXT_PRIMARY())),
                horizontal_spacer(),
                button(icon(Icon::Close, theme::ICON(), 14.0))
                    .on_press(Message::Escape)
                    .padding(6)
                    .style(flat_button),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            scrollable(rows).height(Length::Fill),
        ]
        .spacing(10),
    )
    .width(320.0)
    .height(400.0)
    .padding(14)
    .style(menu_bg);

    // Centered card, no scrim (GL: translucent layers hide the app).
    container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
}

/// Centered modal creating a group or channel: title (+ description for
/// channels), checkable member list for groups, Create/Cancel buttons.
fn create_layer<'a>(state: &'a State) -> Element<'a> {
    let Some(kind) = state.create_dialog else {
        return horizontal_spacer();
    };
    let title = match kind {
        state::CreateKind::Group => "New Group",
        state::CreateKind::Channel => "New Channel",
    };

    let title_field = text_input("Title", &state.create_title)
        .on_input(Message::CreateTitleChanged)
        .on_submit(Message::SubmitCreate)
        .padding(12)
        .style(text_input_style);

    let mut card = column![
        row![
            text(title)
                .size(theme::font::NAME)
                .color(rgb(theme::TEXT_PRIMARY())),
            horizontal_spacer(),
            button(icon(Icon::Close, theme::ICON(), 14.0))
                .on_press(Message::CancelCreate)
                .padding(6)
                .style(flat_button),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        container(title_field)
            .width(Length::Fill)
            .height(40.0)
            .style(field_rounded),
    ]
    .spacing(12);

    if kind == state::CreateKind::Channel {
        let about_field = text_input("Description", &state.create_about)
            .on_input(Message::CreateAboutChanged)
            .on_submit(Message::SubmitCreate)
            .padding(12)
            .style(text_input_style);
        card = card.push(
            container(about_field)
                .width(Length::Fill)
                .height(40.0)
                .style(field_rounded),
        );
    } else {
        // Groups: pick the initial members from the known contacts.
        let mut members = column![text("Members")
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::TEXT_SECONDARY()))]
        .spacing(2);
        for (i, (_, name, on)) in state.member_pick.iter().enumerate() {
            let check: Element<'_> = if *on {
                icon(Icon::Tick { read: true }, theme::ACCENT(), 16.0)
            } else {
                horizontal_spacer()
            };
            members = members.push(
                button(
                    row![
                        avatar_circle(None, name, 28.0),
                        text(name)
                            .size(theme::font::MESSAGE)
                            .color(rgb(theme::TEXT_PRIMARY()))
                            .wrapping(iced::widget::text::Wrapping::None),
                        horizontal_spacer(),
                        check,
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                )
                .on_press(Message::ToggleMember(i))
                .width(Length::Fill)
                .padding([6, 8])
                .style(|t, s| menu_item_style(t, s, false)),
            );
        }
        card = card.push(
            container(scrollable(members).height(Length::Fill))
                .height(180.0)
                .width(Length::Fill),
        );
    }

    card = card.push(
        row![
            button(
                text("Cancel")
                    .size(theme::font::MESSAGE)
                    .color(rgb(theme::ICON()))
            )
            .on_press(Message::CancelCreate)
            .padding([10, 18])
            .style(flat_button),
            button(
                text("Create")
                    .size(theme::font::MESSAGE)
                    .color(Color::WHITE),
            )
            .on_press(Message::SubmitCreate)
            .padding([10, 22])
            .style(accent_button),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    );

    centered_over(card.width(340.0).padding(16))
}

/// Centered confirmation dialog before leaving/deleting a chat.
fn confirm_layer<'a>(state: &'a State) -> Element<'a> {
    let Some((kind, _)) = state.confirm_leave else {
        return horizontal_spacer();
    };
    let (question, action) = match kind {
        state::ConfirmKind::Leave => ("Leave chat?", "Leave"),
        state::ConfirmKind::Delete => ("Delete chat?", "Delete"),
    };
    let card = column![
        text(question)
            .size(theme::font::NAME)
            .color(rgb(theme::TEXT_PRIMARY())),
        row![
            button(
                text("Cancel")
                    .size(theme::font::MESSAGE)
                    .color(rgb(theme::ICON()))
            )
            .on_press(Message::ConfirmNo)
            .padding([10, 18])
            .style(flat_button),
            button(
                text(action.to_string())
                    .size(theme::font::MESSAGE)
                    .color(rgb(theme::ERROR()))
            )
            .on_press(Message::ConfirmYes)
            .padding([10, 18])
            .style(move |t, s| menu_item_style(t, s, true)),
        ]
        .spacing(10)
        .align_y(Alignment::End),
    ]
    .spacing(14);
    centered_over(card.width(280.0).padding(18))
}

/// Centers a modal card over the app (no translucent scrim — see the GL
/// note on `info_layer`). The card's own surface separates it from the UI.
fn centered_over<'a>(card: impl Into<Element<'a>>) -> Element<'a> {
    container(card.into())
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
}

/// Chat header: back, avatar, name + status, search/info icons (+ FPS badge).
fn chat_header(
    title: &str,
    avatar_path: Option<&str>,
    typing: bool,
    perf: Option<String>,
) -> Element<'static> {
    let name = text(ellipsize(title, 40))
        .size(theme::font::NAME)
        .color(rgb(theme::TEXT_PRIMARY()))
        .font(iced::Font {
            weight: iced::font::Weight::Bold,
            ..iced::Font::DEFAULT
        })
        .wrapping(iced::widget::text::Wrapping::None)
        .width(Length::Fill);
    let status: Element<'static> = if title.is_empty() {
        text(" ")
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::TEXT_SECONDARY()))
            .into()
    } else {
        let (label, color) = if typing {
            ("typing…", theme::ACCENT())
        } else {
            ("Chat", theme::TEXT_SECONDARY())
        };
        text(label)
            .size(theme::font::TIMESTAMP)
            .color(rgb(color))
            .into()
    };

    let mut actions = row![].spacing(6).align_y(Alignment::Center);
    actions = actions.push(
        button(icon(Icon::Search, theme::ICON(), 20.0))
            .on_press(Message::OpenInChatSearch)
            .padding(6)
            .style(flat_button),
    );
    actions = actions.push(
        button(icon(Icon::Info, theme::ICON(), 20.0))
            .on_press(Message::ToggleInfo)
            .padding(6)
            .style(flat_button),
    );
    if let Some(fps) = perf {
        actions = actions.push(
            container(
                text(fps)
                    .size(theme::font::TIMESTAMP)
                    .color(rgb(theme::ACCENT())),
            )
            .padding([2, 6])
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(rgb(theme::PERF_BADGE_BG()))),
                border: iced::Border {
                    radius: 6.0.into(),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            }),
        );
    }

    container(
        row![
            // No back button: like Telegram Desktop, a chat always stays
            // open (the first one auto-opens at startup).
            // Avatar + name open/close the info panel (same as the ℹ️ icon).
            // The name column fills the leftover width, pushing the search/info
            // buttons flush against the right edge like Telegram Desktop.
            button(
                row![
                    avatar_circle(avatar_path, title, theme::layout::AVATAR_CHAT),
                    column![name, status].spacing(2),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            )
            .on_press(Message::ToggleInfo)
            // Vertical padding 2, not 4: the 40 px avatar + 2×4 uniform
            // padding overflowed the header's 44 px content area (52 minus
            // the container's 4 px top/bottom), clipping the circle's top
            // and bottom. 40 + 2×2 = 44 fits exactly, 6 px clear of both
            // header edges.
            .padding([2, 4])
            .width(Length::Fill)
            .style(flat_button),
            actions,
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .height(theme::layout::CHAT_HEADER_H)
    .padding(iced::Padding {
        top: 4.0,
        right: 8.0,
        bottom: 4.0,
        left: 14.0,
    })
    .align_y(Alignment::Center)
    .style(header_bg)
    .into()
}

// ---------------------------------------------------------------------------
// Emoji picker panel (composer)
// ---------------------------------------------------------------------------

/// Size of the floating emoji panel (logical px). Kept small on purpose:
/// it pops above the composer like a context menu and must not swallow the
/// conversation behind it.
const EMOJI_PANEL_W: f32 = 300.0;
const EMOJI_PANEL_H: f32 = 240.0;
/// Emoji grid columns.
const EMOJI_COLS: usize = 7;
/// Rendered size of one emoji glyph in the grid.
const EMOJI_FONT_SIZE: f32 = 22.0;

/// Anchors the panel above the composer's left edge inside the stack layer.
fn emoji_panel_floated(state: &State) -> Element<'_> {
    container(emoji_panel(state))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Start)
        .align_y(Alignment::End)
        .padding([0.0, 12.0])
        .into()
}

/// The emoji picker card: "Recents" (or a starter set until anything was
/// picked) followed by the standard groups, scrollable when tall.
fn emoji_panel(state: &State) -> Element<'_> {
    let mut content = column![].spacing(6);

    let recents = if state.emoji_recents.is_empty() {
        crate::emoji::RECENTS_FALLBACK.to_string()
    } else {
        state.emoji_recents.join(" ")
    };
    content = content.push(emoji_section_label("Recents"));
    content = content.push(emoji_grid(&recents));

    for (title, set) in crate::emoji::SETS {
        content = content.push(emoji_section_label(title));
        content = content.push(emoji_grid(set));
    }

    container(scrollable(content).width(Length::Fill).height(Length::Fill))
        .width(EMOJI_PANEL_W)
        .height(EMOJI_PANEL_H)
        .padding(10)
        .style(menu_bg)
        .into()
}

fn emoji_section_label(title: &str) -> Element<'_> {
    text(title.to_string())
        .size(theme::font::TIMESTAMP)
        .color(rgb(theme::TEXT_SECONDARY()))
        .into()
}

/// Font used to render emoji glyphs in color.
///
/// The system color-emoji font is requested by family name; if absent the
/// engine falls back to whatever covers the codepoints (monochrome).
fn emoji_font() -> iced::Font {
    #[cfg(target_os = "macos")]
    {
        iced::Font::with_name("Apple Color Emoji")
    }
    #[cfg(target_os = "windows")]
    {
        iced::Font::with_name("Segoe UI Emoji")
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        iced::Font::with_name("Noto Color Emoji")
    }
}

/// One 7-column grid of emoji buttons (transparent, light hover).
///
/// Cells explicitly request the system color-emoji font: the default
/// font chain resolves some codepoints (❤ ⚠ ✂ …) to monochrome outlines
/// in text fonts like DejaVu before reaching Noto Color Emoji.
fn emoji_grid(emojis: &str) -> Element<'static> {
    let mut grid = column![].spacing(2);
    for line in emojis
        .split_whitespace()
        .collect::<Vec<_>>()
        .chunks(EMOJI_COLS)
    {
        let mut r = row![].spacing(2);
        for e in line {
            r = r.push(
                button(text(e.to_string()).size(EMOJI_FONT_SIZE).font(emoji_font()))
                    .on_press(Message::EmojiPicked((*e).to_string()))
                    .width(Length::Fixed(38.0))
                    .height(Length::Fixed(32.0))
                    .style(|t, s| menu_item_style(t, s, false)),
            );
        }
        grid = grid.push(r);
    }
    grid.into()
}

/// Composer bar: rounded field + attach (or edit check) + send button, with
/// the reply preview bar stacked above when a reply is armed.
fn composer_bar(state: &State) -> Element<'_> {
    let placeholder = if state.editing.is_some() {
        "Edit message…"
    } else {
        "Message…"
    };
    let field = text_input(placeholder, &state.composer)
        .on_input(Message::ComposerChanged)
        .on_submit(Message::Submit)
        .padding(12)
        .style(text_input_style);

    let send = if state.editing.is_some() {
        button(icon(Icon::Tick { read: true }, theme::TEXT_PRIMARY(), 20.0))
            .on_press(Message::Submit)
            .style(accent_circle_button)
    } else {
        button(icon(Icon::Send, theme::TEXT_PRIMARY(), 20.0))
            .on_press(Message::Submit)
            .style(accent_circle_button)
    };

    // Smiley opens the emoji panel; it sits left of the field.
    let emoji_btn = button(icon(Icon::Smile, theme::ICON(), 20.0))
        .on_press(Message::EmojiToggle)
        .padding(8)
        .style(flat_button);

    let bar = row![
        emoji_btn,
        container(field)
            .width(Length::Fill)
            .height(theme::layout::INPUT_H)
            .style(field_rounded),
        send,
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    let mut col = column![].width(Length::Fill);
    if let Some(reply) = &state.reply_target {
        col = col.push(
            container(
                row![
                    icon(Icon::Reply, theme::ACCENT(), 16.0),
                    column![
                        text("Reply to")
                            .size(theme::font::TIMESTAMP)
                            .color(rgb(theme::ACCENT())),
                        container(
                            text(&reply.snippet)
                                .size(theme::font::TIMESTAMP)
                                .color(rgb(theme::TEXT_SECONDARY()))
                                // single-line, clipped to the bar instead of overflowing
                                .wrapping(iced::widget::text::Wrapping::None),
                        )
                        .width(Length::Fill)
                        .clip(true),
                    ]
                    .spacing(1)
                    .width(Length::Fill),
                    button(icon(Icon::Close, theme::ICON(), 14.0))
                        .on_press(Message::CancelReply)
                        .padding(6)
                        .style(flat_button),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([6, 12])
            .style(|_| container::Style {
                background: Some(iced::Background::Color(rgb(theme::INPUT_FILL()))),
                border: iced::Border {
                    radius: 10.0.into(),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            }),
        );
        col = col.push(iced::widget::Space::new().height(6.0));
    }
    col = col.push(bar);

    // The attach + sticker buttons sit left of the field (hidden while
    // editing). The sticker button toggles the floating picker panel.
    let with_attach: Element<'_> = if state.editing.is_some() {
        col.into()
    } else {
        let sticker_btn: Element<'_> = if state.open_chat.is_some() {
            button(icon(Icon::Sticker, theme::ICON(), 20.0))
                .on_press(Message::ToggleStickerPicker)
                .padding(8)
                .style(flat_button)
                .into()
        } else {
            horizontal_spacer()
        };
        row![
            button(icon(Icon::Paperclip, theme::ICON(), 20.0))
                .on_press(Message::AttachFile)
                .padding(8)
                .style(flat_button),
            sticker_btn,
            col,
        ]
        .spacing(4)
        .align_y(Alignment::Center)
        .into()
    };

    container(with_attach)
        .width(Length::Fill)
        .padding([6, 12])
        .style(list_bg)
        .into()
}

// ---------------------------------------------------------------------------
// Message rows
// ---------------------------------------------------------------------------

/// A row of reaction chips (emoji + count) shown under a message bubble.
/// A chip given by the current account is highlighted with the accent colour.
fn reaction_chips<'a>(m: &'a MsgRow) -> Element<'a> {
    let mut chips = iced::widget::Row::new().spacing(3);
    for c in &m.reactions {
        let color = if c.chosen {
            theme::ACCENT()
        } else {
            theme::TEXT_SECONDARY()
        };
        chips = chips.push(
            container(
                row![
                    text(&c.emoji)
                        .font(emoji_font())
                        .size(theme::font::TIMESTAMP),
                    text(c.count.max(1).to_string())
                        .size(theme::font::TIMESTAMP)
                        .color(rgb(color)),
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            )
            .padding([2.0, 8.0])
            .style(|_| container::Style {
                background: Some(iced::Background::Color(rgb(theme::INPUT_FILL()))),
                border: iced::Border {
                    radius: 10.0.into(),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            }),
        );
    }
    chips.into()
}

/// A message row: bubble (sent at right, received at left).
/// `pane_w` is the conversation pane width used to size the bubble (received:
/// 70% of the pane, sent: 60%, matching the winit client).
fn message_row<'a>(idx: usize, m: &'a MsgRow, pane_w: f32, state: &'a State) -> Element<'a> {
    // Stickers render frameless (no bubble): centered image + timestamp.
    if m.sticker.is_some() {
        return sticker_message_row(idx, m);
    }
    // Bubble width (received: 70% of the pane, sent: 60%).
    let bubble_w = if m.out { pane_w * 0.6 } else { pane_w * 0.7 };

    // Quoted header inside the bubble (reply target or forward origin).
    let quote: Option<Element<'a>> = if let Some(reply_id) = m.reply_to {
        let snippet = state
            .messages
            .iter()
            .find(|r| r.id == reply_id)
            .map(|r| crate::state::preview_text(&r.text, &r.photo, &r.doc, &r.sticker))
            .unwrap_or_else(|| "Original message".to_string());
        Some(quote_block("Reply", snippet))
    } else {
        m.forwarded_from
            .as_ref()
            .map(|from| quote_block("Forwarded", from.clone()))
    };

    // Bubble content: media card (video / gif / audio / voice / file), photo
    // or plain text.
    let body: Element<'a> = if let Some(doc) = &m.doc {
        let doc_name = if doc.name.is_empty() {
            match doc.kind {
                bridge::DocKind::Video => "Video",
                bridge::DocKind::Gif => "GIF",
                bridge::DocKind::Audio { .. } => "Audio",
                bridge::DocKind::File => "File",
            }
            .to_string()
        } else {
            doc.name.clone()
        };
        // Metadata line: duration (audio/video) + size + action status.
        let dur = doc.duration.map(duration_fmt);
        let mut meta = match (doc.kind, dur) {
            (bridge::DocKind::Video, Some(d)) => format!("{d} · "),
            (bridge::DocKind::Audio { .. }, Some(d)) => format!("{d} · "),
            _ => String::new(),
        };
        if doc.size > 0 {
            meta.push_str(&fmt_size(doc.size));
            meta.push_str(" · ");
        }
        let status = if m.uploading.is_some() {
            "Uploading…".to_string()
        } else if m.doc_path.is_some() {
            match doc.kind {
                bridge::DocKind::Audio { voice: true } => "Play".to_string(),
                _ => "Open".to_string(),
            }
        } else {
            match doc.kind {
                bridge::DocKind::Audio { voice: true } => "Listen".to_string(),
                _ => "Download".to_string(),
            }
        };
        meta.push_str(&status);
        let media_icon = match doc.kind {
            bridge::DocKind::Video => Icon::Video,
            bridge::DocKind::Gif => Icon::Gif,
            bridge::DocKind::Audio { .. } => Icon::Audio,
            bridge::DocKind::File => Icon::FileDoc,
        };

        // Voice notes get an inline player: play/pause + progress bar driven
        // by the audio engine's `state` (no click routing through `click`).
        let media_card: Element<'a> = match doc.kind {
            bridge::DocKind::Audio { voice: true } => {
                let is_this = state.playing_voice.as_ref().map(|(_, mid, _)| *mid) == Some(m.id);
                let pct = if is_this {
                    let total = doc.duration.unwrap_or(0.0).max(0.1);
                    (state.voice_elapsed / total as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                // Interactive progress: a real slider (click/drag to seek)
                // when the duration is known; the flat indicator otherwise.
                let total = doc.duration.unwrap_or(0.0).max(0.0) as f32;
                let bar: Element<'a> = if total > 0.0 {
                    iced::widget::slider(
                        0.0..=total,
                        state.voice_elapsed.clamp(0.0, total),
                        Message::VoiceSeek,
                    )
                    .width(Length::Fill)
                    .style(voice_slider_style)
                    .into()
                } else {
                    container(
                        container(iced::widget::Space::new())
                            .width(Length::FillPortion((pct * 100.0).max(1.0) as u16))
                            .height(Length::Fill)
                            .style(|_| container::Style {
                                background: Some(iced::Background::Color(rgb(theme::ACCENT()))),
                                border: iced::Border {
                                    radius: 2.0.into(),
                                    ..iced::Border::default()
                                },
                                ..container::Style::default()
                            }),
                    )
                    .width(Length::Fill)
                    .height(4.0)
                    .style(|_| container::Style {
                        background: Some(iced::Background::Color(rgb(theme::DIVIDER()))),
                        border: iced::Border {
                            radius: 2.0.into(),
                            ..iced::Border::default()
                        },
                        ..container::Style::default()
                    })
                    .into()
                };
                let play_icon = if is_this && state.voice_playing {
                    icon(Icon::Pause, theme::ACCENT(), 18.0)
                } else {
                    icon(Icon::Play, theme::ACCENT(), 18.0)
                };
                let voice_path = m.doc_path.clone().unwrap_or_default();
                let click = button(
                    row![
                        play_icon,
                        bar,
                        text(duration_fmt(doc.duration.unwrap_or(0.0)))
                            .size(theme::font::TIMESTAMP)
                            .color(rgb(theme::TEXT_SECONDARY())),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .width(Length::Fill),
                )
                .on_press(Message::VoiceClicked {
                    chat_id: state.open_chat.unwrap_or(0),
                    msg_id: m.id,
                    path: voice_path,
                })
                .padding(0)
                .style(flat_button)
                .width(Length::Fill);
                column![
                    click,
                    if m.text.is_empty() {
                        horizontal_spacer()
                    } else {
                        text(&m.text)
                            .size(theme::font::MESSAGE)
                            .color(rgb(theme::TEXT_PRIMARY()))
                            .into()
                    },
                ]
                .spacing(6)
                .into()
            }
            _ => column![
                row![
                    icon(media_icon, theme::ACCENT(), 30.0),
                    column![
                        text(doc_name)
                            .size(theme::font::MESSAGE)
                            .color(rgb(theme::TEXT_PRIMARY()))
                            .wrapping(iced::widget::text::Wrapping::None),
                        text(meta)
                            .size(theme::font::TIMESTAMP)
                            .color(rgb(theme::TEXT_SECONDARY())),
                    ]
                    .spacing(2)
                    .width(Length::Fill),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                if m.text.is_empty() {
                    horizontal_spacer()
                } else {
                    message_body(&m.text, &m.code)
                },
            ]
            .spacing(6)
            .into(),
        }; // `media_card`
        media_card
    } else if let Some((w, h)) = m.photo {
        if w == 0 || h == 0 {
            // Optimistic media upload before the echo: live progress bar.
            uploading_bar(m.uploading.unwrap_or(0.0))
        } else if let Some(path) = &m.photo_path {
            let photo_el: Element<'_> = image(image::Handle::from_path(path))
                .width(Length::Fill)
                .content_fit(iced::ContentFit::Contain)
                .border_radius(12.0)
                .into();
            column![
                photo_el,
                if m.text.is_empty() {
                    horizontal_spacer()
                } else {
                    message_body(&m.text, &m.code)
                },
            ]
            .spacing(6)
            .into()
        } else {
            placeholder_strip("Loading image…")
        }
    } else {
        message_body(&m.text, &m.code)
    };

    // Stack the sender name (group chats) and quote above the media/text body.
    let sender_line: Option<Element<'a>> = if !m.out {
        m.sender_name
            .as_ref()
            .filter(|_| !m.text.is_empty() || m.photo.is_some() || m.doc.is_some())
            .map(|name| {
                text(name.clone())
                    .size(theme::font::TIMESTAMP)
                    .color(rgb(sender_color(m.sender_id)))
                    .font(iced::Font {
                        weight: iced::font::Weight::Semibold,
                        ..iced::Font::DEFAULT
                    })
                    .into()
            })
    } else {
        None
    };
    let body_full: Element<'a> = {
        let mut stack: iced::widget::Column<'a, Message> = column![];
        if let Some(s) = sender_line {
            stack = stack.push(s);
        }
        if let Some(q) = quote {
            stack = stack.push(q);
        }
        stack = stack.push(body);
        stack.spacing(4).into()
    };

    // Tail corner smaller, opposite corner small — Telegram style.
    let radius = theme::layout::BUBBLE_RADIUS;
    let corner: f32 = 4.0;
    let r = iced::border::Radius::new(radius)
        .top_left(if m.out { radius } else { corner })
        .top_right(if m.out { corner } else { radius })
        .bottom_left(radius)
        .bottom_right(radius);

    let bubble = container(body_full)
        .padding([theme::layout::BUBBLE_PAD_Y, theme::layout::BUBBLE_PAD_X])
        .width(bubble_w)
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(rgb(if m.out {
                theme::BUBBLE_SENT()
            } else {
                theme::BUBBLE_RECV()
            }))),
            border: iced::Border {
                radius: r,
                ..iced::Border::default()
            },
            ..container::Style::default()
        });

    // Timestamp + status tick, OUTSIDE the bubble (like the custom client).
    // `theme::cached_time` memoizes each date's "HH:MM", so a burst of messages
    // sharing a minute pay one chrono/Local format and cheap cloned strings.
    let ts = if m.date > 0 {
        theme::cached_time(m.date)
    } else {
        String::new()
    };
    let meta: Element<'_> = if m.out {
        let ts_text: Element<'_> = text(ts.clone())
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::TEXT_SECONDARY()))
            .into();
        let tick: Element<'_> = if m.read {
            icon(Icon::Tick { read: true }, theme::ACCENT(), 15.0)
        } else {
            icon(Icon::Tick { read: false }, theme::TEXT_SECONDARY(), 15.0)
        };
        row![tick, ts_text].spacing(6).into()
    } else {
        text(ts)
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::TEXT_SECONDARY()))
            .into()
    };

    // Left/right click handling via mouse_area. WinIt put the timestamp on
    // the OUTER side of each bubble (left of outgoing, right of incoming).
    // Reaction chips dock under the bubble (right for sent, left for received),
    // pinned to the bubble's own width so they never bleed into the margin.
    let bubble = if m.reactions.is_empty() {
        bubble
    } else {
        container(
            column![bubble, reaction_chips(m)]
                .spacing(3)
                .align_x(if m.out {
                    iced::alignment::Horizontal::Right
                } else {
                    iced::alignment::Horizontal::Left
                }),
        )
        .width(Length::Fixed(bubble_w.max(60.0)))
        .clip(true)
        .into()
    };

    let wrapped = mouse_area(if m.out {
        row![meta, bubble].spacing(8).align_y(Alignment::Center)
    } else {
        row![bubble, meta].spacing(8).align_y(Alignment::Center)
    })
    .on_press(Message::RowClicked(idx))
    .on_right_press(Message::RowContext(idx));

    let row_el: Element<'_> = if m.out {
        row![horizontal_spacer(), wrapped].into()
    } else {
        row![wrapped, horizontal_spacer()].into()
    };
    container(row_el)
        .padding([0.0, theme::layout::MSG_PAD_X])
        .width(Length::Fill)
        .into()
}

/// Frameless sticker row: centered image (no bubble, no background), the
/// group sender name above it and a discreet timestamp below.
fn sticker_message_row<'a>(idx: usize, m: &'a MsgRow) -> Element<'a> {
    // Sender name only for incoming group messages that show names anyway.
    let sender_line: Option<Element<'a>> = if !m.out {
        m.sender_name.as_ref().map(|name| {
            text(name.clone())
                .size(theme::font::TIMESTAMP)
                .color(rgb(sender_color(m.sender_id)))
                .font(iced::Font {
                    weight: iced::font::Weight::Semibold,
                    ..iced::Font::DEFAULT
                })
                .into()
        })
    } else {
        None
    };

    let img: Element<'a> = match &m.sticker_path {
        Some(path) => image(image::Handle::from_path(path))
            .width(Length::Fixed(STICKER_SIZE))
            .height(Length::Fixed(STICKER_SIZE))
            .content_fit(iced::ContentFit::Contain)
            .into(),
        None => container(
            text("…")
                .size(theme::font::NAME)
                .color(rgb(theme::TEXT_SECONDARY())),
        )
        .width(STICKER_SIZE)
        .height(STICKER_SIZE)
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Center)
        .into(),
    };

    let ts = if m.date > 0 {
        theme::cached_time(m.date)
    } else {
        String::new()
    };
    let meta: Element<'_> = if m.out {
        let tick: Element<'_> = if m.read {
            icon(Icon::Tick { read: true }, theme::ACCENT(), 15.0)
        } else {
            icon(Icon::Tick { read: false }, theme::TEXT_SECONDARY(), 15.0)
        };
        row![
            tick,
            text(ts)
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::TEXT_SECONDARY()))
        ]
        .spacing(6)
        .into()
    } else {
        text(ts)
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::TEXT_SECONDARY()))
            .into()
    };

    let mut stack: iced::widget::Column<'a, Message> = column![];
    if let Some(s) = sender_line {
        stack = stack.push(
            container(s)
                .width(Length::Fill)
                .align_x(iced::alignment::Horizontal::Center),
        );
    }
    stack = stack.push(img);
    stack = stack.push(
        container(meta)
            .width(Length::Fill)
            .align_x(iced::alignment::Horizontal::Center),
    );
    stack = stack.spacing(4);

    let wrapped = mouse_area(stack)
        .on_press(Message::RowClicked(idx))
        .on_right_press(Message::RowContext(idx));

    container(wrapped)
        .padding([0.0, theme::layout::MSG_PAD_X])
        .width(Length::Fill)
        .into()
}

/// The quoted block inside a bubble (reply preview / forward origin): an
/// accent bar + two lines of small text.
fn quote_block(label: &str, content: String) -> Element<'static> {
    row![
        container(iced::widget::Space::new())
            .width(3.0)
            .height(28.0)
            .style(|_| container::Style {
                background: Some(iced::Background::Color(rgb(theme::ACCENT()))),
                border: iced::Border {
                    radius: 2.0.into(),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            }),
        column![
            text(label.to_string())
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::ACCENT())),
            container(
                text(content)
                    .size(theme::font::TIMESTAMP)
                    .color(rgb(theme::TEXT_SECONDARY()))
                    // single-line, clipped to the bubble instead of overflowing
                    .wrapping(iced::widget::text::Wrapping::None),
            )
            .width(Length::Fill)
            .clip(true),
        ]
        .spacing(1)
        .width(Length::Fill),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

/// Thin horizontal upload-progress bar with a percentage label.
fn uploading_bar(p: f32) -> Element<'static> {
    let pct = (p.clamp(0.0, 1.0) * 100.0).round();
    column![
        text(format!("Uploading… {}%", pct))
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::TEXT_SECONDARY())),
        container(
            container(iced::widget::Space::new())
                .width(Length::FillPortion(pct.max(1.0) as u16))
                .height(Length::Fill)
                .style(|_| container::Style {
                    background: Some(iced::Background::Color(rgb(theme::ACCENT()))),
                    border: iced::Border {
                        radius: 2.0.into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                })
        )
        .width(Length::Fill)
        .height(4.0)
        .style(|_| container::Style {
            background: Some(iced::Background::Color(rgb(theme::DIVIDER()))),
            border: iced::Border {
                radius: 2.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        }),
    ]
    .spacing(4)
    .width(Length::Fill)
    .into()
}

/// Rounded grey strip used while a photo thumbnail has not arrived yet.
fn placeholder_strip(label: &str) -> Element<'static> {
    container(
        text(label.to_string())
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::TEXT_SECONDARY())),
    )
    .width(Length::Fill)
    .padding(24)
    .align_x(Alignment::Center)
    .style(|_| container::Style {
        background: Some(iced::Background::Color(rgb(theme::INPUT_FILL()))),
        border: iced::Border {
            radius: 12.0.into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// Human-readable byte size ("1.5 MB" style).
fn fmt_size(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * KB;
    const GB: f64 = MB * KB;
    let b = bytes.max(0) as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Formats a media duration in seconds as `m:ss` (or `s`).
fn duration_fmt(secs: f64) -> String {
    let secs = secs.max(0.0).round() as i64;
    if secs >= 60 {
        format!("{}:{:02}", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

// ---------------------------------------------------------------------------
// Pinned-message banner
// ---------------------------------------------------------------------------

/// Height of the pinned banner under the chat header.
const PINNED_BANNER_H: f32 = 34.0;

/// Thin banner showing the pinned message: pin icon + label + snippet.
/// Clicking jumps to the message in the list.
fn pinned_banner(m: &MsgRow) -> Element<'static> {
    let snippet = crate::ellipsize(
        &state::preview_text(&m.text, &m.photo, &m.doc, &m.sticker),
        48,
    );
    button(
        row![
            icon(Icon::Pin, theme::ACCENT(), 14.0),
            text("Pinned")
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::ACCENT())),
            text(snippet)
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::TEXT_SECONDARY()))
                .wrapping(iced::widget::text::Wrapping::None)
                .width(Length::Fill),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .on_press(Message::PinnedClicked)
    .width(Length::Fill)
    .height(PINNED_BANNER_H)
    .padding([0.0, theme::layout::MSG_PAD_X])
    .style(|_theme, status| {
        let bg = match status {
            iced::widget::button::Status::Hovered | iced::widget::button::Status::Pressed => {
                rgb(theme::ROW_HOVER())
            }
            _ => rgb(theme::LIST_BG()),
        };
        iced::widget::button::Style {
            background: Some(iced::Background::Color(bg)),
            border: iced::Border::default(),
            ..iced::widget::button::Style::default()
        }
    })
    .into()
}

// ---------------------------------------------------------------------------
// Forum topics (chips bar)
// ---------------------------------------------------------------------------

/// Height of the topic chips bar (chips + vertical padding).
const TOPIC_BAR_H: f32 = 34.0;

/// The topic chips bar of a forum chat: an "All messages" chip, one chip
/// per topic and a "+" chip that opens the inline create-topic field. Only
/// rendered for forum chats with a loaded topic list
/// (`State::topic_bar_visible`), between the pinned-banner zone and the
/// message list.
fn topic_chips_bar(state: &State) -> Element<'_> {
    let mut chips = iced::widget::Row::new()
        .spacing(6)
        .align_y(Alignment::Center);
    // The create field leads the row while open: appended at the end it can
    // land past the horizontal-scroll viewport once the chips fill the bar,
    // which reads as "the + chip does nothing".
    if state.topic_creating {
        chips = chips.push(
            text_input("Topic title", &state.topic_title)
                .on_input(Message::TopicCreateTitle)
                .on_submit(Message::TopicCreateSubmit)
                .size(theme::font::TIMESTAMP)
                .width(160.0),
        );
        chips = chips.push(
            button(
                text("Add")
                    .size(theme::font::TIMESTAMP)
                    .color(rgb((255, 255, 255))),
            )
            .on_press(Message::TopicCreateSubmit)
            .padding([3.0, 10.0])
            .style(|_theme, _status| iced::widget::button::Style {
                background: Some(iced::Background::Color(rgb(theme::ACCENT()))),
                border: iced::Border {
                    radius: 14.0.into(),
                    ..iced::Border::default()
                },
                ..iced::widget::button::Style::default()
            }),
        );
        chips = chips.push(
            button(
                text("\u{2715}")
                    .size(theme::font::TIMESTAMP)
                    .color(rgb(theme::TEXT_SECONDARY())),
            )
            .on_press(Message::TopicCreateCancel)
            .padding([3.0, 6.0])
            .style(|_theme, _status| iced::widget::button::Style {
                background: None,
                border: iced::Border::default(),
                ..iced::widget::button::Style::default()
            }),
        );
    }
    chips = chips.push(topic_chip(
        "All messages",
        state.topic_selected.is_none(),
        Message::TopicChipPicked(None),
    ));
    for t in &state.topic_topics {
        chips = chips.push(topic_chip(
            &t.title,
            state.topic_selected == Some(t.root_msg_id),
            Message::TopicChipPicked(Some(t.root_msg_id)),
        ));
    }
    if !state.topic_creating {
        chips = chips.push(topic_chip("+", false, Message::TopicCreateOpen));
    }
    container(
        scrollable(chips).direction(iced::widget::scrollable::Direction::Horizontal(
            // Hidden: a visible scroller overlaps the chips in the 34px bar
            // and makes the topic labels unreadable. The bar still scrolls
            // when the chips overflow (trackpad / Shift+scroll).
            iced::widget::scrollable::Scrollbar::hidden(),
        )),
    )
    .width(Length::Fill)
    .height(TOPIC_BAR_H)
    .padding([0.0, theme::layout::MSG_PAD_X])
    .align_y(Alignment::Center)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(rgb(theme::LIST_BG()))),
        border: iced::Border {
            radius: 0.0.into(),
            width: 0.0,
            color: iced::Color::TRANSPARENT,
        },
        ..container::Style::default()
    })
    .into()
}

/// One rounded topic chip; the active one fills with the accent color.
fn topic_chip<'a>(label: &'a str, active: bool, msg: Message) -> Element<'a> {
    let fg = if active {
        (255, 255, 255)
    } else {
        theme::TEXT_PRIMARY()
    };
    button(
        text(label.to_string())
            .size(theme::font::TIMESTAMP)
            .color(rgb(fg)),
    )
    .on_press(msg)
    .padding([3.0, 12.0])
    .style(move |_theme, status| {
        let (bg, border) = if active {
            (theme::ACCENT(), theme::ACCENT())
        } else {
            let bg = match status {
                iced::widget::button::Status::Hovered | iced::widget::button::Status::Pressed => {
                    theme::ROW_HOVER()
                }
                _ => theme::LIST_BG(),
            };
            (bg, theme::MENU_BORDER())
        };
        iced::widget::button::Style {
            background: Some(iced::Background::Color(rgb(bg))),
            border: iced::Border {
                radius: 14.0.into(),
                width: 1.0,
                color: rgb(border),
            },
            ..iced::widget::button::Style::default()
        }
    })
    .into()
}

// ---------------------------------------------------------------------------
// Group sender names
// ---------------------------------------------------------------------------

/// Telegram's group sender palette (7 hues), picked deterministically from
/// the sender id so a given author always gets the same color.
const SENDER_COLORS: [(u8, u8, u8); 7] = [
    (228, 105, 98),  // red
    (230, 145, 56),  // orange
    (119, 156, 62),  // green
    (46, 174, 176),  // cyan
    (124, 132, 219), // blue-violet
    (214, 96, 178),  // pink
    (240, 122, 90),  // coral
];

/// Color of a sender's display name (by bot-api id).
fn sender_color(id: Option<i64>) -> (u8, u8, u8) {
    let i = id.unwrap_or(0).rem_euclid(SENDER_COLORS.len() as i64) as usize;
    SENDER_COLORS[i]
}

// ---------------------------------------------------------------------------
// Context menu (Reply / Forward / Pin / Edit / Copy / Delete)
// ---------------------------------------------------------------------------

/// Height of one context-menu item (10 px padding per side + ~17 px text).
const CONTEXT_ITEM_H: f32 = 37.0;
/// Inner padding of the menu container (M3 menus pad around their items).
const CONTEXT_MENU_PAD: f32 = 6.0;

/// One context-menu row: icon in a fixed-width column + label, hover state,
/// 8 px item radius (M3: item < container corner).
fn menu_item<'a>(msg: Message, ic: Icon, label: &'a str, destructive: bool) -> Element<'a> {
    let label_color = if destructive {
        rgb(theme::ERROR())
    } else {
        rgb(theme::TEXT_PRIMARY())
    };
    let icon_color = if destructive {
        theme::ERROR()
    } else {
        theme::ICON()
    };
    button(
        row![
            container(icon(ic, icon_color, 16.0))
                .width(18.0)
                .align_x(iced::alignment::Horizontal::Center),
            text(label)
                .size(theme::font::PLACEHOLDER)
                .color(label_color),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .on_press(msg)
    .width(Length::Fill)
    .padding([10, 12])
    .style(move |t, s| menu_item_style(t, s, destructive))
    .into()
}

/// The right-click context menu, rendered inline right under the message
/// that raised it instead of an absolute overlay: floating a menu at the
/// clicked coordinates isn't available in iced, so anchoring it to the row
/// is the closest faithful behaviour. Reply/forward apply to any message;
/// edit stays restricted to the user's own text messages.
fn context_menu_bar(state: &State) -> Element<'static> {
    let can_edit = state.context_can_edit();
    let has_text = state
        .context_menu
        .and_then(|c| state.messages.get(c.row))
        .is_some_and(|m| !m.text.is_empty());

    let mut items = column![].spacing(2);
    items = items.push(menu_item(
        Message::ContextReply,
        Icon::Reply,
        "Reply",
        false,
    ));
    items = items.push(menu_item(
        Message::ContextReact,
        Icon::Smile,
        "React",
        false,
    ));
    items = items.push(menu_item(
        Message::ContextForward,
        Icon::Forward,
        "Forward",
        false,
    ));
    let pinned_label = if state.context_row_pinned() {
        "Unpin"
    } else {
        "Pin"
    };
    items = items.push(menu_item(
        Message::ContextPin,
        Icon::Pin,
        pinned_label,
        false,
    ));
    if can_edit {
        items = items.push(menu_item(Message::ContextEdit, Icon::Edit, "Edit", false));
    }
    if has_text {
        items = items.push(menu_item(Message::ContextCopy, Icon::Copy, "Copy", false));
    }
    if can_edit {
        items = items.push(menu_item(
            Message::ContextDelete,
            Icon::Trash,
            "Delete",
            true,
        ));
    }

    let menu_el = container(items)
        .width(theme::layout::CONTEXT_W)
        .padding(CONTEXT_MENU_PAD)
        .style(menu_bg);

    // Right-aligned under the message (editable messages are ours → right).
    row![horizontal_spacer(), menu_el]
        .padding([0.0, theme::layout::MSG_PAD_X])
        .into()
}

/// Reactions offered by the quick-reaction pill (Telegram's usual favourites).
const REACTIONS: [&str; 9] = ["👍", "💖", "🔥", "👏", "😮", "😢", "🤔", "😂", "🎉"];

/// The wider set offered by the strip's "+" picker grid.
const REACTION_PICKER_EMOJI: &[&str] = &[
    "👍", "💖", "🔥", "🎉", "👏", "😮", "😢", "🤔", "😂", "😍", "🥳", "😎", "🤩", "😇", "🙏", "💪",
    "👀", "🚀", "🥰", "😅", "😭", "😱", "🙈", "💯", "✨", "⚡", "🌟", "🍀", "🌹", "☕", "🎁", "🏆",
    "🎈", "😡", "💔", "😤", "🎶", "🧠", "🫶", "🥇", "🍕", "🐶", "🐱", "💜", "💛", "🤝",
];
const REACT_STRIP_H: f32 = 44.0;
/// Height budget when the "+" picker grid is expanded under the quick pill.
const REACT_PICKER_H: f32 = 210.0;
const REACT_STRIP_EMOJI: f32 = 22.0;

/// The horizontal quick-reaction pill shown after picking React from a
/// message's context menu: one emoji per common reaction, plus a "+" button
/// that expands a grid to pick from a wider set. Clicking an emoji sends the
/// reaction and closes the strip.
fn reaction_strip<'a>(picker: bool) -> Element<'a> {
    let mut quick = iced::widget::Row::new().spacing(2);
    for emoji in REACTIONS {
        quick = quick.push(
            button(
                text(emoji)
                    .font(emoji_font())
                    .size(REACT_STRIP_EMOJI)
                    .align_y(Alignment::Center),
            )
            .padding([6, 10])
            .style(|t, s| menu_item_style(t, s, false))
            .on_press(Message::React(emoji.to_string())),
        );
    }
    quick = quick.push(
        button(
            text(if picker { "−" } else { "+" })
                .size(REACT_STRIP_EMOJI)
                .align_y(Alignment::Center),
        )
        .padding([6, 10])
        .style(|t, s| menu_item_style(t, s, false))
        .on_press(Message::ToggleReactPicker),
    );
    let quick = container(quick.padding(CONTEXT_MENU_PAD)).style(menu_bg);

    if !picker {
        return quick.into();
    }

    // Iced 0.14 has no flow/wrap layout: chunk the set into fixed-width rows
    // and stack them (each row is 8 emojis, keeping the grid compact).
    const ROW_LEN: usize = 8;
    let mut grid = iced::widget::Column::new().spacing(2);
    for chunk in REACTION_PICKER_EMOJI.chunks(ROW_LEN) {
        let mut row = iced::widget::Row::new().spacing(2);
        for emoji in chunk {
            row = row.push(
                button(
                    text(*emoji)
                        .font(emoji_font())
                        .size(REACT_STRIP_EMOJI)
                        .align_y(Alignment::Center),
                )
                .padding([4, 6])
                .style(|t, s| menu_item_style(t, s, false))
                .on_press(Message::React((*emoji).to_string())),
            );
        }
        grid = grid.push(row);
    }
    container(
        column![
            quick,
            container(grid.padding(CONTEXT_MENU_PAD)).style(menu_bg),
        ]
        .spacing(2),
    )
    .into()
}

// ---------------------------------------------------------------------------
// Virtualized message list
// ---------------------------------------------------------------------------

/// Rows kept above/below the visible window so estimated-height drift (and a
/// one-frame-late `on_scroll` offset) never reveals a blank gap.
const LIST_OVERSCAN: usize = 16;

/// Breathing room kept BELOW the last message when the list is scrolled to
/// the end: without it the snap-to-end pins the newest bubble flush against
/// the viewport bottom (hard to read, no separation from the composer).
const LIST_BOTTOM_PAD: f32 = 28.0;

/// Number of items the open context menu shows.
fn context_menu_items(state: &State) -> usize {
    let can_edit = state.context_can_edit();
    let has_text = state
        .context_menu
        .and_then(|c| state.messages.get(c.row))
        .is_some_and(|m| !m.text.is_empty());
    4 + usize::from(can_edit) + usize::from(has_text) + usize::from(can_edit)
}

/// Rendered height of the open context menu (items + spacing + padding) —
/// used to clamp the overlay position inside the content area.
fn context_menu_h(state: &State) -> f32 {
    let n = context_menu_items(state) as f32;
    CONTEXT_ITEM_H * n + (n - 1.0).max(0.0) * 2.0 + 2.0 * CONTEXT_MENU_PAD
}

/// Virtualized message list: only the rows intersecting the scrollable's
/// viewport (plus an over-scan on each side) are built and layed-out each
/// frame. The rest is replaced by height-matched spacers so the scrollbar
/// range, content height and bottom-anchoring stay correct, while the per-frame
/// cost stays proportional to the *visible* rows instead of the whole history
/// (the tiny-skia build is software-rendered: a 200-message chat used to
/// rebuild + shape every message's text on every scroll tick).
///
/// The pinned-to-bottom spacer from the original layout is preserved (`top_pad`
/// always includes a full viewport-height spacer above the first message).
/// `pub` for the `benches/` harness.
pub fn messages_list(state: &State, pane_w: f32, view_h: f32) -> Element<'_> {
    let n = state.messages.len();

    let out_at = |i: usize| state.messages[i].out;
    let gap_between = |a: bool, b: bool| if a == b { 3.0 } else { 10.0 };

    // Undocumented perf-comparison switch: force the pre-virtualization,
    // build-every-row-per-frame behaviour (used by `tools/scroll-perf.sh` to
    // quantify how much virtualization buys).
    let no_virt = std::env::var("TG_NO_VIRT").is_ok();

    if n == 0 {
        return scrollable(iced::widget::Space::new().height(view_h))
            .id(MSG_LIST_ID)
            .on_scroll(|viewport: iced::widget::scrollable::Viewport| {
                Message::Scrolled(viewport.absolute_offset().y)
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }

    // Row heights + cumulative tops only depend on the message rows, the open
    // context menu and the pane size — none of which change while scrolling.
    // They stay cached across frames; rebuilding them (O(all rows), with a
    // `chars().count()` text scan per row) on every scroll tick is what made
    // big chats lag. `no_virt` keeps the pre-cache path for the A/B harness.
    let mut cache_guard = state
        .layout_cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let hit = !no_virt
        && cache_guard.as_ref().is_some_and(|c| {
            c.pane_w == pane_w && c.view_h == view_h && c.epoch == state.layout_epoch
        });
    if !hit {
        *cache_guard = Some(build_layout(state, pane_w, view_h));
    }
    let cache = cache_guard.as_ref().expect("rebuilt in the step above");
    let heights = &cache.heights;
    let tops = &cache.tops;
    let total = cache.total;

    // Visible window from the last notified scroll offset.
    let mut start = 0usize;
    let mut end = n;
    if !no_virt {
        let offset = state.scroll_offset.max(0.0);
        // `tops` grows monotonically (`tops[i+1] = tops[i] + heights[i] +
        // gap`): binary-search the first row whose bottom edge reaches the
        // viewport instead of a linear scan from 0 (O(all rows) per frame on
        // big chats).
        let bottom_edge = |i: usize| tops[i] + heights[i];
        let mut lo = 0usize;
        let mut hi = n;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if bottom_edge(mid) < offset {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        start = lo;
        // First row whose top passes the bottom of the viewport.
        let mut lo_bottom = start;
        let mut hi = n;
        while lo_bottom < hi {
            let mid = (lo_bottom + hi) / 2;
            if tops[mid] < offset + view_h {
                lo_bottom = mid + 1;
            } else {
                hi = mid;
            }
        }
        end = lo_bottom;
        start = start.saturating_sub(LIST_OVERSCAN);
        end = (end + LIST_OVERSCAN).min(n);
    }

    let top_pad = tops[start];
    let bottom_pad = total - tops.get(end).copied().unwrap_or(total);

    let mut cols = column![];
    let mut prev_out = if start > 0 {
        Some(out_at(start - 1))
    } else {
        None
    };
    for i in start..end {
        let m = &state.messages[i];
        if i > 0 {
            cols = cols.push(
                iced::widget::Space::new().height(gap_between(prev_out.unwrap_or(m.out), m.out)),
            );
        }
        cols = cols.push(message_row(i, m, pane_w, state));
        prev_out = Some(m.out);
    }

    // Reaction strip as a floating overlay first (it replaces the context
    // menu when opened from it, so at most one overlay is ever shown): a
    // `stack` layer anchored under the target row, like the context menu.
    let content = if let Some(react_row) = state.react_row {
        let row = react_row.min(n.saturating_sub(1));
        // The expanded "+" picker grid is much taller than the quick pill.
        let strip_h = if state.react_picker {
            REACT_PICKER_H
        } else {
            REACT_STRIP_H
        };
        let row_top_vp = tops[row] - state.scroll_offset;
        let row_bot_vp = row_top_vp + heights[row];
        let desired_vp = if row_bot_vp + strip_h + 3.0 > view_h
            && tops[row] > state.scroll_offset + view_h - strip_h
        {
            // Not enough room below: float above the row, clamped to the
            // viewport top when the row itself hugs it.
            (row_top_vp - strip_h - 3.0).max(2.0)
        } else {
            row_bot_vp + 3.0
        };
        let desired_vp = desired_vp.min((view_h - strip_h - 8.0).max(2.0));
        let y = (desired_vp + state.scroll_offset - top_pad).max(0.0);
        let x = 8.0; // left-anchored to the bubble edge, Telegram-style
        let layer = mouse_area(
            container(reaction_strip(state.react_picker))
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(iced::Padding {
                    top: y,
                    left: x,
                    ..Default::default()
                }),
        )
        // Any click outside the strip dismisses it (and its picker). Emoji
        // buttons are nested deeper, so they still win the click.
        .on_press(Message::CloseReact);
        Element::from(iced::widget::stack![cols, layer])
    // Context menu as a floating overlay: a `stack` layer anchored under (or,
    // near the bottom, above) the target row, right-aligned. It no longer
    // participates in the column layout — opening it must not push the
    // messages down. It floats even if its row is currently virtualized out
    // (`tops` covers every row).
    } else if let Some(menu) = state.context_menu {
        let row = menu.row.min(n.saturating_sub(1));
        // Position in VIEWPORT space: the menu must stay on screen, so the
        // flip compares against the visible height (not the content tail —
        // comparing against it made menus near the list bottom open off-screen).
        let menu_h = context_menu_h(state);
        let row_top_vp = tops[row] - state.scroll_offset;
        let row_bot_vp = row_top_vp + heights[row];
        let desired_vp = if row_bot_vp + menu_h + 3.0 > view_h {
            // Not enough room below: float above the row, clamped to the
            // viewport top when the row itself hugs it.
            (row_top_vp - menu_h - 3.0).max(2.0)
        } else {
            row_bot_vp + 3.0
        };
        // Keep the whole menu inside the viewport with a small breathing
        // margin (the estimate can undershoot the real painted height).
        let desired_vp = desired_vp.min((view_h - menu_h - 8.0).max(2.0));
        // The layer's padding is relative to the slice start (`top_pad`).
        let y = (desired_vp + state.scroll_offset - top_pad).max(0.0);
        let layer = mouse_area(
            container(
                row![horizontal_spacer(), context_menu_bar(state)]
                    .padding([0.0, theme::layout::MSG_PAD_X]),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding {
                top: y,
                ..Default::default()
            }),
        )
        // Clicking anywhere outside the menu dismisses it (Telegram closes the
        // menu on the first click elsewhere); the menu items are nested deeper
        // and so still receive their own clicks.
        .on_press(Message::CloseOverlays);
        Element::from(iced::widget::stack![cols, layer])
    } else {
        cols.into()
    };

    scrollable(column![
        iced::widget::Space::new().height(top_pad),
        content,
        iced::widget::Space::new().height(bottom_pad + LIST_BOTTOM_PAD),
    ])
    .id(MSG_LIST_ID)
    .on_scroll(|viewport: iced::widget::scrollable::Viewport| {
        Message::Scrolled(viewport.absolute_offset().y)
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// Computes and returns the per-row heights / top offsets / total height cache
/// for the current message list. Called only when the cache is stale (message
/// change or a pane resize). The context menu no longer contributes: it is
/// rendered as an overlay (`stack`) that floats above the rows instead of
/// pushing the content down.
fn build_layout(state: &State, pane_w: f32, view_h: f32) -> crate::state::MsgLayoutCache {
    let n = state.messages.len();
    let out_at = |i: usize| state.messages[i].out;
    let gap_between = |a: bool, b: bool| if a == b { 3.0 } else { 10.0 };

    let heights: Vec<f32> = state
        .messages
        .iter()
        .map(|m| est_row_height(m, pane_w))
        .collect();

    let mut tops = Vec::with_capacity(n);
    let mut y = view_h; // pin spacer keeps bubbles at the bottom when short.
    for (i, h) in heights.iter().enumerate() {
        tops.push(y);
        y += h;
        if i + 1 < n {
            y += gap_between(out_at(i), out_at(i + 1));
        }
    }
    let total = y;

    crate::state::MsgLayoutCache {
        pane_w,
        view_h,
        epoch: state.layout_epoch,
        heights,
        tops,
        total,
    }
}

/// Cheap O(1) estimate of `message_row`'s layout height for a pane width.
///
/// It has no text shaping: char count × an average glyph advance is enough for
/// the spacer sizing, and any drift is absorbed by [`LIST_OVERSCAN`]. Must stay
/// in lockstep with `message_row`'s paddings/widths (photo fills the bubble
/// width with `Contain`, caption gets a 6 px gap, bubble padding is
/// [`theme::layout::BUBBLE_PAD_X`]/[`theme::layout::BUBBLE_PAD_Y`]).
fn est_row_height(m: &MsgRow, pane_w: f32) -> f32 {
    // Stickers: fixed-size frameless block (image + timestamp + gaps).
    if m.sticker.is_some() {
        let sender_h = if m.sender_name.is_some() && !m.out {
            theme::font::TIMESTAMP * 1.3 + 4.0
        } else {
            0.0
        };
        return STICKER_SIZE + sender_h + theme::font::TIMESTAMP * 1.3 + 12.0;
    }
    let bubble_w = if m.out { pane_w * 0.6 } else { pane_w * 0.7 };
    let inner = (bubble_w - 2.0 * theme::layout::BUBBLE_PAD_X).max(1.0);
    let font_h = theme::font::MESSAGE;

    // Quoted header (reply / forward): accent bar + 2 small lines + gap.
    let quote_h = if m.reply_to.is_some() || m.forwarded_from.is_some() {
        2.0 * theme::font::TIMESTAMP * 1.3 + 6.0
    } else {
        0.0
    };

    // Group sender name line above the body.
    let sender_h = if m.sender_name.is_some() && !m.out {
        theme::font::TIMESTAMP * 1.3 + 4.0
    } else {
        0.0
    };

    // Document card: icon row (~30 px) + optional caption.
    let doc_h = if m.doc.is_some() { 34.0 } else { 0.0 };

    // Photo: fills the bubble width with `Contain`; the optimistic
    // (0,0)-dimensioned upload shows the progress strip instead.
    let photo_h = match m.photo {
        Some((w, h)) if w > 0 && h > 0 => inner * (h as f32 / w as f32),
        Some(_) => 30.0,
        None => 0.0,
    };
    let text_h = if m.text.is_empty() {
        0.0
    } else {
        // ~0.52em average advance per char, 1.3 line height, wrapped at `inner`.
        let per_line = (inner / (font_h * 0.52)).floor().max(1.0);
        let lines = (m.text.chars().count() as f32 / per_line).ceil();
        lines * font_h * 1.3
    };
    let caption = if (m.photo.is_some() || m.doc.is_some()) && !m.text.is_empty() {
        6.0
    } else {
        0.0
    };
    let body_gap = if quote_h > 0.0 { 6.0 } else { 0.0 };
    2.0 * theme::layout::BUBBLE_PAD_Y
        + quote_h
        + sender_h
        + body_gap
        + doc_h
        + photo_h
        + caption
        + text_h
}

// ---------------------------------------------------------------------------
// Widgets helpers
// ---------------------------------------------------------------------------

fn horizontal_spacer() -> Element<'static> {
    iced::widget::Space::new().width(Length::Fill).into()
}

/// Truncates `s` to at most `max` chars, adding an ellipsis when clipped
/// (matches the winit client's single-line chat rows).
pub fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn avatar_circle(photo: Option<&str>, title: &str, size: f32) -> Element<'static> {
    if let Some(path) = photo {
        if let Some(handle) = circular_avatar_handle(path, size) {
            return container(
                image(handle)
                    .width(Length::Fixed(size))
                    .height(Length::Fixed(size))
                    .content_fit(iced::ContentFit::Cover),
            )
            .width(Length::Fixed(size))
            .height(Length::Fixed(size))
            .into();
        }
    }
    let ch = title
        .chars()
        .next()
        .unwrap_or('?')
        .to_string()
        .to_uppercase();
    let c = theme::avatar_color(title);
    container(
        container(text(ch).size(theme::font::NAME).color(Color::WHITE))
            .width(size)
            .height(size)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .style(move |_| container::Style {
                background: Some(iced::Background::Color(rgb(c))),
                border: iced::Border {
                    radius: (size / 2.0).into(),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            }),
    )
    .into()
}

/// Renders an avatar as a circular, alpha-masked `Handle`.
///
/// The tiny-skia backend ignores `border_radius` on image widgets, so square
/// images stay square. We decode the thumbnail once per (path, size), mask it
/// with a circle and memoize the result — the raster pipeline keys its cache
/// on `Handle::id()`, and `from_rgba` regenerates a fresh id on every call,
/// so recomputing it per frame would defeat that cache.
fn circular_avatar_handle(path: &str, size: f32) -> Option<image::Handle> {
    use std::sync::Mutex;

    static CACHE: std::sync::OnceLock<Mutex<HashMap<(String, u32), image::Handle>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    // Render at 2x so the circle stays crisp on HiDPI displays.
    let px = ((size * 2.0).round() as u32).max(8);
    let key = (path.to_string(), px);
    if let Some(h) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return Some(h);
    }

    let handle = build_circular_avatar(path, px)?;
    if let Ok(mut c) = cache.lock() {
        c.insert(key, handle.clone());
    }
    Some(handle)
}

fn build_circular_avatar(path: &str, px: u32) -> Option<image::Handle> {
    let img = ::image_codec::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    // Cover-crop into a `px` square, then punch out a circle with an alpha
    // mask (the areas outside the disc become transparent).
    let mut rgba = img
        .resize_to_fill(px, px, ::image_codec::imageops::FilterType::Triangle)
        .to_rgba8();
    let center = (px as f32 - 1.0) / 2.0;
    let radius = px as f32 / 2.0;
    for y in 0..px {
        for x in 0..px {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            if dx * dx + dy * dy > radius * radius {
                rgba.get_pixel_mut(x, y)[3] = 0;
            }
        }
    }
    Some(image::Handle::from_rgba(px, px, rgba.into_raw()))
}

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

fn list_bg(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(rgb(theme::LIST_BG()))),
        ..container::Style::default()
    }
}

fn chat_bg(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(rgb(theme::CHAT_BG()))),
        ..container::Style::default()
    }
}

fn divider(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(rgb(theme::DIVIDER()))),
        ..container::Style::default()
    }
}

fn header_bg(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(rgb(theme::LIST_BG()))),
        ..container::Style::default()
    }
}

fn row_style(
    _theme: &iced::Theme,
    status: iced::widget::button::Status,
    selected: bool,
) -> iced::widget::button::Style {
    let bg = if selected {
        theme::ROW_SELECTED()
    } else {
        match status {
            iced::widget::button::Status::Hovered => theme::ROW_HOVER(),
            _ => theme::LIST_BG(),
        }
    };
    iced::widget::button::Style {
        background: Some(iced::Background::Color(rgb(bg))),
        border: iced::Border {
            radius: 12.0.into(),
            ..iced::Border::default()
        },
        ..iced::widget::button::Style::default()
    }
}

fn flat_button(
    theme: &iced::Theme,
    status: iced::widget::button::Status,
) -> iced::widget::button::Style {
    icon_button_style(theme, status)
}

/// Circular icon button (M3 icon-button): transparent at rest, subtle
/// state-layer fill on hover/press.
fn icon_button_style(
    _theme: &iced::Theme,
    status: iced::widget::button::Status,
) -> iced::widget::button::Style {
    let bg = match status {
        iced::widget::button::Status::Hovered => {
            let (r, g, b, a) = theme::HOVER_OVERLAY(false);
            Some(iced::Background::Color(iced::Color::from_rgba8(r, g, b, a)))
        }
        iced::widget::button::Status::Pressed => {
            let (r, g, b, a) = theme::HOVER_OVERLAY(true);
            Some(iced::Background::Color(iced::Color::from_rgba8(r, g, b, a)))
        }
        _ => None,
    };
    iced::widget::button::Style {
        background: bg,
        border: iced::Border {
            radius: 999.0.into(),
            ..iced::Border::default()
        },
        ..iced::widget::button::Style::default()
    }
}

fn accent_circle_button(
    _theme: &iced::Theme,
    status: iced::widget::button::Status,
) -> iced::widget::button::Style {
    let bg = match status {
        iced::widget::button::Status::Hovered => theme::ACCENT_HOVER(),
        iced::widget::button::Status::Pressed => theme::ACCENT_PRESSED(),
        _ => theme::ACCENT(),
    };
    iced::widget::button::Style {
        background: Some(iced::Background::Color(rgb(bg))),
        border: iced::Border {
            radius: 17.0.into(),
            ..iced::Border::default()
        },
        ..iced::widget::button::Style::default()
    }
}

fn accent_button(
    _theme: &iced::Theme,
    status: iced::widget::button::Status,
) -> iced::widget::button::Style {
    let bg = match status {
        iced::widget::button::Status::Hovered => theme::ACCENT_HOVER(),
        iced::widget::button::Status::Pressed => theme::ACCENT_PRESSED(),
        _ => theme::ACCENT(),
    };
    iced::widget::button::Style {
        background: Some(iced::Background::Color(rgb(bg))),
        border: iced::Border {
            radius: 14.0.into(),
            ..iced::Border::default()
        },
        ..iced::widget::button::Style::default()
    }
}

/// Context-menu row: neutral hover fill, red-tinted hover for the
/// destructive item, 8 px radius (M3: item corner < container corner).
fn menu_item_style(
    _theme: &iced::Theme,
    status: iced::widget::button::Status,
    destructive: bool,
) -> iced::widget::button::Style {
    let bg = match status {
        iced::widget::button::Status::Hovered | iced::widget::button::Status::Pressed => {
            if destructive {
                iced::Color::from_rgba(0.95, 0.45, 0.49, 0.14)
            } else {
                let (r, g, b, a) = theme::MENU_ITEM_OVERLAY();
                iced::Color::from_rgba8(r, g, b, a)
            }
        }
        _ => iced::Color::TRANSPARENT,
    };
    iced::widget::button::Style {
        background: Some(iced::Background::Color(bg)),
        border: iced::Border {
            radius: 8.0.into(),
            ..iced::Border::default()
        },
        ..iced::widget::button::Style::default()
    }
}

/// Menu surface: elevated dark fill, hairline border, 12 px corner radius
/// (M3 "medium" component shape).
fn menu_bg(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(rgb(theme::MENU_BG()))),
        border: iced::Border {
            radius: 12.0.into(),
            width: 1.0,
            color: rgb(theme::MENU_BORDER()),
        },
        ..container::Style::default()
    }
}

fn badge_circle(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(rgb(theme::ACCENT()))),
        border: iced::Border {
            radius: 10.0.into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    }
}

fn accent_circle(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(rgb(theme::ACCENT()))),
        border: iced::Border {
            radius: 38.0.into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    }
}

fn field_rounded(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(rgb(theme::INPUT_FILL()))),
        border: iced::Border {
            radius: theme::layout::INPUT_RADIUS.into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    }
}

fn text_input_style(
    _theme: &iced::Theme,
    _status: iced::widget::text_input::Status,
) -> iced::widget::text_input::Style {
    iced::widget::text_input::Style {
        background: iced::Background::Color(rgb(theme::INPUT_FILL())),
        border: iced::Border {
            radius: theme::layout::INPUT_RADIUS.into(),
            width: 1.0,
            color: rgb(theme::INPUT_BORDER()),
        },
        icon: rgb(theme::TEXT_SECONDARY()),
        placeholder: rgb(theme::TEXT_SECONDARY()),
        value: rgb(theme::TEXT_PRIMARY()),
        selection: rgb(theme::ACCENT()),
    }
}

/// Voice-note progress slider: accent rail fill, subtle track, round thumb.
fn voice_slider_style(
    _theme: &iced::Theme,
    status: iced::widget::slider::Status,
) -> iced::widget::slider::Style {
    let (rail_width, handle_r) = match status {
        iced::widget::slider::Status::Dragged | iced::widget::slider::Status::Hovered => (5.0, 9.0),
        _ => (4.0, 7.0),
    };
    iced::widget::slider::Style {
        rail: iced::widget::slider::Rail {
            width: rail_width,
            backgrounds: (
                iced::Background::Color(rgb(theme::ACCENT())),
                iced::Background::Color(rgb(theme::DIVIDER())),
            ),
            border: iced::Border::default(),
        },
        handle: iced::widget::slider::Handle {
            shape: iced::widget::slider::HandleShape::Circle { radius: handle_r },
            background: iced::Background::Color(rgb(theme::ACCENT())),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}

/// Subscription: forwards network `UiMessage`s to the application, plus
/// keyboard shortcuts (Escape closes the context menu / cancels editing).
/// With `--perf`, a 500 ms tick samples the frame cadence for the FPS overlay.
fn subscription(state: &State) -> iced::Subscription<Message> {
    let net = network::network_subscription();
    let keys = iced::keyboard::listen().filter_map(|event| match event {
        iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            ..
        } => Some(Message::Escape),
        _ => None,
    });
    let timer = if state.scroll_perf_dur > 0.0 {
        // ~200 Hz synthetic fling for end-to-end scroll-rate measurement.
        iced::time::every(std::time::Duration::from_millis(5)).map(|_| Message::PerfTick)
    } else if state.continuous {
        // Continuous redraw loop (mirrors the winit client's always-render
        // behaviour): burns CPU but guarantees the compositor always gets a
        // new frame, defeating any event-triggered-only throttling.
        iced::time::every(std::time::Duration::from_millis(4)).map(|_| Message::PerfTickC)
    } else if state.perf_show {
        iced::time::every(std::time::Duration::from_millis(500)).map(|_| Message::PerfTick)
    } else {
        iced::Subscription::none()
    };
    let voice_tick = if state.playing_voice.is_some() {
        iced::time::every(std::time::Duration::from_millis(250)).map(|_| Message::VoiceTick)
    } else {
        iced::Subscription::none()
    };
    iced::Subscription::batch([net, keys, timer, voice_tick])
}

/// Application entry point (called from the `main.rs` binary of this crate).
/// Default the wgpu backend, respecting an explicit `WGPU_BACKEND` first.
fn default_wgpu_backend() {
    if std::env::var_os("WGPU_BACKEND").is_some() {
        return;
    }
    // On hosts running the NVIDIA proprietary driver we prefer Vulkan:
    // asking for GL there routes through Mesa GL + libLLVM (llvmpipe), i.e.
    // pure CPU rasterization, so dense chat panels (glyphs + rounded
    // corners + images) crawl despite the GPU driver being present.
    // Elsewhere GL (EGL) stays the default for its smaller resident set
    // (~47 MB PSS vs ~115 MB Vulkan — the proprietary Vulkan stack alone
    // accounts for ~68 MB of resident pages — at identical CPU cost and
    // scroll throughput). Machines without a usable EGL/GL stack fall
    // through to the tiny-skia software renderer compiled in.
    let prefer_vulkan = std::path::Path::new("/dev/nvidiactl").exists();
    std::env::set_var("WGPU_BACKEND", if prefer_vulkan { "vulkan" } else { "gl" });
}

pub fn run() -> iced::Result {
    default_wgpu_backend();
    let (w, h) = window_size_from_args();
    iced::application(boot, update, view)
        .subscription(subscription)
        .window(iced::window::Settings {
            // Taskbar/compositor icon: the same code-drawn logo as the tray.
            icon: Some(window_icon()),
            ..Default::default()
        })
        .window_size((w, h))
        .title("Telegram RS")
        // Base iced palette follows our mode: default-styled widgets (and
        // the login screen, which has no explicit background) adapt too.
        // Custom colors all come from `theme::*()` regardless.
        .theme(app_theme)
        .run()
}

/// The window icon: the app logo rasterized off the shared `Icon::Logo`
/// renderer (accent disc + white send-plane), same mark as the tray.
fn window_icon() -> iced::window::Icon {
    const PX: u32 = 128;
    let pixmap = crate::icons::render_logo_rgba(PX);
    iced::window::icon::from_rgba(pixmap.data().to_vec(), PX, PX).expect("window icon")
}

/// The application theme: iced's own palette follows our light/dark mode so
/// default-styled widgets (login screen backgrounds, scrollbars) adapt.
fn app_theme(_state: &state::State) -> iced::Theme {
    match theme::mode() {
        theme::ThemeMode::Light => iced::Theme::Light,
        theme::ThemeMode::Dark => iced::Theme::Dark,
    }
}

/// Parses an optional `--win=WxH` (logical px) to shrink the software-rendered
/// buffer for perf measurement (`--win=700x450` isolates compositor present
/// cost from our per-frame view cost).
fn window_size_from_args() -> (f32, f32) {
    for arg in std::env::args() {
        if let Some(rest) = arg.strip_prefix("--win=") {
            if let Some((w, h)) = rest.split_once('x') {
                if let (Ok(w), Ok(h)) = (w.parse::<f32>(), h.parse::<f32>()) {
                    if w >= 200.0 && h >= 150.0 {
                        return (w, h);
                    }
                }
            }
        }
    }
    (1100.0, 700.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_spans_plain_text_stays_none() {
        assert!(body_spans("just words", &[]).is_none());
        assert!(body_spans("", &[]).is_none());
    }

    #[test]
    fn line_spans_cover_previews() {
        // Plain preview: no rich-text cost.
        assert!(line_spans("novel review of the PR?").is_none());
        assert!(line_spans("").is_none());
        // Ellipsis-only decorations stay plain.
        assert!(line_spans("…").is_none());
        // Emoji in a preview split into text/emoji/text spans.
        use iced::font::Font;
        let spans = line_spans("Great! see you tomorrow 👋").unwrap();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].text, "Great! see you tomorrow ");
        assert_eq!(spans[1].text, "👋");
        assert_eq!(spans[1].font, Some(Font::with_name("Noto Color Emoji")));
    }

    #[test]
    fn body_spans_link_only_uses_linkify() {
        let spans = body_spans("see https://rust-lang.org", &[]).unwrap();
        assert!(spans.iter().any(|s| s.link.is_some()));
    }

    #[test]
    fn body_spans_emoji_get_emoji_font() {
        use iced::font::Font;
        let spans = body_spans("Salut 👋 !", &[]).unwrap();
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].text, "Salut ");
        assert_eq!(spans[1].text, "👋");
        assert_eq!(spans[1].font, Some(Font::with_name("Noto Color Emoji")));
        assert_eq!(spans[2].text, " !");
        // Emoji + link in one message: both treatments compose.
        let spans = body_spans("docs 👉 https://doc.rust-lang.org", &[]).unwrap();
        assert!(spans.iter().any(|s| s.link.is_some()));
        assert!(spans.iter().any(|s| s.font.is_some()));
    }

    #[test]
    fn body_spans_inline_code_uses_monospace_font() {
        use iced::font::Font;
        // Byte range 0..4 covers "let ".
        let spans = body_spans("let x = 1", &[(0, 3)]).unwrap();
        assert!(spans
            .iter()
            .any(|s| s.text == "let" && s.font == Some(Font::MONOSPACE)));
        assert!(spans.iter().any(|s| s.text == " x = 1" && s.font.is_none()));
    }

    #[test]
    fn message_list_virtualizes_without_panicking() {
        let (req_tx, _req_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut state = State::new(req_tx);
        state.authenticated = true;
        state.open_chat = Some(42);
        state.chat_title = "Test".into();
        state.messages = (0..300)
            .map(|i| MsgRow {
                read: true,
                ..MsgRow::text(
                    i,
                    format!("message {i} with some text that wraps a bit"),
                    1_700_000_000 - i,
                    i % 2 == 0,
                )
            })
            .collect();

        // The exact function `view()` calls every frame, over a big history.
        let el = std::hint::black_box(messages_list(&state, 820.0, 610.0));
        let _ = el;
        assert_eq!(state.messages.len(), 300);
    }

    #[test]
    fn sticker_rows_render_headlessly_with_fixed_height() {
        use crate::bridge::StickerMeta;

        let (req_tx, _req_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut state = State::new(req_tx);
        state.authenticated = true;
        state.open_chat = Some(42);
        state.messages = vec![
            MsgRow {
                sticker: Some(StickerMeta { alt: "🎉".into() }),
                sticker_path: Some("/nonexistent/sticker.webp".into()),
                sender_name: Some("Léo".into()),
                sender_id: Some(7),
                ..MsgRow::text(1, String::new(), 100, false)
            },
            MsgRow {
                sticker: Some(StickerMeta { alt: "⭐".into() }),
                // Not downloaded yet: placeholder branch.
                ..MsgRow::text(2, String::new(), 101, true)
            },
            MsgRow::text(3, "plain text neighbour", 102, false),
        ];

        // The exact per-frame entry point must build every variant without
        // touching the image file (decoding happens at raster time).
        let el = std::hint::black_box(messages_list(&state, 820.0, 610.0));
        let _ = el;

        // Height estimate: fixed-size block, independent of the pane width.
        let h_narrow = est_row_height(&state.messages[0], 400.0);
        let h_wide = est_row_height(&state.messages[0], 1200.0);
        assert_eq!(h_narrow, h_wide, "sticker rows have a fixed height");
        assert!(
            h_wide > 180.0 && h_wide < 320.0,
            "sane magnitude, got {h_wide}"
        );
        // Sender name adds a line for incoming group stickers only.
        let out_h = est_row_height(&state.messages[1], 800.0);
        assert!(out_h < h_wide, "no sender line on outgoing stickers");
    }

    #[test]
    fn sticker_picker_overlay_renders_in_all_states() {
        use crate::bridge::StickerSetBridge;

        let (req_tx, _req_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut state = State::new(req_tx);
        state.authenticated = true;
        state.open_chat = Some(42);
        // Empty (loading) picker.
        state.sticker_picker_open = true;
        let _ = std::hint::black_box(chat_view(&state));
        // Loaded packs + thumbnails.
        state.sticker_sets = vec![StickerSetBridge {
            title: "Happy Blocks".into(),
            short_name: "happy_blocks".into(),
            docs: (0..8)
                .map(|i| (900_000_000 + i, 10 * i, format!("{i}")))
                .collect(),
        }];
        state
            .sticker_thumbs
            .insert(900_000_000, "/nonexistent/thumb.webp".into());
        let _ = std::hint::black_box(chat_view(&state));
    }

    #[test]
    fn dialog_list_virtualizes_without_panicking() {
        use crate::bridge::UiMessage;

        let (req_tx, _req_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut state = State::new(req_tx);
        state.authenticated = true;
        let dialogs: Vec<ChatRow> = (0..500)
            .map(|i| ChatRow {
                id: i,
                title: format!("Chat {i}"),
                subtitle: format!("preview {i}"),
                date: 1_700_000_000,
                unread: 0,
                avatar_path: None,
            })
            .collect();
        state.on_message(UiMessage::Dialogs(dialogs));
        // Scroll deep into the list: the visible-window math must stay in
        // bounds (overscan clamps to [0, n]) at every offset.
        for y in [0.0f32, 17_000.0, 66.0 * 499.0, 1e6] {
            state.on_dialog_scrolled(y);
            let el = std::hint::black_box(dialog_list(&state, 610.0));
            let _ = el;
        }
        // No-virt fallback path (the perf A/B switch) also must not panic.
        state.dialog_scroll_offset = 17_000.0;
        let el = std::hint::black_box(dialog_list(&state, 610.0));
        let _ = el;
    }

    #[test]
    fn group_management_surfaces_render_without_panicking() {
        use crate::bridge::UiMessage;
        use crate::state::{ConfirmKind, CreateKind};

        let (req_tx, _req_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut state = State::new(req_tx);
        state.on_message(UiMessage::Dialogs(vec![ChatRow {
            id: 1,
            title: "Camille".into(),
            subtitle: String::new(),
            date: 0,
            unread: 0,
            avatar_path: None,
        }]));

        // Creation modals (group with member list, channel with description).
        state.open_create(CreateKind::Group);
        let _ = std::hint::black_box(chat_view(&state));
        state.toggle_member(0);
        let _ = std::hint::black_box(chat_view(&state));
        state.open_create(CreateKind::Channel);
        let _ = std::hint::black_box(chat_view(&state));
        state.cancel_create();

        // Header "+" picker.
        state.toggle_create_menu();
        let _ = std::hint::black_box(chat_view(&state));
        state.toggle_create_menu();

        // Chat-row mini menu then both confirmation variants.
        state.open_row_menu(1);
        let _ = std::hint::black_box(chat_view(&state));
        state.ask_confirm(ConfirmKind::Leave, 1);
        let _ = std::hint::black_box(chat_view(&state));
        state.ask_confirm(ConfirmKind::Delete, 1);
        let _ = std::hint::black_box(chat_view(&state));
        state.cancel_confirm();
    }

    #[test]
    fn emoji_panel_renders_without_panicking() {
        let (req_tx, _req_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut state = State::new(req_tx);
        state.authenticated = true;
        state.open_chat = Some(42);
        state.chat_title = "Test".into();
        state.messages = vec![MsgRow::text(1, "hello", 100, false)];
        // Panel open, with and without persisted recents.
        state.toggle_emoji_panel();
        let _ = std::hint::black_box(chat_view(&state));
        state.pick_emoji("🎉".into());
        state.pick_emoji("😀".into());
        let _ = std::hint::black_box(chat_view(&state));
    }

    #[test]
    fn linkify_returns_none_without_url() {
        assert!(linkify("salut, ça va ? rien à voir").is_none());
        assert!(linkify("").is_none());
        // Bare "www." with nothing behind it is not a link.
        assert!(linkify("regarde www. et dis-moi").is_none());
    }

    #[test]
    fn sender_colors_are_deterministic_and_bounded() {
        // Same id → same color; ids cycle through the palette.
        assert_eq!(sender_color(Some(7)), sender_color(Some(7)));
        assert_eq!(sender_color(None), sender_color(Some(0)));
        let distinct: Vec<_> = (0..7).map(|i| sender_color(Some(i))).collect();
        assert_eq!(distinct.len(), 7);
        for c in &distinct {
            assert!(*c != (0, 0, 0));
        }
    }

    #[test]
    fn linkify_extracts_https_url_with_context() {
        let spans = linkify("voici https://example.com/a?b=1 le lien").unwrap();
        let texts: Vec<&str> = spans.iter().map(|s| s.text.as_ref()).collect();
        assert_eq!(texts, ["voici ", "https://example.com/a?b=1", " le lien"]);
        assert_eq!(spans[1].link.as_deref(), Some("https://example.com/a?b=1"));
        assert!(spans[1].underline);
        assert!(spans[1].color.is_some());
    }

    #[test]
    fn linkify_handles_www_and_strips_trailing_punctuation() {
        let spans = linkify("va sur www.rust-lang.org, c'est top.").unwrap();
        let texts: Vec<&str> = spans.iter().map(|s| s.text.as_ref()).collect();
        assert_eq!(texts, ["va sur ", "www.rust-lang.org", ", c'est top."]);
        assert_eq!(spans[1].link.as_deref(), Some("www.rust-lang.org"));
    }

    #[test]
    fn linkify_finds_multiple_urls() {
        let spans = linkify("http://a.io et www.b.io").unwrap();
        let links: Vec<&str> = spans.iter().filter_map(|s| s.link.as_deref()).collect();
        assert_eq!(links, ["http://a.io", "www.b.io"]);
    }
}
