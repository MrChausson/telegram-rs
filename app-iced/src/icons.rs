//! Icons rendered with tiny-skia into a `Pixmap`, displayed through the Iced
//! `image` widget.
//!
//! We deliberately avoid the iced `canvas` widget: `iced_tiny_skia 0.14`
//! double-translates geometry of nested canvases, making them invisible. The
//! image path is unaffected, and the shapes mirror the custom `ui` client.

use iced::widget::image;
use iced::Length;

use tiny_skia::{Color, Paint, PathBuilder, Pixmap, Stroke, Transform};

/// Icon kinds, matching the custom client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Back,
    Search,
    Dots,
    Compose,
    Send,
    Info,
    Tick { read: bool },
}

fn rgb(c: (u8, u8, u8)) -> Color {
    Color::from_rgba8(c.0, c.1, c.2, 255)
}

fn fill_circle(pixmap: &mut Pixmap, cx: f32, cy: f32, r: f32, color: (u8, u8, u8)) {
    let mut p = Paint::default();
    p.set_color(rgb(color));
    if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
        pixmap.fill_path(
            &path,
            &p,
            tiny_skia::FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
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
        let mut paint = Paint::default();
        paint.set_color(rgb(color));
        pixmap.stroke_path(
            &path,
            &paint,
            &Stroke {
                width,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }
}

fn draw_icon(pixmap: &mut Pixmap, kind: Icon, cx: f32, cy: f32, size: f32, color: (u8, u8, u8)) {
    let half = size / 2.0;
    match kind {
        Icon::Back => {
            let w = size * 0.09;
            polyline(
                pixmap,
                &[
                    (cx + half, cy - half),
                    (cx - half, cy),
                    (cx + half, cy + half),
                ],
                color,
                w,
            );
        }
        Icon::Search => {
            let r = size * 0.32;
            fill_circle(pixmap, cx - r * 0.35, cy - r * 0.35, r, color);
            polyline(
                pixmap,
                &[(cx + r * 0.4, cy + r * 0.4), (cx + r * 0.95, cy + r * 0.95)],
                color,
                size * 0.12,
            );
        }
        Icon::Dots => {
            let r = size * 0.11;
            let d = size * 0.26;
            for x in [-d, 0.0, d] {
                fill_circle(pixmap, cx + x, cy, r, color);
            }
        }
        Icon::Compose => {
            let w = size * 0.1;
            polyline(
                pixmap,
                &[
                    (cx - size * 0.2, cy + size * 0.3),
                    (cx - size * 0.1, cy - size * 0.3),
                    (cx + size * 0.2, cy - size * 0.3),
                ],
                color,
                w,
            );
            polyline(
                pixmap,
                &[(cx - size * 0.1, cy - size * 0.3), (cx + size * 0.25, cy + size * 0.3)],
                color,
                w * 1.3,
            );
        }
        Icon::Send => {
            let w = size * 0.12;
            polyline(
                pixmap,
                &[
                    (cx - half * 0.7, cy - half * 0.8),
                    (cx + half, cy),
                    (cx - half * 0.7, cy + half * 0.8),
                ],
                color,
                w,
            );
            polyline(
                pixmap,
                &[(cx - half * 0.7, cy - half * 0.05), (cx + half * 0.2, cy - half * 0.05)],
                color,
                w,
            );
        }
        Icon::Info => {
            let r = size * 0.42;
            let w = size * 0.1;
            if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
                let mut paint = Paint::default();
                paint.set_color(rgb(color));
                pixmap.stroke_path(
                    &path,
                    &paint,
                    &Stroke {
                        width: w,
                        ..Default::default()
                    },
                    Transform::identity(),
                    None,
                );
            }
            fill_circle(pixmap, cx, cy - r * 0.3, size * 0.06, color);
            polyline(pixmap, &[(cx, cy - r * 0.05), (cx, cy + r * 0.35)], color, w);
        }
        Icon::Tick { read } => {
            let w = size * 0.09;
            let x0 = cx - half;
            let y0 = cy - half;
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
    }
}

/// Renders an icon into a `Pixmap` and returns its size (in pixels).
fn render(kind: Icon, color: (u8, u8, u8), px: u32) -> (Pixmap, u32) {
    let size = px as f32;
    let mut pixmap = Pixmap::new(px, px).expect("icon pixmap");
    draw_icon(&mut pixmap, kind, size / 2.0, size / 2.0, size, color);
    (pixmap, px)
}

/// Builds an `Element` drawing the given icon, `px` logical size.
///
/// The icon is rasterized at 2× the logical size so it stays crisp on HiDPI
/// displays (the window uses a 1.6× scale factor).
pub fn icon<'a, M>(kind: Icon, color: (u8, u8, u8), px: f32) -> iced::Element<'a, M>
where
    M: 'a,
{
    let physical = (px * 2.0).ceil() as u32;
    let (pixmap, size) = render(kind, color, physical);
    let handle = image::Handle::from_rgba(size, size, pixmap.data().to_vec());
    image(handle)
        .width(Length::Fixed(px))
        .height(Length::Fixed(px))
        .into()
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
        for kind in [
            Icon::Back,
            Icon::Search,
            Icon::Dots,
            Icon::Compose,
            Icon::Send,
            Icon::Info,
            Icon::Tick { read: false },
            Icon::Tick { read: true },
        ] {
            let mut c = canvas();
            draw_icon(&mut c, kind, 40.0, 40.0, 24.0, (255, 255, 255));
            assert!(painted(&c) > 20, "{kind:?} painted too little");
        }
    }

    #[test]
    fn read_tick_has_more_pixels_than_single_tick() {
        let mut single = canvas();
        draw_icon(&mut single, Icon::Tick { read: false }, 40.0, 40.0, 24.0, (255, 255, 255));
        let mut double = canvas();
        draw_icon(&mut double, Icon::Tick { read: true }, 40.0, 40.0, 24.0, (255, 255, 255));
        assert!(
            painted(&double) > painted(&single) * 3 / 2,
            "double tick should add a second stroke ({} vs {})",
            painted(&double),
            painted(&single)
        );
    }
}
