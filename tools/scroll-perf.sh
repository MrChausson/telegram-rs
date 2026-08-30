#!/usr/bin/env bash
# Measures end-to-end scroll frame-rate of app-iced WITHOUT needing real input
# (works on Wayland/X11, any machine): --scroll-perf self-drives a synthetic
# fling through the real update -> view -> layout -> draw -> present pipeline.
#
# Usage:
#   tools/scroll-perf.sh [--before] [secs]
#
# --before  rebuilds ALL message rows per frame (TG_NO_VIRT=1): the
#           pre-virtualization behaviour, to quantify what virtualization buys.
#
# IMPORTANT: this script always builds + runs the RELEASE binary. If you launch
# the app with `cargo run` (or a debug build) you get ~8x slower frames
# (measured 25.6 ms/frame vs 3.1 ms/frame at 1250x1514) which alone scrolls at
# ~10 fps.
#
# It prints three cases:
#   Plafond boucle (5 msg)   -> the event-loop ceiling (~120+ on a fast box)
#   420 messages (virtualise)-> the real scroll rate with a real chat
#   (optionally AVANT)       -> same but rebuilding every row per frame
# FINAL lines: fps=update cadence  ms=time between presents  renders_s=TRUE
# presented frames/sec (what you actually see).
set -euo pipefail
cd "$(dirname "$0")/.."

RUN=${RUN_S:-6}
BIN=target/release/telegram-rs
cargo build --release -p app-iced >/dev/null

run_case() { # $1 label, then binary args
  local label="$1"; shift
  local log; log="$(mktemp)"
  timeout "$((RUN + 40))s" env TG_PERF_LOG="$log" \
    "$BIN" "$@" >/dev/null 2>&1 || true
  echo "== $label =="
  grep FINAL "$log" || echo "   (no measurement)"
  rm -f "$log"
}

echo "=== Scroll perf (release build) ==="
run_case "Plafond boucle (petit chat 5 msg)  " --demo --scroll-perf="$RUN"
run_case "420 messages       (virtualise)    " --demo --demo-big --scroll-perf="$RUN"
if [[ " $* " == *" --before "* ]]; then
  TG_NO_VIRT=1 run_case "420 messages       AVANT (rebuild tout) " --demo --demo-big --scroll-perf="$RUN"
fi