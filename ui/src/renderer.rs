//! Pure rendering of the scene into a `Pixmap` (no window dependency).
//! Testable off-screen: render a frame and inspect/compare pixels.

use tiny_skia::{Color, Paint, Pixmap, Rect, Transform};

use crate::chatlist::{ChatList, ROW_BG};
use crate::messages::MessageList;
use crate::text::TextRenderer;

/// Scene background, as an opaque (r, g, b) tuple.
pub const BACKGROUND: (u8, u8, u8) = (30, 31, 34);
/// Text color, as an opaque (r, g, b) tuple.
pub const TEXT: (u8, u8, u8) = (236, 239, 244);

/// Height of the top bar in chat mode (back button + title).
pub const TOPBAR_H: f32 = 36.0;
/// Height of the composer bar at the bottom of the chat view.
pub const INPUTBAR_H: f32 = 40.0;
/// Safety margin at the bottom of the messages area (above the composer).
pub const MESSAGES_BOTTOM_GAP: f32 = 8.0;

/// View to draw.
pub enum View<'a> {
    List,
    Chat { title: &'a str, messages: &'a MessageList },
}

/// Draws the scene into the given `Pixmap`.
///
/// Returns `Err` if the pixmap is empty (zero width or height).
pub fn render(
    pixmap: &mut Pixmap,
    text: &TextRenderer,
    list: &ChatList,
    view: View,
    status: &str,
    input: &str,
    scale: f32,
) -> Result<(), &'static str> {
    if pixmap.width() == 0 || pixmap.height() == 0 {
        return Err("empty pixmap");
    }

    pixmap.fill(bg());

    let s = scale.max(0.1);
    let width = pixmap.width() as f32;
    let height = pixmap.height() as f32;

    match view {
        View::List => {
            if list.rows.is_empty() {
                draw_status(pixmap, text, status, width, height, s);
            } else {
                list.draw(pixmap, text, 0.0, 0.0, width, height, s);
            }
        }
        View::Chat { title, messages } => {
            draw_topbar(pixmap, text, title, width, s);
            let area_h = (height - (TOPBAR_H + INPUTBAR_H + MESSAGES_BOTTOM_GAP) * s).max(0.0);
            let area_y = TOPBAR_H * s;
            if messages.rows.is_empty() {
                if !status.is_empty() {
                    draw_status(pixmap, text, status, width, area_y + area_h / 2.0, s);
                }
            } else {
                messages.draw(pixmap, text, 0.0, area_y, width, area_h, s);
            }
            draw_inputbar(pixmap, text, input, width, height, s);
        }
    }

    Ok(())
}

fn draw_topbar(pixmap: &mut Pixmap, text: &TextRenderer, title: &str, width: f32, s: f32) {
    let bar_h = TOPBAR_H * s;
    let mut bar = Paint::default();
    bar.set_color(Color::from_rgba8(ROW_BG.0, ROW_BG.1, ROW_BG.2, 255));
    pixmap.fill_rect(
        Rect::from_xywh(0.0, 0.0, width, bar_h).unwrap(),
        &bar,
        Transform::identity(),
        None,
    );

    let px = 18.0 * s;
    text.draw(pixmap, "<", 12.0 * s, 24.0 * s, px, TEXT);

    // Title, truncated to the remaining width.
    let title_px = 16.0 * s;
    let (tw, _) = text.measure(title, title_px);
    let max_w = width - 60.0 * s;
    let title = if tw > max_w {
        let mut out = String::new();
        for ch in title.chars() {
            if text.measure(&format!("{out}{ch}…"), title_px).0 <= max_w {
                out.push(ch);
            } else {
                break;
            }
        }
        format!("{out}…")
    } else {
        title.to_string()
    };
    text.draw(pixmap, &title, 34.0 * s, 24.0 * s, title_px, TEXT);
}

fn draw_status(pixmap: &mut Pixmap, text: &TextRenderer, status: &str, width: f32, center_y: f32, s: f32) {
    let px = 16.0 * s;
    let (tw, _) = text.measure(status, px);
    text.draw(pixmap, status, (width - tw) / 2.0, center_y, px, TEXT);
}

/// Composer bar at the bottom of the chat view (typed text).
fn draw_inputbar(
    pixmap: &mut Pixmap,
    text: &TextRenderer,
    input: &str,
    width: f32,
    height: f32,
    s: f32,
) {
    let bar_h = INPUTBAR_H * s;
    let y = height - bar_h;
    let mut bar = Paint::default();
    bar.set_color(Color::from_rgba8(ROW_BG.0, ROW_BG.1, ROW_BG.2, 255));
    pixmap.fill_rect(
        Rect::from_xywh(0.0, y, width, bar_h).unwrap(),
        &bar,
        Transform::identity(),
        None,
    );

    let pad = 12.0 * s;
    let px = 14.0 * s;
    let max_w = (width - pad * 2.0).max(20.0 * s);
    let baseline = y + bar_h / 2.0 + 5.0 * s;

    if input.is_empty() {
        text.draw(pixmap, "Message…", pad, baseline, px, crate::chatlist::SUBTITLE);
    } else {
        let (iw, _) = text.measure(input, px);
        let shown = if iw <= max_w {
            input.to_string()
        } else {
            // Show the tail of the text (the caret stays on the right): build it
            // from the end while it fits.
            let mut out = String::new();
            for ch in input.chars().rev() {
                if text.measure(&format!("{ch}{out}"), px).0 <= max_w {
                    out.insert(0, ch);
                } else {
                    break;
                }
            }
            out
        };
        text.draw(pixmap, &shown, pad, baseline, px, TEXT);
    }
}

fn bg() -> Color {
    Color::from_rgba8(BACKGROUND.0, BACKGROUND.1, BACKGROUND.2, 255)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chatlist::{ChatList, ChatRow};

    fn text() -> TextRenderer {
        TextRenderer::new()
    }

    fn empty() -> ChatList {
        ChatList::new()
    }

    #[test]
    fn render_fills_the_background_with_the_base_color() {
        let mut pixmap = Pixmap::new(200, 100).unwrap();
        render(&mut pixmap, &text(), &empty(), View::List, "test", "", 1.0).unwrap();

        // Corner away from the centered status.
        let px = pixmap.pixel(2, 2).unwrap();
        assert_eq!(
            (px.red(), px.green(), px.blue()),
            (BACKGROUND.0, BACKGROUND.1, BACKGROUND.2)
        );
    }

    #[test]
    fn render_shows_the_status_when_empty() {
        let mut pixmap = Pixmap::new(300, 100).unwrap();
        render(&mut pixmap, &text(), &empty(), View::List, "Connecting…", "", 1.0).unwrap();

        let has_text = (0..100).any(|y| {
            (0..300).any(|x| {
                let p = pixmap.pixel(x, y).unwrap();
                (p.red(), p.green(), p.blue()) != BACKGROUND
            })
        });
        assert!(has_text);
    }

    #[test]
    fn render_draws_the_list_when_not_empty() {
        let mut list = ChatList::new();
        list.rows = vec![ChatRow {
            id: 1,
            title: "My chat".into(),
            subtitle: "hi".into(),
            unread: 1,
        }];
        let mut pixmap = Pixmap::new(300, 200).unwrap();
        render(&mut pixmap, &text(), &list, View::List, "", "", 1.0).unwrap();

        let has_list = (0..200).any(|y| {
            (0..300).any(|x| {
                let p = pixmap.pixel(x, y).unwrap();
                (p.red(), p.green(), p.blue()) != BACKGROUND
            })
        });
        assert!(has_list);
    }

    #[test]
    fn render_draws_the_chat_title_bar() {
        let mut messages = MessageList::new();
        messages.rows = vec![crate::messages::MsgRow {
            id: 0,
            text: "hi".into(),
            out: false,
        }];
        let mut pixmap = Pixmap::new(300, 200).unwrap();
        render(
            &mut pixmap,
            &text(),
            &empty(),
            View::Chat {
                title: "My chat",
                messages: &messages,
            },
            "",
            "hello you",
            1.0,
        )
        .unwrap();

        // The top bar is not pure background.
        let px = pixmap.pixel(10, 10).unwrap();
        assert_ne!(
            (px.red(), px.green(), px.blue()),
            (BACKGROUND.0, BACKGROUND.1, BACKGROUND.2)
        );
    }

    #[test]
    fn render_draws_the_composer_bar() {
        let messages = MessageList::new();
        let mut pixmap = Pixmap::new(300, 200).unwrap();
        render(
            &mut pixmap,
            &text(),
            &empty(),
            View::Chat {
                title: "Chat",
                messages: &messages,
            },
            "",
            "",
            1.0,
        )
        .unwrap();

        // The composer bar (bottom) is not pure background.
        let px = pixmap.pixel(10, 190).unwrap();
        assert_ne!(
            (px.red(), px.green(), px.blue()),
            (BACKGROUND.0, BACKGROUND.1, BACKGROUND.2)
        );
    }

    #[test]
    fn render_rejects_an_empty_pixmap() {
        assert!(Pixmap::new(0, 0).is_none());
    }

    #[test]
    fn render_produces_a_stable_offscreen_png() {
        let mut pixmap = Pixmap::new(400, 600).unwrap();
        render(&mut pixmap, &text(), &empty(), View::List, "test", "", 1.0).unwrap();
        let bytes = pixmap.encode_png().expect("png encode");
        assert!(bytes.len() > 0);
        let reloaded = Pixmap::decode_png(&bytes).expect("png decode");
        assert_eq!(reloaded.as_ref().data(), pixmap.as_ref().data());
    }
}