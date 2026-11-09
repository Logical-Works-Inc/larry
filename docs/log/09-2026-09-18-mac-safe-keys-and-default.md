---
type: Log
title: log.md, part 9 of 15
description: Part 9 of /log.md, in order; the index lists every part.
resource: /docs/log/09-2026-09-18-mac-safe-keys-and-default.md
tags: [governance, decisions, evidence]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: explicit conversation decisions and source inspection}
status: active
stale_after: 2026-10-15
sources: [Director conversations 2026-09-15 through 2026-09-17, /specification.md, /research-ledger.md]
---
## 2026-09-18: mac-safe keys and default terminal background

Director corrections: F-keys do not work on macOS, run an accessibility pass,
meet the universality invariant, keep the default terminal background, and add
no overhead. No spec change: inventory stays 96/53/80/13/9.

`src/native.rs` only (domain `Demo` untouched): surface reset to transparent
cells so AppCUI renders the terminal default fg/bg (its `set` otherwise pins
Black); all six palette roles resolve to Transparent with Bold/symbols/text
carrying emphasis — one branch-free path for dark/light/NO_COLOR/dumb
terminals. Single-letter panel actions (`n/c/i/f/t`, plus `j/k/?///q`) reached
by Tab, so no F-keys are needed; input keeps Mac Ctrl editing meaning
(`^U/^W/^H`) and `^C/^D` quit as interrupt/EOF; F-keys and `/commands` remain
as alternates with no platform branch. References: pageton/tui-design-skill
SKILL + design/interaction/states/color docs (fetched 2026-09-18), live
Claude Code landing observed in PTY (identity block, separators, hero input,
status hints), 0nn0/terminal-mac-cheatsheet Ctrl semantics.

MEASURED on this Mac: 13 Rust tests pass (incl. new panel-letter, Mac-editing
and transparent-bg tests); clippy `-D warnings` clean; fmt clean; 4 JSON +
2 native PTY journeys pass. The PTY runaway bound moved 512 KiB to 1 MiB
after measuring ~566 KiB for the ten-command journey (~5.5 KiB per repaint,
~3 KiB/s idle); runaway/crash detection unchanged. Pre-exec receipt
`tools/receipts/20260918-tui-a11y-pre-exec.json`, post-exec
`tools/receipts/20260918-tui-a11y-post-exec.json`. Remaining limits: no
backend/model/persistence, no physical-Ryzen/fleet/human evidence; PTY
journeys still drive F-key escape sequences where the harness sends them.

## 2026-09-19: blocky Larry raccoon mascot (Clawd-role translation)

Director steering: Larry must do for Logical Works what Clawd does for Claude
Code — a quiet terminal companion whose adoption is psychological acceptance;
"extremely cohesive to bring calm and create attachment"; blocky; same size as
Clawd; canonical asset is the webp, not the SVG. Evidence gathered: ChatGPT
memory recall (Director history: July 2026 Toronto-raccoon origin brief
inspired by Ollama/Clawd roles; identity = paired ears, contrasting mask,
LARGE eyes with catchlights, central nose; voice warm/calm/concrete; Sept 8
"flatter Rocco" flattening), Gemini craft rules (24x24 grid; 16 is a trap,
32 loses chunk; silhouette-first; no outer AA; 4-color cap), Google/OpenAI
Codex pet manifest (1536x1872 8x9 atlas of 192x208 cells; states in row
blocks; half-block fallback for TUIs), clawd-emotes anatomy rig.

`tools/rocco_pixel.py` is the single source of truth: 24x24 mirrored half-grid
(dark ears with gray bases, bandit mask with gray bridge, LARGE brown eyes
with cream catchlights, cream muzzle, dark nose, open smile with red tongue;
blink frame), emitting the `MASCOT` const for `src/native.rs` (half-block
▀/▄ rendering on Transparent cells, ANSI16 brand mapping — estate build has
no TRUE_COLORS), a Codex pet package written to `~/.codex/pets/rocco/`
(pet.json + spritesheet.webp, idle block with 2-frame blink), and
`docs/larry-mascot.svg`. The mascot perches centered on the empty Conversation
landing only, below the safe-fixture lines; it leaves once the conversation
starts and never renders on terminals too small to hold it unclipped.
Deliberate scoped exception to the Transparent rule: the 24x12 mascot cells
paint brand colors (Black/Gray/White/Olive/Red); all UI chrome stays
Transparent. Visual proof: docs/larry-mascot.svg rendered and inspected in
BrowserOS (mask + eyes + muzzle + tongue read correctly).

MEASURED: 9 native + 6 lib Rust tests pass (new: mascot perch/leave,
canonical palette pixel probes); clippy 0 warnings; fmt clean; 2 native PTY +
4 demo journeys pass; governance 96/53/80/13/9 unchanged; pre-exec receipt
`tools/receipts/20260918-rocco-raccoon-pre-exec.json`, post-exec
`tools/receipts/20260918-rocco-raccoon-post-exec.json`. Remaining limits: no
animation in the TUI (single frame; pet package carries the blink), no
sixel/kitty protocol rendering, pet package unverified inside Codex's pet
picker, no human-attachment evidence (design intent only).
