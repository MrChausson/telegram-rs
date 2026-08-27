# Telegram RS — a feather-light Telegram client

A fast, minimalist **Telegram desktop client** written in Rust, real-time via
MTProto push. Rendered with a **GPU by default** (wgpu, GL backend) with an
automatic software fallback — a fraction of the RAM of mainstream clients.

- ⚡ **Real-time by push** — messages arrive in < 1 s, no polling spam
- 💾 **~10× less RAM** — ≈40-50 MB RSS vs 300-500 MB for Telegram Desktop
- 🖥️ **GPU rendering (wgpu/GL)** — buttery scrolling on long chats, ~1% CPU
- 📦 **Tiny footprint** — ~8 MB binary, ~12 KB session file, no database
- 🔒 **Sign in inside the app** — phone → code → 2FA, nothing else to install

## Measured performance

Measured on a real session (HiDPI 1.6×, Arch Linux, NVIDIA/Wayland, `--release`):

| Metric | Telegram RS | Telegram Desktop |
|---|---|---|
| **Resident memory (RSS)** | **~40-50 MB** | 300-500 MB |
| **PSS** (proportional size, +NVIDIA driver) | **~47 MB** | — |
| **Idle CPU** | **~1%** | — |
| **Scroll, 420-msg chat** | **~275-380 rendered fps** | — |
| **Threads** | **2** | dozens |
| **Binary size** | **~8 MB** | ~100+ MB |
| **Persisted session** | **~12 KB** | SQLite DBs |
| **Message latency** | < 1 s (push) | < 1 s |

> Methodology: `VmRSS` from `/proc/<pid>/status` and the sum of `Pss:` entries
> in `/proc/<pid>/smaps`, 15 s after launch with a chat open. VmSize (virtual
> address space) can exceed 1 GB with mimalloc — that's reserved address
> space, **not** physical memory (RSS/PSS are what matter). The GPU path loads
> the driver's private heap (~20 MB Private_Dirty), which is why RSS is higher
> than the old software renderer (which pinned a core at ~24 fps on big chats).

## Quick start

Every [release](https://github.com/MrChausson/telegram-rs/releases) ships
ready-to-run binaries built by CI for Linux, macOS **and** Windows.
Download → run → sign in.

### 1. Install

**Linux** — pick one:

```bash
# Arch Linux / Manjaro (AUR, updated automatically on every release)
paru -S telegram-rs-bin        # or: yay -S telegram-rs-bin

# AppImage (no install needed)
chmod +x telegram-rs-x86_64.AppImage
./telegram-rs-x86_64.AppImage

# Or tarball + installer:
tar xzf telegram-rs-linux-x86_64.tar.gz   # extracts app + install.sh
./install.sh                              # → ~/.local/bin/telegram-rs + menu entry
```

**macOS** (universal: Intel & Apple Silicon):

```bash
tar xzf telegram-rs-macos-universal.tar.gz
./telegram-rs
# Unsigned build → first launch: right-click → Open, or
# xattr -dr com.apple.quarantine telegram-rs
```

**Windows**:

```powershell
Expand-Archive telegram-rs-windows-x86_64.zip -DestinationPath telegram-rs
telegram-rs\telegram-rs.exe  # or right-click → Extract and double-click telegram-rs.exe (SmartScreen: "More info" → "Run anyway")
```

### 2. First launch: sign in inside the app (any OS)

On first launch the window shows a **sign-in screen** — enter your phone
number, then the code you receive by Telegram, and (if enabled) your
two-step verification password. The session is stored per-user (a `telegram-rs` data
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

## Roadmap to V1

**V1 = full parity with the official Telegram Desktop.** Features ship in
batches — each release moves the needle, not dribbles.

### ✅ Already there

- 💬 **Text messaging** — real-time push, < 1 s
- 🖼️ **Avatars** — users, groups and channels
- 📸 **Photos** — thumbnails + full-screen viewer
- 📤 **Send media** — photos, documents, videos and GIFs with a live upload-progress bar
- 🎬 **Media cards** — videos, GIFs and audio files render as dedicated cards (duration shown); open in the system player
- 🎙️ **Voice notes** — recorded voice messages play in-app with a play/pause bar and progress
- 🔔 **Desktop notifications** — new messages in non-open chats raise a desktop notification
- 🫗 **System tray** — tray icon with "Open" / "Quit" actions (StatusNotifier, no GTK)
- 🔗 **Clickable links** — URLs in messages open in the system browser
- 🚪 **Reopen last chat** — the app relaunches straight into the chat you had open
- ↩️ **Replies & forwards** — quote a message in reply, or forward it to any chat
- 🔍 **Search** — global across chats, or inside the open chat
- ✏️ **Live sync** — edits and deletions from any device
- 🔑 **In-app sign-in** — phone → code → 2FA, session persisted
- 🟢 **Presence** — typing indicator, read receipts, unread badges, mark-as-read
- 📋 **Context menu** — reply, forward, pin, edit, copy or delete messages
- 📌 **Pinned messages** — banner under the header; click to jump to the message
- 👥 **Group senders** — author names above bubbles, one color per sender
- ➕ **Create groups & channels** — "+" in the chat list; groups invite picked contacts, channels take a description
- 🚪 **Leave / delete chats** — right-click a chat row and confirm
- ℹ️ **Chat info panel** — details (username, bio, members count), mute and
  in-chat search quick actions, plus the member list of groups/channels with
  owner/admin badges
- 🧩 **Stickers** — frameless rendering in chats, a picker panel next to the
  composer (packs + thumbnail grid) and sending by document reference

> 📋 **Copy / paste note:** message text can be copied from the message's
> context menu (right-click → “Copy”), and text pastes into the composer
> with Ctrl/⌘-V. Drag-to-select inside message bubbles isn't supported by the
> Iced text widget — a documented trade-off.

### 🚧 Next up

| Area | In V1 |
|---|---|
| 👤 Accounts | Logout, QR login *(multi-account: not planned for now)* |
| 👥 Groups | Admin tools (promote/demote, ban/kick), then forum topics *(create/manage groups & channels, member list with roles already shipped)* |
| 🔐 Privacy | Secret (end-to-end) chats *(later)* |
| 📞 Calls | Voice and video calls |

> Custom emoji is premium-gated upstream and deliberately out of scope.

## Why it's light

| Choice | Detail |
|---|---|
| **GPU rendering** | [Iced](https://iced.rs) on **wgpu** with the **GL backend** by default (`WGPU_BACKEND` overrides it; e.g. `vulkan`); **tiny-skia software** is the automatic fallback when no GPU adapter is found. |
| **HiDPI-aware** | Rendered at physical resolution with scaled metrics — crisp text, no upscaling. |
| **No database** | Session persisted in a **small binary file** (~12 KB), no SQLite. |
| **Single network thread** | `current_thread` tokio runtime; 2 threads total (UI + network). |
| **No continuous redraw** | On-demand rendering; ~1% CPU at idle. |
| **Tiny binary** | `opt-level="z"`, `lto="fat"`, `panic="abort"`, stripped symbols. |

## Project layout

```
app-iced/ → Iced-based UI (wgpu/GL, tiny-skia fallback) + app state, headless-tested
tg/       → core networking: grammers (MTProto), persisted session
```

- **MTProto**: [grammers](https://github.com/Lonami/grammers) (Rust) — a real
  user client, compatible with all other Telegram clients.
- **UI**: [iced](https://iced.rs) rendered with **wgpu (GL backend)** — GPU
  acceleration on Linux/macOS/Windows, transparently falling back to the
  software renderer.

## Build & run

Prerequisites: stable Rust.

```bash
# 1. Create an API application at https://my.telegram.org (API development
#    tools) and put the credentials in .env
echo "API_ID=123456"     >> .env
echo "API_HASH=abcdef..." >> .env

# 2. Launch the client and sign in in the window (phone → code → 2FA);
#    the session is saved automatically.
cargo run --release -p app-iced
```

### Demo mode

A canned offline backend exercises the UI without a network session:

```bash
cargo run --release -p app-iced -- --demo --open-first
# Long-history chat for scroll performance (420 msgs; TG_BIG_N=N to override):
cargo run --release -p app-iced -- --demo --demo-big
```

### Measuring performance (scroll, frame rate)

- `--perf` shows a live **FPS badge in the conversation header** counting
  actually-presented frames (verified against `renders_s` in the log).
- `--scroll-perf=SECS` self-drives a synthetic fling through the real
  update→view→layout→draw→present pipeline and prints a `FINAL` line
  (`fps` = event cadence, `renders_s` = TRUE presented frames/s). See
  `tools/scroll-perf.sh` for an automatic 3-way comparison
  (loop ceiling vs 420-msg chat vs pre-virtualization).
- `WGPU_BACKEND=vulkan` restores the Vulkan adapter; `gl` is the default.

> **Always use a release build for perf checks.** Debug is ~8× slower
> (~25 ms/frame vs ~3 ms/frame) and alone looks like a lag bug.

### Real-time test

Open a chat, send a message from your phone: it appears almost instantly (push);
a 15 s safety net catches any missed update.

## Status

- **Working MVP**: read, send and receive in real time, groups and channels,
  message edits and deletions, replies, forwards, photo/file sending, global
  and in-chat search, and in-app sign-in with a persisted session.
- **39 tests** (`cargo test --workspace`), including headless UI-state tests
  and a criterion frame-cost benchmark (`cargo bench -p app-iced --bench frame`).
- Tested on Arch Linux (X11/Wayland); macOS and Windows are compiled by CI on
  every push/PR.

## License

MIT.
