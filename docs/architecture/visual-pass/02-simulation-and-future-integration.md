---
type: Design
title: docs/architecture/visual-pass.md, part 2 of 3
description: Part 2 of /docs/architecture/visual-pass.md, in order; the index lists every part.
resource: /docs/architecture/visual-pass/02-simulation-and-future-integration.md
tags: [larry, demo, appcui, fixtures, evidence]
generated: {by: agent:rust-coder, at: 2026-09-17}
updated: {by: agent:claude-code, at: 2026-09-18, note: Forge terminal language pass}
verified: {by: agent:rust-coder, method: source inspection; execution receipts in log}
status: forge-look-implemented-visual-review-pending
stale_after: 2026-10-18
sources: [Director continuation 2026-09-17, /specification.md, /log.md, https://github.com/gdt050579/AppCUI-rs, https://crates.io/api/v1/crates/lgwks_std/0.6.3, https://crates.io/api/v1/crates/lgwks_deps]
---
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
5. Larry pins the published 0.1.8 and enables AppCUI. No local path
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
