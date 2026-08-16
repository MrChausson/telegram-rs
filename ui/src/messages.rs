//! Renders a chat's messages: bubbles (right-aligned when sent by us),
//! with word wrap and vertical scrolling.

use tiny_skia::{Color, Paint, Pixmap, Rect, Transform};

use crate::chatlist::BG;
use crate::text::TextRenderer;

/// A displayed message.
#[derive(Debug, Clone)]
pub struct MsgRow {
    pub id: i32,
    pub text: String,
    /// True if the message was sent by us.
    pub out: bool,
}

/// Scrollable message list.
pub struct MessageList {
    pub rows: Vec<MsgRow>,
    pub scroll: f32,
    pub line_height: f32,
    pub row_padding: f32,
}

pub const INBOUND_BG: (u8, u8, u8) = (45, 47, 53);
pub const OUTBOUND_BG: (u8, u8, u8) = (50, 78, 60);
pub const TEXT_COLOR: (u8, u8, u8) = (236, 239, 244);

impl Default for MessageList {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageList {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            scroll: 0.0,
            line_height: 16.0,
            row_padding: 10.0,
        }
    }

    /// Bubble width (received: 80% of the window, sent: 65%).
    fn bubble_width(out: bool, width: f32) -> f32 {
        if out {
            width * 0.65
        } else {
            width * 0.8
        }
    }

    /// Height of a message row after wrapping (in px).
    ///
    /// The line count is computed from the *bubble* width (which depends on
    /// `msg.out`), otherwise sent messages would be clipped.
    pub fn row_height(&self, text: &TextRenderer, msg: &MsgRow, width: f32) -> f32 {
        if msg.text.is_empty() {
            return self.line_height + self.row_padding * 2.0;
        }
        let bw = Self::bubble_width(msg.out, width);
        let interior = (bw - 24.0).max(10.0);
        let (w, _) = text.measure(&msg.text, 14.0);
        let lines = (w / interior).ceil().max(1.0) as f32;
        lines * self.line_height + self.row_padding * 2.0
    }

    /// Total content height.
    pub fn content_height(&self, text: &TextRenderer, width: f32) -> f32 {
        self.rows
            .iter()
            .map(|m| self.row_height(text, m, width))
            .sum()
    }

    pub fn scroll_by(&mut self, dy: f32, viewport_height: f32, content: f32) {
        let max = (content - viewport_height).max(0.0);
        self.scroll = (self.scroll + dy).clamp(0.0, max);
    }

    pub fn set_scroll_bottom(&mut self, content: f32, viewport_height: f32) {
        let max = (content - viewport_height).max(0.0);
        self.scroll = max;
    }

    /// Draws the messages into `pixmap`, between `(x, y)` and `(x+w, y+h)`.
    pub fn draw(
        &self,
        pixmap: &mut Pixmap,
        text: &TextRenderer,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        scale: f32,
    ) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let mut bg = Paint::default();
        bg.set_color(Color::from_rgba8(BG.0, BG.1, BG.2, 255));
        pixmap.fill_rect(Rect::from_xywh(x, y, w, h).unwrap(), &bg, Transform::identity(), None);

        let s = scale.max(0.1);
        let lw = w / s;
        // Scroll is stored in logical coordinates; `s` projects it to physical.
        let mut cursor = -self.scroll;
        for msg in &self.rows {
            let rh = self.row_height(text, msg, lw);
            let phys_top = y + cursor * s;
            if phys_top + rh * s > y {
                if phys_top < y + h {
                    self.draw_bubble(pixmap, text, x, phys_top, w, rh, msg, s);
                }
            }
            if phys_top >= y + h {
                break;
            }
            cursor += rh;
        }
    }

    fn draw_bubble(
        &self,
        pixmap: &mut Pixmap,
        text: &TextRenderer,
        x: f32,
        top: f32,
        w: f32,
        height_log: f32,
        msg: &MsgRow,
        s: f32,
    ) {
        let pad = 10.0 * s;
        let bubble_h = height_log * s - self.row_padding * s;
        // Bubble aligned left (received) or right (sent).
        let bw = Self::bubble_width(msg.out, w);
        let bx = if msg.out { x + w - bw - pad } else { x + pad };
        let (br, bg, bb) = if msg.out { OUTBOUND_BG } else { INBOUND_BG };
        let mut bp = Paint::default();
        bp.set_color(Color::from_rgba8(br, bg, bb, 255));
        pixmap.fill_rect(
            Rect::from_xywh(bx, top, bw, bubble_h).unwrap(),
            &bp,
            Transform::identity(),
            None,
        );

        // Text with word wrap, breaking at word boundaries.
        let px = 14.0 * s;
        let interior = bw - 24.0 * s;
        let mut text_y = top + self.row_padding * s + 3.0 * s;
        let mut remaining = msg.text.as_str();
        while !remaining.is_empty() && text_y < top + bubble_h {
            let max_chars = estimate_chars(text, remaining, interior, px);
            let mut idx = max_chars.min(remaining.chars().count());
            let mut prefix = remaining.chars().take(idx).collect::<String>();
            // Break at a word boundary when possible (not mid-word).
            if idx < remaining.chars().count() {
                if let Some(space) = prefix.rfind(' ') {
                    if space > 0 {
                        idx = space;
                        prefix.truncate(space);
                    }
                }
            }
            text.draw(pixmap, &prefix, bx + 12.0 * s, text_y, px, TEXT_COLOR);
            if prefix.is_empty() {
                break;
            }
            let rest = &remaining[prefix.len()..];
            remaining = rest.trim_start();
            text_y += self.line_height * s;
        }
    }
}

/// Number of characters that fit on one line at `px_size` within `width` px.
fn estimate_chars(text: &TextRenderer, s: &str, width: f32, px_size: f32) -> usize {
    if s.is_empty() {
        return 1;
    }
    let mut count = 0;
    let mut w = 0.0;
    for ch in s.chars() {
        let cw = text.measure(&ch.to_string(), px_size).0;
        if w + cw > width {
            break;
        }
        w += cw;
        count += 1;
    }
    count.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text() -> TextRenderer {
        TextRenderer::new()
    }

    fn msg(s: &str) -> MsgRow {
        MsgRow {
            id: 0,
            text: s.to_string(),
            out: false,
        }
    }

    fn msg_out(s: &str) -> MsgRow {
        MsgRow {
            id: 0,
            text: s.to_string(),
            out: true,
        }
    }

    #[test]
    fn row_height_grows_with_text_length() {
        let tr = text();
        let list = MessageList::new();
        let short = list.row_height(&tr, &msg("hello"), 300.0);
        let long = list.row_height(&tr, &msg(&"mot ".repeat(200)), 300.0);
        assert!(long > short);
    }

    #[test]
    fn narrower_sent_bubble_is_taller() {
        // Same text, narrower bubble (sent) => more lines => taller.
        let tr = text();
        let list = MessageList::new();
        let text = "a long message ".repeat(30);
        let recv = list.row_height(&tr, &msg(&text), 300.0);
        let sent = list.row_height(&tr, &msg_out(&text), 300.0);
        assert!(sent > recv);
    }

    #[test]
    fn scroll_is_clamped() {
        let mut list = MessageList::new();
        list.rows = vec![msg("one"), msg("two"), msg("three")];
        let tr = text();
        let content = list.content_height(&tr, 300.0);
        list.scroll_by(10_000.0, 100.0, content);
        let max = (content - 100.0).max(0.0);
        assert_eq!(list.scroll, max);
    }

    #[test]
    fn draw_paints_bubbles() {
        let mut list = MessageList::new();
        list.rows = vec![msg("first chat message"), msg("second")];
        let mut pixmap = Pixmap::new(300, 200).unwrap();
        list.draw(&mut pixmap, &text(), 0.0, 0.0, 300.0, 200.0, 1.0);

        let mut changed = 0;
        for y in 0..200 {
            for x in 0..300 {
                let p = pixmap.pixel(x, y).unwrap();
                if (p.red(), p.green(), p.blue()) != BG {
                    changed += 1;
                }
            }
        }
        assert!(changed > 300);
    }
}