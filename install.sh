#!/usr/bin/env bash
# Linux installation of `tg` (minimalist Telegram client).
# Installs the binary into ~/.local/bin and adds a menu entry.
# Usage: ./install.sh
set -euo pipefail

BIN="${TGRAPH:-$(dirname "$0")/target/release/app}"
PREFIX="${PREFIX:-$HOME/.local}"

if [[ ! -x "$BIN" ]]; then
    echo "Binary not found: $BIN" >&2
    echo "Build it first: cargo build --release" >&2
    exit 1
fi

BINDIR="$PREFIX/bin"
APPDIR="$PREFIX/share/applications"
ICONDIR="$PREFIX/share/icons/hicolor/64x64/apps"
DEST="$BINDIR/tg"

install -d "$BINDIR" "$APPDIR" "$ICONDIR"
install -m 0755 "$BIN" "$DEST"

cat > "$APPDIR/tg.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=tg
Comment=Minimalist Telegram client in Rust
Exec=$DEST
Terminal=false
Categories=Network;Chat;
StartupNotify=true
EOF

# Icon: a small hand-written SVG so the .desktop entry never shows a blank image.
ICON="$ICONDIR/tg.svg"
cat > "$ICON" <<'EOF'
<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128">
  <rect x="4" y="4" width="120" height="120" rx="28" fill="#1e1f22"/>
  <circle cx="64" cy="64" r="48" fill="#2b2f36"/>
  <text x="64" y="82" font-family="sans-serif" font-size="56" font-weight="bold" fill="#32a852" text-anchor="middle">tg</text>
</svg>
EOF

# Refresh the application menu if the tool is available.
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APPDIR" 2>/dev/null || true
fi

echo "Installed: $DEST"
echo "Menu entry: $APPDIR/tg.desktop"
echo "Launch with: tg (or from the application menu)."