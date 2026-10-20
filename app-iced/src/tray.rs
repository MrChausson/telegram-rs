//! Event-driven StatusNotifier tray (Linux/X11/Wayland; no GTK).
//!
//! The subscription wakes Iced directly, even when no window is open. There
//! are no polling timers, process-wide flags, or extra tray runtime threads.

use iced::futures::SinkExt;
use ksni::TrayMethods;
use std::sync::OnceLock;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Open,
    Quit,
    Available(bool),
}

/// The app logo as `ksni::Icon`s (32 px + 64 px), rendered once.
///
/// `ksni::Icon` wants ARGB32 in network byte order; our pixmaps are
/// little-endian RGBA8, so each pixel is rotated right by one byte
/// (rgba → argb). Rendering at 2× keeps the mark crisp on HiDPI panels.
fn tray_icons() -> Vec<ksni::Icon> {
    static ICONS: OnceLock<Vec<ksni::Icon>> = OnceLock::new();
    ICONS
        .get_or_init(|| {
            [32, 64]
                .into_iter()
                .map(|px| {
                    let pixmap = crate::icons::render_logo_rgba(px);
                    let mut data = pixmap.data().to_vec();
                    for px in data.as_chunks_mut::<4>().0 {
                        px.rotate_right(1);
                    }
                    ksni::Icon {
                        width: px as i32,
                        height: px as i32,
                        data,
                    }
                })
                .collect()
        })
        .clone()
}

struct TgTray {
    events: mpsc::UnboundedSender<Event>,
}

impl TgTray {
    fn send(&self, event: Event) {
        let _ = self.events.send(event);
    }
}

impl ksni::Tray for TgTray {
    fn id(&self) -> String {
        "telegram-rs".into()
    }
    fn title(&self) -> String {
        "Telegram RS".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        tray_icons()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Telegram RS".into(),
            description: "Telegram client".into(),
            ..Default::default()
        }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Event::Open);
    }
    fn watcher_online(&self) {
        self.send(Event::Available(true));
    }
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        self.send(Event::Available(false));
        // Keep the service alive so a panel restart can register it again.
        true
    }
    fn menu(&self) -> Vec<ksni::menu::MenuItem<Self>> {
        use ksni::menu::{MenuItem, StandardItem};
        vec![
            MenuItem::Standard(StandardItem {
                label: "Open Telegram RS".into(),
                activate: Box::new(|tray: &mut Self| tray.send(Event::Open)),
                ..Default::default()
            }),
            MenuItem::Standard(StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut Self| tray.send(Event::Quit)),
                ..Default::default()
            }),
        ]
    }
}

pub fn subscription() -> iced::Subscription<Event> {
    iced::Subscription::run(events)
}

fn events() -> impl iced::futures::Stream<Item = Event> {
    iced::stream::channel(16, async move |mut output| {
        let (tx, mut rx) = mpsc::unbounded_channel();
        // Queue readiness before spawning, but do not forward it until spawn
        // succeeds. A racing watcher_offline callback then follows readiness
        // in the same FIFO instead of being overwritten by stale readiness.
        let _ = tx.send(Event::Available(true));
        let tray = TgTray { events: tx };
        let Ok(handle) = tray.spawn().await else {
            let _ = output.send(Event::Available(false)).await;
            // Iced subscriptions must not end spontaneously. Close will exit
            // normally without a tray, and no retry/polling timer is needed.
            iced::futures::future::pending::<()>().await;
            return;
        };
        // Keep the service handle for the subscription's entire lifetime.
        while let Some(event) = rx.recv().await {
            if output.send(event).await.is_err() {
                break;
            }
        }
        handle.shutdown().await;
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ksni::Tray;

    fn tray() -> (TgTray, mpsc::UnboundedReceiver<Event>) {
        let (events, rx) = mpsc::unbounded_channel();
        (TgTray { events }, rx)
    }

    #[test]
    fn icon_activation_sends_open_without_an_unrelated_ui_event() {
        let (mut tray, mut rx) = tray();
        tray.activate(0, 0);
        assert_eq!(rx.try_recv().unwrap(), Event::Open);
    }

    #[test]
    fn menu_actions_send_open_and_quit_in_order() {
        let (mut tray, mut rx) = tray();
        for item in tray.menu() {
            if let ksni::menu::MenuItem::Standard(item) = item {
                (item.activate)(&mut tray);
            }
        }
        assert_eq!(rx.try_recv().unwrap(), Event::Open);
        assert_eq!(rx.try_recv().unwrap(), Event::Quit);
    }

    #[test]
    fn watcher_changes_are_delivered_and_allow_reconnection() {
        let (tray, mut rx) = tray();
        assert!(tray.watcher_offline(ksni::OfflineReason::No));
        tray.watcher_online();
        assert_eq!(rx.try_recv().unwrap(), Event::Available(false));
        assert_eq!(rx.try_recv().unwrap(), Event::Available(true));
    }

    #[tokio::test]
    async fn channel_wakes_a_sleeping_receiver() {
        let (tray, mut rx) = tray();
        let send = tokio::spawn(async move { tray.send(Event::Open) });
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("tray event should wake the receiver");
        assert_eq!(event, Some(Event::Open));
        send.await.unwrap();
    }
}
