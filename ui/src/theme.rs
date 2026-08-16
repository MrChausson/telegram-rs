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