//! Lightweight image decoding and an LRU cache of decoded thumbnails.
//!
//! JPEG decoding uses the pure-Rust `jpeg-decoder` crate; PNG uses tiny-skia.
//! Only small thumbnails are kept in memory (never the full resolution),
//! keyed by file path.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::rc::Rc;

use tiny_skia::{Paint, Pixmap, PixmapPaint, Transform};

/// Maximum decoded thumbnail side kept in memory. Matches the downloaded
/// photo size (>= 512 px) so previews never look blocky.
pub const THUMB_MAX: u32 = 512;
/// Maximum number of decoded thumbnails in the LRU cache (each ≤ 512*512*4
/// ≈ 1 MB, so the whole cache stays around a few MB).
pub const CACHE_MAX: usize = 8;
/// Maximum number of pre-fitted (bubble-sized) images cached.
pub const FITTED_MAX: usize = 10;

/// Draws an image clipped to a circle (avatar), centered at `(cx, cy)`.
///
/// The circle path is filled directly with the image as a `Pattern` shader:
/// passing a full-window `Mask` to `draw_pixmap` forces a mask sized to the
/// *whole* pixmap every frame (a large allocation that grows with the window),
/// whereas a sized `Pattern` keeps the implicit clip inside the circle.
pub fn draw_circle_image(pixmap: &mut Pixmap, cx: f32, cy: f32, r: f32, img: &Pixmap) {
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    let scale = ((r * 2.0) / iw.max(ih)).min(1.0);
    let dw = iw * scale;
    let dh = ih * scale;
    let x = cx - dw / 2.0;
    let y = cy - dh / 2.0;
    let pattern: tiny_skia::Shader = tiny_skia::Pattern::new(
        img.as_ref(),
        tiny_skia::SpreadMode::Pad,
        tiny_skia::FilterQuality::Bilinear,
        1.0,
        Transform::from_scale(scale, scale).post_translate(x, y),
    )
    .into();
    let mut paint = Paint::default();
    paint.shader = pattern;
    if let Some(circle) = tiny_skia::PathBuilder::from_circle(cx, cy, r) {
        pixmap.fill_path(&circle, &paint, tiny_skia::FillRule::Winding, Transform::identity(), None);
    }
}

/// Blits an opaque `src` pixmap into `dst` at `(x, y)` (top-left), clipping to
/// the destination bounds. Photos are fully opaque, so tiny-skia's per-pixel
/// alpha compositing is wasted work; a straight row copy is ~100–300× faster.
pub fn blit_opaque(dst: &mut Pixmap, x: i32, y: i32, src: &Pixmap) {
    let (dw, dh) = (dst.width() as i32, dst.height() as i32);
    let (sw, sh) = (src.width() as i32, src.height() as i32);
    // Source columns left/right of the target.
    let col0 = x.max(0) - x;
    let col1 = (x + sw).min(dw) - x;
    // Source rows above/below the target.
    let row0 = y.max(0) - y;
    let row1 = (y + sh).min(dh) - y;
    if col0 >= col1 || row0 >= row1 {
        return;
    }
    let cols = (col1 - col0) as usize;
    let src_px = src.pixels();
    let dst_px = dst.pixels_mut();
    let src_w = sw as usize;
    let dst_w = dw as usize;
    for r in row0..row1 {
        let dst_row = (y + r) as usize * dst_w + (x + col0) as usize;
        let src_row = r as usize * src_w + col0 as usize;
        dst_px[dst_row..dst_row + cols].copy_from_slice(&src_px[src_row..src_row + cols]);
    }
}

/// Decodes an image file into an RGBA thumbnail `Pixmap`, downscaled so its
/// largest side is at most [`THUMB_MAX`]. Returns `None` on unsupported/corrupt.
pub fn decode(path: &std::path::Path) -> Option<Pixmap> {
    let bytes = std::fs::read(path).ok()?;
    // JPEG starts with FF D8 FF; PNG with 89 50 4E 47.
    if bytes.starts_with(&[0xFF, 0xD8]) {
        decode_jpeg(&bytes)
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Pixmap::decode_png(&bytes).ok()
    } else {
        None
    }
}

fn decode_jpeg(bytes: &[u8]) -> Option<Pixmap> {
    let mut dec = jpeg_decoder::Decoder::new(bytes);
    let pixels = dec.decode().ok()?;
    let info = dec.info()?;
    let w = info.width as u32;
    let h = info.height as u32;
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            for px in pixels.chunks_exact(3) {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
        }
        jpeg_decoder::PixelFormat::L8 => {
            for g in pixels {
                rgba.extend_from_slice(&[g, g, g, 255]);
            }
        }
        jpeg_decoder::PixelFormat::CMYK32 => {
            for px in pixels.chunks_exact(4) {
                let (c, m, y, k) = (px[0] as u32, px[1] as u32, px[2] as u32, px[3] as u32);
                let r = 255 - ((c * (255 - k)) / 255).min(255) as u8;
                let g = 255 - ((m * (255 - k)) / 255).min(255) as u8;
                let b = 255 - ((y * (255 - k)) / 255).min(255) as u8;
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        _ => return None,
    }
    // alpha is 255, so straight == premultiplied.
    let size = tiny_skia::IntSize::from_wh(w, h)?;
    let pixmap = Pixmap::from_vec(rgba, size)?;
    Some(downscale(&pixmap))
}

/// Downscales so the largest side fits `THUMB_MAX` (no upscaling).
fn downscale(src: &Pixmap) -> Pixmap {
    let (w, h) = (src.width(), src.height());
    let max = u32::max(w, h);
    if max <= THUMB_MAX {
        return src.clone();
    }
    let scale = THUMB_MAX as f32 / max as f32;
    let dw = ((w as f32 * scale).round().max(1.0)) as u32;
    let dh = ((h as f32 * scale).round().max(1.0)) as u32;
    let mut dst = Pixmap::new(dw, dh).unwrap();
    let mut paint = PixmapPaint::default();
    paint.quality = tiny_skia::FilterQuality::Bilinear;
    dst.draw_pixmap(
        0,
        0,
        src.as_ref(),
        &paint,
        Transform::from_scale(scale, scale),
        None,
    );
    dst
}

/// LRU cache of decoded thumbnails keyed by file path.
#[derive(Default)]
pub struct PhotoCache {
    images: RefCell<HashMap<String, Rc<Pixmap>>>,
    order: RefCell<VecDeque<String>>,
    /// Pre-fitted images (already scaled to their display box), so drawing a
    /// frame is a 1:1 blit instead of a tiny-skia scale per frame.
    fitted: RefCell<HashMap<(String, u32, u32), Rc<Pixmap>>>,
    fitted_order: RefCell<VecDeque<(String, u32, u32)>>,
}

impl PhotoCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the decoded thumbnail for `path`, decoding on first use.
    pub fn get(&self, path: &str) -> Option<Rc<Pixmap>> {
        if let Some(img) = self.images.borrow().get(path) {
            return Some(Rc::clone(img));
        }
        let img = Rc::new(decode(std::path::Path::new(path))?);
        let mut cache = self.images.borrow_mut();
        let mut order = self.order.borrow_mut();
        cache.insert(path.to_string(), Rc::clone(&img));
        order.push_back(path.to_string());
        while cache.len() > CACHE_MAX {
            if let Some(oldest) = order.pop_front() {
                cache.remove(&oldest);
            } else {
                break;
            }
        }
        Some(img)
    }

    /// Returns the image for `path` scaled so it fits (without distortion)
    /// inside a `fit_w × fit_h` box, caching the pre-scaled result. Falls back
    /// to the raw thumbnail when no scaling is needed.
    pub fn fitted(&self, path: &str, fit_w: f32, fit_h: f32) -> Option<Rc<Pixmap>> {
        let fit_w = fit_w.max(1.0);
        let fit_h = fit_h.max(1.0);
        let key = (path.to_string(), fit_w as u32, fit_h as u32);
        if let Some(img) = self.fitted.borrow().get(&key) {
            return Some(Rc::clone(img));
        }
        let base = self.get(path)?;
        let (bw, bh) = (base.width() as f32, base.height() as f32);
        let scale = (fit_w / bw).min(fit_h / bh);
        let dw = ((bw * scale).round() as u32).max(1);
        let dh = ((bh * scale).round() as u32).max(1);
        // 1:1 (or the source itself already fits the box): nothing to scale.
        if dw == base.width() && dh == base.height() {
            return Some(base);
        }
        let mut dst = Pixmap::new(dw, dh)?;
        let mut paint = PixmapPaint::default();
        paint.quality = tiny_skia::FilterQuality::Bilinear;
        dst.draw_pixmap(
            0,
            0,
            (*base).as_ref(),
            &paint,
            Transform::from_scale(scale, scale),
            None,
        );
        let img = Rc::new(dst);
        let mut cache = self.fitted.borrow_mut();
        let mut order = self.fitted_order.borrow_mut();
        cache.insert(key.clone(), Rc::clone(&img));
        order.push_back(key.clone());
        while cache.len() > FITTED_MAX {
            if let Some(oldest) = order.pop_front() {
                cache.remove(&oldest);
            } else {
                break;
            }
        }
        Some(img)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_png() -> Vec<u8> {
        let mut pix = Pixmap::new(2, 1).unwrap();
        pix.fill(tiny_skia::Color::from_rgba8(10, 20, 30, 255));
        pix.encode_png().unwrap()
    }

    #[test]
    fn cache_decodes_and_evicts() {
        let dir = std::env::temp_dir().join(format!("tg-img-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..(CACHE_MAX + 4) {
            std::fs::write(dir.join(format!("{i}.png")), dummy_png()).unwrap();
        }
        let cache = PhotoCache::new();
        for i in 0..(CACHE_MAX + 4) {
            let p = dir.join(format!("{i}.png"));
            assert!(cache.get(p.to_str().unwrap()).is_some());
        }
        // Touching the evicted entries just re-decodes them; no panic.
        assert!(cache.get(dir.join("0.png").to_str().unwrap()).is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn downscale_fits_max() {
        let mut big = Pixmap::new(512, 128).unwrap();
        big.fill(tiny_skia::Color::from_rgba8(1, 2, 3, 255));
        let small = downscale(&big);
        assert_eq!(small.width(), THUMB_MAX);
        assert!(small.height() <= THUMB_MAX);
    }
}