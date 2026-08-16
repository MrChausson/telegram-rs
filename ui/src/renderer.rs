//! Pure rendering of the scene into a `Pixmap` (no window dependency).
//! Testable off-screen: render a frame and inspect/compare pixels.
//!
//! Split-pane layout: a fixed chat-list column on the left and the open
//! conversation (or a placeholder) on the right, like Telegram Desktop.

use tiny_skia::{Color, FillRule, Paint, Pixmap, Transform};

use crate::chatlist::ChatList;
use crate::icons;
use crate::image::PhotoCache;
use crate::messages::{MessageList, Selection};
use crate::state::{LoginStep, Screen};
use crate::text::TextRenderer;
use crate::theme::{self, font, layout};

/// Snapshot of the sign-in screen, drawn instead of the panes while the
/// account is not authenticated yet.
pub struct LoginView<'a> {
    pub step: &'a LoginStep,
    pub input: &'a str,
    pub status: &'a str,
    pub error: bool,
}

/// Draws the scene into the given `Pixmap`.
///
/// Returns `Err` if the pixmap is empty (zero width or height).
pub fn render(
    pixmap: &mut Pixmap,
    text: &TextRenderer,
    list: &ChatList,
    screen: &Screen,
    messages: &MessageList,
    status: &str,
    input: &str,
    viewer: Option<&str>,
    photos: &PhotoCache,
    login: Option<LoginView<'_>>,
    selection: Option<&Selection>,
    scale: f32,
) -> Result<(), &'static str> {
    if pixmap.width() == 0 || pixmap.height() == 0 {
        return Err("empty pixmap");
    }
    pixmap.fill(color(theme::CHAT_BG));
    if let Some(path) = viewer {
        draw_viewer_overlay(pixmap, text, photos, path, pixmap.width(), pixmap.height());
        return Ok(());
    }
    if let Some(l) = login {
        draw_login(pixmap, text, &l, scale);
        return Ok(());
    }

    let s = scale.max(0.1);
    let width = pixmap.width() as f32;
    let height = pixmap.height() as f32;

    let list_w = layout::LIST_W * s;
    if width <= list_w {
        return Ok(());
    }
    let rx = list_w;
    let rw = width - list_w;

    // ---- Left pane: chat list (header + rows). ----
    draw_list_header(pixmap, text, 0.0, 0.0, list_w, s);
    let selected = match screen {
        Screen::Chat { id, .. } => Some(*id),
        Screen::Idle => None,
    };
    let list_h = (height - layout::LIST_HEADER_H * s).max(0.0);
    list.draw(pixmap, text, 0.0, layout::LIST_HEADER_H * s, list_w, list_h, s, selected, photos);

    // Divider between the panes.
    {
        let mut d = Paint::default();
        d.set_color(Color::from_rgba8(theme::INPUT_BORDER.0, theme::INPUT_BORDER.1, theme::INPUT_BORDER.2, 90));
        pixmap.fill_rect(
            tiny_skia::Rect::from_xywh(rx, 0.0, 1.0, height).unwrap(),
            &d,
            Transform::identity(),
            None,
        );
    }

    // ---- Right pane: conversation or placeholder. ----
    match screen {
        Screen::Idle => {
            let (tw, _) = text.measure("Select a conversation", font::MESSAGE * s);
            text.draw(
                pixmap,
                "Select a conversation",
                rx + (rw - tw) / 2.0,
                height / 2.0,
                font::MESSAGE * s,
                theme::TEXT_SECONDARY,
            );
        }
        Screen::Chat { id, loading } => {
            let row = list.rows.iter().find(|r| r.id == *id);
            let title = row.map(|r| r.title.clone()).unwrap_or_default();
            let avatar = row.and_then(|r| r.avatar_path.clone());
            let _ = loading;
            draw_chat_header(pixmap, text, &title, rx, rw, s, photos, avatar.as_deref());

            let area_y = layout::CHAT_HEADER_H * s;
            let bottom = (height - (layout::INPUT_H + layout::MESSAGES_BOTTOM_GAP) * s).max(area_y + 1.0);
            if messages.rows.is_empty() {
                if !status.is_empty() {
                    let (tw, _) = text.measure(status, font::MESSAGE * s);
                    text.draw(
                        pixmap,
                        status,
                        rx + (rw - tw) / 2.0,
                        (area_y + bottom) / 2.0,
                        font::MESSAGE * s,
                        theme::TEXT_SECONDARY,
                    );
                }
            } else {
                messages.draw(pixmap, text, rx, area_y, rw, bottom - area_y, s, photos, selection);
            }

            draw_composer(pixmap, text, input, rx, height - layout::INPUT_H * s, rw, s);
        }
    }

    Ok(())
}

/// Full-window sign-in screen: centered card with the logo, the step
/// instructions, the input field and the primary action button.
fn draw_login(pixmap: &mut Pixmap, text: &TextRenderer, l: &LoginView, scale: f32) {
    let s = scale.max(0.1);
    let w = pixmap.width() as f32;
    let h = pixmap.height() as f32;
    let layout = theme::login_layout(w / s, h / s);
    let cx = w / 2.0;

    // Soft halo behind the logo for a gentle "brand" focus.
    let r = theme::login::LOGO / 2.0 * s;
    let cy = layout.logo_cy * s;
    let mut halo = Paint::default();
    halo.set_color(Color::from_rgba8(theme::ACCENT.0, theme::ACCENT.1, theme::ACCENT.2, 26));
    if let Some(halo_path) = tiny_skia::PathBuilder::from_circle(cx, cy, r * 2.1) {
        pixmap.fill_path(&halo_path, &halo, FillRule::Winding, Transform::identity(), None);
    }
    let mut ring = Paint::default();
    ring.set_color(Color::from_rgba8(theme::ACCENT.0, theme::ACCENT.1, theme::ACCENT.2, 70));
    if let Some(ring_path) = tiny_skia::PathBuilder::from_circle(cx, cy, r * 1.25) {
        pixmap.fill_path(&ring_path, &ring, FillRule::Winding, Transform::identity(), None);
    }
    let mut accent = Paint::default();
    accent.set_color(color(theme::ACCENT));
    if let Some(circle) = tiny_skia::PathBuilder::from_circle(cx, cy, r) {
        pixmap.fill_path(&circle, &accent, FillRule::Winding, Transform::identity(), None);
    }
    text.draw(pixmap, "tg", cx - 16.0 * s, cy + 11.0 * s, font::TITLE * s, theme::TEXT_PRIMARY);

    // Back navigation (2FA -> code -> phone).
    if *l.step != LoginStep::Phone {
        let (bx, by, _, bh) = layout.back;
        icons::back(pixmap, (bx + 22.0) * s, (by + bh / 2.0) * s, 20.0 * s, theme::ICON);
        text.draw(
            pixmap,
            "Back",
            (bx + 34.0) * s,
            (by + bh / 2.0 + 7.0) * s,
            font::TIMESTAMP * s,
            theme::ICON,
        );
    }

    // Title (per step).
    let title = match l.step {
        LoginStep::Phone => "Sign in to Telegram",
        LoginStep::Code => "Check your phone",
        LoginStep::Password { .. } => "Two-step verification",
    };
    let (tw, _) = text.measure(title, font::TITLE * s);
    text.draw(pixmap, title, cx - tw / 2.0, layout.title_y * s, font::TITLE * s, theme::TEXT_PRIMARY);

    // Subtitle (instructions).
    let subtitle = match l.step {
        LoginStep::Phone => "We will send a code to your phone number.",
        LoginStep::Code => "Enter the code that was sent to your Telegram.",
        LoginStep::Password { hint } => {
            if hint.is_empty() {
                "This account requires an additional password."
            } else {
                "This account requires an additional password."
            }
        }
    };
    let (sw, _) = text.measure(subtitle, font::TIMESTAMP * s);
    text.draw(
        pixmap,
        subtitle,
        cx - sw / 2.0,
        layout.subtitle_y * s,
        font::TIMESTAMP * s,
        theme::TEXT_SECONDARY,
    );
    if let LoginStep::Password { hint } = l.step {
        if !hint.is_empty() {
            let hint_text = format!("Hint: {hint}");
            let (hw, _) = text.measure(&hint_text, font::TIMESTAMP * s);
            text.draw(
                pixmap,
                &hint_text,
                cx - hw / 2.0,
                (layout.subtitle_y + 16.0) * s,
                font::TIMESTAMP * s,
                theme::TEXT_SECONDARY,
            );
        }
    }

    // Input field.
    let (fx, fy, fw, fh) = layout.field;
    draw_login_field(pixmap, text, l, fx * s, fy * s, fw * s, fh * s, s);

    // Primary action button.
    let (btx, bty, btw, bth) = layout.button;
    let label = match l.step {
        LoginStep::Phone => "Continue",
        LoginStep::Code => "Log in",
        LoginStep::Password { .. } => "Sign in",
    };
    let mut bp = Paint::default();
    bp.set_color(
        if l.error && l.input.is_empty() {
            color(theme::ERROR)
        } else {
            color(theme::ACCENT)
        },
    );
    let btn = theme::rounded_rect(btx * s, bty * s, btw * s, bth * s, theme::login::BUTTON_RADIUS * s);
    pixmap.fill_path(&btn, &bp, FillRule::Winding, Transform::identity(), None);
    let (lblw, _) = text.measure(label, font::MESSAGE * s);
    text.draw(
        pixmap,
        label,
        (btx + btw / 2.0) * s - lblw / 2.0,
        (bty + bth / 2.0 + 5.0) * s,
        font::MESSAGE * s,
        theme::TEXT_PRIMARY,
    );

    // Status / error line.
    if !l.status.is_empty() {
        let (stw, _) = text.measure(l.status, font::TIMESTAMP * s);
        let sc = if l.error { theme::ERROR } else { theme::TEXT_SECONDARY };
        text.draw(
            pixmap,
            l.status,
            cx - stw / 2.0,
            layout.status_y * s,
            font::TIMESTAMP * s,
            sc,
        );
    }
}

/// Rounded input field of the sign-in screen, with a masked display for
/// password steps.
fn draw_login_field(
    pixmap: &mut Pixmap,
    text: &TextRenderer,
    l: &LoginView,
    x: f32,
    y: f32,
    field_w: f32,
    field_h: f32,
    s: f32,
) {
    let bg = color((26, 38, 52));
    let r = theme::login::FIELD_RADIUS * s;
    let mut fp = Paint::default();
    fp.set_color(bg);
    let field_path = theme::rounded_rect(x, y, field_w, field_h, r);
    pixmap.fill_path(&field_path, &fp, FillRule::Winding, Transform::identity(), None);

    // Accent border.
    let mut bd = Paint::default();
    bd.set_color(Color::from_rgba8(theme::ACCENT.0, theme::ACCENT.1, theme::ACCENT.2, 150));
    let border_path = theme::rounded_rect(x + 1.0, y + 1.0, field_w - 2.0, field_h - 2.0, r - 1.0);
    let mut stroke = tiny_skia::Stroke::default();
    stroke.width = 1.25;
    pixmap.stroke_path(&border_path, &bd, &stroke, Transform::identity(), None);

    let px = font::MESSAGE * s;
    let placeholder = match l.step {
        LoginStep::Phone => "+33 6 12 34 56 78",
        LoginStep::Code => "Code",
        LoginStep::Password { .. } => "Password",
    };
    let masked = matches!(l.step, LoginStep::Password { .. });
    let content: String = if l.input.is_empty() {
        placeholder.to_string()
    } else if masked {
        l.input.chars().map(|_| '•').collect()
    } else {
        l.input.to_string()
    };
    let fg = if l.input.is_empty() {
        theme::TEXT_SECONDARY
    } else {
        theme::TEXT_PRIMARY
    };

    // Keep the field readable: swap to the tail of a too-long value.
    let max = (field_w - 28.0 * s).max(10.0);
    let shown = if text.measure(&content, px).0 <= max {
        content
    } else {
        let mut out = String::new();
        for ch in content.chars().rev() {
            if text.measure(&format!("{ch}{out}"), px).0 <= max {
                out.insert(0, ch);
            } else {
                break;
            }
        }
        out
    };
    text.draw(pixmap, &shown, x + 18.0 * s, y + field_h / 2.0 + 5.0 * s, px, fg);
}

fn color(c: (u8, u8, u8)) -> Color {
    Color::from_rgba8(c.0, c.1, c.2, 255)
}

/// Full-screen photo viewer: dimmed background + the image centered.
pub fn draw_viewer_overlay(
    pixmap: &mut Pixmap,
    text: &TextRenderer,
    photos: &PhotoCache,
    path: &str,
    w: u32,
    h: u32,
) {
    let mut dim = Paint::default();
    dim.set_color(tiny_skia::Color::from_rgba8(0, 0, 0, 190));
    pixmap.fill_rect(
        tiny_skia::Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap(),
        &dim,
        Transform::identity(),
        None,
    );

if let Some(img) = photos.fitted(path, (w as f32) * 0.72, (h as f32) * 0.72) {
        let (iw, ih) = (img.width() as f32, img.height() as f32);
        let x0 = (w as f32 - iw) / 2.0;
        let y0 = (h as f32 - ih) / 2.0;
        crate::image::blit_opaque(pixmap, x0.round() as i32, y0.round() as i32, &img);
    } else {
        let cue = "Loading…";
        let px = font::MESSAGE * 1.0;
        let (tw, _) = text.measure(cue, px);
        text.draw(pixmap, cue, (w as f32 - tw) / 2.0, h as f32 / 2.0, px, theme::TEXT_PRIMARY);
    }

    let hint = "Click anywhere to close";
    let px = font::TIMESTAMP * 1.0;
    let (tw, _) = text.measure(hint, px);
    text.draw(pixmap, hint, (w as f32 - tw) / 2.0, (h as f32) * 0.93, px, theme::TEXT_SECONDARY);
}


/// Left pane header: "Chats" title + search/compose/menu icons.
fn draw_list_header(pixmap: &mut Pixmap, text: &TextRenderer, x: f32, y: f32, w: f32, s: f32) {
    let h = layout::LIST_HEADER_H * s;
    let mut bg = Paint::default();
    bg.set_color(color(theme::LIST_BG));
    pixmap.fill_rect(tiny_skia::Rect::from_xywh(x, y, w, h).unwrap(), &bg, Transform::identity(), None);

    text.draw(pixmap, "Chats", x + 16.0 * s, y + h / 2.0 + 8.0 * s, font::TITLE * s, theme::TEXT_PRIMARY);

    let ic = 20.0 * s;
    let cx = x + w - 20.0 * s;
    let cy = y + h / 2.0;
    icons::dots(pixmap, cx, cy, ic, theme::ICON);
    icons::compose(pixmap, cx - 30.0 * s, cy, ic, theme::ICON);
    icons::search(pixmap, cx - 60.0 * s, cy, ic, theme::ICON);
}

/// Right pane header: back, avatar, name + status, search/info icons.
fn draw_chat_header(
    pixmap: &mut Pixmap,
    text: &TextRenderer,
    title: &str,
    x: f32,
    w: f32,
    s: f32,
    photos: &PhotoCache,
    avatar_path: Option<&str>,
) {
    let h = layout::CHAT_HEADER_H * s;
    let mut bg = Paint::default();
    bg.set_color(color(theme::LIST_BG));
    pixmap.fill_rect(tiny_skia::Rect::from_xywh(x, 0.0, w, h).unwrap(), &bg, Transform::identity(), None);

    let pad = 12.0 * s;
    // Back arrow.
    let back_cx = x + pad + 10.0 * s;
    let cy = h / 2.0;
    icons::back(pixmap, back_cx, cy, 18.0 * s, theme::ICON);

    // Avatar: profile photo (cached) or accent initial.
    let av = layout::AVATAR_CHAT * s;
    let avx = x + pad * 2.0 + 16.0 * s;
    let mut drew_photo = false;
    if let Some(p) = avatar_path {
        if let Some(img) = photos.fitted(p, av, av) {
            let iw = img.width() as f32;
            let ih = img.height() as f32;
            let mut mask = tiny_skia::Mask::new(pixmap.width(), pixmap.height()).unwrap();
            if let Some(circle) = tiny_skia::PathBuilder::from_circle(avx, cy, av / 2.0) {
                mask.fill_path(&circle, tiny_skia::FillRule::Winding, true, Transform::identity());
            }
            pixmap.draw_pixmap(
                (avx - iw / 2.0).round() as i32,
                (cy - ih / 2.0).round() as i32,
                (*img).as_ref(),
                &tiny_skia::PixmapPaint::default(),
                Transform::identity(),
                Some(&mask),
            );
            drew_photo = true;
        }
    }
    if !drew_photo {
        let mut ap = Paint::default();
        ap.set_color(color(theme::ACCENT));
        let circle = tiny_skia::PathBuilder::from_circle(avx, cy, av / 2.0).unwrap();
        pixmap.fill_path(&circle, &ap, tiny_skia::FillRule::Winding, Transform::identity(), None);
        if let Some(initial) = title.chars().next() {
            let (tw, _) = text.measure(&initial.to_string(), font::NAME * s);
            text.draw(pixmap, &initial.to_string(), avx - tw / 2.0, cy + 6.0 * s, font::NAME * s, theme::TEXT_PRIMARY);
        }
    }

    // Name.
    let name_x = avx + av / 2.0 + 10.0 * s;
    let name_max = (x + w - 84.0 * s - name_x).max(20.0);
    let name = truncate(text, title, name_max, font::NAME * s);
    text.draw(pixmap, &name, name_x, cy + 6.0 * s, font::NAME * s, theme::TEXT_PRIMARY);
    // Status / online placeholder.
    let status = "Chat";
    text.draw(pixmap, status, name_x, cy + 22.0 * s, font::TIMESTAMP * s, theme::TEXT_SECONDARY);

    // Right icons.
    let ic = 20.0 * s;
    let icx = x + w - 22.0 * s;
    icons::info(pixmap, icx, cy, ic, theme::ICON);
    icons::search(pixmap, icx - 32.0 * s, cy, ic, theme::ICON);
}

fn truncate(text: &TextRenderer, title: &str, max_w: f32, px: f32) -> String {
    if text.measure(title, px).0 <= max_w {
        return title.to_string();
    }
    let mut out = String::new();
    for ch in title.chars() {
        if text.measure(&format!("{out}{ch}…"), px).0 <= max_w {
            out.push(ch);
        } else {
            break;
        }
    }
    format!("{out}…")
}

/// Composer bar: rounded input + placeholder + send button.
fn draw_composer(pixmap: &mut Pixmap, text: &TextRenderer, input: &str, x: f32, y: f32, w: f32, s: f32) {
    let h = layout::INPUT_H * s;
    let mut bg = Paint::default();
    bg.set_color(color(theme::LIST_BG));
    pixmap.fill_rect(tiny_skia::Rect::from_xywh(x, y, w, h).unwrap(), &bg, Transform::identity(), None);

    let pad = 12.0 * s;
    let radius = layout::INPUT_RADIUS * s;
    let field_h = h - 2.0 * pad;
    let send = 34.0 * s;
    let field_w = (w - pad * 2.0 - send - 10.0 * s).max(20.0).min(w - pad * 2.0);

    // Rounded input field.
    let fbg = color((26, 38, 52));
    let mut fp = Paint::default();
    fp.set_color(fbg);
    let field_path = theme::rounded_rect(x + pad, y + pad, field_w, field_h, radius);
    pixmap.fill_path(&field_path, &fp, tiny_skia::FillRule::Winding, Transform::identity(), None);

    // Placeholder or typed text.
    let px = font::PLACEHOLDER * s;
    let tc = if input.is_empty() { theme::TEXT_SECONDARY } else { theme::TEXT_PRIMARY };
    let shown = if input.is_empty() {
        "Message…".to_string()
    } else {
        let max = (field_w - 24.0 * s).max(10.0);
        if text.measure(input, px).0 <= max {
            input.to_string()
        } else {
            let mut out = String::new();
            for ch in input.chars().rev() {
                if text.measure(&format!("{ch}{out}"), px).0 <= max {
                    out.insert(0, ch);
                } else {
                    break;
                }
            }
            out
        }
    };
    text.draw(pixmap, &shown, x + pad + 14.0 * s, y + h / 2.0 + 5.0 * s, px, tc);

    // Send button (accent circle + paper plane).
    let sendx = x + w - pad - send / 2.0;
    let sendy = y + h / 2.0;
    let mut sp = Paint::default();
    sp.set_color(color(theme::ACCENT));
    let sc = tiny_skia::PathBuilder::from_circle(sendx, sendy, send / 2.0).unwrap();
    pixmap.fill_path(&sc, &sp, tiny_skia::FillRule::Winding, Transform::identity(), None);
    icons::send(pixmap, sendx, sendy, 20.0 * s, theme::TEXT_PRIMARY);
}