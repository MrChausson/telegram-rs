//! Scrollable chat list: pure rendering into a `Pixmap`, testable off-screen.

use tiny_skia::{Color, Paint, Pixmap, Transform};

use crate::text::TextRenderer;
use crate::theme::{self, font, layout};

/// A single chat row.
#[derive(Debug, Clone)]
pub struct ChatRow {
    pub id: i64,
    pub title: String,
    pub subtitle: String,
    /// Unix timestamp of the last message (0 if none).
    pub date: i32,
    pub unread: i32,
}

/// A vertical scrollable list, split into fixed-height rows.
pub struct ChatList {
    pub rows: Vec<ChatRow>,
    pub scroll: f32,
    pub row_height: f32,
    pub padding: f32,
}

pub const AVATAR_PALETTE: [(u8, u8, u8); 6] = [
    (51, 144, 236),
    (40, 130, 200),
    (210, 120, 60),
    (150, 90, 190),
    (190, 80, 120),
    (70, 150, 140),
];

impl Default for ChatList {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatList {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            scroll: 0.0,
            row_height: 64.0,
            padding: 14.0,
        }
    }

    pub fn content_height(&self) -> f32 {
        self.rows.len() as f32 * self.row_height
    }

    /// Scrolls by `dy` (positive = down), clamped to the content.
    pub fn scroll_by(&mut self, dy: f32, viewport_height: f32) {
        let max = (self.content_height() - viewport_height).max(0.0);
        self.scroll = (self.scroll + dy).clamp(0.0, max);
    }

    pub fn set_scroll(&mut self, value: f32, viewport_height: f32) {
        let max = (self.content_height() - viewport_height).max(0.0);
        self.scroll = value.clamp(0.0, max);
    }

    /// Draws the rows into `pixmap` between `(x, y)` and `(x+w, y+h)`
    /// (physical pixels). `scale` multiplies the internal metrics.
    pub fn draw(
        &self,
        pixmap: &mut Pixmap,
        text: &TextRenderer,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        scale: f32,
        selected: Option<i64>,
    ) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        self.fill_bg(pixmap, x, y, w, h);

        let s = scale.max(0.1);
        let row_h = self.row_height * s;
        let n = self.rows.len();
        let first = (self.scroll / self.row_height).floor().max(0.0) as usize;
        for i in first..n {
            let row_top = y + (i as f32 * self.row_height - self.scroll) * s;
            if row_top >= y + h {
                break;
            }
            if row_top + row_h < y {
                continue;
            }
            self.draw_row(pixmap, text, x, row_top, w, row_h, &self.rows[i], s, selected);
        }
    }

    pub fn fill_bg(&self, pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32) {
        let mut bg = Paint::default();
        bg.set_color(Color::from_rgba8(theme::LIST_BG.0, theme::LIST_BG.1, theme::LIST_BG.2, 255));
        pixmap.fill_rect(tiny_skia::Rect::from_xywh(x, y, w, h).unwrap(), &bg, Transform::identity(), None);
    }

    /// Returns the id of the chat under `py` (logical viewport coordinates),
    /// or `None` if outside any row.
    pub fn row_at(&self, y: f32) -> Option<i64> {
        let idx = ((y + self.scroll) / self.row_height).floor() as usize;
        self.rows.get(idx).map(|r| r.id)
    }

    fn draw_row(
        &self,
        pixmap: &mut Pixmap,
        text: &TextRenderer,
        x: f32,
        top: f32,
        w: f32,
        row_h: f32,
        row: &ChatRow,
        s: f32,
        selected: Option<i64>,
    ) {
        // Selected highlight.
        if selected == Some(row.id) {
            let mut sel = Paint::default();
            sel.set_color(Color::from_rgba8(
                theme::ROW_SELECTED.0,
                theme::ROW_SELECTED.1,
                theme::ROW_SELECTED.2,
                255,
            ));
            pixmap.fill_rect(
                tiny_skia::Rect::from_xywh(x, top, w, row_h).unwrap(),
                &sel,
                Transform::identity(),
                None,
            );
        }

        let pad = self.padding * s;

        // Avatar: colored circle + initial (photo avatars are out of MVP).
        let av_size = layout::AVATAR_LIST * s;
        let (cx, cy) = (x + pad + av_size / 2.0, top + row_h / 2.0);
        let idx = (hash(&row.title) as usize) % AVATAR_PALETTE.len();
        let (ar, ag, ab) = AVATAR_PALETTE[idx];
        let mut ap = Paint::default();
        ap.set_color(Color::from_rgba8(ar, ag, ab, 255));
        let circle = tiny_skia::PathBuilder::from_circle(cx, cy, av_size / 2.0).unwrap();
        pixmap.fill_path(&circle, &ap, tiny_skia::FillRule::Winding, Transform::identity(), None);

        if let Some(initial) = row.title.chars().next() {
            let px = font::NAME * s;
            let (tw, _) = text.measure(&initial.to_string(), px);
            text.draw(pixmap, &initial.to_string(), cx - tw / 2.0, cy + 6.0 * s, px, theme::TEXT_PRIMARY);
        }

        let text_x = x + pad * 2.0 + av_size;
        let right_edge = x + w - pad;

        // Name and preview.
        let name_px = font::NAME * s;
        let msg_px = font::MESSAGE * s;
        let ts_px = font::TIMESTAMP * s;

        let name_max = right_edge - text_x - 60.0 * s;
        let name = truncate(text, &row.title, name_max, name_px);
        text.draw(pixmap, &name, text_x, top + 22.0 * s, name_px, theme::TEXT_PRIMARY);

        let sub_max = right_edge - text_x - 8.0 * s;
        let subtitle = truncate(text, &row.subtitle, sub_max, msg_px);
        text.draw(pixmap, &subtitle, text_x, top + 34.0 * s, msg_px, theme::TEXT_SECONDARY);

        // Timestamp (top-right).
        if row.date > 0 {
            let ts = crate::theme::fmt_time(row.date);
            let (tw, _) = text.measure(&ts, ts_px);
            text.draw(pixmap, &ts, right_edge - tw, top + 18.0 * s, ts_px, theme::TEXT_SECONDARY);
        }

        // Unread badge (blue circle, bottom-right).
        if row.unread > 0 {
            let size = layout::BADGE_SIZE * s;
            let bx = right_edge - size / 2.0;
            let by = top + row_h - size / 2.0 - 4.0 * s;
            let count = row.unread.to_string();
            let (cw, _) = text.measure(&count, font::BADGE * s);
            let mut bp = Paint::default();
            bp.set_color(Color::from_rgba8(theme::ACCENT.0, theme::ACCENT.1, theme::ACCENT.2, 255));
            let badge = tiny_skia::PathBuilder::from_circle(bx, by, size / 2.0).unwrap();
            pixmap.fill_path(&badge, &bp, tiny_skia::FillRule::Winding, Transform::identity(), None);
            text.draw(pixmap, &count, bx - cw / 2.0, by + 4.0 * s, font::BADGE * s, theme::TEXT_PRIMARY);
        }
    }
}

/// Simple deterministic hash (not cryptographic) for the avatar color.
fn hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Truncates `s` with an ellipsis to fit within `max_width` px at `px_size`.
fn truncate(text: &TextRenderer, s: &str, max_width: f32, px_size: f32) -> String {
    if text.measure(s, px_size).0 <= max_width {
        return s.to_string();
    }
    let ellipsis = "…";
    let mut out = String::new();
    for ch in s.chars() {
        let candidate = format!("{out}{ch}{ellipsis}");
        if text.measure(&candidate, px_size).0 <= max_width {
            out.push(ch);
        } else {
            break;
        }
    }
    format!("{out}{ellipsis}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text() -> TextRenderer {
        TextRenderer::new()
    }

    fn row(title: &str) -> ChatRow {
        ChatRow {
            id: title.len() as i64,
            title: title.to_string(),
            subtitle: "Last message".to_string(),
            date: 1_700_000_000,
            unread: 3,
        }
    }

    #[test]
    fn scroll_is_clamped_to_content_height() {
        let mut list = ChatList::new();
        list.rows = vec![row("a"), row("b"), row("c"), row("d")];
        list.scroll_by(10_000.0, 100.0);
        // content = 4*64 = 256 ; max scroll = 256 - 100 = 156.
        assert_eq!(list.scroll, 156.0);
    }

    #[test]
    fn negative_scroll_stops_at_zero() {
        let mut list = ChatList::new();
        list.rows = vec![row("a")];
        list.scroll_by(-50.0, 64.0);
        assert_eq!(list.scroll, 0.0);
    }

    #[test]
    fn draw_paints_rows_and_selection() {
        let mut list = ChatList::new();
        list.rows = (0..5).map(|i| row(&format!("Chat {i}"))).collect();
        let mut pixmap = Pixmap::new(300, 200).unwrap();
        list.draw(&mut pixmap, &text(), 0.0, 0.0, 300.0, 200.0, 1.0, Some(6));

        let mut changed = 0;
        for y in 0..200 {
            for x in 0..300 {
                let p = pixmap.pixel(x, y).unwrap();
                if (p.red(), p.green(), p.blue()) != theme::LIST_BG {
                    changed += 1;
                }
            }
        }
        assert!(changed > 500);
    }

    #[test]
    fn row_at_maps_to_the_right_chat() {
        let mut list = ChatList::new();
        list.rows = vec![row("Alpha"), row("Beta")];
        assert_eq!(list.row_at(0.0), Some(5));
        assert_eq!(list.row_at(64.0), Some(4));
        assert_eq!(list.row_at(1000.0), None);
    }

    #[test]
    fn truncate_adds_an_ellipsis() {
        let tr = text();
        let long = "a very long chat title that widely exceeds the allocated width".repeat(2);
        let short = truncate(&tr, &long, 150.0, 16.0);
        assert!(short.ends_with('…'));
        assert!(tr.measure(&short, 16.0).0 <= 150.0 + 1.0);
    }

    #[test]
    fn fmt_time_formats_hh_mm() {
        // Local timezone is machine-dependent; check the shape only.
        let s = theme::fmt_time(1_683_000_000);
        assert_eq!(s.len(), 5);
        assert!(s.as_bytes()[2] == b':');
        assert!(s.as_bytes()[0].is_ascii_digit());
    }
}