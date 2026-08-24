//! App tray (StatusNotifier on Linux/X11/Wayland; no GTK).
//!
//! `ksni` is pure Rust. The tray writes two flags the shell polls:
//! "open" and "quit". On machines without a StatusNotifier host, ksni fails
//! quietly on its thread — always a no-op, never a crash.

use ksni::TrayMethods;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;

/// Flags the iced shell reacts to.
#[derive(Debug, Default)]
pub struct TrayActions {
    pub open: AtomicBool,
    pub quit: AtomicBool,
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
    actions: Arc<TrayActions>,
}

impl ksni::Tray for TgTray {
    fn id(&self) -> String {
        "tg".into()
    }
    fn title(&self) -> String {
        "tg".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        tray_icons()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "tg".into(),
            description: "Telegram client".into(),
            ..Default::default()
        }
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