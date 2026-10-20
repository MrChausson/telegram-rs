//! Window lifetime is separate from the Telegram session.
//!
//! Closing and recreating the native window also works on Wayland, where
//! winit's `set_visible(false)` (and thus `Mode::Hidden`) is unsupported.

use iced::{window, Element, Point, Size, Subscription, Task};

use crate::{state::State, tray};

// Keep the existing application message inline: boxing it would add a heap
// allocation to every network/UI event just to wrap it in the window shell.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
enum Message {
    App(crate::Message),
    Tray(tray::Event),
    Window(window::Id, window::Event),
    Opened(window::Id),
}

#[derive(Debug, PartialEq)]
enum Action {
    None,
    Open,
    Focus(window::Id),
    Close(window::Id),
    Exit,
}

/// Pure lifecycle decisions, independently testable without a display or D-Bus.
#[derive(Default)]
struct Lifecycle {
    window: Option<window::Id>,
    tray_online: bool,
}

impl Lifecycle {
    fn show(&self) -> Action {
        self.window.map_or(Action::Open, Action::Focus)
    }

    fn tray_event(&mut self, event: tray::Event) -> Action {
        match event {
            tray::Event::Open => self.show(),
            tray::Event::Quit => Action::Exit,
            tray::Event::Available(online) => {
                self.tray_online = online;
                // Never leave the user with neither a window nor a tray icon.
                if !online && self.window.is_none() {
                    Action::Open
                } else {
                    Action::None
                }
            }
        }
    }

    fn close_requested(&mut self, id: window::Id) -> Action {
        if self.window != Some(id) {
            return Action::None;
        }
        if self.tray_online {
            self.window = None;
            Action::Close(id)
        } else {
            Action::Exit
        }
    }

    fn closed(&mut self, id: window::Id) -> Action {
        // A delayed Closed event from the old window must not forget a newly
        // restored one (e.g. Open clicked immediately after the close button).
        if self.window != Some(id) {
            return Action::None;
        }
        self.window = None;
        if self.tray_online {
            Action::None
        } else {
            Action::Exit
        }
    }
}

struct Shell {
    app: State,
    lifecycle: Lifecycle,
    size: Size,
    position: Option<Point>,
}

impl Shell {
    fn boot() -> (Self, Task<Message>) {
        let (app, task) = crate::boot();
        let (width, height) = crate::window_size_from_args();
        let mut shell = Self {
            app,
            lifecycle: Lifecycle::default(),
            size: Size::new(width, height),
            position: None,
        };
        let open = shell.perform(Action::Open);
        (shell, Task::batch([task.map(Message::App), open]))
    }

    fn perform(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::None => Task::none(),
            Action::Exit => iced::exit(),
            Action::Close(id) => window::close(id),
            Action::Focus(id) => window::minimize(id, false).chain(window::gain_focus(id)),
            Action::Open => {
                let (id, open) = window::open(window::Settings {
                    size: self.size,
                    position: self
                        .position
                        .map_or(window::Position::Default, window::Position::Specific),
                    icon: Some(crate::window_icon()),
                    min_size: Some(Size::new(420.0, 320.0)),
                    exit_on_close_request: false,
                    ..Default::default()
                });
                // Reserve the ID immediately, not after the async open task:
                // repeated tray clicks must never create duplicate windows.
                self.lifecycle.window = Some(id);
                open.map(Message::Opened)
            }
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let action = match message {
            Message::App(message) => {
                return crate::update(&mut self.app, message).map(Message::App);
            }
            Message::Tray(event) => self.lifecycle.tray_event(event),
            Message::Opened(id) if self.lifecycle.window == Some(id) => {
                // The widget tree is new, but the session, drafts and chat state
                // survived. Restore the remembered message-list position too.
                use iced::widget::operation::{scroll_to, AbsoluteOffset};
                return Task::batch([
                    window::gain_focus(id),
                    scroll_to(
                        crate::MSG_LIST_ID,
                        AbsoluteOffset {
                            x: 0.0,
                            y: self.app.scroll_offset,
                        },
                    ),
                ]);
            }
            Message::Window(id, window::Event::CloseRequested) => {
                self.lifecycle.close_requested(id)
            }
            Message::Window(id, window::Event::Closed) => self.lifecycle.closed(id),
            Message::Window(id, event) if self.lifecycle.window == Some(id) => {
                match event {
                    window::Event::Opened { size, position } => {
                        self.remember_size(size);
                        self.position = position;
                    }
                    window::Event::Resized(size) => self.remember_size(size),
                    window::Event::Moved(position) => self.position = Some(position),
                    _ => {}
                }
                Action::None
            }
            _ => Action::None,
        };
        self.perform(action)
    }

    fn remember_size(&mut self, size: Size) {
        // Some backends report zero dimensions while minimizing a window.
        if size.width > 0.0 && size.height > 0.0 {
            self.size = size;
        }
    }

    fn view(&self, _id: window::Id) -> Element<'_, Message> {
        crate::view(&self.app).map(Message::App)
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            crate::subscription(&self.app).map(Message::App),
            tray::subscription().map(Message::Tray),
            window::events().filter_map(|(id, event)| match event {
                window::Event::Opened { .. }
                | window::Event::Resized(_)
                | window::Event::Moved(_)
                | window::Event::CloseRequested
                | window::Event::Closed => Some(Message::Window(id, event)),
                _ => None,
            }),
        ])
    }
}

pub(super) fn run() -> iced::Result {
    // Unlike application(), daemon() keeps the network subscription and app
    // state alive with no native windows. Quit explicitly exits the daemon.
    iced::daemon(Shell::boot, Shell::update, Shell::view)
        .subscription(Shell::subscription)
        .title("Telegram RS")
        .theme(|shell: &Shell, _id| crate::app_theme(&shell.app))
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible(online: bool) -> (Lifecycle, window::Id) {
        let id = window::Id::unique();
        (
            Lifecycle {
                window: Some(id),
                tray_online: online,
            },
            id,
        )
    }

    #[test]
    fn close_with_tray_keeps_app_alive_without_a_window() {
        let (mut state, id) = visible(true);
        assert_eq!(state.close_requested(id), Action::Close(id));
        assert_eq!(state.window, None);
        assert_eq!(state.closed(id), Action::None);
    }

    #[test]
    fn close_without_tray_exits_instead_of_becoming_inaccessible() {
        let (mut state, id) = visible(false);
        assert_eq!(state.close_requested(id), Action::Exit);
    }

    #[test]
    fn open_restores_hidden_window_and_focuses_visible_window() {
        let (mut state, id) = visible(true);
        assert_eq!(state.tray_event(tray::Event::Open), Action::Focus(id));
        state.close_requested(id);
        assert_eq!(state.tray_event(tray::Event::Open), Action::Open);
    }

    #[test]
    fn repeated_open_does_not_create_duplicate_windows() {
        let mut state = Lifecycle::default();
        assert_eq!(state.tray_event(tray::Event::Open), Action::Open);
        let id = window::Id::unique();
        // perform(Open) reserves the ID before its task runs.
        state.window = Some(id);
        assert_eq!(state.tray_event(tray::Event::Open), Action::Focus(id));
    }

    #[test]
    fn late_close_of_old_window_cannot_clear_restored_window() {
        let (mut state, old) = visible(true);
        state.close_requested(old);
        let new = window::Id::unique();
        state.window = Some(new);
        assert_eq!(state.closed(old), Action::None);
        assert_eq!(state.close_requested(old), Action::None);
        assert_eq!(state.window, Some(new));
    }

    #[test]
    fn losing_tray_while_hidden_restores_window() {
        let (mut state, id) = visible(true);
        state.close_requested(id);
        assert_eq!(
            state.tray_event(tray::Event::Available(false)),
            Action::Open
        );
        assert!(!state.tray_online);
    }

    #[test]
    fn losing_tray_while_visible_does_not_open_another_window() {
        let (mut state, id) = visible(true);
        assert_eq!(
            state.tray_event(tray::Event::Available(false)),
            Action::None
        );
        assert_eq!(state.window, Some(id));
        assert_eq!(state.close_requested(id), Action::Exit);
    }

    #[test]
    fn tray_reconnecting_reenables_close_to_tray() {
        let (mut state, id) = visible(false);
        assert_eq!(state.tray_event(tray::Event::Available(true)), Action::None);
        assert_eq!(state.close_requested(id), Action::Close(id));
    }

    #[test]
    fn quit_exits_even_with_no_window() {
        let mut state = Lifecycle::default();
        assert_eq!(state.tray_event(tray::Event::Quit), Action::Exit);
    }

    #[test]
    fn unexpected_last_window_close_without_tray_exits() {
        let (mut state, id) = visible(false);
        assert_eq!(state.closed(id), Action::Exit);
    }
}
