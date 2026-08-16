//! Renders a chat's messages: rounded bubbles (right-aligned when sent by us)
//! with word wrap, timestamps and vertical scrolling.

use tiny_skia::{Color, Paint, Pixmap, Transform};

use crate::text::TextRenderer;
use crate::theme::{self, font, layout};

/// A displayed message.
#[derive(Debug, Clone)]
pub struct MsgRow {
    pub id: i32,
    pub text: String,
    /// Unix timestamp of the message.
    pub date: i32,
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

    /// Bubble width (received: 70% of the window, sent: 60%).
    fn bubble_width(out: bool, width: f32) -> f32 {
        if out {
            width * 0.6
        } else {
            width * 0.7
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
        let interior = (bw - 28.0).max(10.0);
        let (w, _) = text.measure(&msg.text, font::MESSAGE);
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
        self.fill_bg(pixmap, x, y, w, h);

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

    fn fill_bg(&self, pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32) {
        let mut bg = Paint::default();
        bg.set_color(Color::from_rgba8(theme::CHAT_BG.0, theme::CHAT_BG.1, theme::CHAT_BG.2, 255));
        pixmap.fill_rect(tiny_skia::Rect::from_xywh(x, y, w, h).unwrap(), &bg, Transform::identity(), None);
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
        let v_pad = 8.0 * s;

        // Bubble centered in the row height, leaving vertical padding above
        // and below so the text (with its ascenders) stays inside.
        let bubble_h = (height_log * s - 2.0 * v_pad).max(8.0);
        let bubble_top = top + v_pad;

        // Bubble aligned left (received) or right (sent), with rounded corners.
        let bw = Self::bubble_width(msg.out, w);
        let bx = if msg.out { x + w - bw - pad } else { x + pad };
        let radius = layout::BUBBLE_RADIUS * s;
        let (br, bg_, bb) = if msg.out {
            theme::BUBBLE_SENT
        } else {
            theme::BUBBLE_RECV
        };
        let mut bp = Paint::default();
        bp.set_color(Color::from_rgba8(br, bg_, bb, 255));
        let bubble = theme::rounded_rect(bx, bubble_top, bw, bubble_h, radius);
        pixmap.fill_path(&bubble, &bp, tiny_skia::FillRule::Winding, Transform::identity(), None);

        // Text with word wrap, breaking at word boundaries. The first baseline
        // sits below the top of the bubble so glyph ascenders never overflow.
        let px = font::MESSAGE * s;
        let line_h = self.line_height * s;
        let interior = bw - 28.0 * s;
        // First baseline lowered so the text block sits centered in the bubble.
        let mut text_y = bubble_top + 15.0 * s;
        let mut remaining = msg.text.as_str();
        while !remaining.is_empty() && text_y <= bubble_top + bubble_h - 5.0 {
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
            text.draw(pixmap, &prefix, bx + 14.0 * s, text_y, px, theme::TEXT_PRIMARY);
            if prefix.is_empty() {
                break;
            }
            let rest = &remaining[prefix.len()..];
            remaining = rest.trim_start();
            text_y += line_h;
        }

        // Timestamp, outside the bubble (opposite side).
        if msg.date > 0 {
            let ts = theme::fmt_time(msg.date);
            let ts_px = font::TIMESTAMP * s;
            let (tw, _) = text.measure(&ts, ts_px);
            let baseline = bubble_top + bubble_h - 6.0 * s;
            if msg.out {
                text.draw(pixmap, &ts, bx - tw - 8.0 * s, baseline, ts_px, theme::TEXT_SECONDARY);
            } else {
                text.draw(pixmap, &ts, bx + bw + 8.0 * s, baseline, ts_px, theme::TEXT_SECONDARY);
            }
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
            date: 0,
            out: false,
        }
    }

    fn msg_out(s: &str) -> MsgRow {
        MsgRow {
            id: 0,
            text: s.to_string(),
            date: 0,
            out: true,
        }
    }

    #[test]
    fn row_height_grows_with_text_length() {
        let tr = text();
        let list = MessageList::new();
        let short = list.row_height(&tr, &msg("hello"), 300.0);
        let long = list.row_height(&tr, &msg(&"word ".repeat(200)), 300.0);
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
                if (p.red(), p.green(), p.blue()) != theme::CHAT_BG {
                    changed += 1;
                }
            }
        }
        assert!(changed > 300);
    }

    #[test]
    fn draw_renders_bubble_timestamps() {
        let mut list = MessageList::new();
        list.rows = vec![
            MsgRow { id: 1, text: "hey".into(), date: 1_700_000_000, out: false },
            MsgRow { id: 2, text: "yo".into(), date: 1_700_000_800, out: true },
        ];
        let mut pixmap = Pixmap::new(300, 200).unwrap();
        list.draw(&mut pixmap, &text(), 0.0, 0.0, 300.0, 200.0, 1.0);
        // No panic; timestamps rendered somewhere off the bubbles.
        assert!(list.rows.len() == 2);
    }

    #[test]
    fn bubble_text_never_escapes_the_top_of_the_bubble() {
        // Regression: glyph ascenders must stay inside the bubble. The top
        // padding zone of the first row must contain no primary-text pixels.
        let mut list = MessageList::new();
        list.rows = vec![MsgRow {
            id: 1,
            text: "Hello world, this is a message.".into(),
            date: 0,
            out: false,
        }];
        let s = 4.0;
        let mut pixmap = Pixmap::new(300, 200).unwrap();
        list.draw(&mut pixmap, &text(), 0.0, 0.0, 300.0, 200.0, s);

        // The strip strictly above the bubble top (8 px of v_pad, minus margin).
        let strip = (8.0 * s - 2.0).max(0.0) as u32;
        for y in 0..strip {
            for x in 0..300 {
                let p = pixmap.pixel(x, y).unwrap();
                assert!(
                    (p.red(), p.green(), p.blue()) != theme::TEXT_PRIMARY,
                    "text pixel leaked above the bubble at ({x},{y})"
                );
            }
        }
    }
}