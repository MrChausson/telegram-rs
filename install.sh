#!/usr/bin/env bash
# Linux installation of Telegram RS (minimalist Telegram client in Rust).
# Installs the binary into ~/.local/bin and adds a menu entry.
# Usage: ./install.sh
set -euo pipefail

BIN="${TG_BIN:-$(dirname "$0")/target/release/telegram-rs}"
PREFIX="${PREFIX:-$HOME/.local}"

if [[ ! -x "$BIN" ]]; then
    echo "Binary not found: $BIN" >&2
    echo "Build it first: cargo build --release" >&2
    exit 1
fi

BINDIR="$PREFIX/bin"
APPDIR="$PREFIX/share/applications"
ICONDIR="$PREFIX/share/icons/hicolor/64x64/apps"
DEST="$BINDIR/telegram-rs"

install -d "$BINDIR" "$APPDIR" "$ICONDIR"
install -m 0755 "$BIN" "$DEST"

cat > "$APPDIR/telegram-rs.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Telegram RS
Comment=Minimalist Telegram client in Rust
Exec=$DEST
Terminal=false
Categories=Network;Chat;
StartupNotify=true
EOF

# Icon: the app logo (accent disc + white send-plane), drawn as a small
# hand-written SVG so the .desktop entry never shows a blank image.
# Same geometry as assets/icon.svg / app-iced/src/icons.rs Icon::Logo.
ICON="$ICONDIR/telegram-rs.svg"
cat > "$ICON" <<'EOF'
<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128">
  <circle cx="64" cy="64" r="64" fill="#3390ec"/>
  <path transform="translate(25.6 24.32) scale(3.30667)"
        d="M2 21 L23 12 L2 3 L2 10 L16 12 L2 17 Z" fill="#ffffff"/>
</svg>
EOF

# Refresh the application menu if the tool is available.
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APPDIR" 2>/dev/null || true
fi

echo "Installed: $DEST"
echo "Menu entry: $APPDIR/telegram-rs.desktop"
echo "Launch with: telegram-rs (or from the application menu)."
