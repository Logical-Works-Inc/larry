---
type: ADR
title: docs/workspace-experience.md, part 2 of 3
description: Part 2 of /docs/workspace-experience.md, in order; the index lists every part.
resource: /docs/workspace-experience/02-adr-w06-rail-rows-are-controls-not-prose.md
tags: [architecture, experience, workspace, evidence]
generated: {by: Codex, at: 2026-09-19}
verified: {by: Codex, method: source inspection and final local PTY receipt}
status: implementation-verified-locally-owner-integration-pending
stale_after: 2026-10-19
sources: [/specification.md, /research-ledger.md, /log.md, Director instructions 2026-09-19]
---
## ADR-W06 — Rail rows are controls, not prose

Observed: the Director could navigate the workspace without research, but the
rail screenshot did not look clickable. Source inspection established the
matching behavioral defect: the rail had no mouse route at all. The small blue
number was therefore decorative rather than a legible hit target.

Decision: each work item owns a compact two-line, 24-cell rail target. The one
selected item gets the same dark-blue focus surface used by the selected board
card, a blue leading marker, and explicit `click / Enter to open` copy. A
pointer press anywhere in that target selects work and invokes the same typed
Continue coordinator as card selection; it still reaches `Awaiting Moo` when
Moo is absent. Resting rows remain neutral. Blue means current selection;
amber remains the explicit disconnected/demo warning; no green success state
is manufactured.

The click must work on the Director's macOS terminal, not only in an event
unit test. AppCUI's documented macOS default is the Termios backend, which
does not receive mouse events; the runtime now selects its already-admitted
CrossTerm backend, which enables mouse capture. The POSIX PTY test now
emulates cursor-addressed terminal output and sends an SGR click through that
backend. This proves the actual launch/input route locally; it does not prove
other terminals or human acceptance.

Read local Forge `forge-tui/src/theme.rs` and `render.rs`: its selected list
uses a single slate-light surface and a leading highlight symbol while the
rest remains quiet. Larry adopts that interaction grammar, not Forge's
Ratatui source or dependencies. Rejected: making every row blue, a hover
animation, or a toolbar—the first recreates the colour fatigue the Director
rejected; the others add attention and runtime work without clarifying action.
Accepted tradeoff: one small, stable selection surface is less spectacular than
a web control, but it is natural in a terminal and maps directly to action.

MEASURED after W06: the mouse-enabled debug `larry` binary completed a
300.124-second 120x32 macOS PTY probe with 0 idle output bytes, 0.0% delta
CPU, 7,824KiB sampled peak RSS, 0.892ms p95 input, 216.412ms first paint,
59.635ms quit-to-reap, and a reaped zero exit. Receipt:
`docs/evidence/workspace-final-metrics.json` (binary SHA-256
`c7c2882f7a186521e6b46424bd3417f92000e54470a6d44e3905d1d9d1b47968`). This
is a measurement of this local macOS debug binary only; Ryzen, other
terminals, networks, fleet behavior and human acceptance remain unknown.

## Research and oracle boundary

READ 2026-09-19: local Codex revision
`bb5054fe47abe73ecbbd454751066a28c89f4bb9`: `app_server_session.rs`
separates thread resume from turn start; `session_resume.rs` preserves workspace
identity; `resume_picker/page_loading.rs` rejects stale completions;
`bottom_pane/mod.rs` prioritizes modal/composer routing; `scroll_state.rs`
clamps selections; `frame_requester.rs` coalesces event-driven redraws.
Apply identity-bound resume and input ownership, not its async dependency tree.
READ local Cline `apps/cli/src/tui/views/home-view.tsx`, history resume and
VS Code history preview: keep direct task restoration, reject hero-first home.
READ local OpenCode `packages/tui/src/routes/session/sidebar.tsx` and
`packages/app/src/pages/layout/sidebar-workspace.tsx`: independently scoped
workspace navigation and fixed context beside chat. No TS code is copied.
AppCUI 0.5.1 local pinned source supplies mouse events and `auto_close(false)`;
Context7 resolution had no AppCUI library. Its unrelated Rust result was not
used as API evidence.

Local pinned Qwen oracle is advisory only; it changes neither owner nor
acceptance requirement. The pre-exec `porsche-jira` comparison ranked
`quiet-torque` 1.0 above `porsche-showroom` 0.348 and `jira-default` 0.307
(`tools/receipts/20260919-porsche-jira-pre-exec.json`). The first invocation
exposed a real query-tool fault: a stray empty NumPy metadata directory caused
`importlib.metadata.version("numpy")` to return `None`, although imported NumPy
reported 2.4.4. `tools/rerank_local.py` now recovers the executing package's
version only when metadata is unusable; six unit checks plus an actual
no-overlay Qwen rerank passed (`20260919-query-tool-fix-post-exec.json`). No
shared Python package, model revision or caller environment was modified.

W05's prior receipt is historical context only. The current W06 metric receipt
names the mouse-enabled binary above. Neither local debug-build observation
proves Ryzen, other terminals, fleet scale or human acceptance.

MEASURED after W05: the actual debug `larry` binary completed a 300.361-second
120x32 macOS PTY probe with 0 idle output bytes, 0.0% delta CPU, 8,992KiB
sampled peak RSS, 1.052ms p95 input, 153.504ms first paint, 73.695ms
quit-to-reap and a reaped zero exit. Receipt:
`docs/evidence/workspace-final-metrics.json` (binary SHA-256
`d160fe08752afad8b54cda92c102b8fde14687815077506cda7bfc178d04ae5b`). This
measures this macOS debug binary only; it does not establish the absent Ryzen,
other-terminal, network, fleet, human or matched-alternative evidence.

Post-exec Qwen ranked the final quiet-torque candidate 1.0, standard Jira 0.563
and a decorative Porsche-showroom candidate 0.0134
(`tools/receipts/20260919-porsche-jira-post-exec.json`). That bounded rank is
advisory selection pressure, not a human study or proof of visual superiority.

READ 2026-09-19: Mirador’s public README documents a glanceable terminal
dashboard with a focused panel, dimmed surrounding panels, and an explicit
"nothing blinks or shimmers" constraint. Adopt focus/dim hierarchy and quiet
at rest; reject its broad always-on dashboard scope. Euporie is a terminal
notebook and Socktop a rich remote monitor: both establish that a dense,
layered terminal surface is feasible; neither justifies fake data or a remote
runtime. Awesome TUIs is a discovery index, not code or API evidence. Local
Forge `forge-tui/src/{theme,brand,render}.rs` was re-read: carry forward its
spatial shell, status/system separation, palette roles and data-ink density;
do not import or copy Ratatui code. The visual-fidelity reranker ranks a
terminal-IDE shell and truthful colour highly. Its low score for event motion
was advisory only, but the actual AppCUI frame-loop trial independently failed
the existing idle-output bound, so continuous animation is rejected on measured
lightweight grounds.
