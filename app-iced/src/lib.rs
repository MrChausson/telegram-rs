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
use state::{LoginStep, State};

/// Id of the open chat's message list, used to auto-scroll to the bottom.
const MSG_LIST_ID: &str = "msg-list";

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

type Element<'a> = iced::Element<'a, Message>;

fn rgb(c: (u8, u8, u8)) -> Color {
    Color::from_rgb8(c.0, c.1, c.2)
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
    /// The context menu was dismissed.
    DismissMenu,
    /// Escape: close the context menu, cancel editing or close the viewer.
    Escape,
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
    /// Periodic tick (only useful with `--perf`): samples the frame cadence.
    PerfTick,
}

fn boot() -> (State, Task<Message>) {
    let demo = std::env::args().any(|a| a == "--demo");
    let open_first = std::env::args().any(|a| a == "--open-first");
    let big = std::env::args().any(|a| a == "--demo-big");
    let perf = std::env::args().any(|a| a == "--perf");
    let scroll_perf = std::env::args()
        .find_map(|a| a.strip_prefix("--scroll-perf=").and_then(|v| v.parse::<f32>().ok()))
        .unwrap_or(0.0);
    let req_tx = network::spawn_network(demo, big);
    let state = State::new(req_tx)
        .with_auto_open_first(open_first || demo)
        .with_perf(perf)
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
        Message::DismissMenu => state.dismiss_menu(),
        Message::Escape => {
            if state.viewer.is_some() {
                state.back();
            } else {
                state.escape();
            }
        }
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
        Message::Scrolled(y) => state.on_scrolled(y),
        Message::PerfTick => {
            if state.scroll_perf_dur > 0.0 {
                state.advance_scroll_sim();
            } else {
                state.on_perf_tick();
            }
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

fn view(state: &State) -> Element<'_> {
    if state.viewer.is_some() {
        return viewer_view(state);
    }
    if !state.authenticated {
        return login_view(state);
    }
    chat_view(state)
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
        Some(text(&state.status).size(theme::font::TIMESTAMP as f32).color(rgb(theme::ERROR)))
    } else {
        Some(text(&state.status).size(theme::font::TIMESTAMP as f32).color(rgb(theme::TEXT_SECONDARY)))
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

/// Left pane: "Chats" header + scrollable list.
fn list_pane(state: &State) -> Element<'_> {
    let mut rows = column![];
    for row in &state.dialogs {
        rows = rows.push(chat_row_button(row, state.open_chat == Some(row.id)));
    }
    let list = scrollable(rows).height(Length::Fill).width(Length::Fill);

    column![
        // Header bar: "Chats" + search / compose / dots. `align_y(Center)`
        // keeps the row away from the top edge (the default is top-aligned,
        // which made the left items hug the window border).
        container(
            row![
                text("Chats").size(theme::font::TITLE as f32).color(Color::WHITE),
                horizontal_spacer(),
                icon(Icon::Search, theme::ICON, 20.0),
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

fn chat_row_button(row: &ChatRow, selected: bool) -> Element<'_> {
    let avatar = avatar_circle(row.avatar_path.as_deref(), &row.title, theme::layout::AVATAR_LIST);

    let unread = row.unread > 0;
    // Matching the winit client: names and previews stay on one line (miss of
    // a built-in ellipsis in iced is handled by `ellipsize` + no wrapping).
    let name = text(ellipsize(&row.title, 15))
        .size(theme::font::NAME as f32)
        .color(Color::WHITE)
        .wrapping(iced::widget::text::Wrapping::None)
        .width(Length::Fill);
    let sub = text(ellipsize(&row.subtitle, 24))
        .size(theme::font::MESSAGE as f32)
        .color(rgb(theme::TEXT_SECONDARY))
        .wrapping(iced::widget::text::Wrapping::None)
        .width(Length::Fill);

    let ts: Element<'_> = if row.date > 0 {
        text(theme::fmt_time(row.date))
            .size(theme::font::TIMESTAMP as f32)
            .color(rgb(theme::TEXT_SECONDARY))
            .into()
    } else {
        horizontal_spacer()
    };

    let badge: Element<'_> = if unread {
        container(text(row.unread).size(theme::font::BADGE as f32).color(Color::WHITE))
            .padding([2, 6])
            .style(badge_circle)
            .into()
    } else {
        horizontal_spacer()
    };

    button(
        row![
            avatar,
            column![name, sub].spacing(2).width(Length::Fill),
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

/// Right pane: chat header + messages + composer (+ context menu overlay).
fn conversation_pane(state: &State) -> Element<'_> {
    let open = state.open_chat;
    let chat = state.dialogs.iter().find(|d| Some(d.id) == open);
    let title = if !state.chat_title.is_empty() {
        state.chat_title.clone()
    } else {
        chat.map(|c| c.title.clone()).unwrap_or_default()
    };
    let avatar_path = chat.and_then(|c| c.avatar_path.clone());

    let header = chat_header(
        &title,
        avatar_path.as_deref(),
        state.typing,
        if state.perf_show {
            Some(format!("{} FPS", state.fps()))
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
                .size(theme::font::MESSAGE as f32)
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

    column![
        header,
        container(iced::widget::Space::new())
            .width(Length::Fill)
            .height(1.0)
            .style(divider),
        body,
        composer
    ]
    .width(Length::Fill)
    .height(Length::Fill)
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
        .size(theme::font::NAME as f32)
        .color(Color::WHITE)
        .font(iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT })
        .wrapping(iced::widget::text::Wrapping::None)
        .width(Length::Fill);
    let status: Element<'static> = if title.is_empty() {
        text(" ")
            .size(theme::font::TIMESTAMP as f32)
            .color(rgb(theme::TEXT_SECONDARY))
            .into()
    } else {
        let (label, color) = if typing {
            ("typing…", theme::ACCENT)
        } else {
            ("Chat", theme::TEXT_SECONDARY)
        };
        text(label)
            .size(theme::font::TIMESTAMP as f32)
            .color(rgb(color))
            .into()
    };

    let perf_badge: Element<'static> = match perf {
        Some(fps) => container(
            text(fps)
                .size(theme::font::TIMESTAMP as f32)
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
            icon(Icon::Search, theme::ICON, 20.0),
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

/// Composer bar: rounded field + send (or edit check) button.
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

    container(
        row![
            container(field)
                .width(Length::Fill)
                .height(theme::layout::INPUT_H)
                .style(field_rounded),
            send,
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .height(theme::layout::INPUT_H + 12.0)
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
fn message_row(idx: usize, m: &MsgRow, pane_w: f32) -> Element<'_> {
    // Bubble width (received: 70% of the pane, sent: 60%).
    let bubble_w = if m.out { pane_w * 0.6 } else { pane_w * 0.7 };

    // Bubble content: photo or text.
    let body: Element<'_> = if let Some(path) = &m.photo_path {
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
                text(&m.text).size(theme::font::MESSAGE as f32).color(Color::WHITE).into()
            },
        ]
        .spacing(6)
        .into()
    } else {
        text(&m.text).size(theme::font::MESSAGE as f32).color(Color::WHITE).into()
    };

    // Tail corner smaller, opposite corner small — Telegram style.
    let radius = theme::layout::BUBBLE_RADIUS;
    let corner: f32 = if m.out { 4.0 } else { 4.0 };
    let r = iced::border::Radius::new(radius)
        .top_left(if m.out { radius } else { corner })
        .top_right(if m.out { corner } else { radius })
        .bottom_left(radius)
        .bottom_right(radius);

    let bubble = container(body)
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
    let ts = if m.date > 0 {
        theme::fmt_time(m.date)
    } else {
        String::new()
    };
    let meta: Element<'_> = if m.out {
        let ts_text: Element<'_> = text(ts.clone())
            .size(theme::font::TIMESTAMP as f32)
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
            .size(theme::font::TIMESTAMP as f32)
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
    .on_right_press(if m.out {
        Message::RowContext(idx)
    } else {
        Message::RowClicked(idx)
    });

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

// ---------------------------------------------------------------------------
// Context menu (Modifier / Supprimer)
// ---------------------------------------------------------------------------

/// The right-click context menu (Modifier / Copier / Supprimer), rendered
/// inline right under the message that raised it instead of an absolute
/// overlay: floating a menu at the clicked coordinates isn't available in
/// iced, so anchoring it to the row is the closest faithful behaviour.
fn context_menu_bar() -> Element<'static> {
    let menu_el = container(
        column![
            button(
                text("Modifier").size(theme::font::PLACEHOLDER as f32).color(rgb(theme::TEXT_PRIMARY))
            )
            .on_press(Message::ContextEdit)
            .width(Length::Fill)
            .padding(10)
            .style(menu_item_style),
            button(
                text("Copier").size(theme::font::PLACEHOLDER as f32).color(rgb(theme::TEXT_PRIMARY))
            )
            .on_press(Message::ContextCopy)
            .width(Length::Fill)
            .padding(10)
            .style(menu_item_style),
            button(
                text("Supprimer").size(theme::font::PLACEHOLDER as f32).color(rgb(theme::ERROR))
            )
            .on_press(Message::ContextDelete)
            .width(Length::Fill)
            .padding(10)
            .style(menu_item_style),
        ]
        .spacing(0),
    )
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
/// Estimated height (logical px) of the 3-item context menu.
const CONTEXT_MENU_H: f32 = 3.0 * 37.0;

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

    // Estimated row heights + cumulative top offsets (content coordinates).
    let heights: Vec<f32> = state
        .messages
        .iter()
        .map(|m| est_row_height(m, pane_w))
        .collect();
    let menu_h: Vec<f32> = (0..n)
        .map(|i| {
            if state.context_menu.map(|c| c.row) == Some(i) {
                CONTEXT_MENU_H
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

    // Visible window from the last notified scroll offset.
    let mut start = 0usize;
    let mut end = n;
    if !no_virt {
        let offset = state.scroll_offset.max(0.0);
        while start < n && tops[start] + heights[start] + menu_h[start] < offset {
            start += 1;
        }
        end = start;
        while end < n && tops[end] < offset + view_h {
            end += 1;
        }
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
        cols = cols.push(message_row(i, m, pane_w));
        if state.context_menu.map(|c| c.row) == Some(i) {
            cols = cols.push(context_menu_bar());
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
    let font_h = theme::font::MESSAGE as f32;

    let photo_h = match m.photo {
        Some((w, h)) if w > 0 && h > 0 => inner * (h as f32 / w as f32),
        _ => 0.0,
    };
    let text_h = if m.text.is_empty() {
        0.0
    } else {
        // ~0.52em average advance per char, 1.3 line height, wrapped at `inner`.
        let per_line = (inner / (font_h * 0.52)).floor().max(1.0);
        let lines = (m.text.chars().count() as f32 / per_line).ceil();
        lines * font_h * 1.3
    };
    let caption = if m.photo.is_some() && !m.text.is_empty() {
        6.0
    } else {
        0.0
    };
    2.0 * theme::layout::BUBBLE_PAD_Y + photo_h + caption + text_h
}

// ---------------------------------------------------------------------------
// Widgets helpers
// ---------------------------------------------------------------------------

fn horizontal_spacer() -> Element<'static> {
    iced::widget::Space::new().width(Length::Fill).into()
}

/// Truncates `s` to at most `max` chars, adding an ellipsis when clipped
/// (matches the winit client's single-line chat rows).
fn ellipsize(s: &str, max: usize) -> String {
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
        container(text(ch).size(theme::font::NAME as f32).color(Color::WHITE))
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
        iced::keyboard::Event::KeyPressed { key, .. }
            if key == iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) =>
        {
            Some(Message::Escape)
        }
        _ => None,
    });
    let timer = if state.scroll_perf_dur > 0.0 {
        // 60 Hz synthetic fling for end-to-end scroll-rate measurement.
        iced::time::every(std::time::Duration::from_millis(16)).map(|_| Message::PerfTick)
    } else if state.perf_show {
        iced::time::every(std::time::Duration::from_millis(500)).map(|_| Message::PerfTick)
    } else {
        iced::Subscription::none()
    };
    iced::Subscription::batch([net, keys, timer])
}

/// Application entry point (called from the `main.rs` binary of this crate).
pub fn run() -> iced::Result {
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
            .map(|i| MsgRow {
                id: i as i32,
                text: format!("message {i} with some text that wraps a bit"),
                date: 1_700_000_000 - i as i32,
                out: i % 2 == 0,
                photo: None,
                photo_path: None,
                read: true,
            })
            .collect();

        // The exact function `view()` calls every frame, over a big history.
        let el = std::hint::black_box(messages_list(&state, 820.0, 610.0));
        let _ = el;
        assert_eq!(state.messages.len(), 300);
    }
}
