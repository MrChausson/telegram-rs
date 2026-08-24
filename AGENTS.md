# AGENTS.md — tg (Rust Telegram client)

Session bootstrap guide: architecture, commands, hard-won build/perf knowledge,
and environment quirks. Read top-to-bottom before touching code.

## Project at a glance

Minimal Telegram desktop client in Rust, real-time via MTProto push.

- **`tg/`** — core networking: wrapper around `grammers` (MTProto), persisted
  session, dialogs/messages/avatars/photos, typing/read-receipts.
- **`app-iced/`** — UI as an **Iced** application (GPU via wgpu, GL backend by
  default; tiny-skia software renderer as automatic fallback), plus pure
  headless-tested application state. This is where all UI work happens.
- The old `app/` + `ui/` (custom winit/softbuffer renderer) crates were
  **removed** in v0.3.0 (PR #5) — only `app-iced` remains.

`version = "0.3.0"` (workspace). Keep-a-Changelog in `CHANGELOG.md`, one
commit per logical change, conventional prefixes (`feat:` `fix:` `perf:`
`chore:` `docs:` `style:`).

## Command reference

```bash
# Run the client (RELEASE. Debug is ~8x slower for rendering — see Perf).
cargo run --release -p app-iced
# Demo mode: canned offline backend, no session needed.
#   --open-first  auto-opens the first chat (demo auto-does it)
cargo run --release -p app-iced -- --demo
# Long-history demo chat for scroll perf (420 msgs; TG_BIG_N=N overrides)
cargo run --release -p app-iced -- --demo --demo-big
# Build everything
cargo build --workspace --release
# Tests (unit tests live next to the code; no integration framework needed)
cargo test --workspace
# Perf regression bench (lib harness; measure build/layout/raster vs N msgs)
cargo bench -p app-iced --bench frame
# Headless per-frame cost probe with a real fling (scroll changes content/frame)
PROBE_SCROLL=1 cargo run --release -p app-iced --example composite_probe 420
# Lint
cargo clippy --workspace --all-targets
```

**Never measure UI speed in a debug build.** Release render is ~3 ms/frame at
1250×1514; debug is ~25.6 ms/frame (~10 fps) — a debug `cargo run` alone looks
like a perf bug.

## Runtime flags (in `app-iced`)

| Flag | Purpose |
|---|---|
| `--demo` | offline canned backend (5 chats, typed previews, generated images) |
| `--demo-big` | first demo chat seeded with a huge generated history (~420 msgs) |
| `--open-first` | auto-open the first chat when the list arrives |
| `--perf` | FPS badge in the conversation header + `TG_PERF_LOG` cadence log |
| `--continuous` | constant 4 ms redraw requests (mirrors winit loop; perf only) |
| `--scroll-perf=SECS` | self-driven synthetic fling through the REAL update→view→layout→draw→present pipeline; prints a `FINAL fps=… ms=… renders_s=…` line then exits (see Perf) |
| `TG_PERF_LOG=/path` | line-based cadence log: `fps=… ms=… events=… renders=… renders_s=…` every ~500 ms |
| `TG_BIG_N=N` | override `--demo-big` message count |
| `TG_NO_VIRT=1` | disable message-list virtualization (restores build-every-row per frame) for A/B |
| `WGPU_BACKEND` | wgpu adapter backend; defaults to `gl` on main (see Perf) |
| `TG_DATA_DIR=/path` | override the per-user data dir (`.env`/session/cache live there) |

`tools/scroll-perf.sh` runs the release binary and prints a 3-way comparison:
loop ceiling (5-msg chat) vs 420-msg chat vs (optional) `--before` full-rebuild.

## Measuring scroll performance (do this before "is it slow" debates)

- The header FPS badge and all `renders_s` figures count **actually presented
  frames** (bumped in `view()`, i.e. once per `RedrawRequested`), NOT the
  scroll-event cadence. The old badge measured events and lied high.
- `FINAL` line: `fps` = event/update cadence (informational), `renders_s` =
  TRUE presented frames/sec, `ms` = seconds between presented frames.

## Chat message flow

`bridge.rs` defines `Request` (UI → network) and `UiMessage` (network → UI).
UI never touches MTProto directly.

- `network::spawn_network(demo, big)` returns an `UnboundedSender<Request>`,
  runs a **single-threaded tokio runtime**; `serve()` pulls `Request`s in
  batches, sorts them with `prioritize()` so `OpenChat` is never queued behind
  slow downloads, and handles them.
- **Downloads run async**: `Downloads { photos: Mutex<…>, sem: Arc<Semaphore> }`
  with `DOWNLOAD_CONCURRENCY = 4`; avatars/photos go through `tokio::spawn`
  (`spawn_avatar`/`spawn_photo`) so they never block the loop. On-disk paths
  are memoized so periodic refresh keeps showing them.
- State is a **pure, unit-testable MVU** (`state.rs`, `State::on_message`);
  it owns `dialogs`, `messages`, `open_chat`, `loading`, scroll state, context
  menu, composer, login steps. Tests in `state.rs` cover open→history flow,
  loading flag, optimistic-send dedup (local row `id = 0`, matched by text).
- Optimistic sends: `submit()` pushes a local `MsgRow { id: 0 }`; the server
  echo is merged by finding `(id == 0 && text == t)`. Keep that invariant.

## Message-list virtualization (important)

`view()` re-runs every frame. Before virtualization, a 400-message chat rebuilt
+ re-shaped every row each scroll tick. Now (`messages_list` in `lib.rs`):

- The scrollable carries `.on_scroll(|viewport| Message::Scrolled(y.absolute()))`
  and `State` stores `scroll_offset`.
- Row heights are **estimated** O(1) per row (`est_row_height`: char count ×
  avg glyph advance ~0.52em, 1.3 line height, accumulated bubble paddings) —
  must stay in lockstep with `message_row`'s real paddings.
- Only rows intersecting `[offset, offset + viewport]` (+`LIST_OVERSCAN = 16`)
  are built; everything else becomes height-matched spacer
  (`top_pad`/`bottom_pad`) so content height + bottom anchoring stay exact.
  A `view_h`-tall spacer above the first row keeps bubbles pinned to bottom.
- Reset/clear `scroll_offset` when switching chats.

Enable `TG_NO_VIRT=1` to get the old every-row-per-frame behaviour for A/B
benchmarks (that is what `tools/scroll-perf.sh --before` does).

## Rendering stack & measured perf (v0.3.0 story)

History in one paragraph:

- winit/softbuffer custom renderer (main @ 1970031) — full scene re-drawn each
  frame, ~11-17 ms/frame at 1250×1514 (render+blit) on this box.
- iced + tiny-skia (software): ~3.1 ms/frame headless under a real fling
  (better than winit), but live presented ~148 fps with a 5-msg chat vs
  ~24-29 fps with 420 msgs — **content-bound**, the software AA re-rasterized
  rounded rects/glyphs every frame and pinned a core high.
- **v0.3.0: iced with wgpu, GL backend — the fix.** `WGPU_BACKEND` defaults to
  `gl` (set in lib.rs) because on NVIDIA/Wayland the automatic probe loads
  Mesa GL (libgalleon+libLLVM) and stays there; `vulkan` costs ~2.4x more RAM
  (PSS 115 vs 47.5 MB) for little gain. tiny-skia remains the fallback if no
  GPU/adapter is found (`"image-without-codecs"` + `"wgpu"` are IMPORTANT
  features in the workspace `iced` dep; image decoding uses separate `image-codec`).

Measured (release, demo, NVIDIA/Wayland) — trust these as the reference:

| metric | software | wgpu/GL |
|---|---|---|
| big-chat scroll | ~24 fps | ~312-380 rendered fps |
| idle/hover/scroll CPU | 0.7% / 30-100% | ~1% |
| PSS (+NVIDIA driver) | ~25-30 MB | ~47.5 MB |

`examples/composite_probe.rs` isolates per-frame view/build/layout/draw cost
(with `renderer.reset()` between frames — that's mandatory or layers stack and
you measure garbage). Use it before blaming the loop.

## Iced 0.14 gotchas (all already encoded in the codebase — don't re-learn them)

- **`image::Handle::from_rgba` mints a fresh `Id::Unique` on every call.** The
  tiny-skia raster cache is keyed by that Id, so images built per-frame
  (avatars, icons) would re-raster every frame. Icon handles are memoized by
  `(kind, color, size)` in `icons.rs`; circular avatars memoize the masked
  handle by `(path, px)`. `Handle::from_path` > `Id::path` is stable/cached.
- **tiny-skia ignores `border_radius` on `image` widgets.** Round avatars are
  decoded, cover-cropped with `resize_to_fill`, then a circle punched into the
  alpha channel → `Handle::from_rgba`. Do not switch to `border_radius`.
- **Nested `canvas` widgets are invisible** in `iced_tiny_skia` (double
  translate). Icons are drawn into a `tiny_skia::Pixmap` and displayed as
  `image`. Keep it that way.
- `iced::widget::responsive` is used to get the conversation-pane width for
  the message columns (fixed-width list + flexible pane).
- The `notify_scroll` viewport gives `absolute_offset()` in content coordinates.

## Environment quirks (this dev box, Hyprland/Arch)

- Two-pane monitor `eDP-2` 2560×1600@165, **scale 1.6**. Iced window is
  logical 1100×700 → physical buffer ~1760×1120; the window is created with
  `window_size_from_args()` unless `--win=WxH`.
- `grim` screenshots use **logical** coords but write physical pixels; always
  re-query window geometry via `hyprctl` before a capture (window position is
  not stable between launches).
- **`ydotool` input injection is broken here** — events are never delivered to
  the app (don't use it for scroll testing; use `--scroll-perf`). `xdotool`
  only reaches XWayland: launch with `env -u WAYLAND_DISPLAY` to make the
  window visible to it.
- shell: there are `rtk` wrapped commands named `rtk …`; be careful: some
  reads of third-party sources come back garbled (e.g. `cache` → `ln`), and
  `git commit -q` through `rtk` has hung. For commits/pushes use plain
  `git …` and add explicit timeouts.
- `pkill -f` can kill your own shell if the pattern matches the running bash;
  prefer `pkill -x` and keep PIDs explicit.
- `.env` holds `API_ID`/`API_HASH`; a real account needs the caps to sign in.
- `vendor/core2-0.4.0` patches a yanked crate required by grammers — do not
  remove; keep the `[patch.crates-io]` block.

## Repository hygiene

- Update `CHANGELOG.md` (Unreleased) on any user-facing change (features/fixes/
  perf); keep section order huge→fixed→performance.
- Commit messages are single-line conventional commits; perf work adds a body
  with measured numbers (before → after) so history stays the source of truth.
- CI runs `cargo test --workspace` and a release build (`.github/workflows/`).
- Branch topology: `main` is the shipped line (auto-release on tags:
  `v0.3.0` etc.); feature work happens on topic branches until merged.

## Multi-agent sessions (Ensemble)

Hard rules for any session where several agents work in parallel. These were
learned the hard way — do not re-learn them.

### Global conventions (confirm with the user BEFORE spawning agents)
- **All user-facing UI text is ENGLISH** (labels, placeholders, demo content,
  error strings, tray tooltips). No French UI, even though early history had
  some — it was translated in `feat/ui-english`.
- Documentation (README, CHANGELOG, AGENTS.md) stays English too.
- Confirm language/scope/style conventions with the user before spawning any
  agent that writes UI or docs.

### Agent workflow discipline (build loop)
- **Iterate with `cargo check -p <crate>`** (seconds), NOT the full test
  suite. The full gate runs ONCE before commit:
  1. `cargo test --workspace`
  2. `cargo clippy --workspace --all-targets` (must be 0 warnings)
  3. `cargo build --release -p app-iced`
  Never run release builds or full tests mid-iteration; never measure perf in
  a debug build.
- Keep each agent's edits grouped and clearly separated so a human (or lead)
  can merge branches sequentially with minimal conflicts.

### Scope isolation between parallel agents
- Each agent gets an explicit file whitelist + a "do not touch" list covering
  files another agent owns. State additions must be namespaced clearly (e.g.
  prefix fields/handlers with the feature) so merges are mechanical.
- Land global/transversal changes (i18n passes, big refactors, renames)
  ALONE first, then spawn feature agents on top of the result. Never run a
  transversal pass concurrently with feature work on the same files.

### Merge & review pipeline (lead's job)
1. Agents push topic branches; they NEVER merge to main themselves.
2. **NEVER push to `main` without explicit user approval** — this is a public
   repository. Direct pushes to main (docs, fixes, anything) require asking
   the user first and waiting for a yes. Work can happen on main's working
   tree, but the push itself is gated.
3. Before merging: a read-only reviewer agent reads the diffs and flags
   inconsistencies, perf risks, missed conventions.
4. Lead merges one branch at a time (squash), resolving conflicts; after each
   merge, run the visual QA: launch `--demo`, capture with `grim`, inspect
   with a vision agent.
5. CI green → PR mergeable. Small PRs over big ones: less conflict surface,
   faster merges.

### Agent reliability (model quirks)
- Teammate models occasionally stall (long idle with no output) or stop
  mid-mission. The lead must react to stall notifications immediately:
  ping the agent once; if it cannot wake (session ended), inspect its
  worktree (`git status`), then respawn a fresh agent with the same mission +
  any corrections baked in, pointing at whatever partial work exists.
- Prefer short, pointed prompts over long explorations; tell agents which
  files matter up front so a stall loses little work.