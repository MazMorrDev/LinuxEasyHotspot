//! QR code → GDK texture. Always dark on white with a 4-module quiet zone,
//! regardless of the app theme, so every phone camera can read it.

use qrcode::{Color, QrCode};
use relm4::gtk::{gdk, glib, prelude::*};

const QUIET: usize = 4;

/// Renders `payload` at roughly `target_px` (rounded down to whole modules).
pub fn texture(payload: &str, target_px: usize) -> Option<gdk::Texture> {
    let code = QrCode::new(payload.as_bytes()).ok()?;
    let w = code.width();
    let total = w + 2 * QUIET;
    let scale = (target_px / total).max(2);
    let size = total * scale;
    let colors = code.to_colors();
    let mut buf = vec![255u8; size * size * 3];
    for y in 0..w {
        for x in 0..w {
            if colors[y * w + x] != Color::Dark {
                continue;
            }
            for dy in 0..scale {
                let row = ((y + QUIET) * scale + dy) * size;
                let start = (row + (x + QUIET) * scale) * 3;
                buf[start..start + scale * 3].fill(0);
            }
        }
    }
    let tex = gdk::MemoryTexture::new(
        size as i32,
        size as i32,
        gdk::MemoryFormat::R8g8b8,
        &glib::Bytes::from_owned(buf),
        size * 3,
    );
    Some(tex.upcast())
}
