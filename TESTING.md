# TESTING.md — Automated visual QA of the desktop GUI

Reusable methodology for doing **visual (screenshot-driven) QA of this
Iced/wgpu desktop app** against a live running instance (the offline `--demo`
backend is ideal for this). The goal is that *any engineer or agent* can run a
headless GUI QA pass from these instructions alone — no account, no special
hardware, no private details.

This file is the companion to AGENTS.md's "Merge & review pipeline" step 5
("run the visual QA: launch `--demo`, capture, inspect"). Record each feature's
QA outcome here after it's validated.

## Why this document exists

A QA pass can burn a lot of time on **stale click coordinates and unreliable
vision reads** — the underlying app is usually fine, the *locating* is not.
Every rule below was learned against the real app driven headless. Follow them
and a full feature QA takes minutes, not hours.

## Golden rules (the ones that save the most time)

1. **`vision_analyze` is for LAYOUT hints only — never for coordinates, never
   for state.** It hallucinates: it can "see" an element after it was cancelled,
   confound pending with actual state, mis-scale positions, and invert
   left/right. Treat it as a rough eyeball; verify everything deterministically.

2. **Ground truth = tesseract OCR (TSV, `--psm 11`) + numpy/PIL pixel/colour
   analysis.** OCR reads text + X/Y positions reliably. Pixel colour analysis
   finds buttons, selected states, and layout boundaries (e.g. a sidebar/chat
   split, or the accent-coloured selected button in a segmented control).

3. **Every click needs a fresh capture *immediately before* it.** The demo is
   LIVE: messages keep arriving, chat auto-scrolls, and popovers shift position
   with content. Any coordinate learned on an older frame may already be stale.
   Do capture → locate → click → capture in one tight loop; never reuse old
   coordinates.

4. **Positive control before blaming the code:** if a click "does nothing",
   first prove a click you *know* works lands (e.g. a primary send button). If
   it does, the problem is your targeting, not the wiring.

5. **Close panels before opening others.** An open panel/sheet shifts the
   layout geometry and can swallow clicks meant for the background. OCR-verify
   the state after every action.

6. **Driving a remote box:** if the target is remote and its default shell
   differs from `bash` (e.g. fish), naive `ssh 'cmd …'` quoting breaks. Use the
   reliable pattern: write a script locally → copy it up → run it with an
   explicit shell (`write_file` → `scp` → `ssh 'bash script.sh'`).

## Tooling split

| Capability                    | Where                           |
|-------------------------------|---------------------------------|
| `tesseract` OCR               | a box that has it (not everywhere) |
| PIL / numpy pixel analysis    | wherever Python + PIL work      |
| `xdotool` input + `import` cap| the X/Xvfb host driving the window |
| crop + resize for OCR         | the analysis host, then push back |

There is **no requirement** that OCR and pixel analysis live on the same
machine — capture on the X host, analyse wherever PIL runs, and run OCR where
`tesseract` exists. That works fine; it just adds a copy step.

OCR helper (run against a capture or crop):
```bash
tesseract img.png out --psm 11 tsv >/dev/null 2>&1
awk -F'\t' 'NR>1 && $12!="" && $11>25 \
  {printf "%s x=%s y=%s w=%s h=%s conf=%s\n",$12,$7,$8,$9,$10,$11}' out.tsv
```
- Always skip the tesseract header line (`NR>1`), or words like `Scheduled` can
  falsely match the `block_num` header column.
- Low-confidence OCR reads are normal for small UX text; cross-check the
  *semantic* state with pixel colour (see the segmented-control pitfall below).

Pixel analysis for finding a selected button in a segmented control (detect the
accent fill):
```python
from PIL import Image; from collections import deque
img = Image.open('cap.png').convert('RGB')
def is_accent(c): r,g,b=c; return b>140 and b>r and g>90   # tune to your palette
# flood-fill / bounding-box the accent mask in the row's region → that's the selected button.
```

## Launching & window setup

```bash
# on the GUI host (adjust to an offline/demo mode if the app supports one)
export DISPLAY=:99 WGPU_BACKEND=vulkan      # vulkan = real GPU under Xvfb;
                                            # gl may fall back to software rendering
./target/debug/telegram-rs --demo --open-first &   # debug build is fine for screenshots
WID=$(DISPLAY=:99 xdotool search --name "Telegram RS" | head -1)
DISPLAY=:99 xdotool windowfocus $WID        # required under Xvfb (no WM);
                                            # windowactivate alone is not enough
```
- **Resolve the WID at runtime** (`xdotool search --name`); do not hard-code it.
- **Clean stray instances:** a leftover instance changes which window your
  clicks hit. Check for them and their WIDs before starting.
- A debug build is fine for visual QA (release is only needed for perf).

Click + capture in one deterministic script (avoids frame races):
```bash
#!/bin/bash
WID=$(DISPLAY=:99 xdotool search --name "Telegram RS" | head -1)
DISPLAY=:99 xdotool windowfocus $WID; sleep 0.3
DISPLAY=:99 xdotool mousemove --window $WID --sync X Y   # options BEFORE coordinates!
DISPLAY=:99 xdotool click 1; sleep 0.6
DISPLAY=:99 import -window $WID /tmp/cap.png
```

## Click-map discipline

Coordinates in a GUI vary with window size, content, and open panels. Capture a
representative map (send button, composer buttons, header icons, tab row,
segmented controls) for *your* window size, then note **which entries drift**:

- **Popovers are the notorious ones** — their item Y positions shift with
  chat/content state. Always re-locate them on a fresh capture.
- **Header icon rows can be thinner than they look** — clicks a few px low can
  hit the element below instead (e.g. a title that opens a panel). Work from an
  actual pixel/colour scan of the icon row, not a guess.

## Workflow summary (one feature)

1. Relaunch/verify the app (fresh demo mode, focus window, resolve WID).
2. Capture → confirm you're on the right screen (OCR the whole frame).
3. Navigate to the feature via your click map, re-capturing and OCR-verifying
   the state at each step (close any open panel first).
4. Interact (click), capture again, verify by pixel colour + OCR.
5. Record the outcome below (feature → PASS / FAIL + notes).

## QA results log

| Feature | Status | Notes |
|---|---|---|
| *(add rows as you validate)* | | |

## Common pitfall: OCR misreads segmented controls

Tesseract often reads the *rightmost* (lower-contrast / co-located) text as the
"current value". In a `Everyone | Contacts | Nobody` segmented control it can
report `Nobody` for every row because it grabs the unselected third button,
when the *actual* default is `Everyone`. **Cross-check selection with pixel
colour** (does the segment carry the accent background?), never just with which
word OCR flags.