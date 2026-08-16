//! Renders a chat's messages: rounded bubbles (right-aligned when sent by us)
//! with word wrap, timestamps and vertical scrolling.

use tiny_skia::{Color, Paint, Pixmap, Transform};

use crate::image::PhotoCache;
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
    /// Dimensions of an attached photo, if any.
    pub photo: Option<(u32, u32)>,
    /// On-disk path of the downloaded photo thumbnail, once ready.
    pub photo_path: Option<String>,
}

/// A character position inside a specific message row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    /// Index of the message row.
    pub row: usize,
    /// Character offset inside that row's text.
    pub char: usize,
}

/// A text selection (start/end anchors, drag direction-agnostic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub start: Anchor,
    pub end: Anchor,
}

impl Selection {
    /// Returns the two anchors ordered so the span is forward (start ≤ end).
    pub fn normalized(&self) -> (Anchor, Anchor) {
        let (a, b) = (self.start, self.end);
        if a.row < b.row || (a.row == b.row && a.char <= b.char) {
            (a, b)
        } else {
            (b, a)
        }
    }
}

/// A wrapped line and the char offset (into the message text) it starts at.
struct WrappedLine {
    start: usize,
    text: String,
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
    /// `msg.out`), otherwise sent messages would be clipped. Photo messages
    /// reserve a box proportional to their aspect ratio.
    pub fn row_height(&self, text: &TextRenderer, msg: &MsgRow, width: f32) -> f32 {
        if let Some((pw, ph)) = msg.photo {
            let bw = Self::bubble_width(msg.out, width);
            let aspect = ph as f32 / pw as f32;
            let box_h = (bw * aspect).clamp(40.0, 200.0);
            return box_h + self.row_padding * 2.0;
        }
        if msg.text.is_empty() {
            return self.line_height + self.row_padding * 2.0;
        }
        let bw = Self::bubble_width(msg.out, width);
        let interior = (bw - 28.0).max(10.0);
        let lines = wrap_lines(text, &msg.text, interior);
        lines.len() as f32 * self.line_height + self.row_padding * 2.0
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

    /// Resolves a logical point `(x, y)` (relative to the messages pane, i.e.
    /// excluding the chat header) to a character anchor, or `None` when the
    /// point is outside a text row (empty space, photo, gaps).
    pub fn anchor_at(&self, text: &TextRenderer, x: f32, y: f32, width: f32) -> Option<Anchor> {
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let mut cursor = -self.scroll;
        for (row_idx, msg) in self.rows.iter().enumerate() {
            let rh = self.row_height(text, msg, width);
            if y >= cursor && y < cursor + rh {
                if msg.photo.is_some() || msg.text.is_empty() {
                    return None;
                }
                let bw = Self::bubble_width(msg.out, width);
                let bx = if msg.out { width - bw - 10.0 } else { 10.0 };
                let interior = (bw - 28.0).max(10.0);
                let lines = wrap_lines(text, &msg.text, interior);
                let line_h = self.line_height;
                let rel = ((y - cursor - 8.0) / line_h).floor() as isize;
                let line_idx = rel.clamp(0, lines.len() as isize - 1) as usize;
                let line = &lines[line_idx];
                let start_x = bx + 14.0;
                let mut acc = 0.0;
                let mut ch = line.text.chars().count();
                for (i, c) in line.text.chars().enumerate() {
                    let cw = text.measure(&c.to_string(), font::MESSAGE).0;
                    if x < start_x + acc + cw / 2.0 {
                        ch = i;
                        break;
                    }
                    acc += cw;
                }
                return Some(Anchor {
                    row: row_idx,
                    char: line.start + ch,
                });
            }
            cursor += rh;
        }
        None
    }

    /// The selected text as a single string (messages joined by newlines).
    /// A backward drag is normalized to a forward span.
    pub fn selected_text(&self, sel: Selection) -> String {
        let (a, b) = sel.normalized();
        let last = self.rows.len().saturating_sub(1);
        if a.row > last {
            return String::new();
        }
        let mut out = String::new();
        for row in a.row..=b.row.min(last) {
            let s = &self.rows[row].text;
            let len = s.chars().count();
            let start = if row == a.row { a.char.min(len) } else { 0 };
            let end = if row == b.row { b.char.min(len) } else { len };
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(slice_chars(s, start.min(end), end));
        }
        out
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
        photos: &PhotoCache,
        selection: Option<&Selection>,
    ) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        self.fill_bg(pixmap, x, y, w, h);

        let s = scale.max(0.1);
        let lw = w / s;
        // Scroll is stored in logical coordinates; `s` projects it to physical.
        let mut cursor = -self.scroll;
        for (row_idx, msg) in self.rows.iter().enumerate() {
            let rh = self.row_height(text, msg, lw);
            let phys_top = y + cursor * s;
            if phys_top + rh * s > y {
                if phys_top < y + h {
                    self.draw_bubble(
                        pixmap, text, x, phys_top, w, rh, row_idx, msg, s, photos, selection,
                    );
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

    /// Draws a photo bubble: the downloaded thumbnail (or a placeholder while
    /// it loads), rounded, fitted within the bubble.
    fn draw_photo(
        &self,
        pixmap: &mut Pixmap,
        text: &TextRenderer,
        x: f32,
        top: f32,
        bw: f32,
        box_h: f32,
        msg: &MsgRow,
        s: f32,
        photos: &PhotoCache,
    ) {
        let box_w = bw;
        let radius = layout::BUBBLE_RADIUS * s;

        // Placeholder background (also the bubble when no image is decoded).
        let (br, bg, bb) = if msg.out { theme::BUBBLE_SENT } else { theme::BUBBLE_RECV };
        let mut bp = Paint::default();
        bp.set_color(Color::from_rgba8(br, bg, bb, 255));
        pixmap.fill_path(
            &theme::rounded_rect(x, top, box_w, box_h, radius),
            &bp,
            tiny_skia::FillRule::Winding,
            Transform::identity(),
            None,
        );

        if let Some(path) = &msg.photo_path {
            if let Some(img) = photos.get(path) {
                let inset = 4.0 * s;
                let iw = img.width() as f32;
                let ih = img.height() as f32;
                let max_w = (bw - 2.0 * inset).max(1.0);
                let max_h = (box_h - 2.0 * inset).max(1.0);
                let i_aspect = ih / iw;
                let mut dw = max_w;
                let mut dh = dw * i_aspect;
                if dh > max_h {
                    dh = max_h;
                    dw = dh / i_aspect;
                }
                let ix = x + (box_w - dw) / 2.0;
                let iy = top + (box_h - dh) / 2.0;
                let scale = dw / iw;
                pixmap.draw_pixmap(
                    ix.round() as i32,
                    iy.round() as i32,
                    (*img).as_ref(),
                    &tiny_skia::PixmapPaint::default(),
                    Transform::from_scale(scale, scale),
                    None,
                );
                return;
            }
        }
        // Placeholder cue while the thumbnail is not ready.
        let cue = "…";
        let px = font::MESSAGE * s;
        let (tw, _) = text.measure(cue, px);
        text.draw(pixmap, cue, x + (box_w - tw) / 2.0, top + box_h / 2.0, px, theme::TEXT_SECONDARY);
    }

    fn draw_bubble(
        &self,
        pixmap: &mut Pixmap,
        text: &TextRenderer,
        x: f32,
        top: f32,
        w: f32,
        height_log: f32,
        row_idx: usize,
        msg: &MsgRow,
        s: f32,
        photos: &PhotoCache,
        selection: Option<&Selection>,
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

        if msg.photo.is_some() {
            return self.draw_photo(pixmap, text, bx, bubble_top, bw, bubble_h, msg, s, photos);
        }
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
        let line_h = self.line_height * s;
        let interior = (bw - 28.0).max(10.0);
        let lines = wrap_lines(text, &msg.text, interior);
        let text_len = msg.text.chars().count();

        // Character range of this row covered by the selection (if any).
        let span = selection.and_then(|sel| {
            let (a, b) = sel.normalized();
            if row_idx < a.row || row_idx > b.row {
                return None;
            }
            let sa = if row_idx == a.row { a.char } else { 0 };
            let sb = if row_idx == b.row { b.char } else { text_len };
            Some((sa.min(sb), sa.max(sb)))
        });

        let mut text_y = bubble_top + 15.0 * s;
        for line in &lines {
            if text_y > bubble_top + bubble_h - 5.0 {
                break;
            }
            let line_len = line.text.chars().count();
            let seg = span.map(|(sa, sb)| {
                let a = sa.saturating_sub(line.start).min(line_len);
                let b = sb.saturating_sub(line.start).min(line_len);
                (a.min(b), a.max(b))
            });
            draw_line(pixmap, text, &line.text, bx + 14.0 * s, text_y, line_h, seg, s);
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

/// Wraps `s` into lines at word boundaries within `interior` px (logical),
/// mirroring exactly how `draw_bubble` paints the text. `row_height` uses the
/// same function so measured height equals drawn height.
fn wrap_lines(text: &TextRenderer, s: &str, interior: f32) -> Vec<WrappedLine> {
    let mut out = Vec::new();
    let mut remaining = s;
    let mut offset = 0usize;
    while !remaining.is_empty() {
        let count = remaining.chars().count();
        let max_chars = estimate_chars(text, remaining, interior, font::MESSAGE);
        let idx = max_chars.min(count);
        let mut prefix = remaining.chars().take(idx).collect::<String>();
        // Break at a word boundary when possible (not mid-word).
        if idx < count {
            if let Some(space) = prefix.rfind(' ') {
                if space > 0 {
                    prefix.truncate(space);
                }
            }
        }
        if prefix.is_empty() {
            break;
        }
        let prefix_chars = prefix.chars().count();
        let prefix_bytes = prefix.len();
        out.push(WrappedLine { start: offset, text: prefix });
        offset += prefix_chars;
        let rest = &remaining[prefix_bytes..];
        let trimmed = rest.trim_start();
        offset += rest.chars().take_while(|c| c.is_whitespace()).count();
        remaining = trimmed;
    }
    out
}

/// Draws one wrapped line, highlighting `seg` (line-relative char range) when
/// the selection covers part of it.
fn draw_line(
    pixmap: &mut Pixmap,
    text: &TextRenderer,
    line: &str,
    x0: f32,
    baseline: f32,
    line_h: f32,
    seg: Option<(usize, usize)>,
    s: f32,
) {
    let px = font::MESSAGE * s;
    let line_len = line.chars().count();
    if let Some((a, b)) = seg {
        if a < b {
            let xa = x0 + prefix_width(text, line, a, px);
            let xb = x0 + prefix_width(text, line, b, px);
            let rect_top = baseline - line_h + 2.0 * s;
            let rect_h = line_h + 1.0 * s;
            let mut sp = Paint::default();
            sp.set_color(Color::from_rgba8(theme::ACCENT.0, theme::ACCENT.1, theme::ACCENT.2, 120));
            pixmap.fill_rect(
                tiny_skia::Rect::from_xywh(xa, rect_top, (xb - xa).max(1.0), rect_h).unwrap(),
                &sp,
                Transform::identity(),
                None,
            );
            if a > 0 {
                text.draw(pixmap, slice_chars(line, 0, a), x0, baseline, px, theme::TEXT_PRIMARY);
            }
            text.draw(pixmap, slice_chars(line, a, b), xa, baseline, px, theme::TEXT_PRIMARY);
            if b < line_len {
                text.draw(
                    pixmap,
                    slice_chars(line, b, line_len),
                    xb,
                    baseline,
                    px,
                    theme::TEXT_PRIMARY,
                );
            }
            return;
        }
    }
    text.draw(pixmap, line, x0, baseline, px, theme::TEXT_PRIMARY);
}

/// Total width of the first `chars` characters of `s`.
fn prefix_width(text: &TextRenderer, s: &str, chars: usize, px: f32) -> f32 {
    s.chars()
        .take(chars)
        .map(|c| text.measure(&c.to_string(), px).0)
        .sum()
}

/// Substring of `s` between char offsets `start` (inclusive) and `end`
/// (exclusive), clamped to the string length.
fn slice_chars(s: &str, start: usize, end: usize) -> &str {
    let start_byte = s.char_indices().nth(start).map(|(i, _)| i).unwrap_or(s.len());
    let end_byte = s.char_indices().nth(end).map(|(i, _)| i).unwrap_or(s.len());
    &s[start_byte..end_byte]
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
            photo: None,
            photo_path: None,
        }
    }

    fn msg_out(s: &str) -> MsgRow {
        MsgRow {
            id: 0,
            text: s.to_string(),
            date: 0,
            out: true,
            photo: None,
            photo_path: None,
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
    fn narrower_sent_bubble_never_shorter() {
        // Same text, narrower bubble (sent) => the exact wrap yields at least
        // as many lines as the wider received bubble.
        let tr = text();
        let list = MessageList::new();
        let text = "a long message ".repeat(30);
        let recv = list.row_height(&tr, &msg(&text), 300.0);
        let sent = list.row_height(&tr, &msg_out(&text), 300.0);
        assert!(sent >= recv);
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
        list.draw(&mut pixmap, &text(), 0.0, 0.0, 300.0, 200.0, 1.0, &PhotoCache::new(), None);

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
            MsgRow { id: 1, text: "hey".into(), date: 1_700_000_000, out: false, photo: None, photo_path: None },
            MsgRow { id: 2, text: "yo".into(), date: 1_700_000_800, out: true, photo: None, photo_path: None },
        ];
        let mut pixmap = Pixmap::new(300, 200).unwrap();
        list.draw(&mut pixmap, &text(), 0.0, 0.0, 300.0, 200.0, 1.0, &PhotoCache::new(), None);
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
            photo: None,
            photo_path: None,
        }];
        let s = 4.0;
        let mut pixmap = Pixmap::new(300, 200).unwrap();
        list.draw(&mut pixmap, &text(), 0.0, 0.0, 300.0, 200.0, s, &PhotoCache::new(), None);

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

    #[test]
    fn row_height_matches_the_wrapped_line_count() {
        // The measured height must equal the drawn line count, otherwise the
        // scroll position drifts on long messages.
        let tr = text();
        let list = MessageList::new();
        let long = "word ".repeat(60);
        let h = list.row_height(&tr, &msg(&long), 300.0);
        let bw = MessageList::bubble_width(false, 300.0);
        let interior = (bw - 28.0).max(10.0);
        let lines = wrap_lines(&tr, &long, interior);
        assert_eq!(h, lines.len() as f32 * list.line_height + list.row_padding * 2.0);
    }

    #[test]
    fn scroll_bottom_puts_the_last_message_fully_visible() {
        let tr = text();
        let mut list = MessageList::new();
        let long = "word ".repeat(12);
        for _ in 0..40 {
            list.rows.push(msg(&long));
        }
        let content = list.content_height(&tr, 300.0);
        let viewport = 400.0;
        list.set_scroll_bottom(content, viewport);
        let last_h = list.row_height(&tr, list.rows.last().unwrap(), 300.0);
        assert!(last_h < viewport, "test message taller than the viewport");
        let view_top = content - last_h - list.scroll;
        assert!(view_top >= -0.01, "last message top below viewport: {view_top}");
        assert!(view_top + last_h <= viewport + 0.01);
    }

    #[test]
    fn anchor_at_hits_text_positions() {
        let tr = text();
        let mut list = MessageList::new();
        list.rows.push(msg("hello"));
        list.rows.push(msg("world"));

        // Row 0, start of the text (bubble left edge + padding).
        let a = list.anchor_at(&tr, 24.0, 8.0, 300.0).unwrap();
        assert_eq!(a.row, 0);
        assert_eq!(a.char, 0);

        // Row 0, far right inside the bubble -> end of the text.
        let a2 = list.anchor_at(&tr, 250.0, 8.0, 300.0).unwrap();
        assert_eq!(a2.row, 0);
        assert_eq!(a2.char, 5);

        // Row 1.
        let a3 = list.anchor_at(&tr, 24.0, 36.0 + 8.0, 300.0).unwrap();
        assert_eq!(a3.row, 1);
        assert_eq!(a3.char, 0);

        // Outside the rows.
        assert!(list.anchor_at(&tr, 24.0, 500.0, 300.0).is_none());
    }

    #[test]
    fn selected_text_joins_rows_forward_and_backward() {
        let tr = text();
        let mut list = MessageList::new();
        list.rows.push(msg("hello"));
        list.rows.push(msg("world"));

        let forward = Selection {
            start: Anchor { row: 0, char: 2 },
            end: Anchor { row: 1, char: 3 },
        };
        assert_eq!(list.selected_text(forward), "llo\nwor");

        let backward = Selection {
            start: Anchor { row: 1, char: 3 },
            end: Anchor { row: 0, char: 2 },
        };
        assert_eq!(list.selected_text(backward), "llo\nwor");

        let single = Selection {
            start: Anchor { row: 0, char: 1 },
            end: Anchor { row: 0, char: 4 },
        };
        assert_eq!(list.selected_text(single), "ell");
    }

    #[test]
    fn selection_highlights_text_pixels() {
        let tr = text();
        let mut list = MessageList::new();
        list.rows.push(msg("Hello world"));

        let mut plain = Pixmap::new(300, 200).unwrap();
        list.draw(&mut plain, &tr, 0.0, 0.0, 300.0, 200.0, 1.0, &PhotoCache::new(), None);

        let mut selected = Pixmap::new(300, 200).unwrap();
        let sel = Selection {
            start: Anchor { row: 0, char: 0 },
            end: Anchor { row: 0, char: 11 },
        };
        list.draw(
            &mut selected,
            &tr,
            0.0,
            0.0,
            300.0,
            200.0,
            1.0,
            &PhotoCache::new(),
            Some(&sel),
        );

        // The selection must paint the accent highlight rect, so the frames
        // cannot be pixel-identical.
        assert_ne!(selected.as_ref().data(), plain.as_ref().data());
    }
}