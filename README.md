# tg — a feather-light Telegram client

A fast, minimalist **Telegram desktop client** written in Rust, rendered
**100% on the CPU** — no GPU required. It does everything a chat client
should while using a fraction of the RAM of mainstream clients.

- ⚡ **Real-time by push** — messages arrive in < 1 s, no polling spam
- 💾 **~10× less RAM** — ≈61 MB RSS vs 300-500 MB for Telegram Desktop
- 🧠 **100% CPU rendering** — runs on any hardware, low power draw
- 📦 **Tiny footprint** — ~4 MB binary, ~12 KB session file, no database
- 🔒 **Sign in inside the app** — phone → code → 2FA, nothing else to install

## Measured performance

Measured on a real session (HiDPI ~2x, Arch Linux, `--release`):

| Metric | tg | Telegram Desktop |
|---|---|---|
| **Resident memory (RSS)** | **~61 MB** | 300-500 MB |
| **PSS** (proportional size) | **~50 MB** | — |
| **Idle CPU** | **~0.7%** | — |
| **Threads** | **2** | dozens |
| **Binary size** | **~4.0 MB** | ~100+ MB |
| **Persisted session** | **~12 KB** | SQLite DBs |
| **Message latency** | < 1 s (push) | < 1 s |

> Methodology: `VmRSS` from `/proc/<pid>/status` and the sum of `Pss:` entries
> in `/proc/<pid>/smaps`, 15 s after launch with a chat open. VmSize (virtual
> address space) can exceed 1 GB with mimalloc — that's reserved address
> space, **not** physical memory (RSS/PSS are what matter).

## Quick start

Every [release](https://github.com/MrChausson/telegram-rs/releases) ships
ready-to-run binaries built by CI for Linux, macOS **and** Windows.
Download → run → sign in.

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

## Features & Roadmap to V1

**V1 = feature parity with the official Telegram Desktop.** That is the
target; below is where the project stands today compared to what V1 will bring.
Features are shipped in batches, so releases stay few and meaningful.

| Today (v0.2.1) | Planned for V1 (parity with Telegram Desktop) |
|---|---|
| Sign-in inside the app (phone → code → 2FA), persisted session | QR-code login, log out, multiple accounts, active-session management |
| Chat list: avatars, previews, unread counts, scrolling | Search, chat folders/archive, pinning, mute |
| Text messages, **real-time push** | Replies, forwards, editing/deleting your own messages, drafts, scheduled messages, polls |
| Photo thumbnails + full-screen photo viewer | Sending photos/videos/files, full-resolution downloads, stickers, GIFs, custom emoji, voice messages |
| Live message edits & deletions (from other devices) | Editing and deleting messages from this client |
| Groups & channels (read + send) | Creating and managing groups/channels, members, admin tools, topics/threads |
| Message timestamps | Date separators, sender names in groups, clickable links, text formatting |
| Dark theme | Light theme, settings screen, keyboard shortcuts, clipboard & context menus, text selection |
| Per-user data dir, HiDPI, 2 threads, ~61 MB RSS | Notifications + system tray, autostart, window-state persistence, audio |
| — | Typing indicator, read receipts, online status, mark-as-read |
| — | Global and in-chat message search |
| — | Secret (end-to-end) chats |
| — | Voice and video calls |

Milestones will be tracked here as the project moves toward V1.

## Why it's light

| Choice | Detail |
|---|---|
| **Pure CPU rendering** | `tiny-skia` (software rasterizer) + `softbuffer`: no GPU, no platform-graphics dependency at runtime. |
| **HiDPI-aware** | Rendered at physical resolution with scaled metrics — crisp text, no upscaling. |
| **No database** | Session persisted in a **small binary file** (~12 KB), no SQLite. |
| **Single network thread** | `current_thread` tokio runtime; 2 threads total (UI + network). |
| **No continuous redraw** | On-demand rendering + glyph cache; ~0.7% CPU at idle. |
| **Tiny binary** | `opt-level="z"`, `lto="fat"`, `panic="abort"`, stripped symbols. |

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

# 2. Launch the client and sign in in the window (phone → code → 2FA);
#    the session is saved automatically.
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

## Status

- **Working MVP**: read, send and receive in real time, groups and channels,
  message edits and deletions.
- 56 unit tests (`cargo test`)
- Tested on Arch Linux (X11/Wayland); macOS and Windows are compiled by CI on
  every push/PR as well.

## License

MIT.
