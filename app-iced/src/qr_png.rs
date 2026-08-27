//! QR sign-in rasterization: turns the login payload into a scannable PNG.
//!
//! The matrix comes from `qrcodegen` (zero-dep); the PNG is produced through
//! the same tiny-skia encoder path used by the demo assets, so no extra
//! image-writer dependency is added.

use anyhow::{anyhow, Result};
use qrcodegen::{QrCode, QrCodeEcc};

/// Scanner-friendly framing: white margin of at least 4 modules around the
/// code and 8 device pixels per module.
const QUIET_MODULES: i32 = 4;
const SCALE: usize = 8;

/// Payload a phone must scan to authorize this client:
/// `tg://login?token=` + URL-safe base64 (unpadded) of the raw token bytes
/// (as mandated by Telegram's QR-login spec).
pub fn login_payload(token: &[u8]) -> String {
    format!("tg://login?token={}", b64_url_nopad(token))
}

/// Renders `payload` as a black-on-white PNG (RGBA) with a quiet zone.
pub fn qr_png_bytes(payload: &str) -> Result<Vec<u8>> {
    let code = QrCode::encode_text(payload, QrCodeEcc::Medium)
        .map_err(|_| anyhow!("payload too long for a QR code"))?;
    let size = code.size() as usize;
    let dim = (size + QUIET_MODULES as usize * 2) * SCALE;
    let mut pm = tiny_skia::Pixmap::new(dim as u32, dim as u32).ok_or_else(|| {
        anyhow!("QR pixmap allocation failed ({dim}×{dim})")
    })?;
    // White opaque canvas (scanners need dark-on-light contrast).
    pm.data_mut().fill(255);
    let w = pm.width() as usize;
    for y in 0..size {
        for x in 0..size {
            if !code.get_module(x as i32, y as i32) {
                continue;
            }
            let x0 = (x + QUIET_MODULES as usize) * SCALE;
            let y0 = (y + QUIET_MODULES as usize) * SCALE;
            for py in y0..y0 + SCALE {
                let off = (py * w + x0) * 4;
                for px in 0..SCALE {
                    let o = off + px * 4;
                    pm.data_mut()[o] = 0; // R
                    pm.data_mut()[o + 1] = 0; // G
                    pm.data_mut()[o + 2] = 0; // B
                                             // alpha stays opaque white=255
                }
            }
        }
    }
    Ok(pm.encode_png()?)
}

const B64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// URL-safe base64 without padding (`+/`→`-_`, no trailing `=`).
fn b64_url_nopad(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = ((chunk[0] as u32) << 16)
            | ((*chunk.get(1).unwrap_or(&0) as u32) << 8)
            | (*chunk.get(2).unwrap_or(&0) as u32);
        out.push(B64_URL[(n >> 18 & 63) as usize] as char);
        out.push(B64_URL[(n >> 12 & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_URL[(n >> 6 & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(B64_URL[(n & 63) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_magic_and_dimensions() {
        let token: Vec<u8> = (0u8..=31).collect();
        let png = qr_png_bytes(&login_payload(&token)).expect("png");
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']), "PNG magic");
        assert!(png.len() > 1000, "plausible image size: {}", png.len());
    }

    #[test]
    fn payload_is_url_safe_base64() {
        let token = vec![0xde, 0xad, 0xbe, 0xef, 0x42];
        let p = login_payload(&token);
        let b64 = p
            .strip_prefix("tg://login?token=")
            .expect("payload prefix");
        assert_eq!(b64, "3q2-70I", "known-good unpadded url-safe encoding");
        assert!(b64.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
    }
}
