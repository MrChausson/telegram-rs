# tg — a minimalist Telegram client in Rust

A lightweight, fast and minimalist **Telegram desktop client** written in Rust,
with a **100% software renderer** (no GPU required). Built for people who care
about resource usage: little RAM, little CPU, a clean and responsive UI — without
the hundreds of megabytes of mainstream clients.

## Why it's light

| Choice | Detail |
|---|---|
| **Pure CPU rendering** | `tiny-skia` (software rasterizer) + `softbuffer`: no GPU, no platform-graphics dependency at runtime. |
| **HiDPI-aware** | Rendered at physical resolution with scaled metrics — crisp text, no upscaling. |
| **No database** | Session persisted in a **small binary file** (~12 KB), no SQLite. |
| **Single network thread** | `current_thread` tokio runtime; 2 threads total (UI + network). |
| **No continuous redraw** | On-demand rendering + glyph cache; ~0.7% CPU at idle. |
| **Tiny binary** | `opt-level="z"`, `lto="fat"`, `panic="abort"`, stripped symbols. |

## Measured performance

Measured on a real session (HiDPI ~2x, Arch Linux, `--release`):

| Metric | Measured value |
|---|---|
| **RSS** (resident memory) | **~61 MB** |
| **PSS** (proportional set size) | **~50 MB** |
| **Idle CPU** | **~0.7%** |
| **Threads** | **2** |
| **Binary size** | **~4.0 MB** |
| **Persisted session** | **~12 KB** |
| **Real-time message latency** | < 1 s (push stream, like official apps) |

For reference, **Telegram Desktop (tdesktop, C++) easily exceeds 300-500 MB of
RSS on an active account**. This client uses **~10x less**, with no GPU.

> Methodology: `VmRSS` from `/proc/<pid>/status` and the sum of `Pss:` entries in
> `/proc/<pid>/smaps`, 15 s after launch with a chat open. VmSize (virtual
> address space) can exceed 1 GB with mimalloc: that's reserved address space,
> **not** physical memory (RSS/PSS are what matters).

## Features (MVP)

- [x] User-account login (phone → code → 2FA), persisted session
- [x] Chat list (avatars, previews, unread counts, scrolling)
- [x] Chat view: history, send messages, **real-time receiving (push)**
- [x] Live message edits and deletions
- [x] HiDPI software rendering with adjustable scale
- [ ] Media, stickers, calls, search (out of MVP scope)

## Project layout

```
app/   → main binary (UI ↔ network bridge)
ui/    → custom UI: winit + softbuffer + tiny-skia + fontdue
tg/    → core networking: grammers (MTProto), persisted session
```

- **MTProto**: [grammers](https://github.com/Lonami/grammers) (Rust) — a real
  user client, compatible with all other Telegram clients.
- **Rendering**: `tiny-skia` (CPU), `softbuffer` (window framebuffer), `winit`
  (window/input) — cross-platform Linux/macOS/Windows.

## Build & run

Prerequisites: stable Rust.

```bash
# 1. Create an API application at https://my.telegram.org (API development
#    tools) and put the credentials in .env
echo "API_ID=123456"     >> .env
echo "API_HASH=abcdef..." >> .env

# 2. Interactive login (phone → code → token), saves the session
cargo run -p tg --example login

# 3. Launch the client
cargo run --release -p app
```

### Settings

| Variable | Effect |
|---|---|
| `TG_UI_SCALE` | UI scale factor (default: auto, 1.6 when undetected) |
| `TG_SCROLL_INVERT` | `1` to flip the scroll direction |

### Real-time test

Open a chat, send a message from your phone: it appears almost instantly (push);
a 15 s safety net catches any missed update.

## Methods

```bash
# Install to ~/.local/bin (+ menu entry)
./install.sh

# Or download the Linux tarball / AppImage from the GitHub Releases
# (triggered by CI on each v* tag).
```

## Notable technical decisions

- **No GPU**: the entire pipeline is on the CPU — low power usage, no shell
  graphics dependencies.
- **Network safety net**: the push stream is the primary source; a discreet
  refresh (15 s on the open chat, ~4 requests/min) guarantees nothing is lost
  without spamming requests.
- **Light session**: custom binary storage (bincode), atomic writes, saved
  periodically so restarts don't re-sync from scratch.

## Status

- **Working MVP**: read, send and receive in real time, groups and channels,
  message edits and deletions.
- 47 unit tests (`cargo test`)
- Tested on Arch Linux (X11/Wayland); targets macOS/Windows (winit/softbuffer).

## CI / CD

`.github/workflows/build.yml` builds on every push/PR, runs the tests, and
produces:
- a Linux tarball (`tg-linux-$(uname -m).tar.gz`) with the binary and
  `install.sh`;
- an **AppImage** (`tg-x86_64.AppImage`).

On `v*` tags it also creates a **GitHub Release** with both artifacts.

## License

MIT.