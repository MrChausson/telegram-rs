//! Visual theme: colors and layout tokens ported from the custom `ui` crate
//! so the Iced build renders as close to the original as possible.

/// Background of the left panel (chat list).
pub const LIST_BG: (u8, u8, u8) = (23, 33, 43);
/// Conversation background (message area).
pub const CHAT_BG: (u8, u8, u8) = (14, 22, 33);
/// Hovered chat row.
pub const ROW_HOVER: (u8, u8, u8) = (31, 44, 57);
/// Highlighted (selected) chat row.
pub const ROW_SELECTED: (u8, u8, u8) = (43, 58, 92);
/// Received message bubble.
pub const BUBBLE_RECV: (u8, u8, u8) = (24, 37, 51);
/// Sent message bubble.
pub const BUBBLE_SENT: (u8, u8, u8) = (43, 82, 120);
/// Accent (badges, send button, actions).
pub const ACCENT: (u8, u8, u8) = (51, 144, 236);
/// Header / composer icons.
pub const ICON: (u8, u8, u8) = (109, 159, 187);
/// Primary text.
pub const TEXT_PRIMARY: (u8, u8, u8) = (255, 255, 255);
/// Secondary text (timestamps, previews, placeholder).
pub const TEXT_SECONDARY: (u8, u8, u8) = (109, 127, 142);
/// Composer input border.
pub const INPUT_BORDER: (u8, u8, u8) = (58, 74, 90);
/// Error / destructive text.
pub const ERROR: (u8, u8, u8) = (240, 114, 124);
/// Composer field fill.
pub const INPUT_FILL: (u8, u8, u8) = (26, 38, 52);
/// Subtle divider between panes / under headers.
pub const DIVIDER: (u8, u8, u8) = (34, 47, 61);
/// Background of the `--perf` FPS badge.
pub const PERF_BADGE_BG: (u8, u8, u8) = (18, 28, 38);

/// Layout metrics (logical units).
pub mod layout {
    /// Explicit width of the left (chat list) panel.
    pub const LIST_W: f32 = 280.0;
    /// Height of the list header bar.
    pub const LIST_HEADER_H: f32 = 44.0;
    /// Height of the conversation header bar.
    pub const CHAT_HEADER_H: f32 = 52.0;
    /// Height of the composer bar.
    pub const INPUT_H: f32 = 44.0;
    /// Corner radius of message bubbles.
    pub const BUBBLE_RADIUS: f32 = 16.0;
    /// Corner radius of the composer field.
    pub const INPUT_RADIUS: f32 = 22.0;
    /// Avatar diameter in the list.
    pub const AVATAR_LIST: f32 = 46.0;
    /// Avatar diameter in the chat header.
    pub const AVATAR_CHAT: f32 = 40.0;
    /// Width of the message context menu.
    pub const CONTEXT_W: f32 = 160.0;
    /// Horizontal padding of the message area.
    pub const MSG_PAD_X: f32 = 16.0;
    /// Bubble inner horizontal padding (2× per side).
    pub const BUBBLE_PAD_X: f32 = 12.0;
    /// Bubble inner vertical padding (2× per side).
    pub const BUBBLE_PAD_Y: f32 = 8.0;
}

/// Font sizes (logical px).
pub mod font {
    pub const TITLE: f32 = 20.0;
    pub const NAME: f32 = 16.0;
    pub const MESSAGE: f32 = 15.0;
    pub const TIMESTAMP: f32 = 13.0;
    pub const BADGE: f32 = 11.0;
    pub const PLACEHOLDER: f32 = 15.0;
}

/// Formats a Unix timestamp as local-time `HH:MM`.
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

/// Avatar palette (same as the custom `chatlist`).
pub const AVATAR_PALETTE: [(u8, u8, u8); 6] = [
    (51, 144, 236),
    (110, 101, 220),
    (235, 125, 58),
    (116, 189, 74),
    (231, 76, 130),
    (37, 178, 178),
];

/// Deterministic avatar color index for a title.
pub fn avatar_color(title: &str) -> (u8, u8, u8) {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in title.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    AVATAR_PALETTE[(h as usize) % AVATAR_PALETTE.len()]
}
