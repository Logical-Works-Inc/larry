---
type: Design
title: Native AppCUI visual pass and fixture evidence
description: Native terminal interactions, explicit simulation inventory and published estate consumption.
resource: /docs/architecture/visual-pass.md
tags: [rocco, demo, appcui, fixtures, evidence]
generated: {by: agent:rust-coder, at: 2026-09-17}
updated: {by: agent:claude-code, at: 2026-09-18, note: Forge terminal language pass}
verified: {by: agent:rust-coder, method: source inspection; execution receipts in log}
status: forge-look-implemented-visual-review-pending
stale_after: 2026-10-18
sources: [Director continuation 2026-09-17, /specification.md, /log.md, https://github.com/gdt050579/AppCUI-rs, https://crates.io/api/v1/crates/lgwks_std/0.6.3, https://crates.io/api/v1/crates/lgwks_deps]
---

# Scope and literal status

This is native pass 1 for D19/D21, R24/R31/R40 and E20/E22, with broader
product acceptance still unverified. `src/lib.rs` owns fixture transitions;
`src/theme.rs` owns the colour language; `src/native.rs` paints and handles
events through AppCUI InputApp, and `src/bin/rocco.rs` launches the native
terminal. `src/main.rs` retains the separately labeled JSON-lines fixture
driver.
The published `lgwks_std = =0.6.3`, feature `json`, actually serializes every
JSON view; published `lgwks_deps = =0.1.8` supplies the actual AppCUI renderer
and input loop. Neither is an unused dependency or documentation-only adoption.
The repository is its own Cargo workspace so it cannot inherit the unrelated
home-directory evaluation workspace. There is no third-party approval invented
in the Rocco register; the foundation is self-exempt in the inspected gate.

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

## Simulation and future integration inventory

| Surface | Current truth | Required real integration / remaining visual work |
|---|---|---|
| DEMO | Permanent native header/footer and JSON labels | Preserve labels until an explicitly separate real mode exists |
| Quiet REPL/header/footer | Native input-driven surface with compact chrome | Human layout/accessibility review remains required |
| Folder drawer | F2 draws the synthetic folder hierarchy | Moo workspace listing and real file navigation remain disconnected |
| Agent rail | F3 draws parent and two clone delegates vertically | Real owner/clone observations; no peer swarm |
| Live code changes | F4 or `/next` paints the next embedded before/after event | Actual Moo event stream; no automatic timer or background work now |
| File/line/trace inspection | Up/Down or `/inspect N` selects; F6 reveals the exact anchor and diff | Version-bound real references; current files are synthetic |
| Feedback and hold | `/feedback TEXT` stores the note and visibly holds selected owner; other owner may advance | Authority-bound real owner acknowledgement |
| Continue | F5 or `/continue` releases remembered owner/anchor despite selection changes | Durable owner reconciliation, not a real job resume now |
| Conversation | Native typed messages and explicitly SIMULATED canned acknowledgement | Real model adapter remains disconnected; messages never execute |
| Telemetry | Contextual trace only, no invented cost or latency dashboard | Measured real owner telemetry |
| Execution/providers/workspace editing | Absent, not successful no-ops | Separately authorized Nova/Secure-Authority/Moo/provider integration |
| Editor | Before/after strings only; no editor implementation claimed | Choose explicit editor contract; stock AppCUI controls do not guarantee full editor |
| Persistence/recovery | Session-only, explicitly resets on restart; no durable storage | Logical-DB readiness remains prerequisite for real persistence |

## Minimal actionable AppCUI admission

Inspected local `lgwks_deps` manifest and crates.io 0.1.7 features expose GPUI
and Tokio, not AppCUI or another terminal-widget framework. GPUI is a desktop
engine, not a native terminal substitute. The standard foundation's JSON
capability supplies these fixtures, not terminal widgets.

Upstream inspected `appcui/Cargo.toml` declares appcui 0.5.1, MIT, including
platform dependencies. This is source evidence, not a package-license audit or
a signed approval. Context7 returned no matching library and local RAG no hits.

Owner change implemented in estate PR #7. The user explicitly authorized
release continuation after being told the open-PR boundary. Actual
`cargo publish --locked -p lgwks_deps` published 0.1.8; fresh `cargo info`
download confirmed the feature. This supersedes the earlier release blocker.
The recorded admission sequence was:

1. In `logicalworks-crates/crates/lgwks-deps/Cargo.toml`, admit optional,
   default-off `appcui = { version = "=0.5.1", optional = true }` and
   feature `appcui = ["dep:appcui"]`; export it behind that feature.
2. Register `crate=appcui`, `tier=boundary`, `version==0.5.1` (the value is
   the string `=0.5.1`), `owner=lgwks_deps`,
   `capability=terminal.widgets`, `source=registry`,
   `allowed_consumers=lgwks_deps`, `allowed_kinds=normal`.
   Reason: "Native terminal widgets and event dispatch are absent from the
   standard library and existing estate surfaces."
3. `approved_by`, `approved_on`, and `review` must come from the actual human
   admission and its retained record. CORRECTION: the earlier statement
   "This task forbids commits" confused a delegation boundary with user
   authority. The Director explicitly authorized the estate admission commit,
   push and PR workflow. The register's commit-as-signature wording does not
   require a cryptographic human signature. The estate admission now records
   that explicit choice; publication remains a separately verified step.
4. Validate upstream version availability, platform closure, license/advisory
   obligations, macro expansion through the facade (especially absolute crate
   paths), update closure assertions/docs/feature matrix/lockfile, and run the
   real estate admission, test, lint and package-smoke gates.
5. Rocco pins the actually published 0.1.8 and enables AppCUI. No local path
   patch, copied vendor tree, direct AppCUI edge, imaginary version or permissive
   register bridges this boundary. Parent independent review remains pending.

## Bounded assertion inventory and verification

The following assertions describe scaffolding only, not visual acceptance:

| ID | Assertion | Falsifying verification |
|---|---|---|
| VF1 | Every view identifies DEMO and no backend effects | Rust default test and subprocess journey snapshots |
| VF2 | Panes default hidden and toggles reverse | Rust toggle test and subprocess pane journey |
| VF3 | Agent data is parent plus clone children, not peers | Rust topology assertions and subprocess clone check |
| VF4 | Next exposes ordered file/line/trace/before/after events | Hold/Continue Rust and subprocess journey |
| VF5 | Only revealed valid anchors can be inspected | Invalid anchor/overflow Rust tests |
| VF6 | Feedback holds affected owner, not independent owner | Hold/Continue Rust and subprocess journey |
| VF7 | Selection changes preserve held anchor; Continue retains note | Hold/Continue Rust and subprocess journey |
| VF8 | Invalid commands and control text preserve state | Atomic rejection Rust test and subprocess recovery |
| VF9 | Input and note storage have finite bounds | Capacity Rust test and oversized transport journey |
| VF10 | Unicode feedback round-trips through estate JSON | Unicode Rust test |
| VF11 | Empty/end/reset/duplicate Continue have explicit outcomes | Rust scenario and hold tests, subprocess scenario |
| VF12 | Session state is isolated | Two-session Rust test and fresh-process restart |
| VF13 | EOF/quit exit; malformed transport/arguments fail nonzero | Transport subprocess journey and Rust quit test |
| VF14 | Actual process loss restarts clean fixture state without modifying source | POSIX crash subprocess journey with before/after digests |
| VF15 | Native keys and text drive the canonical fixture transitions | Native key test and real PTY hold/Continue journey |
| VF16 | Actual AppCUI paint draws quiet/default and optional panes with inspection | Surface render assertions at 120x32; real PTY pane/inspection output |
| VF17 | Small terminal sizes are bounded without panic | Surface cases 1x1, 49x11, 50x12, 80x24 |
| VF18 | Native bounded input, backspace and reset preserve explicit semantics | Native input/reset test |
| VF19 | Native terminal exits normally after interaction | PTY `/quit` journey with zero exit |
| VF20 | Native actual process crash permits fresh terminal recovery | Separate PTY SIGKILL/relaunch journey |
| VF21 | Every authored dashboard and transcript row fits the narrowest column, and no `Clip` row exceeds a default terminal | `every_authored_row_fits_the_narrowest_column` over every fixture state, panel and the help guide, plus the paint-site debug assertion |
| VF22 | The Forge token set stays distinguishable at this colour depth | `tokens_stay_distinguishable` over `theme::TOKENS` |
| VF23 | The surface carries the Forge chrome: prompt, hint, MI/MD hub and dashboard at the 100-column threshold | `forge_chrome_carries_header_input_hub_and_dashboard`, `dashboard_collapses_below_the_forge_threshold` |

Commands: `cargo test --all-targets --locked`,
`cargo clippy --all-targets --locked -- -D warnings`,
`cargo fmt --all -- --check`, `lgwks-deps check .`,
`PYTHONDONTWRITEBYTECODE=1 python3 tools/test_demo_cli.py`, followed by the
repository governance checker and its tests. Run `cargo build --locked` before
the subprocess suite and `python3 tools/test_native_cli.py` for native PTYs.
Two native PTY journeys passed, one inducing actual SIGKILL/relaunch. These
drive terminal output, not a mocked UI. One of four JSON-driver journeys induces actual SIGKILL
(25% on POSIX); that is fixture reset evidence, not durable recovery or native
durability acceptance. Across the six subprocess journeys, two induce a crash
(33.3%). All subprocesses are owned, have five-second deadlines and are
reaped. No production or unrelated process is targeted.

## Excellence and open evidence

Predeclared: at most three events, sixteen notes, 4096 bytes per command, one
owned session, no worker threads/queues/network/storage effects. Fixed fixture
size means bounded snapshot work; no measured speed comparison is implied.
The driver uses blocking stdin with no polling or async runtime. Target the
canonical <=512 MiB steady/1 GiB peak memory and <=100 ms input-feedback p95,
but physical Ryzen 3 and constrained profiles remain NOT RUN.

ESTIMATE: 6–10 commands and 30–90 seconds for an informed person to exercise
inspect/feedback/hold/Continue, based on the listed command sequence; no human
participants or observed human outcomes. Baseline native UI does not exist.
No superiority, fleet capacity, portability or production isolation is claimed.
Native clipping is deliberate: long rows/input show the visible tail or width;
this is not a full editor. Mouse selection, clipboard integration and automatic
background fixture timing are absent, not successful no-ops.

Per-file predicted outcomes: Cargo manifest/lock/register give a reproducible
published-estate graph; `.gitignore` retains bounded local build artifacts out
of VCS; Rust library/driver provide effect-free fixture transitions; Rust and
Python tests falsify VF1–VF14; this document and log retain simulations,
admission requirements and actual receipts. All nine architecture axes in
specification Cell 06 remain binding. Idiomatic Rust/generalized/decoupled and
fixture-ephemeral evidence are partial; frontier/hyperscale/portable/production
multi-tenancy/fastest-correct-Rust, physical hardware, native visual interaction,
branch/condition coverage and independent review remain unverified. Measured
Rust line coverage from a fresh isolated native-pass target is 118/118 in
`src/lib.rs`, 320/372 in `src/native.rs`, 0/43 in `src/main.rs` and 0/11 in
`src/bin/rocco.rs`, 438/544 overall (80.51%). The separately executed PTY and
JSON CLI suites are not included. Native uncovered paths include runtime entry,
close, conversation eviction, help/scroll/navigation branches and some input
paths. The initial reused-target coverage report double-counted the old library
artifact and was rejected; the isolated target replaces it without deleting
the earlier evidence.
The earlier 14/14 fixture inventory is now expanded to 23 assertions with
native tests; final counts/coverage are retained in log. Independent visual
review and full product acceptance remain pending, not established by finite
tests. Retained failures: initial native clippy rejected a test initializer,
fixed without lint suppression; pyte was unavailable (no dependency installed);
both first PTY tests timed out on quit because the harness stopped draining
terminal output. Draining the owned PTY during exit removed that backpressure;
both real journeys then passed. No terminal failure was silently relabeled.

Forge-look pass, retained failures and limits: the row-width invariant
VF21 was added only after the first 120x32 render showed a clipped `+1 -1`, and
it then failed five more times on copy this document had described as
acceptable — the empty-state hint, two panel footnotes, the welcome sentence,
the help table and the held notice. Each was reflowed rather than the limit
relaxed. The paint-site debug assertion then caught a seventh, a landing row
that only overran below 80 columns; three of the seven would have shipped
unnoticed at the default terminal width. The `Fit` split was introduced because
holding diff source lines to the panel floor would have required either
falsifying the fixture's code or wrapping it. Coverage was **not** re-measured
for this pass: the 80.51% figure below predates `src/theme.rs` and the
rewritten `src/native.rs`, so it no longer describes the current tree and is
retained only as the earlier pass's evidence. `docs/architecture/visual-pass.md`
itself changed label references without re-verifying the simulation table row
by row against the new surface.
