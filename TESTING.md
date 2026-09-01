# TESTING.md — Visual QA on the dev VM (telegram-rs)

Hard-won, reusable lessons for doing **visual QA of the Iced/wgpu GUI** against a
live running app. The target is the dev VM (`flux@cachyos-dev-vd.netbird.cloud`,
GTX 1060 passed through, Xvfb `:99`, window scaled ×1.6 from 1100×700 logical to
the ~1760 px physical buffer).

> This file is the companion to AGENTS.md's "Merge & review pipeline" step 5
> ("run the visual QA: launch `--demo`, capture, inspect"). When QA is done and a
> feature is validated, record the outcome here too.

## Why this doc exists

A prior QA pass wasted many turns on **stale click coordinates and unreliable
vision reads** — the underlying app was fine, the *locating* was not. Every rule
below was learned against the real app on the real VM. Follow them and a full
feature QA takes minutes, not hours.

## Golden rules (the ones that save the most time)

1. **`vision_analyze` is for LAYOUT hints only — never for coordinates, never
   for state.** It hallucinates: it "saw" a scheduled chip after it was cancelled,
   confounded pending vs actual, mis-scaled positions, and inverted left/right.
   Treat it as a rough eyeball; verify everything with deterministic tools.

2. **Ground truth = tesseract OCR (TSV, `--psm 11`) + numpy/PIL pixel/color
   analysis.** OCR reads text + X/Y positions reliably. Pixel colour analysis
   finds buttons, selected states, and layout boundaries (e.g. sidebar/chat split
   at x280, the accent-blue selected button in a segmented control).

3. **Every click needs a fresh capture *immediately before* it.** The demo is
   LIVE (messages keep arriving, chat auto-scrolls, popovers shift position with
   content). Any coordinate learned on an older frame may be stale. Do capture →
   locate → click → capture in one tight loop, never reuse old coordinates.

4. **Positive control before blaming the code:** if a click "does nothing",
   first prove a click you *know* works lands (e.g. the blue send button) — if it
   does, the problem is your targeting, not the wiring.

5. **Close panels before opening others.** An open panel (Chat info sheet) shifts
   the geometry (chat shrinks from x280–1100 to x280–810) and swallows clicks
   meant for the sidebar. OCR-verify the state after every action.

6. **Shell on the VM is `fish`.** Complex quoting breaks over `ssh '…'`. Use the
   reliable pattern: `write_file` a script locally → `scp` to `/tmp` → run with
   `ssh 'bash script.sh'`.

## Tooling layout (host vs VM)

| Capability                | Where |
|---------------------------|-------|
| `tesseract` OCR           | **VM only** (host: `command not found`, rc=127) |
| PIL / numpy pixel analysis| **Host only** (VM: `import PIL` fails) |
| `xdotool` input + `import` capture | VM |
| crop + resize for OCR     | host (then scp the crop to the VM) |

Workflow: capture on VM → scp to host → pixel-analyse on host → crop a tight
region → scp crop back → OCR on VM. Awkward but deterministic.

OCR helper (run on VM against a capture or crop):
```bash
tesseract img.png out --psm 11 tsv >/dev/null 2>&1
awk -F'\t' 'NR>1 && $12!="" && $11>25 \
  {printf "%s x=%s y=%s w=%s h=%s conf=%s\n",$12,$7,$8,$9,$10,$11}' out.tsv
```
- Always skip the header (`NR>1`), or text like `Scheduled` matches the `block_num`
  header column.
- Low-confidence tesseract reads are normal for small UX text; cross-check with
  pixel colour (e.g. does the segment have the accent background?).

Pixel analysis for the **selected button in an Iced segmented control** (find the
accent fill):
```python
from PIL import Image; from collections import deque
img = Image.open('cap.png').convert('RGB')
def is_blue(c): r,g,b=c; return b>140 and b>r and g>90
# flood-fill / bounding box the blue mask in the row's region → that's the selected button.
```

## Launching & window setup (validated)

```bash
# on the VM (shell = fish; the repo is ~/telegram-rs)
export DISPLAY=:99 WGPU_BACKEND=vulkan
./target/debug/telegram-rs --demo --open-first &   # debug is fine for visual QA
WID=$(DISPLAY=:99 xdotool search --name "Telegram RS" | head -1)
DISPLAY=:99 xdotool windowfocus $WID                # required under Xvfb (no WM); windowactivate is not enough
```

- **Debug build is OK for screenshots/QA** (release is only needed for perf
  measurement).
- **Clean or note the phantom instances:** a stray `telegram-rs --demo` left
  running changes which window your clicks hit. Check `pgrep -af telegram-rs`
  and its WID first.
- `WGPU_BACKEND=vulkan` reaches the GTX 1060 under Xvfb; `gl` falls back to
  `llvmpipe` (software). Either renders for QA; vulkan is the real-GPU path.

Click + capture in one deterministic script (avoids frame races):
```bash
#!/bin/bash
WID=2097154
DISPLAY=:99 xdotool windowfocus $WID; sleep 0.3
DISPLAY=:99 xdotool mousemove --window $WID --sync X Y   # options BEFORE coordinates!
DISPLAY=:99 xdotool click 1; sleep 0.6
DISPLAY=:99 import -window $WID /tmp/cap.png
```

## Known-good click map (window 1100×700, before any panel opens)

| Target | (x,y) |
|---|---|
| Send button | (1067,671) |
| Schedule clock (composer) | (435,671) |
| Chat-info ⓘ (header) | (1075,31) |
| Chat-info close X | (1072,37) |
| Block / Unblock (info panel) | (891,318) |
| Scheduled-chip X (cancel) | (558,620) |
| Sidebar gear → Settings | (250,22) |
| Settings tabs (Profile / Sessions / **Privacy**) | y≈74 |
| Privacy tab | (1015,74) |

**These drift.** The schedule popover is the notorious one: its item Y positions
shift with chat content (seen anywhere in y≈520–655). Always re-locate on a fresh
capture. The sidebar header icons (+ / search / gear) sit at y≈15–28 (centre y22)
— clicks at y38+ hit the *chat title* instead (clicking the title opens Chat info).

## Feature QA reference (Lot 1, all PASSED)

Validated end-to-end against the live demo on the dev VM (2026-09-01):

- **Drafts (#49)**: typed text persists per-chat; switching chats doesn't leak it;
  returning restores it. Confirmed by sending on one chat, switching, coming back.
- **Scheduled send (#51)**: clock icon → popover (In 15 min / 1 h / 3 h / 1 day) →
  pick a preset → menu closes + chip `Scheduled · HH:MM UTC` appears → send armed
  (blue). The chip X cancels (OCR confirms no more `Scheduled`).
- **Block/Unblock (#50)**: Chat-info ⓘ → Block button → becomes `Unblock` and the
  name is replaced by the phone number; Unblock → back to `Block`.
- **Privacy (#52)**: Settings → Privacy tab → 3 rows, each a segmented control
  `Everyone | Contacts | Nobody`. Default is **Everyone** (not "Nobody"!). Clicking
  a segment moves the accent-blue selection to that button; rows are independent.
  Layout for row 1 ("Last seen & online"): Everyone ≈x859–934, Contacts ≈x939–1011,
  Nobody ≈x1016–1083, button Y ≈184.

### Pitfall: OCR trains you wrong on segmented controls

Tesseract confidently reads the *rightmost* (often lower-contrast or co-located)
text as the "current value". Here it reported **Nobody** for all three rows because
it grabbed the unselected third button. The **actual** truth was `Everyone`
(default, accent-filled). Cross-check selection state by pixel colour, not by
which word OCR flags.

## Workflow summary (one feature)

1. Relaunch/verify the app (fresh `--demo --open-first`, focus window, note WID).
2. Capture → confirm you're on the right screen (OCR the whole frame).
3. Navigate to the feature via the known-click map, re-capturing at each step and
   OCR-verifying state (close any open panel first).
4. Interact (click), capture again, verify by pixel + OCR.
5. Record outcome below or in the PR/CHANGELOG.