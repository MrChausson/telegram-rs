//! Emoji catalogue for the composer's emoji picker and helpers to render
//! emoji glyphs with the color-emoji font inside regular text.
//!
//! Sets are plain whitespace-separated `&str` constants: the view splits them
//! into a 7-column grid, and no parsing/allocation happens at startup.

/// Starter set shown in the "Recents" section until the user picks anything.
pub const RECENTS_FALLBACK: &str = "😀 😂 🥰 😍 😎 🤔 👍 🙏 🎉 ❤️ 😭 🔥 👏 🤣 😢 ✅";

/// Standard emoji groups, in display order: `(section title, emojis)`.
pub const SETS: &[(&str, &str)] = &[
    (
        "Smileys & People",
        "😀 😃 😄 😁 😆 😅 🤣 😂 🙂 😉 😊 😇 🥰 😙 😍 😘 \
         😋 😜 🤪 🤨 🧐 🤓 😎 🥳 😏 😒 😞 😔 😟 😕 🙁 ☹️ \
         😣 😖 😫 😩 🥺 😢 😭 😤 😠 😡 🤬 🤯 😳 🥵 🥶 😱 \
         😨 😰 😥 😓 🤗 🤔 🤭 🤫 😐 😑 😶 😬 🙄 😯 😦 😧",
    ),
    (
        "Animals & Nature",
        "🐶 🐱 🐭 🐹 🐰 🦊 🐻 🐼 🐨 🐯 🦁 🐮 🐷 🐸 🐵 🐔 \
         🐧 🐦 🦆 🦉",
    ),
    (
        "Food & Drink",
        "🍏 🍎 🍐 🍊 🍋 🍌 🍉 🍇 🍓 🫐 🍒 🍑 🥭 🍍 🥝 🍅 \
         🥑 🍕 🍔 🍟",
    ),
    (
        "Activities",
        "⚽ 🏀 🏈 ⚾ 🎾 🏐 🎱 🏓 🏸 🥅 🎮 🎲 🎯 🎳 🎰 🎨",
    ),
    (
        "Objects",
        "⌚ 📱 💻 ⌨️ 🖥️ 🖨️ 💾 💿 📷 📸 📹 🎥 🔍 🔒 🔑 💡",
    ),
    ("Symbols", "❤️ 🧡 💛 💚 💙 💜 🖤 🤍 💔 ❣️ 💯 ✅"),
];

/// Byte ranges of emoji runs in `s`, in order, non-overlapping.
///
/// A run covers one base emoji plus its continuations: variation selector
/// U+FE0F, skin-tone modifiers, ZWJ sequences (👨‍👩‍👧), regional-indicator
/// pairs (🇫🇷) and keycap marks (20E3). Text-presentation characters (© ® ™,
/// the plain © in "© 2024") only count when explicitly followed by U+FE0F,
/// so ordinary text is never hijacked.
///
/// The view uses this to hand each run to the color-emoji font: without it
/// the text engine's fallback chain resolves emoji codepoints to the ugly
/// monochrome outlines of the default sans-serif font (DejaVu).
pub fn emoji_ranges(s: &str) -> Vec<std::ops::Range<usize>> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (start, c) = chars[i];

        // Keycap sequences: [0-9#*] + FE0F? + 20E3.
        if matches!(c, '0'..='9' | '#' | '*') {
            if let Some(end) = keycap_end(&chars[i..]) {
                ranges.push(start..end);
                i += advance_to(&chars[i..], end);
                continue;
            }
        }

        // © ® ™ only become emoji with an explicit FE0F.
        if matches!(c, '\u{00A9}' | '\u{00AE}' | '\u{2122}') {
            if let Some((_, next)) = chars.get(i + 1) {
                if *next == '\u{FE0F}' {
                    let end = next_pos(&chars, i + 1);
                    ranges.push(start..end);
                    i = advance_to(&chars, end);
                    continue;
                }
            }
            i += 1;
            continue;
        }

        if is_emoji_char(c) {
            let end = run_end(&chars, i);
            ranges.push(start..end);
            i = advance_to(&chars, end);
        } else {
            i += 1;
        }
    }
    ranges
}

fn run_end(chars: &[(usize, char)], i: usize) -> usize {
    let mut end = next_pos(chars, i);
    let mut j = i + 1;
    while let Some((pos, c)) = chars.get(j).copied() {
        if c == '\u{FE0F}' || (0x1F3FB..=0x1F3FF).contains(&(c as u32)) {
            end = pos + c.len_utf8();
        } else if c == '\u{200D}' {
            // ZWJ only binds when an emoji base follows it.
            match chars.get(j + 1) {
                Some((npos, nc)) if is_emoji_char(*nc) => {
                    end = npos + nc.len_utf8();
                    j += 1;
                }
                _ => break,
            }
        } else if (0x1F1E6..=0x1F1FF).contains(&(c as u32)) {
            // Second half of a regional-indicator pair completes the flag.
            end = pos + c.len_utf8();
            break;
        } else {
            break;
        }
        j += 1;
    }
    end
}

fn keycap_end(chars: &[(usize, char)]) -> Option<usize> {
    let mut idx = 1;
    if let Some((_, c)) = chars.get(idx) {
        if *c == '\u{FE0F}' {
            idx += 1;
        }
    }
    match chars.get(idx) {
        Some((pos, c)) if *c == '\u{20E3}' => Some(pos + c.len_utf8()),
        _ => None,
    }
}

fn next_pos(chars: &[(usize, char)], i: usize) -> usize {
    let (pos, c) = chars[i];
    pos + c.len_utf8()
}

fn advance_to(chars: &[(usize, char)], end: usize) -> usize {
    chars
        .iter()
        .position(|(pos, _)| *pos >= end)
        .unwrap_or(chars.len())
}

/// Whether `c` can start (or continue) an emoji run. Conservative on
/// purpose: the text-presentation leftovers of the symbol blocks (plain
/// © ® ™, arrows) are handled by the caller's VS16 rule.
fn is_emoji_char(c: char) -> bool {
    let cp = c as u32;
    (0x1F000..=0x1FAFF).contains(&cp) // all modern emoji blocks
        || (0x2600..=0x27BF).contains(&cp) // misc symbols + dingbats
        || (0x2B00..=0x2BFF).contains(&cp) // ⬆ ⬛ ⭐ …
        || matches!(cp,
            0x231A..=0x231B      // ⌚ ⌛
            | 0x23E9..=0x23F3    // ⏩ ⏱ ⏲ ⏳
            | 0x23F8..=0x23FA    // ⏸ ⏹ ⏺
            | 0x25AA..=0x25AB    // ▪ ▫
            | 0x25B6 | 0x25C0    // ▶ ◀
            | 0x25FB..=0x25FE    // ▫▫◾◽
            | 0x2934..=0x2935    // ⤴ ⤵
            | 0x3030 | 0x303D    // 〰 〽
            | 0x3297 | 0x3299    // ㊗ ㊙
            | 0x00A9 | 0x00AE | 0x2122 // © ® ™ (VS16-gated by the caller)
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(s: &str) -> Vec<&str> {
        emoji_ranges(s).iter().map(|r| &s[r.clone()]).collect()
    }

    #[test]
    fn plain_text_has_no_emoji_runs() {
        assert!(emoji_ranges("").is_empty());
        assert!(emoji_ranges("Salut, ça va ? https://x.io").is_empty());
        // Text-presentation symbols stay untouched without VS16.
        assert!(emoji_ranges("© 2024 Acme®").is_empty());
    }

    #[test]
    fn basic_emoji_forms_a_run() {
        assert_eq!(ranges("Salut 👋 !"), vec!["👋"]);
        assert_eq!(ranges("Vue 🏖 demain"), vec!["🏖"]);
        // Text-presentation ☺ is still upgraded (nicer than DejaVu's mono).
        assert_eq!(ranges("Look at this one ☺"), vec!["☺"]);
    }

    #[test]
    fn vs16_extends_the_run() {
        // ❤ = U+2764 + FE0F
        assert_eq!(ranges("je ❤️ ça"), vec!["❤️"]);
        assert_eq!(ranges("©️ 2024"), vec!["©️"]);
    }

    #[test]
    fn zwj_family_is_one_run() {
        assert_eq!(ranges("famille 👨‍👩‍👧 ok"), vec!["👨‍👩‍👧"]);
        // ZWJ not followed by an emoji does not bind.
        assert_eq!(ranges("a 👋\u{200D}b"), vec!["👋"]);
    }

    #[test]
    fn skin_tone_and_flags_are_one_run() {
        assert_eq!(ranges("ok 👍🏽 merci"), vec!["👍🏽"]);
        assert_eq!(ranges("vive 🇫🇷 !"), vec!["🇫🇷"]);
    }

    #[test]
    fn keycaps_are_one_run() {
        assert_eq!(ranges("presse #️⃣"), vec!["#️⃣"]);
        assert_eq!(ranges("1️⃣2️⃣"), vec!["1️⃣", "2️⃣"]);
    }

    #[test]
    fn sets_have_expected_shapes() {
        assert_eq!(SETS.len(), 6);
        let counts: Vec<usize> = SETS
            .iter()
            .map(|(_, e)| e.split_whitespace().count())
            .collect();
        // ~60 / ~20 / ~20 / ~16 / ~16 / ~12 per the picker spec.
        assert_eq!(counts, vec![64, 20, 20, 16, 16, 12]);
        assert_eq!(RECENTS_FALLBACK.split_whitespace().count(), 16);
    }

    #[test]
    fn every_emoji_is_a_single_grapheme_word() {
        for (_, set) in SETS {
            for e in set.split_whitespace() {
                // No internal whitespace; each token is short (emoji + VS16).
                assert!(e.chars().count() <= 3, "token too long: {e}");
            }
        }
    }
}
