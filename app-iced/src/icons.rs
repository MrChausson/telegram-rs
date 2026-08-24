//! Icons rendered with tiny-skia into a `Pixmap`, displayed through the Iced
//! `image` widget.
//!
//! We deliberately avoid the iced `canvas` widget: `iced_tiny_skia 0.14`
//! double-translates geometry of nested canvases, making them invisible. The
//! image path is unaffected, and the shapes mirror the custom `ui` client.

use iced::widget::image;
use iced::Length;

use std::collections::HashMap;
use std::sync::Mutex;

use tiny_skia::{Color, Paint, PathBuilder, Pixmap, Stroke, Transform};

/// Icon kinds, matching the custom client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Back,
    Search,
    Dots,
    Compose,
    Send,
    Info,
    Tick { read: bool },
    Paperclip,
    Reply,
    Forward,
    FileDoc,
    Close,
    Video,
    Audio,
    Gif,
    /// App logo: paper plane on an accent disc.
    Logo,
    /// Pencil (context menu "Modifier").
    Edit,
    /// Two overlapping sheets (context menu "Copier").
    Copy,
    /// Trash bin (context menu "Supprimer").
    Trash,
    /// Filled play triangle (voice notes).
    Play,
    /// Pause bars (voice notes).
    Pause,
    /// Push pin (message "Épingler" / pinned banner).
    Pin,
    /// Plus sign (chat-list header "new group/channel" button).
    Plus,
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
        stroke(pixmap, &path, color, width);
    }
}

fn stroke(pixmap: &mut Pixmap, path: &tiny_skia::Path, color: (u8, u8, u8), width: f32) {
    let mut paint = Paint::default();
    paint.set_color(rgb(color));
    pixmap.stroke_path(
        path,
        &paint,
        &Stroke {
            width,
            line_cap: tiny_skia::LineCap::Round,
            line_join: tiny_skia::LineJoin::Round,            ..Default::default()
        },
        Transform::identity(),
        None,
    );
}

/// Fills a closed polygon (winding rule).
fn fill_polygon(pixmap: &mut Pixmap, points: &[(f32, f32)], color: (u8, u8, u8)) {
    let mut pb = PathBuilder::new();
    if let Some((first, rest)) = points.split_first() {
        pb.move_to(first.0, first.1);
        for p in rest {
            pb.line_to(p.0, p.1);
        }
        pb.close();
    }
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color(rgb(color));
        pixmap.fill_path(
            &path,
            &paint,
            tiny_skia::FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// Fills a triangle given its three corners.
fn fill_triangle(
    pixmap: &mut Pixmap,
    a: (f32, f32),
    b: (f32, f32),
    c: (f32, f32),
    color: (u8, u8, u8),
) {
    fill_polygon(pixmap, &[a, b, c], color);
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
        Icon::Paperclip => {
            // Classic vertical paperclip: outer staple down-around-up, then
            // a short inner leg. Arcs are approximated with dense polylines.
            let w = size * 0.09;
            let r = size * 0.17; // outer half-width
            let ri = size * 0.065; // inner half-width
            let top = cy - size * 0.40;
            let bot = cy + size * 0.24;
            let arc = |ox: f32, oy: f32, rad: f32, a0: f32, a1: f32| {
                let steps = 14;
                (0..=steps)
                    .map(|i| {
                        let a = a0 + (a1 - a0) * i as f32 / steps as f32;
                        (ox + rad * a.cos(), oy + rad * a.sin())
                    })
                    .collect::<Vec<_>>()
            };
            // Right leg down, around the bottom, up the left side.
            let mut pts = vec![(cx + r, top), (cx + r, bot)];
            pts.extend(arc(cx, bot, r, 0.0, std::f32::consts::PI));
            pts.push((cx - r, top + r));
            // Over the top (inner), then a short inner leg back down.
            pts.extend(arc(cx - r + ri, top + r, ri, std::f32::consts::PI, 2.0 * std::f32::consts::PI));
            pts.push((cx - r + 2.0 * ri, cy));
            polyline(pixmap, &pts, color, w);
        }
        Icon::Reply | Icon::Forward => {
            // Reply/forward arrow: filled head + smooth curved tail
            // (Material "reply"/"forward" outline, mirrored for forward).
            let w = size * 0.10;
            let dir: f32 = if kind == Icon::Reply { -1.0 } else { 1.0 };
            let hx = cx + dir * half * 0.75; // head base
            let tip = cx - dir * half * 0.8;
            fill_triangle(
                pixmap,
                (tip, cy),
                (hx, cy - half * 0.42),
                (hx, cy + half * 0.42),
                color,
            );
            // Tail drops from the head base, then sweeps back horizontally.
            let mut pb = PathBuilder::new();
            pb.move_to(hx - dir * w * 0.4, cy - half * 0.30);
            pb.cubic_to(
                hx - dir * w * 0.4,
                cy + half * 0.55,
                cx - dir * size * 0.05,
                cy + half * 0.62,
                cx + dir * half * 0.85,
                cy + half * 0.62,
            );
            if let Some(path) = pb.finish() {
                stroke(pixmap, &path, color, w);
            }
        }
        Icon::FileDoc => {
            // Sheet with a folded corner.
            let w = size * 0.09;
            let x0 = cx - half * 0.62;
            let y0 = cy - half * 0.85;
            let x1 = cx + half * 0.62;
            let y1 = cy + half * 0.85;
            let fold = size * 0.28;
            polyline(
                pixmap,
                &[
                    (x0, y0),
                    (x1 - fold, y0),
                    (x1, y0 + fold),
                    (x1, y1),
                    (x0, y1),
                    (x0, y0),
                ],
                color,
                w,
            );
            polyline(
                pixmap,
                &[(x1 - fold, y0), (x1 - fold, y0 + fold), (x1, y0 + fold)],
                color,
                w * 0.8,
            );
            polyline(
                pixmap,
                &[(x0 + size * 0.14, cy), (x0 + size * 0.52, cy)],
                color,
                w * 0.8,
            );
            polyline(
                pixmap,
                &[(x0 + size * 0.14, cy + size * 0.18), (x0 + size * 0.4, cy + size * 0.18)],
                color,
                w * 0.8,
            );
        }
        Icon::Close => {
            let w = size * 0.1;
            let d = half * 0.65;
            polyline(pixmap, &[(cx - d, cy - d), (cx + d, cy + d)], color, w);
            polyline(pixmap, &[(cx - d, cy + d), (cx + d, cy - d)], color, w);
        }
        Icon::Video => {
            // Film strip: rounded frame + a play triangle in the middle.
            let w = size * 0.07;
            let x0 = cx - half * 0.72;
            let y0 = cy - half * 0.55;
            let x1 = cx + half * 0.72;
            let y1 = cy + half * 0.55;
            polyline(
                pixmap,
                &[(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)],
                color,
                w,
            );
            // Play triangle.
            polyline(
                pixmap,
                &[
                    (cx - half * 0.2, cy - half * 0.28),
                    (cx + half * 0.35, cy),
                    (cx - half * 0.2, cy + half * 0.28),
                ],
                color,
                w * 1.3,
            );
        }
        Icon::Audio => {
            // Musical note: stem + filled head + a beam.
            let w = size * 0.09;
            let x0 = cx - half * 0.5;
            let y0 = cy - half * 0.05;
            let head_r = size * 0.11;
            fill_circle(pixmap, x0, y0 + head_r, head_r, color);
            polyline(pixmap, &[(x0, y0 + head_r), (x0, y0 - size * 0.42)], color, w);
            // Beam curving to the right.
            polyline(
                pixmap,
                &[
                    (x0, y0 - size * 0.42),
                    (x0 + size * 0.42, y0 - size * 0.2),
                ],
                color,
                w,
            );
        }
        Icon::Gif => {
            // Three vertical bars of decreasing height, like a mini 'GIF'.
            let w = size * 0.12;
            let x0 = cx - half * 0.6;
            let y0 = cy;
            let heights = [size * 0.6, size * 0.42, size * 0.72];
            for (i, h) in heights.iter().enumerate() {
                let px = x0 + i as f32 * size * 0.3;
                polyline(
                    pixmap,
                    &[(px, y0 + h * 0.5), (px, y0 - h * 0.5)],
                    color,
                    w,
                );
            }
        }
        Icon::Logo => {
            // App logo: accent disc + white paper plane (classic "send" mark,
            // Material send polygon scaled into the disc).
            fill_circle(pixmap, cx, cy, half, color);
            // Send-plane in a 24-unit box, scaled to 62% of the icon size.
            let s = size * 0.62 / 24.0;
            let bx = cx - 12.0 * s + size * 0.01; // nudge right: plane is asymmetric
            let by = cy - 12.0 * s;
            let p = |x: f32, y: f32| (bx + x * s, by + y * s);
            fill_polygon(
                pixmap,
                &[
                    p(2.0, 21.0),
                    p(23.0, 12.0),
                    p(2.0, 3.0),
                    p(2.0, 10.0),
                    p(16.0, 12.0),
                    p(2.0, 17.0),
                ],
                (255, 255, 255),
            );
        }
        Icon::Edit => {
            // Pencil: rotated body + small tip, stroked (Material "edit").
            let w = size * 0.09;
            let r = half * 0.78; // body half-length
            let t = half * 0.20; // body half-width
            let (sn, cs) = (std::f32::consts::FRAC_PI_4.sin(), std::f32::consts::FRAC_PI_4.cos());
            // Body corners (rotated rectangle), open at the tip end.
            let corner = |dx: f32, dy: f32| (cx + dx * cs - dy * sn, cy + dx * sn + dy * cs);
            polyline(
                pixmap,
                &[
                    corner(-r * 0.55, -t),
                    corner(r, -t),
                    corner(r, t),
                    corner(-r * 0.55, t),
                ],
                color,
                w,
            );
            // Tip triangle closing the bottom-left end.
            let tip_a = corner(-r * 0.55, -t);
            let tip_b = corner(-r * 0.55, t);
            let tip_c = corner(-r - t * 0.4, 0.0);
            polyline(pixmap, &[tip_a, tip_c, tip_b], color, w);
            // Eraser edge on the top-right end.
            let e0 = corner(r, -t);
            let e1 = corner(r, t);
            polyline(pixmap, &[e0, e1], color, w);
        }
        Icon::Copy => {
            // Two overlapping sheets (Material "content_copy"): back sheet
            // shows only its top+left edges, front sheet is a full outline.
            let w = size * 0.09;
            let d = half * 0.32; // back-sheet offset
            let x0 = cx - half * 0.55;
            let y0 = cy - half * 0.55;
            let x1 = cx + half * 0.55;
            let y1 = cy + half * 0.55;
            // Back sheet: top + left edges only.
            polyline(pixmap, &[(x0 + d, y0), (x0, y0 + d)], color, w);
            // Front sheet outline.
            polyline(
                pixmap,
                &[(x0, y0 + d), (x0, y1), (x1 - d, y1), (x1, y1 - d), (x1, y0 + d * 2.0), (x0 + d * 2.0, y0)],
                color,
                w,
            );
        }
        Icon::Trash => {
            // Trash bin (Material "delete"): lid, handle and ribbed body.
            let w = size * 0.09;
            let x0 = cx - half * 0.5;
            let x1 = cx + half * 0.5;
            let y_lid = cy - half * 0.42;
            // Lid.
            polyline(pixmap, &[(x0, y_lid), (x1, y_lid)], color, w);
            // Handle.
            polyline(
                pixmap,
                &[(cx - half * 0.16, y_lid), (cx - half * 0.16, y_lid - half * 0.22), (cx + half * 0.16, y_lid - half * 0.22), (cx + half * 0.16, y_lid)],
                color,
                w,
            );
            // Body (tapered).
            let by0 = y_lid + half * 0.14;
            let by1 = cy + half * 0.72;
            polyline(
                pixmap,
                &[
                    (x0 + half * 0.1, by0),
                    (x0 + half * 0.2, by1),
                    (x1 - half * 0.2, by1),
                    (x1 - half * 0.1, by0),
                ],
                color,
                w,
            );
            // Ribs.
            polyline(pixmap, &[(cx - half * 0.16, by0 + half * 0.1), (cx - half * 0.08, by1 - half * 0.12)], color, w * 0.9);
            polyline(pixmap, &[(cx + half * 0.16, by0 + half * 0.1), (cx + half * 0.08, by1 - half * 0.12)], color, w * 0.9);
        }
        Icon::Play => {
            // Filled play triangle (voice notes), slightly offset right to
            // look optically centered.
            fill_triangle(
                pixmap,
                (cx - half * 0.45, cy - half * 0.7),
                (cx - half * 0.45, cy + half * 0.7),
                (cx + half * 0.75, cy),
                color,
            );
        }
        Icon::Pause => {
            // Two rounded bars (voice notes).
            let w = size * 0.22;
            let d = half * 0.32;
            polyline(pixmap, &[(cx - d, cy - half * 0.55), (cx - d, cy + half * 0.55)], color, w);
            polyline(pixmap, &[(cx + d, cy - half * 0.55), (cx + d, cy + half * 0.55)], color, w);
        }
        Icon::Pin => {
            // Push pin (Material "push_pin" outline): slanted pin head with a
            // hollow inner line and a needle pointing down.
            let w = size * 0.09;
            let head_w = half * 0.52; // half-width of the head
            let head_top = cy - half * 0.72;
            let head_bot = cy - half * 0.05;
            let neck_x = cx + half * 0.18;
            // Head: vertical slab with a chamfered bottom-right (neck).
            polyline(
                pixmap,
                &[
                    (cx - head_w, head_bot),
                    (cx - head_w, head_top),
                    (neck_x, head_top),
                    (neck_x + half * 0.12, head_bot - half * 0.12),
                    (cx - head_w, head_bot),
                ],
                color,
                w,
            );
            // Hollow center line of the head.
            polyline(
                pixmap,
                &[
                    (cx - head_w * 0.45, head_top + half * 0.16),
                    (cx - head_w * 0.45, head_bot - half * 0.14),
                ],
                color,
                w * 0.8,
            );
            // Needle.
            polyline(
                pixmap,
                &[(cx - half * 0.02, head_bot + w * 0.4), (cx - half * 0.02, cy + half * 0.72)],
                color,
                w,
            );
        }
        Icon::Plus => {
            // Material "add": two centered bars.
            let w = size * 0.12;
            polyline(pixmap, &[(cx - half * 0.7, cy), (cx + half * 0.7, cy)], color, w);
            polyline(pixmap, &[(cx, cy - half * 0.7), (cx, cy + half * 0.7)], color, w);
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

/// Renders the app logo (accent disc + paper plane) as a standalone RGBA8
/// pixmap — used by the tray (`ksni::Icon`) and any other embedder.
pub fn render_logo_rgba(px: u32) -> Pixmap {
    render(Icon::Logo, crate::theme::ACCENT, px).0
}

/// Builds an `Element` drawing the given icon, `px` logical size.
///
/// The icon is rasterized at 2× the logical size so it stays crisp on HiDPI
/// displays (the window uses a 1.6× scale factor).
///
/// Handles are **memoized**: `Handle::from_rgba` mints a fresh `Id` per call
/// and the tiny-skia backend keys its raster cache on that `Id`. Without the
/// cache below every icon would be re-rasterized on every frame (scroll lag);
/// a stable handle keeps the entry warm.
pub fn icon<'a, M>(kind: Icon, color: (u8, u8, u8), px: f32) -> iced::Element<'a, M>
where
    M: 'a,
{
    let physical = (px * 2.0).ceil() as u32;
    let handle = cached_handle(kind, color, physical);
    image(handle)
        .width(Length::Fixed(px))
        .height(Length::Fixed(px))
        .into()
}

/// Memoized icon handles, keyed by (kind, color, physical size).
type IconCache = std::sync::OnceLock<Mutex<HashMap<(Icon, (u8, u8, u8), u32), image::Handle>>>;

fn cached_handle(kind: Icon, color: (u8, u8, u8), physical: u32) -> image::Handle {
    static CACHE: IconCache = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    let key = (kind, color, physical);
    if let Some(handle) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return handle;
    }

    let (pixmap, size) = render(kind, color, physical);
    let handle = image::Handle::from_rgba(size, size, pixmap.data().to_vec());
    if let Ok(mut c) = cache.lock() {
        c.insert(key, handle.clone());
    }
    handle
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
            Icon::Paperclip,
            Icon::Reply,
            Icon::Forward,
            Icon::FileDoc,
            Icon::Close,
            Icon::Video,
            Icon::Audio,
            Icon::Gif,
            Icon::Logo,
            Icon::Edit,
            Icon::Copy,
            Icon::Trash,
            Icon::Play,
            Icon::Pause,
            Icon::Plus,
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
