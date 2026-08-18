//! `app-iced`: the same Telegram client, but with the UI rendered by Iced
//! (tiny-skia software backend) instead of the custom winit renderer.
//!
//! Experimental prototype on `experiment/iced`: proves RAM consumption and
//! testability of an Iced-based build before committing to a migration.

mod bridge;
mod icons;
mod network;
mod state;
mod theme;

use iced::widget::{
    button, column, container, image, mouse_area, row, scrollable, stack, text, text_input,
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
}

fn boot() -> (State, Task<Message>) {
    let demo = std::env::args().any(|a| a == "--demo");
    let open_first = std::env::args().any(|a| a == "--open-first");
    let req_tx = network::spawn_network(demo);
    let state = State::new(req_tx).with_auto_open_first(open_first || demo);
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

    let mut card = column![
        logo,
        text(title).size(20),
        text(subtitle).size(13).color(rgb(theme::TEXT_SECONDARY)),
        text_input(field_placeholder, &state.login_input)
            .on_input(Message::LoginChanged)
            .on_submit(Message::LoginSubmit)
            .width(400)
            .padding(14)
            .style(text_input_style),
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

fn chat_view(state: &State) -> Element<'_> {
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
        // Header bar: "Chats" + search / compose / dots.
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
    // Unread chats: bold name + brighter preview (like Telegram).
    let name = text(&row.title)
        .size(theme::font::NAME as f32)
        .color(Color::WHITE)
        .font(if unread {
            iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT }
        } else {
            iced::Font::DEFAULT
        });
    let sub_color = if unread {
        rgb(theme::TEXT_PRIMARY)
    } else {
        rgb(theme::TEXT_SECONDARY)
    };
    let sub = text(&row.subtitle)
        .size(theme::font::MESSAGE as f32)
        .color(sub_color);

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

    let header = chat_header(&title, avatar_path.as_deref(), state.typing);
    let body: Element<'_> = if state.messages.is_empty() {
        let msg = if state.open_chat.is_some() {
            if state.status.is_empty() {
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
                // A full-height spacer above the messages pins the bubbles to
                // the bottom when the history is shorter than the viewport.
                let mut cols = column![];
                let mut prev_out: Option<bool> = None;
                for (i, m) in state.messages.iter().enumerate() {
                    // Group consecutive messages from the same author tightly.
                    let gap = if prev_out == Some(m.out) { 3.0 } else { 10.0 };
                    if i > 0 {
                        cols = cols.push(iced::widget::Space::new().height(gap));
                    }
                    cols = cols.push(message_row(i, m));
                    prev_out = Some(m.out);
                }
                scrollable(
                    column![
                        iced::widget::Space::new().height(size.height),
                        cols,
                    ]
                )
                .id(MSG_LIST_ID)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
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

    // Context menu overlay (floating over the composer area).
    let menu = context_overlay(state);

    stack![
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
        .height(Length::Fill),
        menu
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// Chat header: back, avatar, name + status, search/info icons.
fn chat_header(title: &str, avatar_path: Option<&str>, typing: bool) -> Element<'static> {
    let name = text(title.to_string())
        .size(theme::font::NAME as f32)
        .color(Color::WHITE)
        .font(iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT });
    let status: Element<'static> = if title.is_empty() {
        text(" ")
            .size(theme::font::TIMESTAMP as f32)
            .color(rgb(theme::TEXT_SECONDARY))
            .into()
    } else if typing {
        text("typing…")
            .size(theme::font::TIMESTAMP as f32)
            .color(rgb(theme::ACCENT))
            .into()
    } else {
        row![
            container(iced::widget::Space::new())
                .width(8)
                .height(8)
                .style(|_| container::Style {
                    background: Some(iced::Background::Color(rgb(theme::ONLINE))),
                    border: iced::Border {
                        radius: 4.0.into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                }),
            text("online")
                .size(theme::font::TIMESTAMP as f32)
                .color(rgb(theme::ONLINE)),
        ]
        .spacing(5)
        .align_y(Alignment::Center)
        .into()
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
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .height(theme::layout::CHAT_HEADER_H)
    .padding([0, 12])
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

fn message_row(idx: usize, m: &MsgRow) -> Element<'_> {
    // Bubble content: photo or text.
    let body: Element<'_> = if let Some(path) = &m.photo_path {
        let photo_el: Element<'_> = image(image::Handle::from_path(path))
            .width(240)
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
        .padding([8, 12])
        .max_width(420)
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

    // Left/right click handling via mouse_area.
    let wrapped = mouse_area(row![bubble, meta].spacing(8).align_y(Alignment::Center))
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

fn context_overlay(state: &State) -> Element<'_> {
    let Some(menu) = &state.context_menu else {
        return container("").into();
    };
    if state.messages.get(menu.row).is_none() {
        return container("").into();
    };
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

    // Fixed, floating over the message area (bottom-right of the chat pane).
    container(menu_el)
        .align_x(Alignment::End)
        .align_y(Alignment::End)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

// ---------------------------------------------------------------------------
// Widgets helpers
// ---------------------------------------------------------------------------

fn horizontal_spacer() -> Element<'static> {
    iced::widget::Space::new().width(Length::Fill).into()
}

fn avatar_circle(photo: Option<&str>, title: &str, size: f32) -> Element<'static> {
    if let Some(path) = photo {
        let handle = image::Handle::from_path(path);
        return container(
            image(handle)
                .width(Length::Fixed(size))
                .height(Length::Fixed(size))
                .content_fit(iced::ContentFit::Cover)
                .border_radius(size / 2.0),
        )
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .into();
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
        background: Some(iced::Background::Color(rgb(theme::CHAT_BG))),
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
fn subscription(_state: &State) -> iced::Subscription<Message> {
    let net = network::network_subscription();
    let keys = iced::keyboard::listen().filter_map(|event| match event {
        iced::keyboard::Event::KeyPressed { key, .. }
            if key == iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) =>
        {
            Some(Message::Escape)
        }
        _ => None,
    });
    iced::Subscription::batch([net, keys])
}

fn main() -> iced::Result {
    iced::application(boot, update, view)
        .subscription(subscription)
        .window_size((1100.0, 700.0))
        .title("tg — Iced prototype")
        .theme(iced::Theme::Dark)
        .run()
}
