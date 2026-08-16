# tg — a minimalist Telegram client in Rust

A lightweight, fast and minimalist **Telegram desktop client** written in Rust,
with a **100% software renderer** (no GPU required). Built for people who care
about resource usage: little RAM, little CPU, a clean and responsive UI — without
the hundreds of megabytes of mainstream clients.

## Quick start

Every [release](https://github.com/MrChausson/telegram-rs/releases) ships
ready-to-run binaries built by CI for Linux, macOS **and** Windows.

### 1. Install

**Linux** — pick one:

```bash
# AppImage (no install needed)
chmod +x tg-x86_64.AppImage
./tg-x86_64.AppImage

# Or tarball + installer:
tar xzf tg-linux-x86_64.tar.gz        # extracts app + install.sh
./install.sh                          # → ~/.local/bin/tg + menu entry
```

**macOS** (universal: Intel & Apple Silicon):

```bash
tar xzf tg-macos-universal.tar.gz
./tg
# Unsigned build → first launch: right-click → Open, or
# xattr -dr com.apple.quarantine tg
```

**Windows**:

```powershell
Expand-Archive tg-windows-x86_64.zip -DestinationPath tg
tg\tg.exe  # or right-click → Extract and double-click tg.exe (SmartScreen: "More info" → "Run anyway")
```

### 2. First launch: sign in inside the app (any OS)

On first launch the window shows a **sign-in screen** — enter your phone
number, then the code you receive by Telegram, and (if enabled) your
two-step verification password. The session is stored per-user (`tg` data
directory), and the app starts signed in next time.

No configuration file is needed for the released binaries: API credentials
are embedded at build time. Custom builds can override them with a `.env`
(next to the repository) or environment variables `API_ID` / `API_HASH`:

```bash
git clone https://github.com/MrChausson/telegram-rs
cd telegram-rs
echo "API_ID=123456"     >> .env   # from https://my.telegram.org (API tools)
echo "API_HASH=abcdef…"  >> .env
cargo build --release
```

> Receiving messages across sessions requires the same API credentials every
> time you launch — re-embed them at build time or keep the `.env` around.
> The chat list will not open until the session is valid.

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

Pre-built binaries for all platforms are attached to each
[release](https://github.com/MrChausson/telegram-rs/releases):

| Platform | Artifact | Contents |
|---|---|---|
| Linux (x86_64) | `tg-x86_64.AppImage` | Double-click, nothing to install |
| Linux (x86_64) | `tg-linux-x86_64.tar.gz` | `app` binary + `install.sh` |
| macOS (universal) | `tg-macos-universal.tar.gz` | Intel + Apple Silicon binary `tg` |
| Windows (x86_64) | `tg-windows-x86_64.zip` | `tg.exe` |

```bash
# Install the tarball build to ~/.local/bin (+ menu entry)
./install.sh

# Run the development build straight from the repo
cargo run --release -p app
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

## Status

- **Working MVP**: read, send and receive in real time, groups and channels,
  message edits and deletions.
- 47 unit tests (`cargo test`)
- Tested on Arch Linux (X11/Wayland); macOS and Windows are compiled by CI on
  every push/PR as well.

## CI / CD

`.github/workflows/build.yml` builds on every push/PR and runs the tests on
Linux, macOS **and** Windows. It produces per-platform artifacts:
- Linux: `tg-linux-x86_64.tar.gz` (binary + `install.sh`) and `tg-x86_64.AppImage`;
- macOS: `tg-macos-universal.tar.gz` (Intel + Apple Silicon via `lipo`);
- Windows: `tg-windows-x86_64.zip` (`tg.exe`).

On `v*` tags it also creates a **GitHub Release** with all three platforms'
artifacts. Releasing a new version is done from the **Actions** tab →
**Release** workflow → pick `patch` / `minor` / `major`; it bumps the version,
rotates the changelog, tags, and publishes the release.

## License

MIT.