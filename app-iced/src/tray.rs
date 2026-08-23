//! App tray (StatusNotifier on Linux/X11/Wayland; no GTK).
//!
//! `ksni` is pure Rust. The tray writes two flags the shell polls:
//! "open" and "quit". On machines without a StatusNotifier host, ksni fails
//! quietly on its thread — always a no-op, never a crash.

use ksni::TrayMethods;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Flags the iced shell reacts to.
#[derive(Debug, Default)]
pub struct TrayActions {
    pub open: AtomicBool,
    pub quit: AtomicBool,
}

struct TgTray {
    actions: Arc<TrayActions>,
}

impl ksni::Tray for TgTray {
    fn id(&self) -> String {
        "tg".into()
    }
    fn title(&self) -> String {
        "tg".into()
    }
    fn icon_name(&self) -> String {
        "telegram".into()
    }
    fn menu(&self) -> Vec<ksni::menu::MenuItem<Self>> {
        use ksni::menu::{MenuItem, StandardItem};
        vec![
            MenuItem::Standard(StandardItem {
                label: "Open tg".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.actions.open.store(true, Ordering::SeqCst);
                }),
                ..Default::default()
            }),
            MenuItem::Standard(StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.actions.quit.store(true, Ordering::SeqCst);
                }),
                ..Default::default()
            }),
        ]
    }
}

/// Start the tray. Spawns an inner current-thread tokio runtime so the async
/// `spawn()` future can be polled; a missing StatusNotifier host makes `spawn`
/// return `Err`, and the thread silently exits.
pub fn start(actions: Arc<TrayActions>) {
    std::thread::Builder::new()
        .name("tg-tray".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tray tokio rt");
            rt.block_on(async move {
                let tray = TgTray { actions };
                let Ok(handle) = tray.spawn().await else {
                    return;
                };
                // Keep `handle` (and the tray) alive for this thread's lifetime.
                let _ = handle;
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                }
            });
        })
        .ok();
}