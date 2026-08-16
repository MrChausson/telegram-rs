//! Lightweight image decoding and an LRU cache of decoded thumbnails.
//!
//! JPEG decoding uses the pure-Rust `jpeg-decoder` crate; PNG uses tiny-skia.
//! Only small thumbnails are kept in memory (never the full resolution),
//! keyed by file path.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::rc::Rc;

use tiny_skia::{Pixmap, PixmapPaint, Transform};

/// Maximum decoded thumbnail side kept in memory.
pub const THUMB_MAX: u32 = 256;
/// Maximum number of decoded thumbnails in the LRU cache (256*256*4 ≈ 256 KB
/// each, so the whole cache stays under a few MB).
pub const CACHE_MAX: usize = 12;

/// Transform for `Pixmap::draw_pixmap(x, y, …)`: tiny-skia translates the
/// image's pattern to `(x, y)` but applies the caller's transform around the
/// origin, so `Transform::from_scale` alone would pin the scaled image at
/// `(x·scale, y·scale)` instead of `(x, y)`. Compose the translation so the
/// scaled image keeps its top-left corner at `(x, y)`.
pub fn draw_scale_transform(scale: f32, x: f32, y: f32) -> Transform {
    Transform::from_scale(scale, scale).post_translate(x * (1.0 - scale), y * (1.0 - scale))
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
    dst.draw_pixmap(
        0,
        0,
        src.as_ref(),
        &PixmapPaint::default(),
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