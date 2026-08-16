//! Converts a tiny-skia `Pixmap` (premultiplied RGBA) into the u32 buffer
//! expected by softbuffer (native little-endian RGBX, X11/Wayland format).

use tiny_skia::PixmapRef;

/// Possible errors during a blit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlitError {
    BufferTooSmall { expected: usize, got: usize },
    SizeMismatch { pix: (u32, u32), buf: (u32, u32) },
}

impl core::fmt::Display for BlitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BufferTooSmall { expected, got } => write!(
                f,
                "buffer too small: expected {expected} pixels, got {got}"
            ),
            Self::SizeMismatch { pix, buf } => write!(
                f,
                "size mismatch: pixmap {pix:?}, buffer {buf:?}"
            ),
        }
    }
}

impl std::error::Error for BlitError {}

/// Copies a premultiplied RGBA `Pixmap` into a softbuffer u32 buffer.
///
/// `buffer` must hold at least `width * height` items. Each RGBA pixel
/// `[r,g,b,a]` is written as a native little-endian `u32` `0x00RRGGBB`.
pub fn blit_pixmap(
    pixmap: PixmapRef,
    buffer: &mut [u32],
    width: u32,
    height: u32,
) -> Result<(), BlitError> {
    let expected = (width as usize) * (height as usize);
    if buffer.len() < expected {
        return Err(BlitError::BufferTooSmall {
            expected,
            got: buffer.len(),
        });
    }
    if pixmap.width() != width || pixmap.height() != height {
        return Err(BlitError::SizeMismatch {
            pix: (pixmap.width(), pixmap.height()),
            buf: (width, height),
        });
    }

    for (i, px) in pixmap.data().chunks_exact(4).enumerate() {
        buffer[i] = u32::from_le_bytes([px[2], px[1], px[0], 0x00]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::Pixmap;

    #[test]
    fn blit_converts_rgb_to_0x00rrggbb() {
        let mut pixmap = Pixmap::new(2, 1).unwrap();
        pixmap.fill(tiny_skia::Color::from_rgba8(10, 20, 30, 255));
        let mut buffer = [0u32; 2];

        blit_pixmap(pixmap.as_ref(), &mut buffer, 2, 1).unwrap();

        assert_eq!(buffer, [0x00_0A_14_1E, 0x00_0A_14_1E]);
    }

    #[test]
    fn blit_fills_the_whole_buffer() {
        let mut pixmap = Pixmap::new(3, 3).unwrap();
        pixmap.fill(tiny_skia::Color::from_rgba8(255, 255, 255, 255));
        let mut buffer = [0u32; 9];

        blit_pixmap(pixmap.as_ref(), &mut buffer, 3, 3).unwrap();

        assert!(buffer.iter().all(|&px| px == 0x00_FF_FF_FF));
    }

    #[test]
    fn blit_rejects_a_too_small_buffer() {
        let mut pixmap = Pixmap::new(4, 4).unwrap();
        pixmap.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));
        let mut buffer = [0u32; 10];

        let err = blit_pixmap(pixmap.as_ref(), &mut buffer, 4, 4).unwrap_err();

        assert_eq!(
            err,
            BlitError::BufferTooSmall {
                expected: 16,
                got: 10
            }
        );
    }

    #[test]
    fn blit_rejects_a_mismatched_pixmap_size() {
        let pixmap = Pixmap::new(2, 2).unwrap();
        let mut buffer = [0u32; 4];

        let err = blit_pixmap(pixmap.as_ref(), &mut buffer, 1, 1).unwrap_err();

        assert_eq!(
            err,
            BlitError::SizeMismatch {
                pix: (2, 2),
                buf: (1, 1)
            }
        );
    }
}