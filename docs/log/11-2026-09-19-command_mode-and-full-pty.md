---
type: Log
title: log.md, part 11 of 15
description: Part 11 of /log.md, in order; the index lists every part.
resource: /docs/log/11-2026-09-19-command_mode-and-full-pty.md
tags: [governance, decisions, evidence]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: explicit conversation decisions and source inspection}
status: active
stale_after: 2026-10-15
sources: [Director conversations 2026-09-15 through 2026-09-17, /specification.md, /research-ledger.md]
---
## 2026-09-19: command_mode and full PTY pass

New `command_mode: bool` field on `Native` solves the `/` key interaction
across all panels. Typing `/` in any non-Input panel enters command mode:
the `/` is pushed to the input buffer, characters route to the input, and
Enter processes the slash command without switching focus. Escape cancels
command mode; Backspace removes characters and auto-exits when the input
empties; Tab/Left/Right also exit command mode. Space toggles inspection
in command mode only if not routing to input. This follows the Forge TUI
single-input-mode pattern: all `/` commands go through the same buffer
regardless of which panel is focused.

PTY journey test `test_native_crash_and_fresh_terminal_recovery` updated to
expect "Safe fixture" on fresh terminal (the Conversation panel is the
default, not Changes). Recovery test now passes.

MEASURED: 6 Rust unit tests pass; clippy clean; fmt clean; 2 native PTY +
4 demo CLI journeys pass (6/6 total). Remaining limits: no backend/model/
persistence, no physical-Ryzen/fleet/human evidence.

## 2026-09-18: Forge terminal language replaces the first AppCUI look

Director steering, verbatim: "I dont like the current appcui implementation,
the original forge look is a lot better. I want that in appcui". This replaces
the surface composition; it does not change the fixture domain or the spec.
Inventory unchanged: 96/53/80/13/9. The bounded assertion inventory moves
20 → 23 (VF21–VF23).

New `src/theme.rs` is the single source for the colour language: the seven
Forge tokens read from `forge-harness/forge-tui/src/theme.rs` (slate, cream,
muted, emerald, amber, ruby, cyan) each paired with the ANSI16 stand-in this
build can render. `src/native.rs` was rewritten against that palette and the
Forge region structure read from `forge-tui/src/render.rs` — transcript, a
one-row input line (`❯` prompt, right-pinned dim hint), a rule, and a two-row
MI/MD status hub, with a right-hand dashboard at Forge's 100-column threshold
holding its 34% share behind a left rule. Forge's activity line and sensor
dashboard carry Larry's real fixture state (CLEAR/HELD/END canary, `DEMO
fixture · no model called`); no cost, latency or sensor number is invented.
Forge remains reference reading only: no Forge code is imported, copied or
ported, per the Director correction of 2026-09-16. Simulated paths stay
labelled DEMO and every action still says `no backend action executed`.

Two deviations are recorded rather than hidden. Colour depth: `lgwks_deps`
pins appcui `default-features = false`, so `Color` is the 16-colour enum and
`from_rgb` quantizes per-channel at `> 64`, sending Forge's emerald and cyan
both to `Silver`; `theme::TOKENS` names an explicit stand-in per token, with
`tokens_stay_distinguishable` as the falsifier, and the stand-ins become
`Color::RGB` unchanged when the estate forwards `appcui/TRUE_COLORS`. Theme
universality: Forge paints body text `fg(CREAM)` assuming a dark terminal;
body text here stays `Transparent` so the 2026-09-18 accessibility pass keeps
one branch-free path, and Forge's accent language is rendered.

`Row` gained a `Fit` marking whether a row holds copy this code authored
(`Fit::Panel`) or text it did not — a diff source line or an operator note
(`Fit::Clip`). Holding the latter to the panel floor would have required
either falsifying the fixture's code or wrapping it, and wrapping would invent
a line break the diff never had; clipping is therefore the honest rendering and
is now marked with `…` in the last cell, so a truncated diff cannot read as a
complete one. A `Fit::Panel` overrun additionally trips a debug assertion at
the paint site.

MEASURED on this Mac: 19 Rust lib tests and 6 integration tests pass; clippy
`-D warnings` clean; `cargo fmt --all -- --check` clean; 2 real-PTY native
journeys and 4 JSON-driver journeys pass; `python3 -m unittest discover -s
tests` 45/45; `lgwks-deps check .` reports every authored external edge owned;
`tools/check_governance.py` PASS. Reranker receipts
`tools/receipts/20260918-forge-look-pre-exec.json` and
`tools/receipts/20260918-forge-look-post-exec.json`.

Retained failures, all repaired rather than relabelled. VF21 failed seven
times as it was tightened: the first 120x32 render showed a clipped `+1 -1`
(the two fixture paths differ in length, so no single-line width fits both);
then the empty-state hint, two panel footnotes, the welcome sentence, the help
table and the held notice each overran the column. All were reflowed; no limit
was relaxed. The paint-site debug assertion then caught a seventh, a landing
row that only overran below 80 columns — three of the seven would not have been
visible at the default terminal width. The help table's three-column layout
became two rows per entry for the same reason. One earlier failure was mine,
not the code's: I first reconstructed a frame by tail-slicing the PTY byte
stream and read a stale frame as a UI defect; AppCUI repaints differentially,
so the surface render in `paint_keeps_terminal_default_background` and the
passing PTY journeys are the real evidence, and the disposable capture script
was discarded rather than trusted.

Limits: coverage is NOT re-measured for this pass. The 80.51% figure in
`docs/architecture/visual-pass.md` predates `src/theme.rs` and the rewritten
`src/native.rs` and no longer describes this tree; it is retained only as the
earlier pass's evidence. Independent visual review, physical-Ryzen/fleet
evidence, human usability trials and hosted CI all remain unresolved. The
Forge look is verified by source-faithful rendering and by real-PTY journeys,
not by a human having looked at it.
