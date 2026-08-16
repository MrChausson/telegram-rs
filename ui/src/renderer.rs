//! Pure rendering of the scene into a `Pixmap` (no window dependency).
//! Testable off-screen: render a frame and inspect/compare pixels.
//!
//! Split-pane layout: a fixed chat-list column on the left and the open
//! conversation (or a placeholder) on the right, like Telegram Desktop.

use tiny_skia::{Color, Paint, Pixmap, Transform};

use crate::chatlist::ChatList;
use crate::icons;
use crate::image::PhotoCache;
use crate::messages::MessageList;
use crate::state::Screen;
use crate::text::TextRenderer;
use crate::theme::{self, font, layout};

/// Draws the scene into the given `Pixmap`.
///
/// Returns `Err` if the pixmap is empty (zero width or height).
pub fn render(
    pixmap: &mut Pixmap,
    text: &TextRenderer,
    list: &ChatList,
    screen: &Screen,
    messages: &MessageList,
    status: &str,
    input: &str,
    photos: &PhotoCache,
    scale: f32,
) -> Result<(), &'static str> {
    if pixmap.width() == 0 || pixmap.height() == 0 {
        return Err("empty pixmap");
    }
    pixmap.fill(color(theme::CHAT_BG));

    let s = scale.max(0.1);
    let width = pixmap.width() as f32;
    let height = pixmap.height() as f32;

    let list_w = layout::LIST_W * s;
    if width <= list_w {
        return Ok(());
    }
    let rx = list_w;
    let rw = width - list_w;

    // ---- Left pane: chat list (header + rows). ----
    draw_list_header(pixmap, text, 0.0, 0.0, list_w, s);
    let selected = match screen {
        Screen::Chat { id, .. } => Some(*id),
        Screen::Idle => None,
    };
    let list_h = (height - layout::LIST_HEADER_H * s).max(0.0);
    list.draw(pixmap, text, 0.0, layout::LIST_HEADER_H * s, list_w, list_h, s, selected);

    // Divider between the panes.
    {
        let mut d = Paint::default();
        d.set_color(Color::from_rgba8(theme::INPUT_BORDER.0, theme::INPUT_BORDER.1, theme::INPUT_BORDER.2, 90));
        pixmap.fill_rect(
            tiny_skia::Rect::from_xywh(rx, 0.0, 1.0, height).unwrap(),
            &d,
            Transform::identity(),
            None,
        );
    }

    // ---- Right pane: conversation or placeholder. ----
    match screen {
        Screen::Idle => {
            let (tw, _) = text.measure("Select a conversation", font::MESSAGE * s);
            text.draw(
                pixmap,
                "Select a conversation",
                rx + (rw - tw) / 2.0,
                height / 2.0,
                font::MESSAGE * s,
                theme::TEXT_SECONDARY,
            );
        }
        Screen::Chat { id, loading } => {
            let title = list
                .rows
                .iter()
                .find(|r| r.id == *id)
                .map(|r| r.title.clone())
                .unwrap_or_default();
            let _ = loading;
            draw_chat_header(pixmap, text, &title, rx, rw, s);

            let area_y = layout::CHAT_HEADER_H * s;
            let bottom = (height - (layout::INPUT_H + layout::MESSAGES_BOTTOM_GAP) * s).max(area_y + 1.0);
            if messages.rows.is_empty() {
                if !status.is_empty() {
                    let (tw, _) = text.measure(status, font::MESSAGE * s);
                    text.draw(
                        pixmap,
                        status,
                        rx + (rw - tw) / 2.0,
                        (area_y + bottom) / 2.0,
                        font::MESSAGE * s,
                        theme::TEXT_SECONDARY,
                    );
                }
            } else {
                messages.draw(pixmap, text, rx, area_y, rw, bottom - area_y, s, photos);
            }

            draw_composer(pixmap, text, input, rx, height - layout::INPUT_H * s, rw, s);
        }
    }

    Ok(())
}

fn color(c: (u8, u8, u8)) -> Color {
    Color::from_rgba8(c.0, c.1, c.2, 255)
}


/// Left pane header: "Chats" title + search/compose/menu icons.
fn draw_list_header(pixmap: &mut Pixmap, text: &TextRenderer, x: f32, y: f32, w: f32, s: f32) {
    let h = layout::LIST_HEADER_H * s;
    let mut bg = Paint::default();
    bg.set_color(color(theme::LIST_BG));
    pixmap.fill_rect(tiny_skia::Rect::from_xywh(x, y, w, h).unwrap(), &bg, Transform::identity(), None);

    text.draw(pixmap, "Chats", x + 16.0 * s, y + h / 2.0 + 8.0 * s, font::TITLE * s, theme::TEXT_PRIMARY);

    let ic = 20.0 * s;
    let cx = x + w - 20.0 * s;
    let cy = y + h / 2.0;
    icons::dots(pixmap, cx, cy, ic, theme::ICON);
    icons::compose(pixmap, cx - 30.0 * s, cy, ic, theme::ICON);
    icons::search(pixmap, cx - 60.0 * s, cy, ic, theme::ICON);
}

/// Right pane header: back, avatar, name + status, search/info icons.
fn draw_chat_header(pixmap: &mut Pixmap, text: &TextRenderer, title: &str, x: f32, w: f32, s: f32) {
    let h = layout::CHAT_HEADER_H * s;
    let mut bg = Paint::default();
    bg.set_color(color(theme::LIST_BG));
    pixmap.fill_rect(tiny_skia::Rect::from_xywh(x, 0.0, w, h).unwrap(), &bg, Transform::identity(), None);

    let pad = 12.0 * s;
    // Back arrow.
    let back_cx = x + pad + 10.0 * s;
    let cy = h / 2.0;
    icons::back(pixmap, back_cx, cy, 18.0 * s, theme::ICON);

    // Avatar (placeholder initial) — square for now, tinted by title.
    let av = layout::AVATAR_CHAT * s;
    let avx = x + pad * 2.0 + 16.0 * s;
    let mut ap = Paint::default();
    ap.set_color(color(theme::ACCENT));
    let circle = tiny_skia::PathBuilder::from_circle(avx, cy, av / 2.0).unwrap();
    pixmap.fill_path(&circle, &ap, tiny_skia::FillRule::Winding, Transform::identity(), None);
    if let Some(initial) = title.chars().next() {
        let px = font::NAME * s;
        let (tw, _) = text.measure(&initial.to_string(), px);
        text.draw(pixmap, &initial.to_string(), avx - tw / 2.0, cy + 6.0 * s, px, theme::TEXT_PRIMARY);
    }

    // Name.
    let name_x = avx + av / 2.0 + 10.0 * s;
    let name_max = (x + w - 84.0 * s - name_x).max(20.0);
    let name = truncate(text, title, name_max, font::NAME * s);
    text.draw(pixmap, &name, name_x, cy + 6.0 * s, font::NAME * s, theme::TEXT_PRIMARY);
    // Status / online placeholder.
    let status = "Chat";
    text.draw(pixmap, status, name_x, cy + 22.0 * s, font::TIMESTAMP * s, theme::TEXT_SECONDARY);

    // Right icons.
    let ic = 20.0 * s;
    let icx = x + w - 22.0 * s;
    icons::info(pixmap, icx, cy, ic, theme::ICON);
    icons::search(pixmap, icx - 32.0 * s, cy, ic, theme::ICON);
}

fn truncate(text: &TextRenderer, title: &str, max_w: f32, px: f32) -> String {
    if text.measure(title, px).0 <= max_w {
        return title.to_string();
    }
    let mut out = String::new();
    for ch in title.chars() {
        if text.measure(&format!("{out}{ch}…"), px).0 <= max_w {
            out.push(ch);
        } else {
            break;
        }
    }
    format!("{out}…")
}

/// Composer bar: rounded input + placeholder + send button.
fn draw_composer(pixmap: &mut Pixmap, text: &TextRenderer, input: &str, x: f32, y: f32, w: f32, s: f32) {
    let h = layout::INPUT_H * s;
    let mut bg = Paint::default();
    bg.set_color(color(theme::LIST_BG));
    pixmap.fill_rect(tiny_skia::Rect::from_xywh(x, y, w, h).unwrap(), &bg, Transform::identity(), None);

    let pad = 12.0 * s;
    let radius = layout::INPUT_RADIUS * s;
    let field_h = h - 2.0 * pad;
    let send = 34.0 * s;
    let field_w = (w - pad * 2.0 - send - 10.0 * s).max(20.0).min(w - pad * 2.0);

    // Rounded input field.
    let fbg = color((26, 38, 52));
    let mut fp = Paint::default();
    fp.set_color(fbg);
    let field_path = theme::rounded_rect(x + pad, y + pad, field_w, field_h, radius);
    pixmap.fill_path(&field_path, &fp, tiny_skia::FillRule::Winding, Transform::identity(), None);

    // Placeholder or typed text.
    let px = font::PLACEHOLDER * s;
    let tc = if input.is_empty() { theme::TEXT_SECONDARY } else { theme::TEXT_PRIMARY };
    let shown = if input.is_empty() {
        "Message…".to_string()
    } else {
        let max = (field_w - 24.0 * s).max(10.0);
        if text.measure(input, px).0 <= max {
            input.to_string()
        } else {
            let mut out = String::new();
            for ch in input.chars().rev() {
                if text.measure(&format!("{ch}{out}"), px).0 <= max {
                    out.insert(0, ch);
                } else {
                    break;
                }
            }
            out
        }
    };
    text.draw(pixmap, &shown, x + pad + 14.0 * s, y + h / 2.0 + 5.0 * s, px, tc);

    // Send button (accent circle + paper plane).
    let sendx = x + w - pad - send / 2.0;
    let sendy = y + h / 2.0;
    let mut sp = Paint::default();
    sp.set_color(color(theme::ACCENT));
    let sc = tiny_skia::PathBuilder::from_circle(sendx, sendy, send / 2.0).unwrap();
    pixmap.fill_path(&sc, &sp, tiny_skia::FillRule::Winding, Transform::identity(), None);
    icons::send(pixmap, sendx, sendy, 20.0 * s, theme::TEXT_PRIMARY);
}