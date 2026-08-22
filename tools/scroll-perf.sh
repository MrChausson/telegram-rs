#!/usr/bin/env bash
# Measures end-to-end scroll frame-rate of app-iced WITHOUT needing real input
# (works on Wayland/X11, any machine): --scroll-perf self-drives a synthetic
# fling through the real update -> view -> layout -> draw -> present pipeline.
#
# Usage:
#   tools/scroll-perf.sh [--before] [secs]
#
# --before  rebuilds ALL message rows per frame (TG_NO_VIRT=1), i.e. the
#           pre-virtualization behaviour, to quantify what virtualization buys.
# Run it twice (default then --before) to get an honest avant/apres.
#
set -euo pipefail
cd "$(dirname "$0")/.."

RUN=${RUN_S:-8}
BIN=target/release/app-iced
cargo build --release -p app-iced >/dev/null

if [[ " $* " == *" --before "* ]]; then
    export TG_NO_VIRT=1
    TAG="AVANT (chaque frame reconstruit toute l'histoire)"
else
    TAG="APRES (liste virtualisee)"
fi

LOG="$(mktemp)"
export TG_PERF_LOG="$LOG"

timeout "$((RUN + 40))s" "$BIN" --demo --demo-big --scroll-perf="$RUN" >/dev/null 2>&1

echo "== $TAG =="
# FINAL writes: fps (event-update cadence) and renders_s (TRUE presented fps).
grep FINAL "$LOG" || echo "pas de mesure (fenetre Rate croisee?)"
rm -f "$LOG"