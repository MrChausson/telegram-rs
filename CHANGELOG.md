# Changelog

All notable changes to this project are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- **Last name editing**: the profile edit form gains a "Last name" field.
  `SelfProfile`/`MyProfile` carry the server's `last_name` end-to-end
  (tg → network → bridge → state) instead of re-splitting the display name,
  and Save now sends it (`None` semantics dropped: an emptied field clears it).

### Changed
- **Emoji picker** pops above the composer like a context menu (300×240,
  bottom-left anchored) so the conversation behind it stays readable.

### Fixed
- Chat header search/info buttons sit flush against the right edge again:
  a stray `Length::Fill` spacer (plus the idle perf-badge slot) was splitting
  the free width and floating the icons mid-header.

## [v0.8.0] - 2026-08-26

### Added
- **Light theme**: a full light palette alongside the dark one, switchable
  from the settings panel and persisted across sessions (data-dir marker,
  disabled under `--demo`). Iced's own base palette follows the mode so
  default-styled surfaces adapt too.

### Fixed
- **Chat header icons** (search / info) are right-aligned with a proper
  margin and vertically centered again.
- **Unread badges** no longer sit over the last-message preview: the right
  meta column (timestamp + badge) has a reserved fixed width and previews
  ellipsize earlier.

## [v0.7.0] - 2026-08-24

### Changed
- **UI language switched to English**: every user-facing string (context menu,
  pinned banner, composer, upload bar, media placeholders/actions, forward
  overlay, search views, byte sizes, demo content) is now English.

### Added
- **Stickers**: incoming and outgoing stickers render frameless (no bubble) as
  a centered ~180 px image with a discreet timestamp. A sticker button next to
  the attach 📎 opens a floating picker panel above the composer: installed
  packs (title + 4-column thumbnail grid, images cached on disk); clicking a
  sticker sends it by document reference. Stickers classify ahead of other
  document attributes (`DocumentAttributeSticker`), download through the
  shared concurrency-capped pipeline into `cache/stickers/`, and the demo
  ships two generated packs plus incoming/outgoing sticker messages in Rust
  Group.
- **Group & channel creation**: a "+" button in the chat-list header opens a
  New Group / New Channel picker; the modal takes a title (+ description for
  channels) and, for groups, a checkable member list seeded from your known
  contacts. Groups go through `messages.createChat` (initial invites), groups
  without members and channels through `channels.createChannel`; the new chat
  opens as soon as the server confirms.
- **Leave or delete chats**: right-click a chat row for a Leave / Delete mini
  menu with a confirmation dialog. Leaving ends membership (`channels.leaveChannel`
  / `messages.deleteChatUser`), deleting removes the dialog from the account;
  the list refreshes and the open chat closes if it was the one removed.
- **Rename plumbing**: `EditChatTitle` request + network handler (channels via
  `channels.editTitle`, basic groups via `messages.editChatTitle`), ready to be
  wired to UI later.
- **Chat info panel**: clicking the chat header (or the ℹ️ icon) opens a
  right-hand side panel with the chat's details — avatar, title, members
  count / presence, @username (click to copy), bio and phone. Quick actions
  cover mute/unmute and in-chat search; ✕ or Escape closes it.
- **Members list**: groups and channels list their participants in the info
  panel with role badges ("Owner" / "Admin") and an inline remove action
  (kick) with an in-panel confirmation step.
- **Pinned messages**: pin/unpin any message from its context menu; a banner
  under the chat header shows the pinned snippet ("Pinned") and clicking it
  jumps straight to the message in the list. Pin state syncs live across
  devices (`updatePinnedMessages`), and deleting the pinned message clears the
  banner.
- **Group sender names**: incoming messages in groups/channels show their
  author's name above the bubble, in a deterministic per-sender color from
  Telegram's 7-hue palette (private chats are unaffected). The virtualized
  row-height estimate accounts for the extra line.
- **Emoji picker in the composer**: a smiley button left of the input opens a
  floating panel above it — "Recents" (persisted across sessions, capped at
  24, with a starter set until first use) plus standard grouped sets (Smileys
  & People, Animals & Nature, Food & Drink, Activities, Objects, Symbols).
  Picking an emoji appends it to the composer without sending; a click
  outside or Escape closes the panel.

## [v0.6.1] - 2026-08-24

### Added
- **Settings panel**: a right sheet from the "Chats" header gear with
  Profile (edit name/bio via `account.updateProfile`), a notifications
  toggle (persists, gates desktop notifications), storage usage with a
  clear-cache action (confirmation + safe paths), and the active sessions
  list with per-device termination (`account.getAuthorizations` /
  `resetAuthorization`). Includes the theme switch row.

### Fixed
- **Long unbreakable words (URLs) no longer overflow their bubble**: message
  text wraps at the glyph level when a single word exceeds the bubble width
  (`WordOrGlyph` wrapping).
- **The context menu no longer pushes the conversation down**: it now floats
  as an overlay anchored under the right-clicked message (above it when there
  is no room below), without participating in the list layout. It also stays
  anchored while its row is scrolled out of the virtualized window.

### Added
- **Clickable links**: http(s)/`www.` URLs inside messages render in the
  accent color, underlined, and open in the system browser on click. Messages
  without links keep the plain-text hot path (zero scroll-perf cost).
- **Reopen the last chat**: the app now reopens the chat you had open when it
  was closed (persisted in the data dir; falls back to the first chat, and is
  disabled in `--demo` so QA runs stay hermetic). Going back to the list
  forgets the marker.
- **Tray logo**: the StatusNotifier item now embeds the app mark (accent disc
  + paper plane, rendered at 32/64 px) instead of relying on a themed icon
  name — plus a tooltip.

## [v0.6.0] - 2026-08-24

### Changed
- **UI design pass (Material 3 inspired)**: the message context menu is now a
  proper menu surface — 12 px rounded corners, hairline border, inner padding,
  icons on **every** item (reply/forward arrows redrawn with filled heads,
  new pencil/copy/trash icons) in an aligned column, hover state per item and
  a red-tinted destructive "Supprimer".
- **App logo**: paper-plane mark on an accent disc in the "Chats" header;
  the non-functional compose/dots glyphs were removed (dead chrome).
- **Icon buttons** (search, back, close, paperclip…) get a circular hover
  state; send/accent buttons get hover/press color variants.
- **Voice notes** use drawn play/pause icons instead of font glyphs.
- All icon strokes now use round caps/joins (softer, crisper at small sizes).

## [v0.5.0] - 2026-08-23

### Added
- **Media cards for videos, GIFs and audio files**: received media render as
  dedicated cards (icon + name + size, duration shown for video/audio) instead
  of generic documents; clicking downloads then opens them with the system
  player (`xdg-open`).
- **Voice-note playback in-app**: voice messages get a play/pause bar with a
  live progress line and elapsed time; playback uses `rodio`, pauses/resumes on
  click, stops when another voice note starts.
- **Send richer media**: outgoing photos are classified by extension — images
  go as compressed photos, GIFs/videos/audio as documents with proper MTProto
  attributes (animated/video/audio) so receivers see the right kind.
- **Desktop notifications**: new messages in chats that aren't open raise a
  desktop notification (`notify-rust`); best-effort, never blocks the loop.
- **System tray**: StatusNotifier tray icon (`ksni`, pure Rust — no GTK) with
  "Open tg" / "Quit" actions; silently a no-op where no tray host exists.

## [v0.4.0] - 2026-08-23

### Added
- **Replies**: right-click any message → "Répondre" arms a reply bar above the
  composer (with snippet + ✕ to cancel); sent messages carry a quoted "Réponse"
  header inside the bubble.
- **Forwards**: right-click → "Transférer" opens a chat picker; the copy lands
  in the destination with a "Transféré de …" header (originating chat resolved
  from the dialog list, or the anonymous forward name).
- **Send photos & documents**: a paperclip button opens the native file dialog
  (`rfd` / xdg-desktop-portal); images are sent as compressed photos, any
  other file as a document. Media rows show a **live upload-progress bar**
  (per-frame `%` + fill) fed by the real stream, then merge with the server
  echo. Sent photos render as photo bubbles, documents as a file card
  (icon + name + size) — click to download (cached), click again to open with
  the system opener.
- Document messages from other devices render as file cards too; a small
  "Télécharger"/"Ouvrir" status guides the first/second click.
- **Search**: the chat-list header's 🔍 opens a global search across all chats
  (result rows show chat title + snippet + time; clicking opens the chat), and
  the conversation header's 🔍 searches inside the open chat (results listed
  in place, click jumps to the message when it's in the loaded history).
  Queries are throttled/de-duplicated in the network layer so typing doesn't
  flood MTProto.

### Changed
- The message context menu now opens on **any** message (reply/forward), with
  Modifier/Copier/Supprimer still restricted to your own text messages.
- The demo backend echoes media sends with simulated upload progress, seeds
  reply/forward/photo/document rows in the first chat, and throttles its
  simulated incoming messages so it stays a conversation, not a flood.
- The `--perf` FPS badge no longer shows up in plain `--demo` runs (it was
  force-enabled by the `--scroll-perf` default).

### Fixed
- A media send echo merges into its optimistic row even when the message has
  no caption anymore (an uploading row is matched by its upload state, not
  only by text), so a photo/doc send without caption no longer duplicates.

## [v0.3.0] - 2026-08-23

### Changed
- **Switched the renderer to wgpu (GPU)** with the tiny-skia software renderer
  kept as automatic fallback for machines without a usable GPU stack. The
  software raster path was the source of the interaction lag: every frame re-
  rasterized all rounded rects through tiny-skia's anti-aliased scan pipeline
  on the CPU (66-250 ms/frame in debug, ~15 ms/frame in release), pinning a
  core at 30-100% CPU. With wgpu: idle/hover/scroll all sit at ~1% CPU and
  the big chat scroll rate goes from ~24 to ~380 rendered fps.
- **RAM: the wgpu GL backend is the default** — measured on the reference
  machine (NVIDIA, Wayland, demo mode, release): PSS 47 MB on EGL/GL vs
  115 MB on Vulkan (-68 MB of resident proprietary-Vulkan driver pages) at
  identical CPU cost and higher scroll throughput. For comparison, the
  software tiny-skia path sits at 25-28 MB PSS. Override with `WGPU_BACKEND`
  (`vulkan`, `gles`, …) if a machine has a broken GL stack — the tiny-skia
  fallback still catches total GPU failure.
- Rewrote the UI on **Iced (tiny-skia software backend)** — replaces the
  custom winit/softbuffer renderer. Roughly half the RAM (~30 MB RSS), with
  headless-tested application state.
- Dropped the old `app/` and `ui/` crates; the workspace is now `tg` (MTProto
  core) + `app-iced` (UI).
- **Winit visual parity pass**: the iced UI now matches the winit client's
  design tokens — same conversation background, chat header on the list
  background, proportional 60/70% bubbles with outside timestamps (left of
  sent, right of received), single-line ellipsized chat rows, "Chat" /
  "typing…" header status and the 2FA password masked with bullets.
- The message context menu now floats under the message that raised it
  (right-aligned), instead of a fixed corner overlay.

### Added
- Auto-focus isn't feasible in iced's layout; the composer is a plain text
  input (click to focus). Copy: message text can be copied from the context
  menu ("Copier"); drag-to-select inside bubbles isn't supported by the iced
  text widget and is a documented trade-off of the software renderer (see
  README).
- Live typing is now also sent to the server while the user types
  (`Request::Typing`), and the demo backend simulates typing + incoming
  messages in Camille's chat.
- While a chat's history is being fetched the pane shows "Loading…" instead
  of the misleading "No messages yet" (which now only means an actually empty
  chat).

### Fixed
- Opening a chat no longer takes ~a minute: avatar and photo-thumbnail
  downloads now run in the background through a shared semaphore (4 concurrent
  transfers) instead of being awaited one-by-one in the network loop, which
  queued every click behind the whole startup avatar flood. `OpenChat` is also
  explicitly prioritized over slower downloads. (fix regressions: tests
  assert the open-chat → history flow and the priority ordering)
- Profile avatars are rendered as circles again: the tiny-skia backend ignores
  `border_radius` on image widgets, so avatars are now decoded, cover-cropped
  and alpha-masked into a disc once per (path, size) and memoized (keyed by the
  raster pipeline's handle id, so the decoded image stays cached across frames).
- The list and conversation headers no longer hug the top edge: their content
  is vertically centered (`.align_y(Center)`), matching the winit look.
- Restarting with a valid session no longer shows the sign-in screen: the
  chat list arriving now marks the account as authenticated, so a persisted
  session opens straight into the chats.
- Nested icon rendering: icons are rasterized with tiny-skia and shown as
  images, sidestepping an `iced_tiny_skia` canvas-translation bug that made
  embedded icons invisible.
- The sign-in screen is now centered in the window.
- The open chat no longer leaks messages from other chats (filtered by id)
  and receives live updates (incoming message, edit, delete, read state),
  replacing the earlier renderer-loop simulation. Unread badges clear on
  open (mark-as-read) and only sync down from other devices.
- Optimistic sends are deduplicated when the server echoes them back, and
  Escape closes the context menu / cancels editing / closes the viewer.
- Demo mode now handles the full cycle: echo of sent messages, simulated
  typing + incoming messages with generated images for avatars and photos.

### Performance
- **Message-list layout cache** (Phase 1): row heights / cumulative top
  offsets are computed once and cached in `State`, invalidated only when the
  messages or the context menu change (or on resize). Scroll frames now cost
  O(visible rows) via binary search over the cached offsets instead of
  re-estimating every row's height each tick — `messages_list` build is flat
  ~15 µs whether the chat holds 50 or 5000 messages (was scaling linearly).
- **Cheaper per-frame view** (Phase 2): memoized "HH:MM" timestamps (no more
  chrono/timezone work per visible row per frame), pre-ellipsized chat-list
  labels kept aligned with the dialog list (`State::dialog_short`, refreshed
  only when a preview changes), and borrowed header title/avatar instead of
  per-frame clones. Full-view build: 75.7 → 48.3 µs with 50 dialogs; message
  list build at 200 messages: 19.9 → 11.9 µs.
- **Dialog-list virtualization** (Phase 2bis): the left pane now builds only
  the rows intersecting its viewport (uniform row height, O(1) windowing,
  ±16 rows overscan), like the message list. Whole-view build drops from
  48 µs to 2.3 µs; scrolling 800 chats costs about the same as scrolling 50.
  New `dialog_list/scroll/{n}` criterion benches guard both.
- **Perf regression harness** (`cargo bench -p app-iced`): the app is now a
  lib + thin bin so `benches/frame.rs` drives the *exact* per-frame view
  headlessly and measures **build / layout / frame** (diff + layout + draw on a
  tiny-skia software canvas) for 10 / 50 / 200 / 500 messages, publishing
  estimated FPS. Criterion-change data pins regressions in CI.
- **Message list virtualization**: only the rows intersecting the scrollable's
  viewport (plus over-scan) are built and layed-out each frame; the rest is
  height-matched spacers keeping the content height and bottom-anchoring
  correct. A scroll tick used to rebuild + text-shape every row of the whole
  history (the tiny-skia backend is software-rendered), which lagged badly on
  chat open / long histories. Estimated heights (char-based, no shaping) for
  the spacers, O(1) per row.
- **Icon handles are now memoized** (keyed by kind/color/size): `Handle::from_rgba`
  mints a fresh cache id per call, so without memoization every icon was
  re-rasterized on every frame — scroll lag again.
- `--perf` flag: draws a live FPS badge in the conversation header (sampled on
  scroll + a 500 ms tick) to measure on the real display.
- **End-to-end scroll measurement** without input bindings: `--scroll-perf=SECS`
  self-drives a synthetic fling through the real update → view → layout → draw
  → present pipeline and logs per-frame ms / fps (`TG_PERF_LOG`), so scroll
  performance is measurable on any machine. `tools/scroll-perf.sh` runs it
  automatically. `--demo-big` (+`TG_BIG_N`) seeds a ~420-message chat to
  exercise long histories, and `--win=WxH` shrinks the render buffer.
  `TG_NO_VIRT=1` restores the pre-virtualization "build every row per frame"
  behaviour for honest before/after numbers: at 1500 messages the virtualized
  list sustains **79 fps vs 63 fps** unvirtualized on the reference machine,
  with the per-frame cost staying flat as the history grows.
- **FPS instrumentation now reports the TRUE presented frame rate** (`renders_s`,
  counted in the redraw path), not the scroll-event cadence. Measured on the
  reference host: headless per-frame is **3.1 ms under a real fling** (vs
  **11–17 ms** for the winit client's whole-scene render at the same
  resolution), and the live loop presents **~127 frames/s with a 5-message
  chat** vs **~24–29 frames/s** with a 420+ history — i.e. the presented rate is
  content-bound (per-frame software cost), not a fixed loop cap. Ships
  `tools/scroll-perf.sh` comparing both cases and a `--continuous` redraw mode.
  **Debug builds are ~8× slower (25.6 vs 3.1 ms/frame)**: always use the
  release binary when checking scroll feel (`cargo run --release`).

## [v0.2.2] - 2026-08-16

### Changed
- README: added a V1 roadmap comparing current features with the full
  Telegram Desktop feature set.

### Changed
- README: added a V1 roadmap comparing current features with the full
  Telegram Desktop feature set.

## [v0.2.1] - 2026-08-16

### Fixed
- Restarting with a valid session no longer shows the sign-in screen: the
  chat list arriving now marks the account as authenticated, so a persisted
  session opens straight into the chats.

## [v0.2.0] - 2026-08-16

### Added
- In-app sign-in: the released binary can log in by itself (phone → code →
  two-step verification), no console helper or Rust toolchain required.
- Per-user data directory (`.env`, session and cache) so the installed binary
  works from anywhere; `TG_DATA_DIR` overrides it.
- API credentials are embedded at build time from the `TG_API_ID` /
  `TG_API_HASH` Action secrets, overridable via `.env` — the first launch
  shows a branded login screen instead of a dead end.

## [v0.1.0] - 2026-08-16

First public release of **tg**, a minimal Telegram client written in Rust,
rendered natively with tiny-skia and winit (no GPU or browser engine).

### Added
- Chat list showing real profile photos for users, groups, and channels,
  downloaded and cropped into round avatars.
- Text and photo message bubbles; click a photo to view it full screen.
- Local session storage via the telegram-rs low-level API (`.tg.session`).
- Release automation: Linux build, test suite, tarball and AppImage, tagged
  GitHub releases with assets (`v0.1.0`, 2026-08-16).