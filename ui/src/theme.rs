//! Visual theme: colors, fonts and layout tokens drawn from the designer
//! mockups (dark Telegram-style blue theme).

/// Background of the right panel (conversation area).
pub const CHAT_BG: (u8, u8, u8) = (14, 22, 33);
/// Background of the left panel (chat list).
pub const LIST_BG: (u8, u8, u8) = (23, 33, 43);
/// Highlighted (selected) chat row.
pub const ROW_SELECTED: (u8, u8, u8) = (43, 58, 92);
/// Received message bubble.
pub const BUBBLE_RECV: (u8, u8, u8) = (24, 37, 51);
/// Sent message bubble.
pub const BUBBLE_SENT: (u8, u8, u8) = (43, 82, 120);
/// Accent (badges, send button, actions).
pub const ACCENT: (u8, u8, u8) = (51, 144, 236);
/// Online status dot.
pub const ONLINE: (u8, u8, u8) = (77, 205, 94);
/// Header / composer icons.
pub const ICON: (u8, u8, u8) = (109, 159, 187);
/// Primary text.
pub const TEXT_PRIMARY: (u8, u8, u8) = (255, 255, 255);
/// Secondary text (timestamps, previews, placeholder).
pub const TEXT_SECONDARY: (u8, u8, u8) = (109, 127, 142);
/// Composer input border.
pub const INPUT_BORDER: (u8, u8, u8) = (58, 74, 90);
/// Error / destructive text (login failures, status errors).
pub const ERROR: (u8, u8, u8) = (240, 114, 124);

/// Geometry of the sign-in screen (logical units), shared by the renderer
/// and the click handler so both agree on where the widgets are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoginLayout {
    /// Y of the logo circle center.
    pub logo_cy: f32,
    /// Baseline of the screen title.
    pub title_y: f32,
    /// Baseline of the step subtitle.
    pub subtitle_y: f32,
    /// Round input field (x, y, w, h).
    pub field: (f32, f32, f32, f32),
    /// Primary action button (x, y, w, h).
    pub button: (f32, f32, f32, f32),
    /// Baseline of the status / error line.
    pub status_y: f32,
    /// "Back" hit area (top-left), empty if on the first step.
    pub back: (f32, f32, f32, f32),
    /// Total vertical span used (for centering).
    pub total: f32,
}

impl LoginLayout {
    pub fn contains(rect: (f32, f32, f32, f32), x: f32, y: f32) -> bool {
        let (rx, ry, rw, rh) = rect;
        x >= rx && x <= rx + rw && y >= ry && y <= ry + rh && rw > 0.0 && rh > 0.0
    }
}

/// Computes the login-card geometry for a logical window of `w`×`h`.
pub fn login_layout(w: f32, h: f32) -> LoginLayout {
    let title_h = 26.0;
    let subtitle_h = 18.0;
    let status_h = 16.0;
    let total = login::LOGO
        + login::GAP_TITLE
        + title_h
        + login::GAP_SUBTITLE
        + subtitle_h
        + login::GAP_FIELD
        + login::FIELD_H
        + login::GAP_BUTTON
        + login::BUTTON_H
        + login::GAP_STATUS
        + status_h;
    let y0 = ((h - total) / 2.0).max(16.0);

    let mut y = y0;
    let logo_cy = y + login::LOGO / 2.0;
    y += login::LOGO + login::GAP_TITLE;
    let title_y = y + title_h * 0.78;
    y += title_h + login::GAP_SUBTITLE;
    let subtitle_y = y + subtitle_h * 0.8;
    y += subtitle_h + login::GAP_FIELD;
    let field = (w / 2.0 - login::FIELD_W / 2.0, y, login::FIELD_W, login::FIELD_H);
    y += login::FIELD_H + login::GAP_BUTTON;
    let button = (w / 2.0 - login::BUTTON_W / 2.0, y, login::BUTTON_W, login::BUTTON_H);
    y += login::BUTTON_H + login::GAP_STATUS;
    let status_y = y + status_h * 0.8;

    LoginLayout {
        logo_cy,
        title_y,
        subtitle_y,
        field,
        button,
        status_y,
        back: (16.0, 16.0, 72.0, 44.0),
        total,
    }
}

/// Layout tokens for the login screen.
pub mod login {
    /// Input field / button width.
    pub const FIELD_W: f32 = 400.0;
    /// Input field height.
    pub const FIELD_H: f32 = 48.0;
    /// Input corner radius.
    pub const FIELD_RADIUS: f32 = 14.0;
    /// Primary action button width.
    pub const BUTTON_W: f32 = 400.0;
    /// Primary action button height.
    pub const BUTTON_H: f32 = 46.0;
    /// Corner radius of the primary action button.
    pub const BUTTON_RADIUS: f32 = 14.0;

    /// Accent logo circle diameter.
    pub const LOGO: f32 = 76.0;
    /// Gap between the logo and the title.
    pub const GAP_TITLE: f32 = 26.0;
    /// Gap between the title and the subtitle.
    pub const GAP_SUBTITLE: f32 = 10.0;
    /// Gap between the subtitle and the input field.
    pub const GAP_FIELD: f32 = 28.0;
    /// Gap between the input field and the button.
    pub const GAP_BUTTON: f32 = 14.0;
    /// Gap between the button and the status line.
    pub const GAP_STATUS: f32 = 16.0;
}

/// Layout metrics (logical units, multiplied by the UI scale when drawn).
pub mod layout {
    /// Explicit width of the left (chat list) panel.
    pub const LIST_W: f32 = 280.0;
    /// Height of the list header bar.
    pub const LIST_HEADER_H: f32 = 44.0;
    /// Height of the conversation header bar.
    pub const CHAT_HEADER_H: f32 = 52.0;
    /// Height of the composer bar.
    pub const INPUT_H: f32 = 44.0;
    /// Bottom safety margin above the composer.
    pub const MESSAGES_BOTTOM_GAP: f32 = 8.0;
    /// Corner radius of message bubbles.
    pub const BUBBLE_RADIUS: f32 = 16.0;
    /// Corner radius of the composer field.
    pub const INPUT_RADIUS: f32 = 22.0;
    /// Avatar diameter in the list.
    pub const AVATAR_LIST: f32 = 46.0;
    /// Avatar diameter in the chat header.
    pub const AVATAR_CHAT: f32 = 40.0;
    /// Unread badge diameter.
    pub const BADGE_SIZE: f32 = 20.0;
    /// Vertical gap between message rows.
    pub const BUBBLE_GAP: f32 = 18.0;
}

/// Font sizes (logical px, scaled at draw time).
pub mod font {
    pub const TITLE: f32 = 20.0;
    pub const NAME: f32 = 16.0;
    pub const MESSAGE: f32 = 15.0;
    pub const TIMESTAMP: f32 = 13.0;
    pub const BADGE: f32 = 11.0;
    pub const PLACEHOLDER: f32 = 15.0;
}

/// Formats a Unix timestamp as local-time `HH:MM` (handles the local
/// timezone and DST via chrono, already in the dependency tree).
pub fn fmt_time(ts: i32) -> String {
    use chrono::{DateTime, Local};
    match DateTime::from_timestamp(ts as i64, 0) {
        Some(utc) => {
            let local = utc.with_timezone(&Local);
            local.format("%H:%M").to_string()
        }
        None => String::new(),
    }
}

/// Builds a rounded-rectangle path (for bubbles, badges, the composer).
pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, radius: f32) -> tiny_skia::Path {
    let r = radius.min(w / 2.0).min(h / 2.0);
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish().expect("valid rounded rect")
}