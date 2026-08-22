//! One-off probe: measures the true per-frame cost of the app's window render,
//! including the final software composite (`renderer.draw`) into a 1250x1514
//! buffer — exactly what `present` does every frame. This is what the live app
//! pays per frame; the `frame` criterion bench only records draw ops.
//!
//!    cargo run --release -p app-iced --example composite_probe

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

fn state(n: usize, no_text: bool, no_bubbles: bool) -> State {
    let (req_tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut s = State::new(req_tx);
    s.authenticated = true;
    s.open_chat = Some(1);
    s.chat_title = "Probe".into();
    let now = 1_700_000_000i32;
    s.messages = (0..n)
        .map(|i| MsgRow {
            id: i as i32,
            text: if no_text {
                String::new()
            } else if i % 7 == 0 {
                "Un message plus long pour forcer le retour à la ligne et tester le calcul de hauteur des bulles sur plusieurs lignes, avec quelques émojis et un peu de texte pour mesurer le wrap.".into()
            } else {
                format!("Message {i} de la grande conversation de test — le lent fox saute pas-dessus le chien paresseux {i}")
            },
            date: now - i as i32,
            out: i % 2 == 0,
            photo: None,
            photo_path: None,
            read: true,
        })
        .collect();
    if no_bubbles {
        s.messages = vec![];
        s.open_chat = None;
    }
    s
}

fn main() {
    let n = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(400);
    let no_text = std::env::var("PROBE_NO_TEXT").is_ok();
    let no_bubbles = std::env::var("PROBE_NO_BUBBLES").is_ok();

    let logical = Size::new(W as f32 / 1.6, H as f32 / 1.6);
    let viewport = iced_tiny_skia::graphics::Viewport::with_physical_size(Size::new(W, H), 1.6);
    let limits = Limits::new(Size::ZERO, logical);
    let style = Style::default();
    let bg = iced_core::Color::from_rgb8(14, 22, 33);
    let damage = vec![Rectangle::new(Point::new(0.0, 0.0), logical)]; // full window per frame
    let cursor = Cursor::Available(Point::new(logical.width / 2.0, logical.height / 2.0));

    let s = state(n, no_text, no_bubbles);
    let bounds = Rectangle::new(Point::new(0.0, 0.0), logical);

    let mut renderer = iced_tiny_skia::Renderer::new(Font::default(), Pixels(16.0));
    let mut tree = Tree::empty();
    let mut clip = tiny_skia::Mask::new(W, H).unwrap();
    let mut buf: Vec<u32> = vec![0; (W * H) as usize];

    // warm everything (shaping caches, image cache)
    for _ in 0..10 {
        let mut el = chat_view(&s);
        Widget::diff(el.as_widget(), &mut tree);
        let node = Widget::layout(el.as_widget_mut(), &mut tree, &mut renderer, &limits);
        let layout = Layout::new(&node);
        Widget::draw(el.as_widget(), &tree, &mut renderer, &Theme::Dark, &style, layout, cursor, &Rectangle::new(Point::new(0., 0.), logical));
        renderer.reset(bounds);
        let mut pm = tiny_skia::PixmapMut::from_bytes(bytemuck::cast_slice_mut(&mut buf), W, H).unwrap();
        renderer.draw(&mut pm, &mut clip, &viewport, &damage, bg);
    }

    // measure: composite-only, then full frame
    let t0 = Instant::now();
    const ITERS: u32 = 200;
    for _ in 0..ITERS {
        let mut pm = tiny_skia::PixmapMut::from_bytes(bytemuck::cast_slice_mut(&mut buf), W, H).unwrap();
        renderer.draw(&mut pm, &mut clip, &viewport, &damage, bg);
        renderer.reset(bounds);
    }
    let comp = t0.elapsed().as_secs_f64() / ITERS as f64;

    let t1 = Instant::now();
    for _ in 0..ITERS {
        let mut el = chat_view(&s);
        Widget::diff(el.as_widget(), &mut tree);
        let node = Widget::layout(el.as_widget_mut(), &mut tree, &mut renderer, &limits);
        let layout = Layout::new(&node);
        Widget::draw(el.as_widget(), &tree, &mut renderer, &Theme::Dark, &style, layout, cursor, &Rectangle::new(Point::new(0., 0.), logical));
        renderer.reset(bounds);
        let mut pm = tiny_skia::PixmapMut::from_bytes(bytemuck::cast_slice_mut(&mut buf), W, H).unwrap();
        renderer.draw(&mut pm, &mut clip, &viewport, &damage, bg);
    }
    let full = t1.elapsed().as_secs_f64() / ITERS as f64;

    println!("n={n} physical={W}x{H}");
    println!("composite only   : {:.2} ms   => max {:.0} fps", comp*1000.0, 1.0/comp);
    println!("full frame (view+l): {:.2} ms  => max {:.0} fps", full*1000.0, 1.0/full);
}