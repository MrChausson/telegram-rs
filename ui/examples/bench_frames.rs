//! Headless benchmark: measures the cost of rendering the whole scene (chat
//! list + open conversation with text/photo messages) into pixmaps of growing
//! physical size, plus the blit conversion. Run with `cargo run -p ui
//! --release --example bench_frames`.

use tiny_skia::Pixmap;
use ui::chatlist::ChatList;
use ui::image::PhotoCache;
use ui::messages::{MessageList, MsgRow};
use ui::renderer::render;
use ui::state::Screen;
use ui::text::TextRenderer;

fn scene() -> (ChatList, MessageList) {
    let mut list = ChatList::new();
    for i in 0..40 {
        list.rows.push(ui::chatlist::ChatRow {
            id: i,
            title: format!("Chat number {i}"),
            subtitle: "a preview line with some text and an emoji…".into(),
            date: 1_700_000_000 + i as i32,
            unread: ((i % 17) as i32) * 3,
            avatar_path: None,
        });
    }
    let mut messages = MessageList::new();
    for i in 0..120 {
        messages.rows.push(MsgRow {
            id: i,
            text: if i % 8 == 0 {
                format!("A longer message number {i} with several words to force wrapping across multiple lines of text again and again until it grows.")
            } else if i % 8 == 1 {
                "".into()
            } else {
                format!("message {i}, court mais avec du contenu")
            },
            date: 1_700_000_000 + (i as i32) * 60,
            out: i % 2 == 0,
            photo: if i % 8 == 1 {
                Some((640, 480))
            } else {
                None
            },
            photo_path: None,
            read: i % 6 == 0,
        });
    }
    (list, messages)
}

fn main() { bench() }

fn bench() {
    let text = TextRenderer::new();
    let photos = PhotoCache::new();
    let (list, messages) = scene();
    let screen = Screen::Chat {
        id: 5,
        loading: false,
    };
    let _ = &messages;
    let _ = &screen;

    for &(w, h) in &[
        (790u32, 950u32),
        (1250, 1514),
        (1700, 2000),
        (2500, 2700),
    ] {
        let mut pixmap = Pixmap::new(w, h).unwrap();
        let mut best = f32::MAX;
        let mut sum = 0.0;
        // Warm-up (glyph rasterization, height cache).
        let _ = render(
            &mut pixmap,
            &text,
            &list,
            &screen,
            &messages,
            "",
            "",
            None,
            &photos,
            None,
            None,
            false,
            None,
            None,
            1.6,
        );
        for _ in 0..60 {
            let t = std::time::Instant::now();
            let _ = render(
                &mut pixmap,
                &text,
                &list,
                &screen,
                &messages,
                "",
                "",
                None,
                &photos,
                None,
                None,
                false,
                None,
                None,
                1.6,
            );
            let dt = t.elapsed().as_secs_f32() * 1000.0;
            best = best.min(dt);
            sum += dt;
        }
        // blit cost
        let mut buf = vec![0u32; (w * h) as usize];
        let t = std::time::Instant::now();
        for _ in 0..60 {
            ui::blit::blit_pixmap(pixmap.as_ref(), &mut buf, w, h).unwrap();
        }
        let blit_ms = t.elapsed().as_secs_f32() * 1000.0 / 60.0;

        // Isolate: chat list only (top of the left pane).
        let list_h = (h as f32 - 48.0 * 1.6).max(50.0);
        let mut l_best = f32::MAX;
        for _ in 0..60 {
            let t = std::time::Instant::now();
            list.draw(&mut pixmap, &text, 0.0, 48.0 * 1.6, 390.0 * 1.6, list_h, 1.6, Some(5), &photos);
            l_best = l_best.min(t.elapsed().as_secs_f32() * 1000.0);
        }
        // Isolate: messages pane (right side).
        let mw = w as f32 - 390.0 * 1.6;
        let mh = h as f32 - (48.0 + 60.0) * 1.6;
        let mut messages2 = scene().1;
        messages2.set_scroll_bottom(messages2.content_height(&text, mw / 1.6), mh);
        let mut m_best = f32::MAX;
        for _ in 0..20 {
            let t = std::time::Instant::now();
            messages2.draw(&mut pixmap, &text, 390.0 * 1.6, 48.0 * 1.6, mw, mh, 1.6, &photos, None);
            m_best = m_best.min(t.elapsed().as_secs_f32() * 1000.0);
        }
        let avg = sum / 60.0;
        let line = format!(
            "{w}x{h}: render avg {avg:.3} ms, best {best:.3} ms (blit {blit_ms:.2} ms; list {l_best:.2} ms; msgs {m_best:.2} ms)"
        );
        println!("{line}");
    }
}