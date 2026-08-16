//! Tiny vector icons drawn with tiny-skia primitives (no icon font).
//! Functions take physical coordinates/sizes (callers scale by `ui_scale`).

use tiny_skia::{Color, Paint, PathBuilder, Pixmap, Stroke, Transform};

fn fill_circle(pixmap: &mut Pixmap, cx: f32, cy: f32, r: f32, color: (u8, u8, u8)) {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba8(color.0, color.1, color.2, 255));
    if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
        pixmap.fill_path(&path, &p, tiny_skia::FillRule::Winding, Transform::identity(), None);
    }
}

fn stroke_path(pixmap: &mut Pixmap, path: &tiny_skia::Path, color: (u8, u8, u8), width: f32) {
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(color.0, color.1, color.2, 255));
    pixmap.stroke_path(
        path,
        &paint,
        &Stroke {
            width,
            ..Default::default()
        },
        Transform::identity(),
        None,
    );
}

fn polyline(pixmap: &mut Pixmap, points: &[(f32, f32)], color: (u8, u8, u8), width: f32) {
    let mut pb = PathBuilder::new();
    if let Some((first, rest)) = points.split_first() {
        pb.move_to(first.0, first.1);
        for p in rest {
            pb.line_to(p.0, p.1);
        }
    }
    if let Some(path) = pb.finish() {
        stroke_path(pixmap, &path, color, width);
    }
}

/// Back chevron (`<`).
pub fn back(pixmap: &mut Pixmap, cx: f32, cy: f32, size: f32, color: (u8, u8, u8)) {
    let w = size * 0.09;
    polyline(pixmap, &[(cx + size / 2.0, cy - size / 2.0), (cx - size / 2.0, cy), (cx + size / 2.0, cy + size / 2.0)], color, w);
}

/// Magnifier (search).
pub fn search(pixmap: &mut Pixmap, cx: f32, cy: f32, size: f32, color: (u8, u8, u8)) {
    let r = size * 0.32;
    fill_circle(pixmap, cx - r * 0.35, cy - r * 0.35, r, color);
    polyline(pixmap, &[(cx + r * 0.4, cy + r * 0.4), (cx + r * 0.95, cy + r * 0.95)], color, size * 0.12);
}

/// Three-dot menu.
pub fn dots(pixmap: &mut Pixmap, cx: f32, cy: f32, size: f32, color: (u8, u8, u8)) {
    let r = size * 0.11;
    let d = size * 0.26;
    for x in [-d, 0.0, d] {
        fill_circle(pixmap, cx + x, cy, r, color);
    }
}

/// Compose (write / new message): a short pen stroke.
pub fn compose(pixmap: &mut Pixmap, cx: f32, cy: f32, size: f32, color: (u8, u8, u8)) {
    let w = size * 0.1;
    polyline(pixmap, &[(cx - size * 0.2, cy + size * 0.3), (cx - size * 0.1, cy - size * 0.3), (cx + size * 0.2, cy - size * 0.3)], color, w);
    polyline(pixmap, &[(cx - size * 0.1, cy - size * 0.3), (cx + size * 0.25, cy + size * 0.3)], color, w * 1.3);
}

/// Paper plane (send).
pub fn send(pixmap: &mut Pixmap, cx: f32, cy: f32, size: f32, color: (u8, u8, u8)) {
    let w = size * 0.12;
    let half = size / 2.0;
    // Plane: triangle + tail.
    polyline(pixmap, &[(cx - half * 0.7, cy - half * 0.8), (cx + half, cy), (cx - half * 0.7, cy + half * 0.8)], color, w);
    polyline(pixmap, &[(cx - half * 0.7, cy - half * 0.05), (cx + half * 0.2, cy - half * 0.05)], color, w);
}

/// Smiley.
pub fn smiley(pixmap: &mut Pixmap, cx: f32, cy: f32, size: f32, color: (u8, u8, u8)) {
    let r = size * 0.42;
    let w = size * 0.08;
    if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
        stroke_path(pixmap, &path, color, w);
    }
    fill_circle(pixmap, cx - r * 0.38, cy - r * 0.2, size * 0.07, color);
    fill_circle(pixmap, cx + r * 0.38, cy - r * 0.2, size * 0.07, color);
    let j = size * 0.07;
    polyline(pixmap, &[
        (cx - r * 0.42, cy + r * 0.1),
        (cx - r * 0.2, cy + r * 0.35),
        (cx + r * 0.2, cy + r * 0.35),
        (cx + r * 0.42, cy + r * 0.1),
    ], color, w);
    let _ = j;
}

/// Paperclip.
pub fn attach(pixmap: &mut Pixmap, cx: f32, cy: f32, size: f32, color: (u8, u8, u8)) {
    let w = size * 0.1;
    polyline(pixmap, &[
        (cx - size * 0.3, cy + size * 0.1),
        (cx + size * 0.12, cy - size * 0.35),
        (cx + size * 0.35, cy - size * 0.12),
        (cx - size * 0.12, cy + size * 0.35),
        (cx - size * 0.28, cy + size * 0.12),
    ], color, w);
}

/// Info (i).
pub fn info(pixmap: &mut Pixmap, cx: f32, cy: f32, size: f32, color: (u8, u8, u8)) {
    let r = size * 0.42;
    let w = size * 0.1;
    if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
        stroke_path(pixmap, &path, color, w);
    }
    fill_circle(pixmap, cx, cy - r * 0.3, size * 0.06, color);
    polyline(pixmap, &[(cx, cy - r * 0.05), (cx, cy + r * 0.35)], color, w);
}

/// Online presence dot (filled circle).
pub fn presence_dot(pixmap: &mut Pixmap, cx: f32, cy: f32, r: f32, color: (u8, u8, u8)) {
    fill_circle(pixmap, cx, cy, r, color);
}

/// Check marks like Telegram's message status: `sent` (single tick) or
/// `read` (double tick). Center `(cx, cy)`, overall `size` in physical px.
pub fn tick(pixmap: &mut Pixmap, cx: f32, cy: f32, size: f32, read: bool, color: (u8, u8, u8)) {
    let w = size * 0.09;
    let x0 = cx - size / 2.0;
    let y0 = cy - size / 2.0;
    // A check slopes from bottom-left to top-right: (0.55, 0.62) -> (0.90,
    // 0.28) shadow... single tick path:
    //   (0.28, 0.62) -> (0.45, 0.78) -> (0.72, 0.30)
    // Double tick = that vertical span shifted left by `gap`.
    let gap = size * 0.26;
    let segs: [&[(f32, f32)]; 2] = [
        &[
            (x0 + size * 0.24, y0 + size * 0.60),
            (x0 + size * 0.44, y0 + size * 0.78),
            (x0 + size * 0.74, y0 + size * 0.28),
        ],
        &[
            (x0 + size * 0.24 - gap, y0 + size * 0.60),
            (x0 + size * 0.44 - gap, y0 + size * 0.78),
            (x0 + size * 0.74 - gap, y0 + size * 0.28),
        ],
    ];
    polyline(pixmap, segs[0], color, w);
    if read {
        polyline(pixmap, segs[1], color, w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> Pixmap {
        let mut p = Pixmap::new(80, 80).unwrap();
        p.fill(Color::from_rgba8(0, 0, 0, 255));
        p
    }

    fn painted(p: &Pixmap) -> usize {
        let mut n = 0;
        for y in 0..p.height() {
            for x in 0..p.width() {
                let px = p.pixel(x, y).unwrap();
                if px.red() > 0 || px.green() > 0 || px.blue() > 0 {
                    n += 1;
                }
            }
        }
        n
    }

    #[test]
    fn icons_render_something() {
        for icon in [
            |p: &mut Pixmap| back(p, 40.0, 40.0, 32.0, (255, 255, 255)),
            |p: &mut Pixmap| search(p, 40.0, 40.0, 24.0, (255, 255, 255)),
            |p: &mut Pixmap| dots(p, 40.0, 40.0, 24.0, (255, 255, 255)),
            |p: &mut Pixmap| compose(p, 40.0, 40.0, 24.0, (255, 255, 255)),
            |p: &mut Pixmap| send(p, 40.0, 40.0, 24.0, (255, 255, 255)),
            |p: &mut Pixmap| smiley(p, 40.0, 40.0, 24.0, (255, 255, 255)),
            |p: &mut Pixmap| attach(p, 40.0, 40.0, 24.0, (255, 255, 255)),
            |p: &mut Pixmap| info(p, 40.0, 40.0, 24.0, (255, 255, 255)),
            |p: &mut Pixmap| tick(p, 40.0, 40.0, 24.0, false, (255, 255, 255)),
            |p: &mut Pixmap| tick(p, 40.0, 40.0, 24.0, true, (255, 255, 255)),
        ] {
            let mut canvas = canvas();
            icon(&mut canvas);
            assert!(painted(&canvas) > 20, "icon painted too little");
        }
    }

    #[test]
    fn read_tick_has_more_pixels_than_single_tick() {
        let mut single = canvas();
        tick(&mut single, 40.0, 40.0, 24.0, false, (255, 255, 255));
        let mut double = canvas();
        tick(&mut double, 40.0, 40.0, 24.0, true, (255, 255, 255));
        assert!(
            painted(&double) > painted(&single) * 3 / 2,
            "double tick should add a second stroke ({} vs {})",
            painted(&double),
            painted(&single)
        );
    }
}