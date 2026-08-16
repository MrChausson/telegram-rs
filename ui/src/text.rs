//! Pure text rendering via `fontdue` (CPU rasterization), composited into a
//! tiny-skia `Pixmap` without per-glyph intermediate allocations.

use std::cell::RefCell;
use std::collections::HashMap;

use fontdue::{Font, FontSettings};
use tiny_skia::{Pixmap, PremultipliedColorU8};

pub const FONT_BYTES: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

/// Typographic normalization: the straight apostrophe is rendered by
/// Liberation Sans with a loop that looks like a "9"; we replace it with
/// the curly apostrophe (more readable).
fn normalize(ch: char) -> char {
    match ch {
        '\u{0027}' => '\u{2019}',
        c => c,
    }
}

/// Text rendering on a given baseline, with optional fallback fonts
/// (e.g. for non-Latin scripts), and a glyph cache to avoid re-rasterizing
/// the font every frame.
pub struct TextRenderer {
    fonts: Vec<Font>,
    cache: RefCell<HashMap<(char, u32), CachedGlyph>>,
}

struct CachedGlyph {
    metrics: fontdue::Metrics,
    coverage: Vec<u8>,
}

const CACHE_MAX: usize = 2048;

impl TextRenderer {
    /// Loads the embedded font (and, in the future, fallback fonts).
    pub fn new() -> Self {
        let mut fonts = Vec::with_capacity(2);
        fonts.push(
            Font::from_bytes(FONT_BYTES.to_vec(), FontSettings::default())
                .expect("invalid embedded font"),
        );
        Self {
            fonts,
            cache: RefCell::new(HashMap::new()),
        }
    }

    /// Adds a fallback font (used when the primary font lacks a glyph).
    pub fn add_fallback(&mut self, bytes: &'static [u8]) -> bool {
        if let Ok(font) = Font::from_bytes(bytes.to_vec(), FontSettings::default()) {
            self.fonts.push(font);
            true
        } else {
            false
        }
    }

    /// The font that has `ch`, or the primary font by default.
    fn glyph_font(&self, ch: char) -> &Font {
        self.fonts
            .iter()
            .find(|f| f.has_glyph(ch))
            .unwrap_or(&self.fonts[0])
    }

    /// Glyph rasterization (cached per (character, size)).
    fn cached_glyph(&self, ch: char, px: f32) -> (fontdue::Metrics, Vec<u8>) {
        let key = (ch, (px * 4.0).round() as u32);
        if let Some(g) = self.cache.borrow().get(&key) {
            return (g.metrics, g.coverage.clone());
        }
        let font = self.glyph_font(ch);
        let (metrics, coverage) = font.rasterize(ch, px);
        let mut cache = self.cache.borrow_mut();
        if cache.len() >= CACHE_MAX {
            cache.clear();
        }
        cache.insert(key, CachedGlyph { metrics, coverage: coverage.clone() });
        (metrics, coverage)
    }

    /// Width (in px) of a string and line height at size `px`.
    pub fn measure(&self, s: &str, px: f32) -> (f32, f32) {
        let width: f32 = s
            .chars()
            .map(normalize)
            .map(|c| self.cached_glyph(c, px).0.advance_width)
            .sum();
        let height = self
            .fonts[0]
            .horizontal_line_metrics(px)
            .map(|m| m.new_line_size)
            .unwrap_or(px);
        (width, height)
    }

    /// Draws `s` into `pixmap`, the baseline starting at `(x, y)`.
    pub fn draw(&self, pixmap: &mut Pixmap, s: &str, x: f32, y: f32, px: f32, color: (u8, u8, u8)) {
        let mut pen_x = x;
        for ch in s.chars().map(normalize) {
            let (metrics, coverage) = self.cached_glyph(ch, px);
            // `ymin` is the offset of the glyph's bottom edge from the baseline
            // (negative = descent below the baseline). In screen coordinates
            // (y grows downward): bottom = baseline − ymin. Round-bottomed
            // glyphs (a, e, s, u, t, G…) show an AA artifact at `ymin = -1`;
            // we snap them to the baseline (vertical hinting) without touching
            // real descenders (g, p, q…).
            let ymin = if metrics.ymin == -1 { 0 } else { metrics.ymin };
            put_glyph(
                pixmap,
                pen_x.round() as i32 + metrics.xmin,
                y.round() as i32 - ymin - metrics.height as i32,
                metrics.width,
                metrics.height,
                &coverage,
                color,
            );
            pen_x += metrics.advance_width;
        }
    }
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

/// Composits a glyph source-over, full color with alpha = coverage.
fn put_glyph(
    target: &mut Pixmap,
    x: i32,
    y: i32,
    width: usize,
    height: usize,
    coverage: &[u8],
    color: (u8, u8, u8),
) {
    let tgt_w = target.width();
    let tgt_h = target.height();
    if x >= tgt_w as i32 || y >= tgt_h as i32 {
        return;
    }
    let px = target.pixels_mut();
    for row in 0..height {
        for col in 0..width {
            let cov = coverage[row * width + col];
            if cov == 0 {
                continue;
            }
            let tx = x + col as i32;
            let ty = y + row as i32;
            if tx < 0 || ty < 0 || tx >= tgt_w as i32 || ty >= tgt_h as i32 {
                continue;
            }
            let idx = (ty as usize) * tgt_w as usize + tx as usize;
            let sa = cov as u16;
            let inv = 255 - sa;
            let dst = px[idx];
            let r = (color.0 as u16 * sa + dst.red() as u16 * inv) / 255;
            let g = (color.1 as u16 * sa + dst.green() as u16 * inv) / 255;
            let b = (color.2 as u16 * sa + dst.blue() as u16 * inv) / 255;
            let a = (sa + dst.alpha() as u16 * inv / 255) as u8;
            px[idx] =
                PremultipliedColorU8::from_rgba(r as u8, g as u8, b as u8, a).expect("color");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::Pixmap;

    #[test]
    fn measure_returns_a_non_zero_width() {
        let tr = TextRenderer::new();
        let (w, h) = tr.measure("hello", 16.0);
        assert!(w > 0.0);
        assert!(h > 0.0);
    }

    #[test]
    fn measure_grows_with_the_number_of_characters() {
        let tr = TextRenderer::new();
        let (w1, _) = tr.measure("a", 16.0);
        let (w2, _) = tr.measure("aaa", 16.0);
        assert!(w2 > w1);
    }

    #[test]
    fn straight_and_curly_apostrophes_render_identically() {
        // U+0027 is normalized to U+2019: the rendering must be identical.
        let tr = TextRenderer::new();
        let mut a = Pixmap::new(120, 60).unwrap();
        let mut b = Pixmap::new(120, 60).unwrap();
        a.fill(tiny_skia::Color::from_rgba8(30, 31, 34, 255));
        b.fill(tiny_skia::Color::from_rgba8(30, 31, 34, 255));
        tr.draw(&mut a, "c'est", 10.0, 40.0, 32.0, (255, 255, 255));
        tr.draw(&mut b, "c\u{2019}est", 10.0, 40.0, 32.0, (255, 255, 255));
        assert_eq!(a.as_ref().data(), b.as_ref().data());
    }

    #[test]
    fn draw_paints_non_background_pixels() {
        let tr = TextRenderer::new();
        let mut pixmap = Pixmap::new(200, 100).unwrap();
        pixmap.fill(tiny_skia::Color::from_rgba8(30, 31, 34, 255));

        tr.draw(&mut pixmap, "Test", 20.0, 60.0, 24.0, (255, 255, 255));

        let mut changed = 0;
        for y in 0..100 {
            for x in 0..200 {
                let p = pixmap.pixel(x, y).unwrap();
                if (p.red(), p.green(), p.blue()) != (30, 31, 34) {
                    changed += 1;
                }
            }
        }
        assert!(changed > 100, "few text pixels painted: {changed}");
    }

    #[test]
    fn draw_produces_a_self_stable_render() {
        let tr = TextRenderer::new();
        let mut pixmap = Pixmap::new(200, 100).unwrap();
        pixmap.fill(tiny_skia::Color::from_rgba8(30, 31, 34, 255));
        tr.draw(&mut pixmap, "tg phase 2", 20.0, 60.0, 24.0, (255, 255, 255));

        let bytes = pixmap.encode_png().unwrap();
        let reloaded = Pixmap::decode_png(&bytes).unwrap();
        assert_eq!(reloaded.as_ref().data(), pixmap.as_ref().data());
    }

    #[test]
    fn baseline_aligns_the_bottom_edge_of_an_x() {
        let tr = TextRenderer::new();
        let baseline = 50.0;
        let mut pixmap = Pixmap::new(120, 80).unwrap();
        pixmap.fill(tiny_skia::Color::from_rgba8(30, 31, 34, 255));
        tr.draw(&mut pixmap, "x", 10.0, baseline, 40.0, (255, 255, 255));

        let mut bottom = 0usize;
        for y in 0..80 {
            for x in 0..120 {
                let p = pixmap.pixel(x, y).unwrap();
                if (p.red(), p.green(), p.blue()) != (30, 31, 34) {
                    bottom = y as usize;
                }
            }
        }
        assert_eq!(bottom, baseline as usize - 1);
    }

    #[test]
    fn lowercase_g_has_a_descender_loop() {
        // The original bug: fontdue rasterized Liberation Sans's 'g' as a '9'
        // (no descender loop). With DejaVu, 'g' must have a wide loop below
        // the baseline, unlike '9' (thin tail).
        let tr = TextRenderer::new();
        let (_, hg) = tr.measure("g", 32.0);
        let baseline = 40.0;
        let mut pg = Pixmap::new(60, 70).unwrap();
        let mut p9 = Pixmap::new(60, 70).unwrap();
        pg.fill(tiny_skia::Color::from_rgba8(30, 31, 34, 255));
        p9.fill(tiny_skia::Color::from_rgba8(30, 31, 34, 255));
        tr.draw(&mut pg, "g", 10.0, baseline, 32.0, (255, 255, 255));
        tr.draw(&mut p9, "9", 10.0, baseline, 32.0, (255, 255, 255));

        // Max ink width in the area below the baseline (rows > baseline).
        let bg = (30u8, 31u8, 34u8);
        let is_ink = |p: &Pixmap, x: u32, y: u32| -> bool {
            let px = p.pixel(x, y).unwrap();
            (px.red(), px.green(), px.blue()) != bg
        };
        let span = |p: &Pixmap| -> u32 {
            let mut max = 0u32;
            for y in (baseline as u32)..p.height() {
                let row_span = (0..p.width()).filter(|&x| is_ink(p, x, y)).count() as u32;
                max = max.max(row_span);
            }
            max
        };
        let g_span = span(&pg);
        let nine_span = span(&p9);
        // 'g' has a wide loop; '9' has a thin tail.
        assert!(g_span >= 4, "g without a descender loop (span={g_span})");
        assert!(g_span > nine_span, "g({g_span}) must be wider than 9({nine_span})");
        let _ = hg;
    }

    #[test]
    fn all_glyphs_share_the_same_baseline() {
        let tr = TextRenderer::new();
        let baseline = 60.0;
        for ch in ['x', 'P', 'h', 'a', 'e', 's', 'u', 't', 'G', 'n'] {
            let mut pixmap = Pixmap::new(60, 80).unwrap();
            pixmap.fill(tiny_skia::Color::from_rgba8(30, 31, 34, 255));
            tr.draw(&mut pixmap, &ch.to_string(), 10.0, baseline, 32.0, (255, 255, 255));

            let mut bottom = 0usize;
            for y in 0..80 {
                for x in 0..60 {
                    let p = pixmap.pixel(x, y).unwrap();
                    if (p.red(), p.green(), p.blue()) != (30, 31, 34) {
                        bottom = y as usize;
                    }
                }
            }
            assert_eq!(
                bottom,
                baseline as usize - 1,
                "glyph {ch:?} not aligned on the baseline"
            );
        }
    }
}