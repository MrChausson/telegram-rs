//! Scrollable chat list: pure rendering into a `Pixmap`, testable off-screen.

use tiny_skia::{Color, Paint, Pixmap, Rect, Transform};

use crate::text::TextRenderer;

/// A single chat row.
#[derive(Debug, Clone)]
pub struct ChatRow {
    pub id: i64,
    pub title: String,
    pub subtitle: String,
    pub unread: i32,
}

/// A vertical scrollable list, split into fixed-height rows.
pub struct ChatList {
    pub rows: Vec<ChatRow>,
    pub scroll: f32,
    pub row_height: f32,
    pub padding: f32,
}

pub const BG: (u8, u8, u8) = (30, 31, 34);
pub const ROW_BG: (u8, u8, u8) = (35, 37, 42);
pub const TEXT_COLOR: (u8, u8, u8) = (236, 239, 244);
pub const SUBTITLE: (u8, u8, u8) = (150, 155, 165);
pub const ACCENT: (u8, u8, u8) = (50, 168, 82);
pub const AVATAR_PALETTE: [(u8, u8, u8); 6] = [
    (50, 168, 82),
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
            padding: 8.0,
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

    /// Draws the list into `pixmap` between `(x, y)` and `(x+w, y+h)` (physical
    /// pixels). `scale` multiplies the internal metrics (rows, text).
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
            self.draw_row(pixmap, text, x, row_top, w, row_h, &self.rows[i], s);
        }
    }

    /// Returns the id of the chat under `py` (viewport coordinates, `y` at the
    /// top of the list), or `None` if outside any row.
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
    ) {
        // Row background.
        let mut bg = Paint::default();
        bg.set_color(Color::from_rgba8(ROW_BG.0, ROW_BG.1, ROW_BG.2, 255));
        pixmap.fill_rect(
            Rect::from_xywh(x, top, w, row_h).unwrap(),
            &bg,
            Transform::identity(),
            None,
        );

        // Avatar: colored circle + initial.
        let pad = self.padding * s;
        let av_size = row_h - pad * 2.0;
        let (cx, cy) = (x + pad + av_size / 2.0, top + row_h / 2.0);
        let idx = (hash(&row.title) as usize) % AVATAR_PALETTE.len();
        let (ar, ag, ab) = AVATAR_PALETTE[idx];
        let mut ap = Paint::default();
        ap.set_color(Color::from_rgba8(ar, ag, ab, 255));
        let circle = tiny_skia::PathBuilder::from_circle(cx, cy, av_size / 2.0).unwrap();
        pixmap.fill_path(&circle, &ap, tiny_skia::FillRule::Winding, Transform::identity(), None);

        // Initial at the center of the avatar.
        if let Some(initial) = row.title.chars().next() {
            let px = 20.0 * s;
            let (tw, _) = text.measure(&initial.to_string(), px);
            text.draw(pixmap, &initial.to_string(), cx - tw / 2.0, cy + 6.0 * s, px, TEXT_COLOR);
        }

        let text_x = x + pad * 2.0 + av_size;

        // Title (truncated) + subtitle.
        let title_max = w - text_x - 16.0 * s;
        let title_px = 16.0 * s;
        let title = truncate(text, &row.title, title_max, title_px);
        text.draw(pixmap, &title, text_x, top + pad + 16.0 * s, title_px, TEXT_COLOR);

        let sub_px = 12.0 * s;
        let subtitle = truncate(text, &row.subtitle, title_max, sub_px);
        text.draw(
            pixmap,
            &subtitle,
            text_x,
            top + pad + 32.0 * s,
            sub_px,
            SUBTITLE,
        );

        // Unread badge.
        if row.unread > 0 {
            let count = row.unread.to_string();
            let (cw, _) = text.measure(&count, sub_px);
            let badge_r = 9.0 * s;
            let bw = (cw + 12.0 * s).max(badge_r * 2.0);
            let bx = x + w - bw - pad;
            let by = top + pad;
            let mut bp = Paint::default();
            bp.set_color(Color::from_rgba8(ACCENT.0, ACCENT.1, ACCENT.2, 255));
            let badge_rect = Rect::from_xywh(bx, by, bw, 18.0 * s).unwrap();
            pixmap.fill_rect(badge_rect, &bp, Transform::identity(), None);
            text.draw(
                pixmap,
                &count,
                bx + (bw - cw) / 2.0,
                by + 13.0 * s,
                sub_px,
                TEXT_COLOR,
            );
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
    fn draw_paints_rows() {
        let mut list = ChatList::new();
        list.rows = (0..5).map(|i| row(&format!("Chat {i}"))).collect();
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
        assert!(changed > 500);
    }

    #[test]
    fn truncate_adds_an_ellipsis() {
        let tr = text();
        let long = "a very long chat title that widely exceeds the allocated width".repeat(2);
        let short = truncate(&tr, &long, 150.0, 16.0);
        assert!(short.ends_with('…'));
        assert!(tr.measure(&short, 16.0).0 <= 150.0 + 1.0);
    }
}