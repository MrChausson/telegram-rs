# ENSEMBLE.md — Reusable multi-agent team playbook

How we spin up agent teams for feature lots in this repo. Complements the
"Multi-agent sessions" rules in [AGENTS.md](AGENTS.md) — read that first.
This file is the operational recipe: roles, charters, gates, merge pipeline.

## Team shape

| Role | Count | Job | Hard limits |
|---|---|---|---|
| **Lead** | 1 (human-side agent) | Wave 0 prep, spawn/respawn builders, open draft PRs on first push, resolve conflicts, squash-merge one branch at a time, user comms | never codes features; never pushes main without explicit user approval |
| **Builder** | 1 per lot | Implement the lot: code + colocated unit tests + docs strings | whitelist files only; namespaced state fields; iterate `cargo check -p <crate>` |
| **Tester** | 1 | Runs the full gate per pushed branch + runtime smoke (`--demo` in a `TG_DATA_DIR` sandbox) | reports PASS/FAIL + evidence paths; never edits code |
| **Reviewer** | 1 (read-only) | Reads diffs before draft→ready promotion | flags only: conventions, perf hot paths, missing tests |
| **Vision-QA** | sub-agent | Post-merge visual pass: launch `--demo`, `grim` capture, inspect | — |

Spawn builders from the same base commit (fresh `origin/main`), each in its
own git worktree.

## Builder charter template

Every builder gets this skeleton, filled per lot:

```text
Mission: implement LOT-X as specified below. Ship code + unit tests.

You own (whitelist):            <exact file list>
You must NOT touch:             <everything else, explicitly>
Naming namespace:               prefix all new State fields/methods,
                                Message variants and tests with <ns>_
Loop discipline:                cargo check -p app-iced between steps
                                (seconds). NEVER full tests / release
                                builds mid-iteration.
Full gate (once, at the end):   cargo test --workspace &&
                                cargo clippy --workspace --all-targets
                                (0 warnings)
Commit style:                   single-line conventional commits
Deliver:                        push branch lot-x → announce to lead.
                                Lead opens the draft PR.
Report back:                    list of changes per file, test evidence,
                                known limitations.
```

Hard rules baked into every charter:
- All user-facing UI text in **English**; docs English too.
- Never block the message-list virtualization hot path with per-frame
  allocations (see perf notes in AGENTS.md).
- Popups/menus follow the established backdrop + Escape + click-outside
  pattern (see conversation pane stack).
- If you stall or get stuck > 15 min, report instead of exploring.

## Lead protocol

1. **Wave 0 (lead alone)**: any cross-cutting refactor or housekeeping that
   builders would otherwise collide on (module extraction, docs, .gitignore).
   Landed *before* spawning, via its own PR.
2. **Charter split**: disjoint file whitelists + strict namespaces
   (see the conflict map table below); assign static append-only regions of
   shared big files (`client.rs`, `lib.rs`) when two lots touch one.
3. **Draft PR immediately** after the first push of every branch — a pushed
   branch without an open PR is a bug.
4. **Watchdog**: on stall notification ping once; if dead, inspect
   `git status` in its worktree, respawn a fresh builder with the same
   mission + corrections baked into the prompt, pointing at partial work.
5. **Merge order**: fixed in advance (smallest/most-foundational first).
   Squash-merge ONE branch at a time; CI green before the next; run
   Vision-QA on merged main after each.
6. **Direct pushes to main require explicit user approval** — everything
   else goes through PRs.

## Conflict map (fill per wave)

```text
file                  owner A         owner B         owner C
tg/src/<area>.rs      append region X append region Y —
app-iced/src/state.rs ns:<a>_         ns:<b>_         —
app-iced/src/lib.rs   UI zone: …      UI zone: …      Message enum (lead)
CHANGELOG.md          separate bullets, merged by lead
```

## Tester checklist (per branch)

- [ ] `cargo test --workspace` green
- [ ] `cargo clippy --workspace --all-targets` 0 warnings
- [ ] `--demo` boots headless-clean; smoke flow exercised via scripted input
      if interaction matters (`xdotool` needs `env -u WAYLAND_DISPLAY`)
- [ ] Screenshots captured under `/tmp`, referenced by path
- [ ] New UI strings are English; Escape/backdrop close works
- Report format: PASS/FAIL + evidence paths + regressions noticed

### Visual QA runbook (this dev box — battle-tested, follow exactly)

1. **Sandbox first**: `TG_DATA_DIR=$(mktemp -d)` — never touch the real
   `~/.local/share/telegram-rs` or the repo `.env`. Gotcha: with `TG_DATA_DIR`
   unset AND a `.env` in the CWD, `data_dir()` resolves to `"."` (repo root)
   — pre-existing demo-asset behavior, not a bug.
2. **Launch for input injection**: `env -u WAYLAND_DISPLAY cargo run -p
   app-iced -- --demo` (XWayland — `xdotool` cannot reach Wayland windows;
   `ydotool` is broken on this box, don't try).
3. **Locate the window fresh EVERY run**: `xdotool search --name "Telegram
   RS"` then `xdotool getwindowgeometry --shell`. Position is NOT stable
   between launches. Geometry is in **physical pixels** (logical × 1.6).
4. **One coordinate system**: window-relative **physical px** — the same
   space as a grim capture cropped at `(X, Y, X+W, Y+H)` with PIL. Do NOT
   mix with `grim -g` which takes **logical** coords (applies the 1.6 scale
   itself). If a crop looks shifted, you mixed the two.
5. **Interactions**: `xdotool windowactivate` before the first action;
   `xdotool mousemove --window $WID X Y sleep 0.3 click 1`; inject exactly
   ONE key event per action (double-delivery was observed: one Escape
   disarms, a second closes the panel — by design).
6. **Never trust stale coordinates**: element positions move with layout
   (e.g. the login pills sit at a different Y on the QR pane than on the
   phone pane). Re-locate elements from a FRESH screenshot before each
   click; verify the click landed by capturing again.
7. **Verify by looking, then by measuring**: read the capture (vision) for
   the state you expect; for "nothing changed" claims prefer pixel-diff
   (`ImageChops.difference(...).getbbox()`), but beware **blinking carets**
   and clock timestamps — a non-empty diff alone is NOT proof of change.
8. **Demo-mode facts** (v0.8.1+): boots SIGNED-IN (LoginOk "Demo" at boot;
   `--open-first` opens the first chat). To reach the sign-in screen:
   Settings ▸ Account ▸ Log out ▸ Yes. Demo peer activity stops after
   logout. QR-login demo auto-completes ~8 s after the pane opens
   (ScanConfirmed flash at 4 s) and re-pushes the canned dialogs.
9. **Evidence**: save captures as `/tmp/opencode/<run>-<step>.png`, list
   paths in the report; reference them when claiming PASS/FAIL.

## Reviewer checklist (per diff)

- [ ] Whitelist respected (no out-of-scope files touched)
- [ ] Namespaces respected; no leaked generic field names
- [ ] No comments bloat; docstrings where non-obvious only
- [ ] Perf: no per-frame allocs/render work added to hot view() paths
- [ ] Tests actually cover the new behavior (not just compile)
- Verdict: READY / CHANGES REQUESTED (+ line-referenced notes)

## Wave log (append after each completed wave)

| Wave | Lots | Branches → PRs | Conflicts hit | Notes |
|---|---|---|---|---|
| 0 | auth.rs extraction + playbook + housekeeping | chore/wave0-prep → #26 | — | seed entry |
| 1 | A: logout (#27) · B: QR login (#28) | feat/logout, feat/qr-login2 | mechanical (Message enum, append regions) | builder session aborts ×2 → WIP patch rescue worked; usage-limit kills → lead ran final review+smoke; demo-QR parity fix (dialogs re-push) found only by runtime smoke |
