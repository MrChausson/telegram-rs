//! `app-iced`: the same Telegram client, but with the UI rendered by Iced
//! (tiny-skia software backend) instead of the custom winit renderer.
//!
//! Experimental prototype on `experiment/iced`: proves RAM consumption and
//! testability of an Iced-based build before committing to a migration.
//!
//! A library target (`app_iced`) is exposed so the `benches/frame.rs`
//! performance harness can drive the exact same view code headlessly.

pub mod bridge;
pub mod icons;
pub mod network;
pub mod state;
pub mod theme;
use std::collections::HashMap;

use iced::widget::{
    button, column, container, image, mouse_area, row, scrollable, text, text_input,
};
use iced::{Alignment, Color, Length, Task};

use bridge::{ChatRow, MsgRow, Request, UiMessage};
use icons::{icon, Icon};
use state::{LoginStep, SearchMode, State};

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
    /// "Modifier" pressed in the context menu.
    ContextEdit,
    /// "Copier" pressed in the context menu.
    ContextCopy,
    /// "Supprimer" pressed in the context menu.
    ContextDelete,
    /// "Répondre" pressed in the context menu.
    ContextReply,
    /// The reply bar's ✕ was pressed (cancel the armed reply).
    CancelReply,
    /// "Transférer" pressed in the context menu (opens the chat picker).
    ContextForward,
    /// A destination chat was picked in the forward overlay.
    ForwardTo(i64),
    /// The attach 📎 button was pressed: open a file dialog.
    AttachFile,
    /// The file dialog returned a path (None = cancelled).
    FilePicked(Option<String>),
    /// The context menu was dismissed.
    DismissMenu,
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
    /// Composer text changed.
    ComposerChanged(String),
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
}

fn boot() -> (State, Task<Message>) {
    let demo = std::env::args().any(|a| a == "--demo");
    let open_first = std::env::args().any(|a| a == "--open-first");
    let big = std::env::args().any(|a| a == "--demo-big");
    let perf = std::env::args().any(|a| a == "--perf");
    let continuous = std::env::args().any(|a| a == "--continuous");
    let scroll_perf = std::env::args()
        .find_map(|a| a.strip_prefix("--scroll-perf=").and_then(|v| v.parse::<f32>().ok()))
        .unwrap_or(0.0);
    let req_tx = network::spawn_network(demo, big);
    let state = State::new(req_tx)
        .with_auto_open_first(open_first || demo)
        .with_perf(perf)
        .with_continuous(continuous)
        .with_scroll_perf(scroll_perf);
    (state, Task::none())
}

fn update(state: &mut State, msg: Message) -> Task<Message> {
    match msg {
        Message::Ui(ui) => {
            // Auto-open the first chat once the dialog list arrives.
            if matches!(ui, UiMessage::Dialogs(_)) {
                state.on_message(ui);
                if state.auto_open_first && state.open_chat.is_none() {
                    if let Some(id) = state.dialogs.first().map(|d| d.id) {
                        state.open_chat(id);
                    }
                }
            } else {
                state.on_message(ui);
            }
        }
        Message::OpenChat(id) => state.open_chat(id),
        Message::BackToChats => {
            state.open_chat = None;
            state.messages.clear();
            state.editing = None;
            state.context_menu = None;
            state.invalidate_layout();
        }
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
        Message::CancelReply => state.reply_target = None,
        Message::ContextForward => state.context_forward(),
        Message::ForwardTo(chat) => {
            state.forward_to(chat);
            return Task::none();
        }
        Message::AttachFile => {
            let task = iced::Task::future(async {
                let picked = rfd::AsyncFileDialog::new()
                    .set_title("Envoyer un fichier")
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
                state.status = format!("Fichier introuvable : {path}");
            }
        }
        Message::FilePicked(None) => {}
        Message::DismissMenu => state.dismiss_menu(),
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
        Message::ComposerChanged(text) => {
            state.composer = text;
            // Notify the server while the user types (best-effort).
            if let Some(id) = state.open_chat {
                let _ = state.req_tx.send(Request::Typing { id, typing: true });
            }
        }
        Message::Submit => {
            if let Some(id) = state.open_chat {
                let _ = state.req_tx.send(Request::Typing { id, typing: false });
            }
            state.submit();
        }
        Message::CloseViewer => state.back(),
        Message::LoginChanged(text) => state.login_input = text,
        Message::LoginSubmit => submit_login(state),
        Message::LoginBack => login_back(state),
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
    }
    // A downloaded document was clicked: hand it to the system opener.
    if let Some(path) = state.open_file.take() {
        let _ = std::process::Command::new("xdg-open")
            .arg(path)
            .spawn();
    }
    // A search jump was armed: scroll the message list to the target.
    {
        use iced::widget::operation::{scroll_to, AbsoluteOffset};
        if let Some(y) = state.take_scroll_target() {
            return scroll_to::<Message>(MSG_LIST_ID, AbsoluteOffset { x: 0.0, y });
        }
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
        while t.front().is_some_and(|x| now.duration_since(*x).as_secs_f32() > 1.0) {
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
    if state.viewer.is_some() {
        return viewer_view(state);
    }
    if !state.authenticated {
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
        Some(SearchMode::Global) => "Recherche dans tous les chats…",
        Some(SearchMode::InChat) => "Rechercher dans ce chat…",
        None => "Recherche…",
    };
    let field = text_input(mode_label, &state.search_query)
        .on_input(Message::SearchChanged)
        .padding(12)
        .style(text_input_style);

    let mut results = column![].spacing(2);
    if state.search_query.trim().is_empty() {
        results = results.push(search_hint("Tapez un mot-clé pour lancer la recherche…"));
    } else if state.search_hits.is_empty() {
        if state.search_pending {
            results = results.push(search_hint("Recherche…"));
        } else {
            results = results.push(search_hint("Aucun résultat"));
        }
    } else {
        let highlighted = &state.search_query;
        for (i, hit) in state.search_hits.iter().enumerate() {
            results = results.push(search_hit_row(hit, highlighted, i));
        }
    }

    let header = container(
        row![
            button(icon(Icon::Back, theme::ICON, 18.0))
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
fn search_hit_row( hit: &bridge::SearchHit, _query: &str, idx: usize) -> Element<'static> {
    let snippet = state::preview_text(&hit.row.text, &hit.row.photo, &hit.row.doc);
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
                    .color(Color::WHITE)
                    .wrapping(iced::widget::text::Wrapping::None),
                text(snippet)
                    .size(theme::font::PLACEHOLDER)
                    .color(rgb(theme::TEXT_SECONDARY))
                    .wrapping(iced::widget::text::Wrapping::None),
            ]
            .spacing(2)
            .width(Length::Fill),
            text(ts)
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::TEXT_SECONDARY)),
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
            .color(rgb(theme::TEXT_SECONDARY)),
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
    let Some(path) = &state.viewer else { return container("").into() };
    let close = button(icon(Icon::Back, theme::ICON, 16.0))
        .on_press(Message::CloseViewer)
        .padding(8)
        .style(flat_button);
    column![
        row![close]
            .padding(8)
            .width(Length::Fill),
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
    .padding(16)
    .into()
}

// ---------------------------------------------------------------------------
// Login screen (mirrors the custom `draw_login`)
// ---------------------------------------------------------------------------

fn login_view(state: &State) -> Element<'_> {
    let s = 1.0f32; // logical pixels (Iced handles HiDPI internally)
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
        Some(text(&state.status).size(theme::font::TIMESTAMP).color(rgb(theme::ERROR)))
    } else {
        Some(text(&state.status).size(theme::font::TIMESTAMP).color(rgb(theme::TEXT_SECONDARY)))
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
            text(subtitle).size(13).color(rgb(theme::TEXT_SECONDARY)),
            input,
        button(
            text(button_label).size(15).color(Color::WHITE)
        )
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
                icon(Icon::Back, theme::ICON, 18.0),
                text("Back").size(13).color(rgb(theme::ICON)),
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
    let _ = s;
    container(
        col.align_x(Alignment::Center).spacing(8).padding(48),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .into()
}

// ---------------------------------------------------------------------------
// Chat view: list pane + conversation pane
// ---------------------------------------------------------------------------

/// Full chat UI (list pane + conversation pane). `pub` so the `benches/`
/// harness can drive the exact per-frame view headlessly.
pub fn chat_view(state: &State) -> Element<'_> {
    row![list_pane(state), conversation_pane(state)].into()
}

/// Estimated dialog-row height (logical px): avatar diameter + the button's
/// vertical padding (`[10, 14]` → 10 per side). All rows share this height, so
/// the virtualized slice is exact for equal offsets — no drift, no height
/// cache needed.
const DIALOG_ROW_H: f32 = theme::layout::AVATAR_LIST + 20.0;

/// Left pane: "Chats" header + scrollable, virtualized, dialog list.
fn list_pane(state: &State) -> Element<'_> {
    // The scrollable only knows its own height at layout time; `responsive`
    // hands it the viewport height used to pick the visible rows.
    let list = iced::widget::responsive(move |size| dialog_list(state, size.height));

    column![
        // Header bar: "Chats" + search / compose / dots. `align_y(Center)`
        // keeps the row away from the top edge (the default is top-aligned,
        // which made the left items hug the window border).
        container(
            row![
                text("Chats").size(theme::font::TITLE).color(Color::WHITE),
                horizontal_spacer(),
                button(icon(Icon::Search, theme::ICON, 20.0))
                    .on_press(Message::OpenGlobalSearch)
                    .padding(6)
                    .style(flat_button),
                icon(Icon::Compose, theme::ICON, 20.0),
                icon(Icon::Dots, theme::ICON, 20.0),
            ]
            .spacing(14)
            .align_y(Alignment::Center)
        )
        .width(Length::Fill)
        .height(theme::layout::LIST_HEADER_H)
        .padding([0, 14])
        .align_y(Alignment::Center)
        .style(list_bg),
        container(list)
            .width(theme::layout::LIST_W)
            .height(Length::Fill)
            .style(list_bg),
    ]
    .width(theme::layout::LIST_W)
    .height(Length::Fill)
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
    for (i, row) in state.dialogs.iter().enumerate().skip(start).take(end - start) {
        // `state.dialog_short` holds the already-ellipsized labels (kept
        // aligned with `dialogs`), so the rows borrow those strings instead of
        // allocating new ones per frame. The mismatch fallback (freshly-built
        // state, unit tests) borrows the full strings until the list arrives.
        let (title, sub) = state
            .dialog_short
            .get(i)
            .map(|(t, s)| (t.as_str(), s.as_str()))
            .unwrap_or((&row.title, &row.subtitle));
        rows = rows.push(chat_row_button(row, state.open_chat == Some(row.id), title, sub));
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

fn chat_row_button<'a>(row: &'a ChatRow, selected: bool, title: &'a str, sub: &'a str) -> Element<'a> {
    let avatar = avatar_circle(row.avatar_path.as_deref(), &row.title, theme::layout::AVATAR_LIST);

    let unread = row.unread > 0;
    // Matching the winit client: names and previews stay on one line (miss of
    // a built-in ellipsis in iced is handled by `ellipsize` + no wrapping).
    // `title`/`sub` are the pre-ellipsized strings from `State::dialog_short`.
    let name = text(title)
        .size(theme::font::NAME)
        .color(Color::WHITE)
        .wrapping(iced::widget::text::Wrapping::None)
        .width(Length::Fill);
    let sub_text = text(sub)
        .size(theme::font::MESSAGE)
        .color(rgb(theme::TEXT_SECONDARY))
        .wrapping(iced::widget::text::Wrapping::None)
        .width(Length::Fill);

    let ts: Element<'_> = if row.date > 0 {
        text(theme::cached_time(row.date))
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::TEXT_SECONDARY))
            .into()
    } else {
        horizontal_spacer()
    };

    let badge: Element<'_> = if unread {
        container(text(row.unread).size(theme::font::BADGE).color(Color::WHITE))
            .padding([2, 6])
            .style(badge_circle)
            .into()
    } else {
        horizontal_spacer()
    };

    button(
        row![
            avatar,
            column![name, sub_text].spacing(2).width(Length::Fill),
            column![ts, badge].spacing(4).align_x(Alignment::End),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .on_press(Message::OpenChat(row.id))
    .width(Length::Fill)
    .padding([10, 14])
    .style(move |theme, status| row_style(theme, status, selected))
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
                .color(rgb(theme::TEXT_SECONDARY)),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
    } else {
        container(
            iced::widget::responsive(move |size| {
                messages_list(state, size.width, size.height)
            })
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(chat_bg)
        .into()
    };

    let composer = composer_bar(state);

    let pane = column![
        header,
        container(iced::widget::Space::new())
            .width(Length::Fill)
            .height(1.0)
            .style(divider),
        body,
        composer
    ]
    .width(Length::Fill)
    .height(Length::Fill);

    if state.forward_pick.is_some() {
        forward_overlay(state, pane.into())
    } else {
        pane.into()
    }
}

/// Full-pane modal listing the chats as forward destinations. Rendered on
/// top of the conversation pane when a "Transférer" is armed; Escape or the
/// header ✕ cancels.
fn forward_overlay<'a>(state: &'a State, under: Element<'a>) -> Element<'a> {
    let mut rows = column![].spacing(2);
    for d in &state.dialogs {
        rows = rows.push(
            button(
                row![
                    avatar_circle(d.avatar_path.as_deref(), &d.title, theme::layout::AVATAR_LIST - 12.0),
                    text(&d.title)
                        .size(theme::font::MESSAGE)
                        .color(Color::WHITE)
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
                icon(Icon::Forward, theme::ACCENT, 16.0),
                text("Transférer vers…").size(theme::font::NAME).color(Color::WHITE),
                horizontal_spacer(),
                button(icon(Icon::Close, theme::ICON, 14.0))
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

    // Dim + center the card over the conversation pane.
    let _ = under;
    container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(|_| container::Style {
            background: Some(iced::Background::Color(Color::from_rgba8(0, 0, 0, 160.0))),
            ..container::Style::default()
        })
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
        .color(Color::WHITE)
        .font(iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT })
        .wrapping(iced::widget::text::Wrapping::None)
        .width(Length::Fill);
    let status: Element<'static> = if title.is_empty() {
        text(" ")
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::TEXT_SECONDARY))
            .into()
    } else {
        let (label, color) = if typing {
            ("typing…", theme::ACCENT)
        } else {
            ("Chat", theme::TEXT_SECONDARY)
        };
        text(label)
            .size(theme::font::TIMESTAMP)
            .color(rgb(color))
            .into()
    };

    let perf_badge: Element<'static> = match perf {
        Some(fps) => container(
            text(fps)
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::ACCENT)),
        )
        .padding([2, 6])
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(rgb(theme::PERF_BADGE_BG))),
            border: iced::Border {
                radius: 6.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
        .into(),
        None => horizontal_spacer(),
    };

    container(
        row![
            button(icon(Icon::Back, theme::ICON, 18.0))
                .on_press(Message::BackToChats)
                .padding(8)
                .style(flat_button),
            avatar_circle(avatar_path, title, theme::layout::AVATAR_CHAT),
            column![name, status].spacing(2),
            horizontal_spacer(),
            button(icon(Icon::Search, theme::ICON, 20.0))
                .on_press(Message::OpenInChatSearch)
                .padding(6)
                .style(flat_button),
            icon(Icon::Info, theme::ICON, 20.0),
            perf_badge,
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .height(theme::layout::CHAT_HEADER_H)
    .padding([0, 12])
    .align_y(Alignment::Center)
    .style(header_bg)
    .into()
}

/// Composer bar: rounded field + attach (or edit check) + send button, with
/// the reply preview bar stacked above when a reply is armed.
fn composer_bar(state: &State) -> Element<'_> {
    let placeholder = if state.editing.is_some() {
        "Modifier le message…"
    } else {
        "Message…"
    };
    let field = text_input(placeholder, &state.composer)
        .on_input(Message::ComposerChanged)
        .on_submit(Message::Submit)
        .padding(12)
        .style(text_input_style);

    let send = if state.editing.is_some() {
        button(icon(Icon::Tick { read: true }, theme::TEXT_PRIMARY, 20.0))
            .on_press(Message::Submit)
            .style(accent_circle_button)
    } else {
        button(icon(Icon::Send, theme::TEXT_PRIMARY, 20.0))
            .on_press(Message::Submit)
            .style(accent_circle_button)
    };

    let bar = row![
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
                    icon(Icon::Reply, theme::ACCENT, 16.0),
                    column![
                        text("Réponse à")
                            .size(theme::font::TIMESTAMP)
                            .color(rgb(theme::ACCENT)),
                        text(&reply.snippet)
                            .size(theme::font::TIMESTAMP)
                            .color(rgb(theme::TEXT_SECONDARY))
                            .wrapping(iced::widget::text::Wrapping::None),
                    ]
                    .spacing(1)
                    .width(Length::Fill),
                    button(icon(Icon::Close, theme::ICON, 14.0))
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
                background: Some(iced::Background::Color(rgb(theme::INPUT_FILL))),
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

    // The attach button sits left of the field (hidden while editing).
    let with_attach: Element<'_> = if state.editing.is_some() {
        col.into()
    } else {
        row![
            button(icon(Icon::Paperclip, theme::ICON, 20.0))
                .on_press(Message::AttachFile)
                .padding(8)
                .style(flat_button),
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

/// A message row: bubble (sent at right, received at left) + timestamp.
/// `pane_w` is the conversation pane width used to size the bubble (received:
/// 70% of the pane, sent: 60%, matching the winit client).
fn message_row<'a>(idx: usize, m: &'a MsgRow, pane_w: f32, state: &'a State) -> Element<'a> {
    // Bubble width (received: 70% of the pane, sent: 60%).
    let bubble_w = if m.out { pane_w * 0.6 } else { pane_w * 0.7 };

    // Quoted header inside the bubble (reply target or forward origin).
    let quote: Option<Element<'a>> = if let Some(reply_id) = m.reply_to {
        let snippet = state
            .messages
            .iter()
            .find(|r| r.id == reply_id)
            .map(|r| crate::state::preview_text(&r.text, &r.photo, &r.doc))
            .unwrap_or_else(|| "Message original".to_string());
        Some(quote_block("Réponse", snippet))
    } else {
        m.forwarded_from
            .as_ref()
            .map(|from| quote_block("Transféré", from.clone()))
    };

    // Bubble content: document card, photo or text.
    let body: Element<'a> = if let Some(doc) = &m.doc {
        let doc_name = if doc.name.is_empty() { "Fichier" } else { doc.name.as_str() };
        let status = if m.uploading.is_some() {
            "Envoi…".to_string()
        } else if m.doc_path.is_some() {
            "Ouvrir".to_string()
        } else {
            "Télécharger".to_string()
        };
        column![
            row![
                icon(Icon::FileDoc, theme::ACCENT, 30.0),
                column![
                    text(doc_name.to_string())
                        .size(theme::font::MESSAGE)
                        .color(Color::WHITE)
                        .wrapping(iced::widget::text::Wrapping::None),
                    text(format!("{} · {}", fmt_size(doc.size), status))
                        .size(theme::font::TIMESTAMP)
                        .color(rgb(theme::TEXT_SECONDARY)),
                ]
                .spacing(2)
                .width(Length::Fill),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            if m.text.is_empty() {
                horizontal_spacer()
            } else {
                text(&m.text).size(theme::font::MESSAGE).color(Color::WHITE).into()
            },
        ]
        .spacing(6)
        .into()
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
                    text(&m.text).size(theme::font::MESSAGE).color(Color::WHITE).into()
                },
            ]
            .spacing(6)
            .into()
        } else {
            placeholder_strip("Chargement de l'image…")
        }
    } else {
        text(&m.text).size(theme::font::MESSAGE).color(Color::WHITE).into()
    };

    // Stack the quote above the media/text body.
    let body_full: Element<'a> = match quote {
        Some(q) => column![q, body].spacing(6).into(),
        None => body,
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
        .padding([
            theme::layout::BUBBLE_PAD_Y,
            theme::layout::BUBBLE_PAD_X,
        ])
        .width(bubble_w)
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(rgb(if m.out {
                theme::BUBBLE_SENT
            } else {
                theme::BUBBLE_RECV
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
            .color(rgb(theme::TEXT_SECONDARY))
            .into();
        let tick: Element<'_> = if m.read {
            icon(Icon::Tick { read: true }, theme::ACCENT, 15.0)
        } else {
            icon(Icon::Tick { read: false }, theme::TEXT_SECONDARY, 15.0)
        };
        row![tick, ts_text].spacing(6).into()
    } else {
        text(ts)
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::TEXT_SECONDARY))
            .into()
    };

    // Left/right click handling via mouse_area. WinIt put the timestamp on
    // the OUTER side of each bubble (left of outgoing, right of incoming).
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

/// The quoted block inside a bubble (reply preview / forward origin): an
/// accent bar + two lines of small text.
fn quote_block(label: &str, content: String) -> Element<'static> {
    row![
        container(iced::widget::Space::new())
            .width(3.0)
            .height(28.0)
            .style(|_| container::Style {
                background: Some(iced::Background::Color(rgb(theme::ACCENT))),
                border: iced::Border {
                    radius: 2.0.into(),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            }),
        column![
            text(label.to_string())
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::ACCENT)),
            text(content)
                .size(theme::font::TIMESTAMP)
                .color(rgb(theme::TEXT_SECONDARY))
                .wrapping(iced::widget::text::Wrapping::None),
        ]
        .spacing(1),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

/// Thin horizontal upload-progress bar with a percentage label.
fn uploading_bar(p: f32) -> Element<'static> {
    let pct = (p.clamp(0.0, 1.0) * 100.0).round();
    column![
        text(format!("Envoi… {} %", pct))
            .size(theme::font::TIMESTAMP)
            .color(rgb(theme::TEXT_SECONDARY)),
        container(
            container(iced::widget::Space::new())
                .width(Length::FillPortion(pct.max(1.0) as u16))
                .height(Length::Fill)
                .style(|_| container::Style {
                    background: Some(iced::Background::Color(rgb(theme::ACCENT))),
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
            background: Some(iced::Background::Color(rgb(theme::DIVIDER))),
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
            .color(rgb(theme::TEXT_SECONDARY)),
    )
    .width(Length::Fill)
    .padding(24)
    .align_x(Alignment::Center)
    .style(|_| container::Style {
        background: Some(iced::Background::Color(rgb(theme::INPUT_FILL))),
        border: iced::Border {
            radius: 12.0.into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// Human-readable byte size ("1.5 Mo" style).
fn fmt_size(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * KB;
    const GB: f64 = MB * KB;
    let b = bytes.max(0) as f64;
    if b >= GB {
        format!("{:.1} Go", b / GB)
    } else if b >= MB {
        format!("{:.1} Mo", b / MB)
    } else if b >= KB {
        format!("{:.0} Ko", b / KB)
    } else {
        format!("{bytes} o")
    }
}

// ---------------------------------------------------------------------------
// Context menu (Répondre / Transférer / Modifier / Copier / Supprimer)
// ---------------------------------------------------------------------------

/// Height of one context-menu item (10 px padding per side + ~17 px text).
const CONTEXT_ITEM_H: f32 = 37.0;

/// The right-click context menu, rendered inline right under the message
/// that raised it instead of an absolute overlay: floating a menu at the
/// clicked coordinates isn't available in iced, so anchoring it to the row
/// is the closest faithful behaviour. Reply/forward apply to any message;
/// edit stays restricted to the user's own text messages.
fn context_menu_bar(state: &State) -> Element<'static> {
    let can_edit = state.context_can_edit();
    let mut items = column![].spacing(0);
    items = items.push(
        button(
            row![
                icon(Icon::Reply, theme::ICON, 15.0),
                text("Répondre").size(theme::font::PLACEHOLDER).color(rgb(theme::TEXT_PRIMARY)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .on_press(Message::ContextReply)
        .width(Length::Fill)
        .padding(10)
        .style(menu_item_style),
    );
    items = items.push(
        button(
            row![
                icon(Icon::Forward, theme::ICON, 15.0),
                text("Transférer").size(theme::font::PLACEHOLDER).color(rgb(theme::TEXT_PRIMARY)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .on_press(Message::ContextForward)
        .width(Length::Fill)
        .padding(10)
        .style(menu_item_style),
    );
    if can_edit {
        items = items.push(
            button(
                text("Modifier").size(theme::font::PLACEHOLDER).color(rgb(theme::TEXT_PRIMARY)),
            )
            .on_press(Message::ContextEdit)
            .width(Length::Fill)
            .padding(10)
            .style(menu_item_style),
        );
    }
    if state
        .context_menu
        .and_then(|c| state.messages.get(c.row))
        .is_some_and(|m| !m.text.is_empty())
    {
        items = items.push(
            button(
                text("Copier").size(theme::font::PLACEHOLDER).color(rgb(theme::TEXT_PRIMARY)),
            )
            .on_press(Message::ContextCopy)
            .width(Length::Fill)
            .padding(10)
            .style(menu_item_style),
        );
    }
    if can_edit {
        items = items.push(
            button(
                text("Supprimer").size(theme::font::PLACEHOLDER).color(rgb(theme::ERROR)),
            )
            .on_press(Message::ContextDelete)
            .width(Length::Fill)
            .padding(10)
            .style(menu_item_style),
        );
    }

    let menu_el = container(items)
        .width(theme::layout::CONTEXT_W)
        .style(menu_bg);

    // Right-aligned under the message (editable messages are ours → right).
    row![horizontal_spacer(), menu_el]
        .padding([0.0, theme::layout::MSG_PAD_X])
        .into()
}

// ---------------------------------------------------------------------------
// Virtualized message list
// ---------------------------------------------------------------------------

/// Rows kept above/below the visible window so estimated-height drift (and a
/// one-frame-late `on_scroll` offset) never reveals a blank gap.
const LIST_OVERSCAN: usize = 16;

/// Number of items the open context menu shows (drives its cached height).
fn context_menu_items(state: &State) -> usize {
    let can_edit = state.context_can_edit();
    let has_text = state
        .context_menu
        .and_then(|c| state.messages.get(c.row))
        .is_some_and(|m| !m.text.is_empty());
    2 + usize::from(can_edit) + usize::from(has_text) + usize::from(can_edit)
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
        && cache_guard
            .as_ref()
            .is_some_and(|c| c.pane_w == pane_w && c.view_h == view_h && c.epoch == state.layout_epoch);
    if !hit {
        *cache_guard = Some(build_layout(state, pane_w, view_h));
    }
    let cache = cache_guard.as_ref().expect("rebuilt in the step above");
    let heights = &cache.heights;
    let menu_h = &cache.menu_h;
    let tops = &cache.tops;
    let total = cache.total;

    // Visible window from the last notified scroll offset.
    let mut start = 0usize;
    let mut end = n;
    if !no_virt {
        let offset = state.scroll_offset.max(0.0);
        // `tops` grows monotonically, so does `tops[i] + heights[i] + menu_h[i]`
        // (`tops[i+1] = tops[i] + heights[i] + menu_h[i] + gap`): binary-search
        // the first row whose bottom edge reaches the viewport instead of a
        // linear scan from 0 (O(all rows) per frame on big chats).
        let bottom_edge = |i: usize| tops[i] + heights[i] + menu_h[i];
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
            cols =
                cols.push(iced::widget::Space::new().height(gap_between(prev_out.unwrap_or(m.out), m.out)));
        }
        cols = cols.push(message_row(i, m, pane_w, state));
        if state.context_menu.map(|c| c.row) == Some(i) {
            cols = cols.push(context_menu_bar(state));
        }
        prev_out = Some(m.out);
    }

    scrollable(column![
        iced::widget::Space::new().height(top_pad),
        cols,
        iced::widget::Space::new().height(bottom_pad),
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
/// or context-menu change, or a pane resize).
fn build_layout(state: &State, pane_w: f32, view_h: f32) -> crate::state::MsgLayoutCache {
    let n = state.messages.len();
    let out_at = |i: usize| state.messages[i].out;
    let gap_between = |a: bool, b: bool| if a == b { 3.0 } else { 10.0 };

    let heights: Vec<f32> = state
        .messages
        .iter()
        .map(|m| est_row_height(m, pane_w))
        .collect();
    let menu_row = state.context_menu.map(|c| c.row);
    let menu_h_total = CONTEXT_ITEM_H * context_menu_items(state) as f32;
    let menu_h: Vec<f32> = (0..n)
        .map(|i| {
            if menu_row == Some(i) {
                menu_h_total
            } else {
                0.0
            }
        })
        .collect();

    let mut tops = Vec::with_capacity(n);
    let mut y = view_h; // pin spacer keeps bubbles at the bottom when short.
    for i in 0..n {
        tops.push(y);
        y += heights[i] + menu_h[i];
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
        menu_h,
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
    let bubble_w = if m.out { pane_w * 0.6 } else { pane_w * 0.7 };
    let inner = (bubble_w - 2.0 * theme::layout::BUBBLE_PAD_X).max(1.0);
    let font_h = theme::font::MESSAGE;

    // Quoted header (reply / forward): accent bar + 2 small lines + gap.
    let quote_h = if m.reply_to.is_some() || m.forwarded_from.is_some() {
        2.0 * theme::font::TIMESTAMP * 1.3 + 6.0
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
    2.0 * theme::layout::BUBBLE_PAD_Y + quote_h + body_gap + doc_h + photo_h + caption + text_h
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
    let ch = title.chars().next().unwrap_or('?').to_string().to_uppercase();
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
        background: Some(iced::Background::Color(rgb(theme::LIST_BG))),
        ..container::Style::default()
    }
}

fn chat_bg(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(rgb(theme::CHAT_BG))),
        ..container::Style::default()
    }
}

fn divider(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(rgb(theme::DIVIDER))),
        ..container::Style::default()
    }
}

fn header_bg(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(rgb(theme::LIST_BG))),
        ..container::Style::default()
    }
}

fn row_style(
    _theme: &iced::Theme,
    status: iced::widget::button::Status,
    selected: bool,
) -> iced::widget::button::Style {
    let bg = if selected {
        theme::ROW_SELECTED
    } else {
        match status {
            iced::widget::button::Status::Hovered => theme::ROW_HOVER,
            _ => theme::LIST_BG,
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

fn flat_button(_theme: &iced::Theme, _status: iced::widget::button::Status) -> iced::widget::button::Style {
    iced::widget::button::Style {
        background: None,
        ..iced::widget::button::Style::default()
    }
}

fn accent_circle_button(
    _theme: &iced::Theme,
    _status: iced::widget::button::Status,
) -> iced::widget::button::Style {
    iced::widget::button::Style {
        background: Some(iced::Background::Color(rgb(theme::ACCENT))),
        border: iced::Border {
            radius: 17.0.into(),
            ..iced::Border::default()
        },
        ..iced::widget::button::Style::default()
    }
}

fn accent_button(_theme: &iced::Theme, _status: iced::widget::button::Status) -> iced::widget::button::Style {
    iced::widget::button::Style {
        background: Some(iced::Background::Color(rgb(theme::ACCENT))),
        border: iced::Border {
            radius: 14.0.into(),
            ..iced::Border::default()
        },
        ..iced::widget::button::Style::default()
    }
}

fn menu_item_style(
    _theme: &iced::Theme,
    _status: iced::widget::button::Status,
) -> iced::widget::button::Style {
    iced::widget::button::Style {
        background: None,
        ..iced::widget::button::Style::default()
    }
}

fn menu_bg(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(rgb((30, 42, 58)))),
        border: iced::Border {
            radius: 8.0.into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    }
}

fn badge_circle(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(rgb(theme::ACCENT))),
        border: iced::Border {
            radius: 10.0.into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    }
}

fn accent_circle(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(rgb(theme::ACCENT))),
        border: iced::Border {
            radius: 38.0.into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    }
}

fn field_rounded(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(rgb(theme::INPUT_FILL))),
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
        background: iced::Background::Color(rgb(theme::INPUT_FILL)),
        border: iced::Border {
            radius: theme::layout::INPUT_RADIUS.into(),
            width: 1.0,
            color: rgb(theme::INPUT_BORDER),
        },
        icon: rgb(theme::TEXT_SECONDARY),
        placeholder: rgb(theme::TEXT_SECONDARY),
        value: rgb(theme::TEXT_PRIMARY),
        selection: rgb(theme::ACCENT),
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
    iced::Subscription::batch([net, keys, timer])
}

/// Application entry point (called from the `main.rs` binary of this crate).
pub fn run() -> iced::Result {
    // Default the wgpu backend to GL (EGL): measured on the reference
    // machine (NVIDIA, Wayland) it costs ~47 MB PSS vs ~115 MB for Vulkan —
    // the proprietary Vulkan stack alone accounts for ~68 MB of resident
    // driver pages — at identical CPU cost and scroll throughput. Machines
    // without a usable EGL/GL stack fall through to the tiny-skia software
    // renderer compiled in. `WGPU_BACKEND` still wins when set (e.g.
    // `WGPU_BACKEND=vulkan` restores the previous behaviour).
    if std::env::var_os("WGPU_BACKEND").is_none() {
        std::env::set_var("WGPU_BACKEND", "gl");
    }
    let (w, h) = window_size_from_args();
    iced::application(boot, update, view)
        .subscription(subscription)
        .window_size((w, h))
        .title("tg — Iced prototype")
        .theme(iced::Theme::Dark)
        .run()
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
    fn message_list_virtualizes_without_panicking() {
        let (req_tx, _req_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut state = State::new(req_tx);
        state.authenticated = true;
        state.open_chat = Some(42);
        state.chat_title = "Test".into();
        state.messages = (0..300)
            .map(|i| {
                MsgRow {
                    read: true,
                    ..MsgRow::text(
                        i,
                        format!("message {i} with some text that wraps a bit"),
                        1_700_000_000 - i,
                        i % 2 == 0,
                    )
                }
            })
            .collect();

        // The exact function `view()` calls every frame, over a big history.
        let el = std::hint::black_box(messages_list(&state, 820.0, 610.0));
        let _ = el;
        assert_eq!(state.messages.len(), 300);
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
}
