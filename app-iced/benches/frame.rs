//! Frame-cost regression harness for `app_iced`.
//!
//! Drives the *exact* view the app builds every frame — `app_iced::chat_view`
//! (full window) and its hot spot `app_iced::messages_list` (virtualized
//! message list) — and measures:
//!
//! * **build** — [`Element`] construction cost (`view()` re-creates it per frame);
//! * **layout** — full [`Widget::layout`] pass over a warm, diffed tree;
//! * **frame** — the complete per-frame cycle (diff + layout + draw against a
//!   tiny-skia software canvas, matching the real `iced` run).
//!
//! The point of the `messages/{n}` cases: before virtualization every scroll
//! tick rebuilt and re-shaped the text of *all* N rows; the cost should now be
//! ~flat in N (it depends on the visible window only). Run with:
//!
//! ```text
//! cargo bench -p app-iced -- --warm-up-time 2 --measurement-time 5
//! ```

use app_iced::bridge::{ChatRow, MsgRow, UiMessage};
use app_iced::state::State;
use app_iced::{chat_view, dialog_list, messages_list};

use iced_core::layout::{Layout, Limits};
use iced_core::mouse::Cursor;
use iced_core::renderer::Style;
use iced_core::theme::Theme;
use iced_core::widget::{Tree, Widget};
use iced_core::{Font, Pixels, Point, Rectangle, Size};

use criterion::{black_box, criterion_group, criterion_main, Criterion};

const W: f32 = 820.0;
const H: f32 = 610.0;

fn demo_state(n: usize) -> State {
    let (req_tx, _req_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut state = State::new(req_tx);
    state.authenticated = true;
    state.open_chat = Some(42);
    state.chat_title = "Bench".into();
    let now = 1_700_000_000i32;
    let photo = demo_photo();
    let photo = Some(photo.to_string_lossy().into_owned());
    state.messages = (0..n)
        .map(|i| MsgRow {
            id: i as i32,
            text: if i % 5 == 0 {
                "Long message that wraps, with a few emojis and a photo link 🖼️ for realism".into()
            } else {
                format!("Message number {i}: the quick brown fox jumps over the lazy dog – padding")
            },
            date: now - i as i32,
            out: i % 2 == 0,
            photo: if i % 5 == 0 { Some((640, 480)) } else { None },
            photo_path: if i % 5 == 0 { photo.clone() } else { None },
            read: i % 2 == 0,
        })
        .collect();
    // A realistic dialog list so the left pane (list_pane) is exercised:
    // `chat_view` builds every dialog row each frame. Going through
    // `on_message` also runs the same cache bookkeeping the real app does.
    let dialogs: Vec<ChatRow> = (0..50)
        .map(|i| ChatRow {
            id: i,
            title: format!("Chat number {i}, a fairly long name that will be ellipsized"),
            subtitle: format!("Last message preview for chat {i}, also fairly long to wrap"),
            date: now - i as i32,
            unread: (i % 5) as i32,
            avatar_path: None,
        })
        .collect();
    state.on_message(UiMessage::Dialogs(dialogs));
    state
}

/// Writes a small demo photo once, returns its path (used by the message list
/// so photo-bearing rows take the same code path as the real app).
fn demo_photo() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("app-iced-bench");
    let path = dir.join("demo-photo.png");
    if !path.exists() {
        std::fs::create_dir_all(&dir).unwrap();
        let mut pm = tiny_skia::Pixmap::new(640, 480).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(28, 94, 191, 255));
        std::fs::write(&path, pm.encode_png().unwrap()).unwrap();
    }
    path
}

fn bench_build_list(c: &mut Criterion) {
    for &n in &[10usize, 50, 200, 500] {
        let state = demo_state(n);
        c.bench_function(&format!("messages_list/build/{n}"), |b| {
            b.iter(|| {
                let el = black_box(messages_list(&state, W, H));
                std::hint::black_box(el);
            });
        });
    }
}

/// A realistic fling over the DIALOG list (left pane): with a big chat account
/// the per-frame build of `dialog_list` must be O(visible rows), not O(N).
fn bench_dialog_scroll(c: &mut Criterion) {
    for &n in &[50usize, 200, 800] {
        let (req_tx, _req_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut state = State::new(req_tx);
        state.authenticated = true;
        state.open_chat = Some(1);
        let now = 1_700_000_000i32;
        let dialogs: Vec<ChatRow> = (0..n)
            .map(|i| ChatRow {
                id: i as i64,
                title: format!("Chat {i}, a long-ish name that gets ellipsized"),
                subtitle: format!("preview {i}"),
                date: now - i as i32,
                unread: (i % 7) as i32,
                avatar_path: None,
            })
            .collect();
        state.on_message(UiMessage::Dialogs(dialogs));
        let max_y = (n as f32) * 66.0;
        let mut phase = 0.0f32;
        c.bench_function(&format!("dialog_list/scroll/{n}"), |b| {
            b.iter(|| {
                phase = (phase + 46.0) % (max_y * 2.0);
                let y = if phase <= max_y { phase } else { max_y * 2.0 - phase };
                state.dialog_scroll_offset = y;
                let el = black_box(dialog_list(&state, H));
                std::hint::black_box(el);
            });
        });
    }
}

fn bench_build_full(c: &mut Criterion) {
    let state = demo_state(200);
    c.bench_function("chat_view/build/200", |b| {
        b.iter(|| {
            let el = black_box(chat_view(&state));
            std::hint::black_box(el);
        });
    });
}

/// A realistic fling: the scroll offset advances every iteration, so the
/// visible window slides through the history like a real scroll tick. With the
/// layout cache this cost is O(visible rows) regardless of `n` (before Phase 1
/// it rebuilt every row's height/offset tables for the whole history each
/// frame, making the build scale linearly with `n`).
fn bench_scroll(c: &mut Criterion) {
    for &n in &[200usize, 500, 2000] {
        let mut state = demo_state(n);
        let max_y = (n as f32) * 70.0;
        let mut phase = 0.0f32;
        c.bench_function(&format!("messages_list/scroll/{n}"), |b| {
            b.iter(|| {
                phase = (phase + 46.0) % (max_y * 2.0);
                let y = if phase <= max_y { phase } else { max_y * 2.0 - phase };
                state.scroll_offset = y;
                let el = black_box(messages_list(&state, W, H));
                std::hint::black_box(el);
            });
        });
    }
}

fn bench_layout(c: &mut Criterion) {
    let limits = Limits::new(Size::new(0.0, 0.0), Size::new(W, H));
    for &n in &[10usize, 50, 200, 500] {
        let state = demo_state(n);
        // The app's default renderer is the iced fallback enum (wgpu with a
        // tiny-skia fallback); headless benches drive the tiny-skia variant.
        let renderer =
            iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(Font::default(), Pixels(16.0)));
        let mut tree = Tree::empty();

        c.bench_function(&format!("chat_view/layout/{n}"), |b| {
            b.iter(|| {
                let mut el = chat_view(&state);
                Widget::diff(el.as_widget(), &mut tree);
                let node = Widget::layout(el.as_widget_mut(), &mut tree, &renderer, &limits);
                std::hint::black_box(node);
            });
        });
    }
}

fn bench_frame(c: &mut Criterion) {
    let limits = Limits::new(Size::new(0.0, 0.0), Size::new(W, H));
    let style = Style::default();
    for &n in &[10usize, 50, 200, 500] {
        let state = demo_state(n);
        let mut renderer =
            iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(Font::default(), Pixels(16.0)));
        let mut tree = Tree::empty();

        c.bench_function(&format!("chat_view/frame/{n}"), |b| {
            b.iter(|| {
                let mut el = chat_view(&state);
                Widget::diff(el.as_widget(), &mut tree);
                let node = Widget::layout(el.as_widget_mut(), &mut tree, &renderer, &limits);
                let layout = Layout::new(&node);
                let viewport = Rectangle::new(Point::new(0.0, 0.0), Size::new(W, H));
                Widget::draw(
                    el.as_widget(),
                    &tree,
                    &mut renderer,
                    &Theme::Dark,
                    &style,
                    layout,
                    Cursor::Available(Point::new(W / 2.0, H / 2.0)),
                    &viewport,
                );
                std::hint::black_box(node);
            });
        });
    }
}

criterion_group!(benches, bench_build_list, bench_build_full, bench_scroll, bench_dialog_scroll, bench_layout, bench_frame);
criterion_main!(benches);
