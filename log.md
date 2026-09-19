---
type: Log
title: Rocco decisions and change evidence
description: Accepted directions, scoped corrections and current specification inventory.
resource: /log.md
tags: [governance, decisions, evidence]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: explicit conversation decisions and source inspection}
status: active
stale_after: 2026-10-15
sources: [Director conversations 2026-09-15 through 2026-09-17, /specification.md, /research-ledger.md]
---

# Current specification inventory

requirements: 96
decisions: 53
falsifiers: 80
execution_cells: 13
axes: 9
planned_crash_cases: E02 E13 E19 E28 E31 E39 E40 E42 E50 E72 E77

Counts are document structure, not implemented features or passing tests.

## 2026-09-19: board-first workspace and visual-fidelity correction

Director correction: the earlier mockup looked like a drawing. Rocco must be a
lightweight terminal workspace for a human, Rocco the System and an AI: start
with Kanban state, open work as Continue, retain dedicated chat without making
the whole page chat, and make disconnected fixture data visibly honest.

Decision: retain AppCUI and the single terminal binary, but replace the sparse
transparent composition with an ANSI16 spatial shell: work rail, teal task
cards, green focus, purple System attention stack, and a distinct Human + AI
pane. The counterpart is an input-driven focus response, not an idle loop.
This is informed by READ-only study of local Codex/Cline/OpenCode/Forge source
and public Mirador/Euporie/Socktop/Awesome TUIs material; no external source
code was copied. The full decision and rejected alternatives are ADR-W01–W04
in `docs/workspace-experience.md`.

MEASURED failure and repair: an AppCUI 8fps frame experiment emitted more than
the existing 1MiB idle PTY bound in under 30 seconds at 120x32. It was removed
instead of loosening the limit. The independently input-driven debug binary
then completed a 300.197-second actual 120x32 macOS PTY probe: 0 idle output
bytes, sampled max RSS 7,680KiB, delta CPU 0.0%, p95 input 3.576ms, first
paint 18.716ms and quit-to-reap 75.058ms; receipt
`docs/evidence/workspace-final-metrics.json`. This is a host-specific
measurement, not a Ryzen, fleet or web-runtime assertion. Static SVG output
was converted and visually inspected locally after native terminal control was
denied; this is presentation evidence, not orchestration, human-acceptance or
cross-platform proof.

Truth contract: all board cards, activities, diffs and fixture feedback state
`DEMO DATA`; the truth view names the real future owners. The typed Continue
coordinator refuses with `Awaiting Moo` when Moo is absent; it does not create
a Running/Completed fiction. Session-only notes stay visibly local and unsent.
The remaining nine-axis limits remain those stated in the ADR; no load,
multi-tenant, Linux/Windows/Ryzen, persistence, real Moo/model or human study
has been claimed.

The bounded local post-exec reranker independently ranked the terminal-IDE
shell and truthful-colour candidates 1.0 and interaction-driven motion 0.934
(`tools/receipts/20260919-workspace-ui-post-exec.json`). It is advisory design
pressure only: PTY measurements, source owners and the acceptance table remain
the controlling evidence.

## 2026-09-19: Quiet Torque correction and query-tool recovery

Director correction: the first high-fidelity terminal pass used too much colour
and caused visual fatigue. W04's teal work, green focus and purple System
regions are superseded by ADR-W05: black baseline, gray structure, white
content, one blue selected-work surface, and amber/red only for text-labelled
operational warning/error. Porsche v3 gives the monochrome hierarchy and
purposeful-motion premise; Atlassian gives neutral Kanban/elevation roles; Apple
layout supplies adjacent-pane and grouping discipline. The accepted result is
Porsche wrapped around Jira—not a showroom and not a stock Jira board. The
input-only focus marker remains because the prior 8fps redraw experiment failed
the 1MiB idle-output constraint.

MEASURED query-tool failure and repair: the local Qwen query process first
failed because a stray empty `numpy-*.dist-info` directory made
`importlib.metadata.version("numpy")` return `None` despite imported NumPy
reporting 2.4.4. `tools/rerank_local.py` now recovers that package version only
when metadata is unusable, before Transformers imports. The regression suite
adds both fallback and healthy-metadata cases; an actual Qwen rerank then
completed with no caller metadata overlay. The overlay was task-owned and moved
to Trash after that proof; no shared Python package was changed. The reranker
is a bounded advisory selector, never a design authority or orchestration path.

MEASURED W05 UI receipt: the actual debug binary completed a 300.361-second
120x32 PTY run with 0 idle bytes, 0.0% delta CPU, 8,992KiB sampled peak RSS,
1.052ms p95 input, 153.504ms first paint, 73.695ms quit-to-reap and a clean
reaped exit. New Home/Work/Truth SVG previews were visually inspected; they are
faithful cell-render evidence, not terminal screenshots or user-study evidence.
The Qwen post-exec rank was quiet-torque 1.0, Jira-default 0.563 and showroom
0.0134; it is advisory only. The prior colourful SVG and metric files remain
historical and are not used as W05 evidence.

The first refreshed PTY pass also rejected a weak test oracle: "Workspace"
appears in a work title and was therefore not sufficient evidence that Escape
had returned Home. The native journey now waits for the Home-only strapline
"Everything else stays quiet"; no product behavior or acceptance was weakened.

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

## 2026-09-19: blocky Rocco raccoon mascot (Clawd-role translation)

Director steering: Rocco must do for Logical Works what Clawd does for Claude
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
`docs/rocco-mascot.svg`. The mascot perches centered on the empty Conversation
landing only, below the safe-fixture lines; it leaves once the conversation
starts and never renders on terminals too small to hold it unclipped.
Deliberate scoped exception to the Transparent rule: the 24x12 mascot cells
paint brand colors (Black/Gray/White/Olive/Red); all UI chrome stays
Transparent. Visual proof: docs/rocco-mascot.svg rendered and inspected in
BrowserOS (mask + eyes + muzzle + tongue read correctly).

MEASURED: 9 native + 6 lib Rust tests pass (new: mascot perch/leave,
canonical palette pixel probes); clippy 0 warnings; fmt clean; 2 native PTY +
4 demo journeys pass; governance 96/53/80/13/9 unchanged; pre-exec receipt
`tools/receipts/20260918-rocco-raccoon-pre-exec.json`, post-exec
`tools/receipts/20260918-rocco-raccoon-post-exec.json`. Remaining limits: no
animation in the TUI (single frame; pet package carries the blink), no
sixel/kitty protocol rendering, pet package unverified inside Codex's pet
picker, no human-attachment evidence (design intent only).

## 2026-09-18: one repository — the contract crate merges into Rocco

Director order, verbatim: "NO ITS ONE REPO. FIX IT", following "Why is it not 1
repo?". The Director authorized both paths. `srinji-kaggss/rocco-runtime` was a
second repository holding `crates/rocco-contracts` (C02, 3,657 LOC across nine
modules); the crate is now a member of this repository's workspace and the
second repository is no longer part of the project.

Inventory unchanged: 96/53/80/13/9. No specification item moved or was
redefined; the merge changed where code lives, not what it must do.

ONE MANIFEST. `Cargo.toml` is now both the `rocco-demo` package and the
workspace root, with `members = ["crates/rocco-contracts"]`. This is one
workspace with one Cargo.lock, one lint set and one dependency register. The
root package previously declared `[workspace]` alone, and the runtime manifest
declared the same crate set from the other side; both are replaced.

ONE LINT SET — the stricter of the two. `missing_docs = "deny"`,
`unsafe_code = "forbid"`, and `clippy::unwrap_used`/`expect_used`/`panic`/
`todo`/`unimplemented`/`dbg_macro` all denied, plus `all = "warn"`. Two
consequences had to be paid rather than waived. First, `missing_docs` reached
`rocco-demo`, which had never been held to it: ten `Rejected` variants, the
`rocco` binary, and `tests/demo.rs` had no crate or item documentation, and
each got a real doc comment naming what it means. Second, `unwrap_used` reached
`rocco-demo`'s test code. Rather than lower the lint, the two allow sites are
scoped to the code that is itself the test — the `native` test module and the
`demo` integration crate — and each states why an unwrap is the right failure
there. No production path is exempted.

Cargo refused the first lint block outright: a lint group and an individual
lint at the same priority is an error, not a warning, so `all` is pinned to
priority `-1` beneath the explicit denies.

ONE REGISTER. `contract/APPROVED.toml` merged from both. `lgwks-deps check .`
now reports **1** semantic approval where the runtime repository's register
held four rows: the checker's own ladder treats `lgwks_std` as rung 2
(workspace stdlib+, self-exempt) and `lgwks_deps` as the estate storefront, so
neither needs a row, and `lgwks_bot` is a catalog entry no member depends on.
The single real third-party edge is `thiserror =2.0.20` for
`rocco-contracts`, registered in the schema the tool actually enforces — with
`tier`, `capability`, `source`, `allowed_consumers`, `allowed_kinds` and a
sign-off naming the Director's own admission commit
(`rocco-runtime@53c7512`, 2026-09-16) rather than a freshly invented approval.

Two findings worth keeping. The runtime register was **never schema-checked**:
that repository's gate list ran only `cargo fmt`/`clippy`/`test`, so its rows
carried `features` and `default_features` keys this schema does not have and
the merge is the first time the register was validated at all. And
`lgwks_deps` moved `=0.1.7` → `=0.1.8`, because 0.1.8 is the published release
carrying the `appcui` feature the native surface builds against.

The `rocco-contracts` manifest declared `rust-version = "1.85"`. That was
never satisfiable: its own dependency `lgwks_std 0.6.3` declares
`rust-version = "1.98"`. The merge sets one workspace floor of 1.98 and thereby
corrects a false claim rather than raising a real requirement.

Resolver moved 2 → 3, matching edition 2024 on the `rocco-demo` side; the
runtime workspace had pinned 2.

ONE CI. `.github/workflows/ci.yml` now carries both former workflows as two
jobs, step for step — the Rust lanes widened to `--workspace`, and the
governance lanes with their pinned action SHAs and artifact upload unchanged.
Two conventions coexist in the file (tag refs in one job, SHA pins in the
other) because merging did not change either.

MEASURED on this Mac after the merge: `cargo test --workspace --all-targets`
passes 82 tests (57 from `rocco-contracts`, 19 from `rocco_demo`, 6 integration)
— the contracts suite runs here, in this workspace, not in a second repository.
`cargo clippy --workspace --all-targets --locked -- -D warnings` clean;
`cargo fmt --all -- --check` clean; `lgwks-deps check .` OK; 45 governance unit
tests pass; governance checker PASS; 2 native PTY and 4 JSON-driver journeys
pass. `lgwks-deps freshness` reports 151 packages checked, 41 stale, 0 errors —
every *authored* pin (lgwks_std, lgwks_deps, thiserror, blake2/thiserror-impl)
is current; the 41 are transitive platform crates under AppCUI.

`docs/dependencies/ADAPTER-MANIFEST.md` described the second repository and a
register read at its commit `1154959`, including version and key names that no
longer match. Its section is rewritten and retitled against a fresh read and a
real `lgwks-deps check .` run; the correction is marked in the document rather
than silently edited.

Limits: the merged workspace's Linux CI has never run. AppCUI's Linux closure
is pure-Rust in the lockfile as far as this machine can see, but this machine
is a Mac, and hosted CI for this repository was already unresolved (see the
GitHub verification boundary below) — so "it will build on ubuntu-latest" is
NOT RUN, not implied. The second repository's git history is not folded into
this one: the files were copied, not `filter-repo`'d, so the crate's commits
live only in the remote `srinji-kaggss/rocco-runtime`. That remote was not
touched, per the standing instruction to leave it alone without explicit
confirmation. Coverage is not re-measured for the merged workspace.

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
dashboard carry Rocco's real fixture state (CLEAR/HELD/END canary, `DEMO
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

## 2026-09-17: native AppCUI pass implemented through published estate

CORRECTION: the former no-commit/human-signature blocker was invalid. The
Director clarified the delegation boundary and authorized necessary estate
admission, then explicitly selected "Authorize release continuation". The
estate optional default-off AppCUI feature and register landed in commits
4c4dcb2 and 822307d, PR #7. Real `cargo publish --locked -p lgwks_deps`
published 0.1.8; fresh download confirmed its feature. Rocco now pins published
`lgwks_deps = =0.1.8` plus `lgwks_std = =0.6.3`, with no path dependency.

`src/native.rs` and `src/bin/rocco.rs` add the real input-driven AppCUI view:
quiet default conversation, compact DEMO header/footer, optional folder drawer
and vertical parent/clone rail, manual live fixture events, selectable anchors,
diff inspection, anchored feedback/hold and remembered Continue. Input, pane
toggles and fixture changes actually run. No provider/job/workspace effects
are connected. All simulations and future integrations remain explicit in
`docs/architecture/visual-pass.md`; D19 now records the authorized visual scope.
AGENTS/GOVERNANCE document the additional native verification commands.

MEASURED on this Mac: 9 Rust tests passed; clippy with warnings denied passed;
4 JSON subprocess journeys and 2 real native PTY journeys passed. Two of the
six journeys induce actual owned SIGKILL and restart (33.3%). Native PTY checks
exercise drawing, typed commands, panes, hold, Continue, inspection and exit.
The actual AppCUI Surface at 120x32 was inspected; smaller-size component
renders exercised 1x1, 49x11, 50x12 and 80x24 without panic. This is not a GUI
screenshot or a physical-device/human visual study. Bounded assertion mapping
is 20/20 across component and subprocess evidence, not universal acceptance.

Fresh isolated coverage: 438/544 lines (80.51%), including 118/118 model and
320/372 native lines. Binary entry paths are not included in instrumented tests;
real PTY/CLI tests are separately uninstrumented. Branch coverage is unmeasured.
The reused-target coverage attempt duplicated old model counters and was
replaced by an isolated target, retaining the old output. Initial native clippy
failed on a test initializer and was fixed; initial PTY quit checks timed out
because the harness did not drain output, then passed after bounded draining.
No limit was relaxed. Missing pyte was not worked around by installing a package.

Estate local gates passed 206 workspace tests, 38 selected-feature tests and
the macro doctest; hosted CI run 35281748262 passed at 822307d on the published
source, including Linux/macOS/Windows AppCUI feature lanes. Parent independent
review remains pending. Existing default-branch grid advisory
GHSA-38c5-483c-4qqp remains reported, not silently fixed outside scope.

Native UI launch: `cargo run --locked --bin rocco`. Remaining limits: no full
editor or real backend, no physical Ryzen/fleet/human/performance comparisons,
no production durable recovery or authenticated tenancy proof. Pre-exec receipt
is `tools/receipts/20260917-appcui-admission-pre-exec.json`; post-exec receipt
is `tools/receipts/20260917-appcui-admission-post-exec.json`. Rocco edits remain
scoped for parent integration; estate release is real, not a dry-run claim.

## 2026-09-17: visual-pass independent fixture scaffolding

Director authorized native AppCUI visual pass 1 and explicitly required actual
Logicalworks-Crates use, then directed independent work to continue despite
the admission boundary. The initial delegation limited commits; the Director
subsequently authorized necessary estate commit/push/PR workflow. Parent owns
integration and review.
D19/D21 and R24/R31/R40 remain unchanged. The Rust fixture driver now uses
published `lgwks_std = =0.6.3` JSON for every snapshot. This is not the native
view: all missing visual work and simulated behavior is inventoried in
[visual-pass evidence](/docs/architecture/visual-pass.md).

The real gate initially walked up to an unrelated home evaluation workspace
and failed (exit 2); an explicit Rocco contract diagnosis refused its missing
register (exit 2). Rocco now has an independent Cargo workspace, lockfile and
fail-closed register with no fabricated third-party approvals. The new graph
printed `OK . — 0 semantic approvals, every authored external edge is owned`.
The published foundation is self-exempt; this verdict does not admit AppCUI.
Initial Rust verification passed six fixture tests and clippy with warnings
denied. Further final verification receipts are reported after execution.

AppCUI 0.5.1 needs an optional `lgwks_deps` storefront edge, genuine signed
admission and a new published estate version. The current published 0.1.7 has
no AppCUI feature. CORRECTION: the former claim that a forbidden commit blocked
human signoff confused delegation scope with user authority. The Director has
authorized the estate admission workflow; cryptographic human signing is not
required by the inspected policy. The original fixture-only turn changed no
sibling source; the continuation now adds the owned AppCUI feature there.
No unpublished version is consumed here. Pre-exec relevance receipt:
`tools/receipts/20260917-visual-fixtures-pre-exec.json`; two scoped candidates
ranked 1.0 using the pinned local model. Runtime/provider calls remain absent.

MEASURED: `cargo build --locked` exit 0; `python3 tools/test_demo_cli.py`
exit 0, 4/4 real subprocess journeys, including 1/4 actual SIGKILL/reset
journey with source/manifest/lock digests unchanged. Six Rust tests passed,
and `cargo llvm-cov --no-clean --all-targets --locked --show-missing-lines`
reported 115/118 library lines and 0/43 main lines (115/161 total, 71.43%).
The main driver was exercised separately without coverage instrumentation;
branch/condition coverage remains unmeasured. VF1–VF14 all have bounded
falsifying checks; this is not complete native UI or product acceptance.
Governance checker passed the unchanged 96/53/80/13/9 inventory; its Python
suite passed 45/45; fmt and diff checks exited 0. The editor's initial optional
pipe-type diagnostics in the Python crash test were corrected by asserting
the configured pipes before use; no test failure was hidden or weakened.

The final post-exec relevance receipt is
`tools/receipts/20260917-visual-fixtures-post-exec.json`. The original native
visual requirements, physical Ryzen profiles, million-user evidence, human
outcomes, matched performance comparisons, production recovery and independent
review remain unmet. Owned local `target/` build/coverage artifacts are retained
for parent verification, ignored by VCS; no resident worker or provider exists.

## 2026-09-17: mandated local reranker evidence for repository execution

Director direction: Rocco repository work now uses the local,
on-demand `tools/rerank_local.py` before and after non-read-only execution.
The tool pins `Qwen/Qwen3-Reranker-0.6B` at
`e61197ed45024b0ed8a2d74b80b4d909f1255473`, accepts already access-filtered
JSONL candidates, caps them at 256, and returns ranked JSON evidence. Pre-exec
receipts inform planning; post-exec receipts rank verification evidence. Neither
is an authority decision, source of instructions, automatic model download,
daemon, host hook or product-runtime acceptance claim.

READ: the Hugging Face model card specifies a 0.6B reranker with 32k context and
the documented Transformers yes/no-logit method. MEASURED on this macOS host:
the exact pinned local model ranked the relevant synthetic candidate above the
irrelevant candidate (0.9921875 vs 0.0000166893). This is a two-candidate tool
smoke test, not retrieval quality, low-resource hardware, portability,
multi-tenant, fleet or human-outcome evidence. The local package import failed
for `sentence_transformers` because its installed TorchCodec dependency lacks a
compatible FFmpeg library; the tool uses the model card's direct Transformers
method instead. No Rust file changed.

Retained execution receipts: `pre-exec` ranked `local-reranker` 1.0 over
`unrelated` 0.00014019012451171875; `post-exec` ranked `verification` 0.9921875
over `unrelated` 0.0004177093505859375. Both used two synthetic, scope-safe
candidates and the pinned local artifact. Their query/candidate digests are
retained under `tools/receipts/`; each evidence command uses a new receipt
path so a prior receipt cannot be overwritten.
These receipts prove the CLI path and ordering for this bounded sample only.

## 2026-09-17: AI query workflow made executable in AGENTS.md

`AGENTS.md` now gives the agent-facing local query sequence: scope and filter
untrusted candidates, produce a unique pre-exec receipt, use rank only as
relevance evidence, perform authorized work, retain a distinct post-exec
receipt over final evidence, then run the actual repository checks. It names
the local failure modes and explicitly preserves source provenance, authority,
scope, test and human-experience boundaries.

## 2026-09-17: local reranker lifecycle bounded

The local reranker remains one-shot: it loads only during a query, defaults to
a 900-second wall-clock bound (maximum 3600), writes an evidence receipt, and
exits so operating-system process teardown reclaims model memory. A timeout is
a loud failed query, never a silently resident model service. This is lifecycle
semantics for the repository tool, not a measured RAM budget or hardware claim.

## 2026-09-17: hostile-model defense and the 8B survival gate

The Director required strict defense in depth against malicious or unreliable
models, malicious authorized compositions and the untrusted internet, with Nova
retained as sandbox owner and Secure-Authority as the authenticated capability
owner. R79–R96, D42–D53 and E63–E80 make model obedience unnecessary for
security, bind resolved effects at their domain owners, add compartment-scoped
information flow, cumulative composition limits, secret/egress binding,
collaboration isolation, revocation/recovery/update fences, exact low-friction
consent and independent mutation oracles.

The named-artifact 8B gate separates zero-violation containment from useful work:
target at least 90% verified benign completion, no more than five percentage
points below the matched same-model baseline, zero repeated approvals within
unchanged authority and no stronger-model/cloud fallback. The Ryzen 3 4300U
8 GiB and 4 GiB profiles, local-admission p95 <=50 ms and human estimate of 0–1
routine interventions are targets, NOT RUN measurements.

No Rust file changed in Rocco, Secure-Authority or Nova. No runtime, model,
hardware, coverage, fleet or human result is claimed. The prior Wasmtime owner
assumption was corrected before specification delivery: an engine can only be a
Nova implementation mechanism. Failed research/browser receipts and source limits
are retained in `research-ledger.md` rather than converted into evidence.

Local document verification on the final working-tree bytes: `python3
tools/check_governance.py` passed with 96 requirements, 53 decisions, 80
falsifiers, 13 execution cells and 9 axes; `python3 -m unittest discover -s tests
-v` passed 38/38 governance tests; `git diff --check` passed. The checker states
that Mermaid rendering and runtime/model/hardware/human trials are not checked.

## 2026-09-15: settled Director directions

| Direction | Disposition |
|---|---|
| Codex means the OSS harness | Inspect actual source structure; do not confuse it with a model review |
| Rocco is the new placeholder name; create private repository now | Private `srinji-kaggss/Rocco`, documentation/governance only |
| Core + TUI first | D19 delivery order settled; phone/app/web preserved |
| Yes to bounded unattended work | D31 capability required, no background job installed now |
| Adapters, not clones | D28 compatibility commitment settled |
| Defer memory until Logical-DB is done; auto decay, customizable on a scale | D33 records the hold and real typed controls; memory remains required |
| Strict and self-updating governance in the repo being worked on | AGENTS/GOVERNANCE maintenance, deterministic report and CI; not a runtime subsystem |
| Add control flow and flowchart/DAG | Specification includes component map, runtime flow, effect states and implementation DAG |
| MatrAIx is one evaluation source; do not modify it | D34 portfolio with independent benchmarks, fault tests and human trials |

Finalizing the specification is not blanket approval of every proposed
architecture decision, implementation work or billable benchmark execution.

## 2026-09-16: final structure, dependency root and SOTA gap fold-in

| Direction | Disposition |
|---|---|
| Create the full final repository structures, READMEs and documentation in full OKF so the package is finalized | docs/ tree created (README, architecture, dependencies, evaluation); every document carries the complete OKF field set; structural gate extended to admit and validate the tree with adversarial negative controls; governance version 1.1 |
| Logicalworks-Crates is the core starting point for dependencies | Settled starting point recorded in AGENTS.md and /docs/dependencies/README.md with ledger-pinned revisions; D21 reuse mechanics unchanged and still pending ratification |
| Fold the 2026-09-16 SOTA gap research into the specification | R71–R78, D36–D41 and E55–E62 added as proposed, approval pending; triad review dispositions and arXiv/protocol receipts recorded in research-ledger.md |

Semantic change: specification v1.1 adds pure-authorization replay identity,
unified reconciliation metadata, ecosystem protocol adapters (Agent Skills,
pinned MCP revision, A2A boundary), operations telemetry with cost
attribution and request shedding, budget leases, memory edge semantics and
evaluation hardening. Falsifier inventory grows 54 to 62; the planned crash
fraction stays 9/62 (above 10 percent). Every new case is allocated to an
execution cell in Cell 11. The docs/ tree adds navigation only; no canonical
inventory item is defined twice. No Rust files changed; no runtime, model,
hardware or human result is implied.

Authority: the Director requested the final structure, the OKF finalization
and the Logicalworks-Crates dependency root in conversation 2026-09-16.
External reviewer content (Gemini, ChatGPT, Claude) remains advisory.

Local gate verification: `python3 tools/check_governance.py` passed with 78
requirements, 41 decisions, 62 falsifiers, 13 execution cells and 9 axes;
`unittest discover -s tests -v` passed 38 tests (34 prior plus 4 new
docs-tree negative controls). `git diff --check` passed. Two intermediate
failures were retained and repaired: the checker correctly reported log
inventory drift while the specification was already extended (12 downstream
test failures until counts were reconciled), and one new negative control
first mutated a string absent from docs/README.md and then asserted a wrong
error-message substring; the control now mutates a real link and matches the
validator's actual "unresolved or escaping local link" message. The drift
detection was exercised, not weakened. Mermaid rendering is NOT claimed for
this change: no new diagrams were added, and fence checks do not render.

## Corrective lessons with concrete repair

- Repo-governance language was misread as a runtime-policy choice, causing an
  unnecessary clarification. Repair: keep it repo-local in AGENTS/GOVERNANCE;
  regression: review the product inventory for an invented governance feature.
- A narrow earlier Forge search missed existing runtime types. Repair: preserve
  corrected paths and revision evidence in the ledger; do not claim global
  absence from an index/search failure or a partial directory search.
- Patch attempts with nonexistent context failed without changing files. Repair:
  remove the invalid hunk, apply against inspected bytes, verify moved paths and
  run the full structural validator. Never describe a rejected patch as applied.
- The first repo check failed with `FAIL: AGENTS.md: missing governance anchor memory`
  because the checker treated capitalization as semantic. Repair: case-insensitive
  anchor matching; positive capitalization and missing-owner negative controls.
- Independent review found that substring `kill` also matched `skill`, fences
  missed tilde/indented forms, link fragments were unchecked, a test froze counts
  against legitimate additions, and CI compared HEAD to itself. Repair: word-bound
  fault actions, Markdown fence matching, heading checks, extensible positive
  controls and actual base-to-head whitespace checks; retain adversarial tests.
- Independent specification review found E06 unallocated, E36 assigned before its
  implementation, C04 missing its durable-store prerequisite and compensation
  looping into the original effect's success state. Repair: explicit case ownership,
  a Logical-DB-to-C04 edge and compensation as a new identity with original failure
  retained. Final review checks these exact counterexamples, not general reassurance.
- Follow-up gate review found phantom headings inside code fences, inconsistent
  tilde-diagram counting, arbitrary case mentions counted as allocations, and
  remaining hardcoded growth-test fixtures. Repair: one shared fence parser,
  explicit execution-cell assignment forms, dynamic two-step growth fixtures and
  retained false-positive/false-negative regressions. The focused document reviewer
  confirmed all five earlier specification findings resolved.

## Specification v1 semantic change

R01–R43 preserved; R44–R70 specify previously implicit operational boundaries.
D25–D35 define admission, config, compaction, adapters, provider/artifact handling,
routing, schedules, lifecycle, user controls, evaluation and evolution.
E28–E54 give falsifiers; 9/54 planned cases induce actual crash/recovery.
Four harness snapshots and the completed Gemini/ChatGPT review dispositions are
in the ledger. Sources support design comparisons, not runtime readiness.

Authority: the Director requested finalization, additional requirements, visual
flows, a private repository and strict self-maintaining repo governance. No
upstream harness code or MatrAIx files were edited; unrelated work was preserved.

Verification commands live in README and AGENTS. Their actual results are the
local command receipts and the PR's CI report for its exact commit; this entry
does not predeclare a pass. Unresolved: architecture ratification, Logical-DB and
other owner readiness, P1–P8, all runtime/physical-device/fleet/human results.

Visual verification: Mermaid 11.4.1 rendered all four specification diagrams and
the governance diagram from the actual Markdown bytes in BrowserOS (5/5).
Component and DAG screenshots were inspected. This verifies that renderer, not
GitHub's runtime. The first preview script failed with a JavaScript regex syntax
error; replacing the transport-escaped regex with literal fence splitting repaired
the preview without changing diagram semantics. The preview server is owned,
loopback-only and time-bounded; no persistent service was installed.
The preview listener subsequently closed, verified by a failed loopback connect.

Local gate verification: checker passed with 70 requirements, 35 decisions,
54 falsifiers, 13 execution cells and 9 axes; `unittest discover -s tests -v`
passed 34 tests. Four tests drive isolated Python subprocess journeys: successful
CLI, failing CLI, JSON report and actual SIGKILL followed by successful re-run
with unchanged document digests. One of four is a crash/recovery journey for the
read-only GOVERNANCE CHECKER, not evidence that any Rocco runtime crash case passed.
No test changes the repository documents. `git diff --check` passed locally.

Structural coverage remains separately labeled: the standard-library in-process
trace does not include the separately executed CLI subprocesses; branch/condition
coverage is unmeasured. An initial trace reporting command failed sorting a
Python 3.14 non-line sentinel, then was corrected to report actual positive line
numbers. No total-coverage or universal checker correctness claim is made.

## GitHub verification boundary

PR #1 is open against unprotected main in the confirmed private repository.
Hosted runs 35040820654 and 35040872578 failed before execution (zero steps,
runner_id 0). The check annotations contain this literal failure:

> The job was not started because recent account payments have failed or your spending limit needs to be increased. Please check the 'Billing & plans' section in your settings

`gh run view 35040872578 --repo srinji-kaggss/Rocco --log-failed` returned
`log not found: 104620215078`; the jobs and annotation APIs established that no
job steps started. Recovery inspection found Actions enabled and zero registered
repository runners. No existing no-cost runner was available for this repo.
Local checks are not relabeled as hosted CI. The workflow remains enabled and
unchanged; no billing change, new runner service, bypass or fabricated check was
made. Hosted CI is unresolved at the open-PR endpoint, not accepted as passing.
