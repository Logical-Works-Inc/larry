---
type: Design
title: docs/architecture/visual-pass.md, part 1 of 3
description: Part 1 of /docs/architecture/visual-pass.md, in order; the index lists every part.
resource: /docs/architecture/visual-pass/01-launch-and-interaction.md
tags: [rocco, demo, appcui, fixtures, evidence]
generated: {by: agent:rust-coder, at: 2026-09-17}
updated: {by: agent:claude-code, at: 2026-09-18, note: Forge terminal language pass}
verified: {by: agent:rust-coder, method: source inspection; execution receipts in log}
status: forge-look-implemented-visual-review-pending
stale_after: 2026-10-18
sources: [Director continuation 2026-09-17, /specification.md, /log.md, https://github.com/gdt050579/AppCUI-rs, https://crates.io/api/v1/crates/lgwks_std/0.6.3, https://crates.io/api/v1/crates/lgwks_deps]
---
## Launch and interaction

Requires Rust 1.98 and Python 3.11+ for verification:

```sh
cargo build --locked
cargo run --locked --bin rocco
```
In the native UI, type `/next`, `/feedback bound retries`, `/next`,
`/inspect 2`, `/next`, `/continue`, `/next`, `/quit`, each followed by Enter.
F6 toggles the inspection area. The third `next` is safely
rejected because the code owner is held. The test clone may advance while that
owner is held. Inspecting the test event does not retarget the original hold.
`continue` releases the remembered owner, retains the note, and permits the
pending code event. No command is interpreted as shell or model instructions.
F2 and F3 toggle the actual folder drawer and agent rail. They collapse at
narrow widths to preserve the conversation. `/scenario empty`, `/scenario change`, and `/reset`
explicitly discard the synthetic session. `help` or `status` emits the view and
command guide in the JSON harness. Native F1 or `/help` toggles help, Up/Down
select events, PageUp/PageDown scroll, Backspace edits input, and Esc or `/quit`
exits. Native input is bounded to 4096 bytes, with a visible limit notice;

### Mac-safe keys and default-background rendering (2026-09-18)

macOS reserves F-keys for system functions, and the Mac cheatsheet baseline
gives Ctrl+E/F/T/R/N fixed editing meanings, so the TUI takes neither: every
fixture action also works as a single letter outside the input (`n` next,
`c` continue, `i` inspect, `f` files, `t` agents; `j/k` move, `?` help,
`/` focuses input, `q` quits), reached by Tab. Inside the input, Ctrl chords
keep Mac meaning (`^U` clear line, `^W` delete word, `^H` backspace) and
`^C`/`^D` quit like interrupt/EOF. F-keys and `/commands` are retained
alternates; there is no platform branch.

Rendering uses the terminal default foreground and background everywhere:
`on_paint` resets the surface to transparent cells and paints glyphs only
(AppCUI renders Transparent as the terminal default; its `set` would
otherwise pin cells to Black). Emphasis is Bold plus symbols/text only
(`▸` selection/focus, `●` mode, `-`/`+` diffs, HELD/note words), so dark,
light, NO_COLOR and dumb terminals share one code path with no branches.

### Forge terminal language (2026-09-18)

The Director rejected the first native surface's look and asked for the Forge
TUI look instead. `src/theme.rs` and the rewritten `src/native.rs` reproduce
the Forge *structure* read from `forge-harness/forge-tui/src/{render,theme,
brand}.rs`: a transcript region, a one-row input line with an accent `❯`
prompt and a right-pinned dim hint, a rule, and a two-row MI/MD status hub,
with an optional right-hand dashboard at Forge's 100-column threshold taking
Forge's 34% share behind a left rule. Forge is reference material only; no
Forge code is imported, copied or ported, per the Director correction of
2026-09-16.

Two deviations are deliberate. **Colour depth:** `lgwks_deps` pins appcui with
`default-features = false`, so `Color` is the 16-colour enum and
`Color::from_rgb` quantizes per-channel at `> 64`; Forge's emerald (80,200,120)
and cyan (88,166,255) both land on `Silver`, so the accents and the MI chip
would read as grey. `theme::TOKENS` therefore names an ANSI16 stand-in for
each Forge token, with `tokens_stay_distinguishable` as the falsifier. When
the estate forwards `appcui/TRUE_COLORS` through `lgwks_deps`, the stand-ins
become `Color::RGB` and nothing outside `src/theme.rs` changes. **Theme
universality:** Forge paints body text `fg(CREAM)`, which assumes a dark
terminal; body text here stays `Transparent` so the accessibility path above
is preserved, and Forge's accent language — the part that makes the surface
recognisable — is rendered.

Forge's whimsical activity line and sensor dashboard do not map onto a
fixture with no model and no telemetry, so they carry Rocco's real state
instead: the canary chip reports CLEAR/HELD/END from the actual work state, the
activity suffix reads `DEMO fixture · no model called`, and the dashboard's
Changes/Files/Agents tabs show the same synthetic events the JSON view does.
No invented cost, latency or sensor number is displayed.

Row-width discipline: `Row` carries a `Fit` marking whether the row holds
copy this code authored (`Fit::Panel`, must fit the narrowest column) or text
it did not (`Fit::Clip` — a diff source line or an operator note, where
clipping is the honest rendering because wrapping a code line would invent a
line break the diff never had). A row that overruns its column draws `…` in
the last cell, so a truncated diff cannot read as a complete one, and a
`Fit::Panel` overrun additionally trips a debug assertion at the paint site.

MEASURED 2026-09-18: the ten-command PTY journey emits ~566 KiB total
(~5.5 KiB per event-driven repaint, ~3 KiB/s idle), so the harness
runaway-output bound moved 512 KiB to 1 MiB (`OUTPUT_BOUND` in
`tools/test_native_cli.py`); a spew loop still trips it within a second,
and a crash closes the PTY and fails distinctly.
sixteen messages are retained, evicting the oldest when full. Notes reject at
sixteen without losing existing notes. The JSON driver remains available via
`cargo run --locked --bin rocco-demo`; its EOF/quit exits, while oversized or invalid UTF-8 transport input exits
nonzero. Recoverable invalid commands emit an error and the unchanged view.
