//! Visual theme: colors and layout tokens ported from the custom `ui` crate
//! so the Iced build renders as close to the original as possible.
//!
//! Colors are **mode-dependent** (dark / light). The active mode lives in a
//! process-global `AtomicU8` (see [`set_mode`]) so the zero-argument accessor
//! functions below (`ACCENT()`, …) can keep the call sites terse while still
//! re-resolving on every frame — a toggle takes effect on the next redraw.
//!
//! Consumer API note: every former `pub const X` is now `pub fn X()`; all
//! call sites read `theme::X()`. The [`layout`] / [`font`] tokens are genuine
//! constants and unchanged.

#![allow(non_snake_case)]

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

/// Light/dark selection. `Default` is dark (the historical palette).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

impl ThemeMode {
    /// Index into [`PALETTE`] (0 = dark, 1 = light).
    fn idx(self) -> usize {
        match self {
            ThemeMode::Dark => 0,
            ThemeMode::Light => 1,
        }
    }

    /// Parses the persisted marker content (`"dark"` / `"light"`).
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "light" => ThemeMode::Light,
            _ => ThemeMode::Dark,
        }
    }

    /// Marker content persisted in the data dir.
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeMode::Dark => "dark",
            ThemeMode::Light => "light",
        }
    }
}

/// One complete palette. All values opaque `(r, g, b)` tuples.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub list_bg: (u8, u8, u8),
    pub chat_bg: (u8, u8, u8),
    pub row_hover: (u8, u8, u8),
    pub row_selected: (u8, u8, u8),
    pub bubble_recv: (u8, u8, u8),
    pub bubble_sent: (u8, u8, u8),
    pub accent: (u8, u8, u8),
    pub accent_hover: (u8, u8, u8),
    pub accent_pressed: (u8, u8, u8),
    pub icon: (u8, u8, u8),
    pub text_primary: (u8, u8, u8),
    pub text_secondary: (u8, u8, u8),
    pub input_border: (u8, u8, u8),
    pub error: (u8, u8, u8),
    pub input_fill: (u8, u8, u8),
    pub divider: (u8, u8, u8),
    pub perf_badge_bg: (u8, u8, u8),
    pub menu_bg: (u8, u8, u8),
    pub menu_border: (u8, u8, u8),
}

/// Historical dark palette.
const DARK: Palette = Palette {
    list_bg: (23, 33, 43),
    chat_bg: (14, 22, 33),
    row_hover: (31, 44, 57),
    row_selected: (43, 58, 92),
    bubble_recv: (24, 37, 51),
    bubble_sent: (43, 82, 120),
    accent: (51, 144, 236),
    accent_hover: (66, 160, 242),
    accent_pressed: (40, 120, 200),
    icon: (109, 159, 187),
    text_primary: (255, 255, 255),
    text_secondary: (109, 127, 142),
    input_border: (58, 74, 90),
    error: (240, 114, 124),
    input_fill: (26, 38, 52),
    divider: (34, 47, 61),
    perf_badge_bg: (18, 28, 38),
    menu_bg: (33, 47, 64),
    menu_border: (56, 73, 92),
};

/// Light palette modeled after Telegram Desktop defaults: white list,
/// pale-blue conversation canvas, near-black ink, same accent blue.
const LIGHT: Palette = Palette {
    list_bg: (255, 255, 255),      // #FFFFFF
    chat_bg: (219, 231, 240),      // #DBE7F0 (flat stand-in for the pattern)
    row_hover: (244, 244, 245),    // #F4F4F5
    row_selected: (216, 235, 252), // ~#3390EC @12% over #FFFFFF (opaque)
    bubble_recv: (255, 255, 255),  // #FFFFFF
    bubble_sent: (238, 255, 222),  // #EEFFDE
    accent: (51, 144, 236),        // #3390EC
    accent_hover: (47, 132, 222),  // #2F84DE
    accent_pressed: (28, 108, 190),// #1C6CBE
    icon: (100, 116, 130),         // muted gray-blue, legible on white
    text_primary: (0, 0, 0),       // #000000
    text_secondary: (112, 117, 121), // #707579
    input_border: (218, 220, 224), // #DADCE0
    error: (223, 63, 64),          // #DF3F40
    input_fill: (244, 244, 245),   // #F4F4F5
    divider: (230, 236, 240),      // #E6ECF0
    perf_badge_bg: (18, 28, 38),   // stays dark so the FPS badge reads anywhere
    menu_bg: (255, 255, 255),      // #FFFFFF
    menu_border: (218, 220, 224),  // #DADCE0
};

/// Both palettes indexed by [`ThemeMode::idx`].
pub static PALETTE: [Palette; 2] = [DARK, LIGHT];

/// Active palette slot: `0` = dark, `1` = light. Process-global because view
/// code reaches colors through free functions without threading state around.
static ACTIVE: OnceLock<AtomicU8> = OnceLock::new();

fn slot() -> &'static AtomicU8 {
    ACTIVE.get_or_init(|| AtomicU8::new(0))
}

/// The currently applied mode (defaults to dark until [`set_mode`]).
pub fn mode() -> ThemeMode {
    match slot().load(Ordering::Relaxed) {
        1 => ThemeMode::Light,
        _ => ThemeMode::Dark,
    }
}

/// Applies `m` to the live palette (idempotent). Called by
/// `State::toggle_theme` and at boot when restoring the persisted mode.
pub fn set_mode(m: ThemeMode) {
    slot().store(m.idx() as u8, Ordering::Relaxed);
}

fn cur() -> &'static Palette {
    &PALETTE[mode().idx()]
}

/// Background of the left panel (chat list).
pub fn LIST_BG() -> (u8, u8, u8) {
    cur().list_bg
}
/// Conversation background (message area).
pub fn CHAT_BG() -> (u8, u8, u8) {
    cur().chat_bg
}
/// Hovered chat row.
pub fn ROW_HOVER() -> (u8, u8, u8) {
    cur().row_hover
}
/// Highlighted (selected) chat row.
pub fn ROW_SELECTED() -> (u8, u8, u8) {
    cur().row_selected
}
/// Received message bubble.
pub fn BUBBLE_RECV() -> (u8, u8, u8) {
    cur().bubble_recv
}
/// Sent message bubble.
pub fn BUBBLE_SENT() -> (u8, u8, u8) {
    cur().bubble_sent
}
/// Accent (badges, send button, actions).
pub fn ACCENT() -> (u8, u8, u8) {
    cur().accent
}
/// Header / composer icons.
pub fn ICON() -> (u8, u8, u8) {
    cur().icon
}
/// Primary text.
pub fn TEXT_PRIMARY() -> (u8, u8, u8) {
    cur().text_primary
}
/// Secondary text (timestamps, previews, placeholder).
pub fn TEXT_SECONDARY() -> (u8, u8, u8) {
    cur().text_secondary
}
/// Composer input border.
pub fn INPUT_BORDER() -> (u8, u8, u8) {
    cur().input_border
}
/// Error / destructive text.
pub fn ERROR() -> (u8, u8, u8) {
    cur().error
}
/// Composer field fill.
pub fn INPUT_FILL() -> (u8, u8, u8) {
    cur().input_fill
}
/// Subtle divider between panes / under headers.
pub fn DIVIDER() -> (u8, u8, u8) {
    cur().divider
}
/// Background of the `--perf` FPS badge.
pub fn PERF_BADGE_BG() -> (u8, u8, u8) {
    cur().perf_badge_bg
}
/// Menu surface (context menu, pickers) — one step above the chat bg.
pub fn MENU_BG() -> (u8, u8, u8) {
    cur().menu_bg
}
/// Hairline border around menu surfaces.
pub fn MENU_BORDER() -> (u8, u8, u8) {
    cur().menu_border
}
/// Accent hover variant (buttons).
pub fn ACCENT_HOVER() -> (u8, u8, u8) {
    cur().accent_hover
}
/// Accent pressed variant (buttons).
pub fn ACCENT_PRESSED() -> (u8, u8, u8) {
    cur().accent_pressed
}

/// State-layer overlay for flat/icon buttons at rest → hover → pressed:
/// translucent white in dark mode, translucent black in light mode.
/// Returns `(r, g, b, a)`.
pub fn HOVER_OVERLAY(pressed: bool) -> (u8, u8, u8, f32) {
    match mode() {
        ThemeMode::Light => (0, 0, 0, if pressed { 0.10 } else { 0.05 }),
        ThemeMode::Dark => (255, 255, 255, if pressed { 0.16 } else { 0.09 }),
    }
}

/// Context-menu row hover fill: neutral state layer that flips with the
/// mode (the destructive variant is handled by the caller). Returns
/// `(r, g, b, a)`.
pub fn MENU_ITEM_OVERLAY() -> (u8, u8, u8, f32) {
    match mode() {
        ThemeMode::Light => (0, 0, 0, 0.06),
        ThemeMode::Dark => (255, 255, 255, 0.08),
    }
}

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
    pub const CONTEXT_W: f32 = 190.0;
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

/// Memoized local-time `HH:MM` for a Unix timestamp.
///
/// `message_row`/`chat_row_button` call this once per visible row **per
/// frame**; the chrono/`Local` formatting below is comparatively expensive and
/// most rows keep the same date between frames (a burst of messages share the
/// same minute). The first call per distinct date formats it, the rest clone a
/// cached string — no timezone work on the hot path.
pub fn cached_time(ts: i32) -> String {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static CACHE: OnceLock<Mutex<HashMap<i32, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    if let Some(t) = cache.lock().ok().and_then(|c| c.get(&ts).cloned()) {
        return t;
    }
    let formatted = fmt_time(ts);
    cache
        .lock()
        .ok()
        .and_then(|mut c| c.insert(ts, formatted.clone()));
    formatted
}

/// Avatar palette (same as the custom `chatlist`) — identical in both modes.
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

/// Serializes tests that touch the process-global mode slot (cargo runs unit
/// tests on parallel threads; `State::toggle_theme` tests mutate it too).
#[cfg(test)]
pub(crate) fn mode_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static GUARD: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    match GUARD.get_or_init(|| std::sync::Mutex::new(())).lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_is_dark_and_palette_matches() {
        assert_eq!(ThemeMode::default(), ThemeMode::Dark);
        assert_eq!(PALETTE.len(), 2);
        // Dark keeps the historical values…
        assert_eq!(PALETTE[0].list_bg, (23, 33, 43));
        assert_eq!(PALETTE[0].text_primary, (255, 255, 255));
        // …light follows Telegram Desktop's light scheme.
        assert_eq!(PALETTE[1].list_bg, (255, 255, 255));
        assert_eq!(PALETTE[1].chat_bg, (219, 231, 240));
        assert_eq!(PALETTE[1].bubble_sent, (238, 255, 222));
        assert_eq!(PALETTE[1].bubble_recv, (255, 255, 255));
        assert_eq!(PALETTE[1].text_primary, (0, 0, 0));
        assert_eq!(PALETTE[1].accent, (51, 144, 236));
    }

    /// One test for everything that touches the process-global mode: cargo
    /// runs tests on parallel threads and `ACTIVE` is shared state.
    #[test]
    fn set_mode_and_marker_behaviour() {
        let _guard = mode_test_guard();
        // Marker roundtrip (pure).
        assert_eq!(ThemeMode::parse("dark"), ThemeMode::Dark);
        assert_eq!(ThemeMode::parse(" light \n"), ThemeMode::Light);
        assert_eq!(ThemeMode::parse("garbage"), ThemeMode::Dark);
        for m in [ThemeMode::Dark, ThemeMode::Light] {
            assert_eq!(ThemeMode::parse(m.as_str()), m);
        }

        // Global switch drives every accessor.
        set_mode(ThemeMode::Light);
        let light_overlay = HOVER_OVERLAY(false);
        assert_eq!(mode(), ThemeMode::Light);
        assert_eq!(LIST_BG(), (255, 255, 255));
        assert_eq!(TEXT_PRIMARY(), (0, 0, 0));

        set_mode(ThemeMode::Dark);
        let dark_overlay = HOVER_OVERLAY(false);
        assert_eq!(mode(), ThemeMode::Dark);
        assert_eq!(LIST_BG(), (23, 33, 43));
        assert_eq!(TEXT_PRIMARY(), (255, 255, 255));

        assert_ne!(dark_overlay, light_overlay);
    }
}
