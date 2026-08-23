//! Headless probe of the app's REAL per-frame cost at the compositor's actual
//! physical resolution. Two modes:
//!   - default: static scene (what a "render 200x same frame" bench hides)
//!   - PROBE_SCROLL=1: REAL fling (scroll offset advances every frame, so each
//!     frame lays-out + shapes different rows) — this is the honest number for
//!     "is scrolling slow because of our per-frame work?"
//!
//! Usage:
//! - `cargo run --release -p app-iced --example composite_probe [N]`
//! - `PROBE_SCROLL=1 cargo run --release -p app-iced --example composite_probe [N]`

use app_iced::bridge::MsgRow;
use app_iced::state::State;
use app_iced::chat_view;
use std::time::Instant;

const W: u32 = 1250;
const H: u32 = 1514;

use iced_core::layout::{Limits, Layout};
use iced_core::mouse::Cursor;
use iced_core::renderer::Style;
use iced_core::theme::Theme;
use iced_core::widget::{Tree, Widget};
use iced_core::{Font, Pixels, Point, Rectangle, Size};
use iced_core::Renderer as _;

fn state(n: usize) -> State {
    let (req_tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut s = State::new(req_tx);
    s.authenticated = true;
    s.open_chat = Some(1);
    s.chat_title = "Probe".into();
    let now = 1_700_000_000i32;
    s.messages = (0..n)
        .map(|i| MsgRow {
            id: i as i32,
            text: if i % 7 == 0 {
                "Un message plus long pour forcer le retour à la ligne et tester le calcul de hauteur des bulles sur plusieurs lignes, avec quelques émojis et un peu de texte pour mesurer le wrap.".into()
            } else {
                format!("Message {i} de la grande conversation de test — le lent fox saute pas-dessus le chien paresseux {i}")
            },
            date: now - i as i32,
            out: i % 2 == 0,
            photo: if i % 40 == 7 { Some((640, 480)) } else { None },
            photo_path: None,
            read: true,
        })
        .collect();
    s
}

fn main() {
    let n = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(400);
    let scrolling = std::env::var("PROBE_SCROLL").is_ok();

    let logical = Size::new(W as f32 / 1.6, H as f32 / 1.6);
    let viewport = iced_tiny_skia::graphics::Viewport::with_physical_size(Size::new(W, H), 1.6);
    let limits = Limits::new(Size::ZERO, logical);
    let style = Style::default();
    let bg = iced_core::Color::from_rgb8(14, 22, 33);
    let damage = vec![Rectangle::new(Point::new(0.0, 0.0), logical)];
    let cursor = Cursor::Available(Point::new(logical.width / 2.0, logical.height / 2.0));
    let bounds = Rectangle::new(Point::new(0.0, 0.0), logical);

    let mut s = state(n);
    let max_y = (n as f32) * 70.0;

    // The app's Element is typed on the iced fallback enum (wgpu + tiny-skia);
    // headless probes drive the tiny-skia variant through it.
    let mut renderer = iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(Font::default(), Pixels(16.0)));
    let mut tree = Tree::empty();
    let mut clip = tiny_skia::Mask::new(W, H).unwrap();
    let mut buf: Vec<u32> = vec![0; (W * H) as usize];

    let mut phase = 0.0f32;
    for _ in 0..10 {
        let mut el = chat_view(&s);
        Widget::diff(el.as_widget(), &mut tree);
        let node = Widget::layout(el.as_widget_mut(), &mut tree, &renderer, &limits);
        let layout = Layout::new(&node);
        Widget::draw(el.as_widget(), &tree, &mut renderer, &Theme::Dark, &style, layout, cursor, &bounds);
        renderer.reset(bounds);
        let mut pm = tiny_skia::PixmapMut::from_bytes(bytemuck::cast_slice_mut(&mut buf), W, H).unwrap();
        if let iced::Renderer::Secondary(r) = &mut renderer {
            r.draw(&mut pm, &mut clip, &viewport, &damage, bg);
        }
    }

    const ITERS: u32 = 300;
    let t1 = Instant::now();
    for _ in 0..ITERS {
        if scrolling {
            // Advance the fling BEFORE this frame (like a real scroll tick).
            phase = (phase + 46.0) % (max_y * 2.0);
            let y = if phase <= max_y { phase } else { max_y * 2.0 - phase };
            s.scroll_offset = y;
        }
        let mut el = chat_view(&s);
        Widget::diff(el.as_widget(), &mut tree);
        let node = Widget::layout(el.as_widget_mut(), &mut tree, &renderer, &limits);
        let layout = Layout::new(&node);
        Widget::draw(el.as_widget(), &tree, &mut renderer, &Theme::Dark, &style, layout, cursor, &bounds);
        renderer.reset(bounds);
        let mut pm = tiny_skia::PixmapMut::from_bytes(bytemuck::cast_slice_mut(&mut buf), W, H).unwrap();
        if let iced::Renderer::Secondary(r) = &mut renderer {
            r.draw(&mut pm, &mut clip, &viewport, &damage, bg);
        }
    }
    let full = t1.elapsed().as_secs_f64() / ITERS as f64;

    println!(
        "n={n} physical={W}x{H} mode={}  frame={:.3} ms  => {:.0} fps",
        if scrolling { "scroll" } else { "static" },
        full * 1000.0,
        1.0 / full
    );
}
