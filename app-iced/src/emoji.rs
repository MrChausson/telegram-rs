//! Emoji catalogue for the composer's emoji picker.
//!
//! Sets are plain whitespace-separated `&str` constants: the view splits them
//! into an 8-column grid, and no parsing/allocation happens at startup.

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

#[cfg(test)]
mod tests {
    use super::*;

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
