---
type: ADR
title: Board-first Rocco workspace
description: Multi-step workspace, attention contract, and explicit demo-to-owner truth path.
resource: /docs/workspace-experience.md
tags: [architecture, experience, workspace, evidence]
generated: {by: Codex, at: 2026-09-19}
verified: {by: Codex, method: source inspection and final local PTY receipt}
status: implementation-in-progress
stale_after: 2026-10-19
sources: [/specification.md, /research-ledger.md, /log.md, Director instructions 2026-09-19]
---

# Outcome and authority

The Director requests an ADHD-friendly, lightweight workspace with Rocco the
system and an AI, not a chat page decorated with a dashboard. Start with a
mini-Jira Kanban state update; opening work automatically invokes Continue.
Moo powers the workspace. The Director explicitly authorizes one backend
agent, reuse from prior harness attempts, new ADRs, and synthetic DATA with
visible provenance and a path to truth. Fake orchestration is not authorized.
This expands the older fixture-only implementation authorization, not Moo's
ownership or the Logical-DB memory prerequisite. R20/R44/R51/R52/R64 and
D19/D21/D25 remain in force. This elaborates their surface, not a new runtime
state algebra. All nine acceptance axes below remain unfulfilled until their
own evidence exists; a working demo cannot waive them.

## ADR-W01 — Orient, then enter work

Decision: stable Kanban lanes, compact next-action cards and recent activity
precede chat. Enter/click work restores its own draft, selection and inspection
state and invokes the SAME typed Continue coordinator as the explicit command.
Escape returns Home without discarding the draft. No automatic execution claim
on navigation. Completed work opens for inspection and refuses a continuation
through the same terminal-state guard.

Rejected: Cline's hero/prompt landing because it requires remembering what to
ask; a single full-screen chat because context vanishes into chronology; a
full Jira clone because maintenance controls bury the next action. Accepted
tradeoff: fewer simultaneous controls; deeper inspection is one step away.

## ADR-W02 — Stable attention, contextual system cards

Decision: chat is a dedicated central area, with a separate system/context
panel. The highest-priority system fact changes with actual local state
(dependency missing, feedback held, change selected), in a FIXED location.
No auto-rotating carousel, timer, notification count inflation or invented
cost/latency. Narrow terminals expose the same context through panel focus.
Text labels carry state independently of color; ordinary keys work without
function keys. The raccoon identity remains available without becoming the
landing page's main content. Human ADHD benefit is an ESTIMATE, not a clinical
or measured usability claim. Human aesthetic acceptance remains unknown.

## ADR-W03 — Truth is part of the interface

Decision: persistent DEMO DATA labeling plus a discoverable truth view lists
which information is synthetic and what supplies it later. Cards, recent
activity, source diffs and fixture feedback are synthetic. Continue dispatch
is real typed validation, with MissingMooAdapter returning AwaitingDependency;
it never creates a fake Running/Completed state. Chat without a model records
a local unsent note, not a pretend AI response. Drafts are session-only, lost
on process exit, and visibly described that way.

Truth path: replace the fixture projection with authenticated Moo workspace
summary/event/check snapshots; revalidate tenant, workspace, baseline, fence
and revision at Moo's atomic claim/admission boundary; retain operation and
command identities durably in their real owner before enabling effects.
Replace unsent notes with a bounded model transport only when admission,
cancellation and provenance work. Logical-DB owns future durable memory.
No generic filesystem/process tool is introduced by this surface change.

## ADR-W04 — A terminal workspace needs depth, not decorative telemetry

Decision: Rocco uses a dark spatial shell, a persistent work rail, large
coloured work cards, an independent purple System attention stack, and a
separate Human + AI work pane. Green means selected focus, teal means work
surface, amber marks synthetic/disconnected data, and every one of those
states also has explicit text. Card/panel colour is presentation, not a source
of orchestration truth. The AppCUI admission is ANSI16, so the palette names
its actual terminal colours instead of pretending to deliver 24-bit RGB.

Opening, selecting, and returning home change an input-driven focus marker;
there is no idle animation. A first implementation used AppCUI's 8fps frame
loop for six focus frames. It failed the pre-existing 1MiB idle-output bound in
under 30 seconds on the actual 120x32 PTY, because the framework redraws the
whole surface every tick. That implementation was removed rather than raising
the bound. The retained feedback is local response to human input; it neither
cycles status nor depicts a model, Moo, or any process as working.

Rejected: the prior sparse transparent board (it read as an annotated drawing,
not a place to work); a full monitoring dashboard (would manufacture telemetry
Rocco does not have); the measured frame-loop animation and ambient
shimmer/rotating cards (would turn a glanceable workspace into an attention
demand). The accepted tradeoff is ANSI16 depth and static density over web-like
gradients and continuous animation.

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

Local pinned Qwen oracle pre-exec receipt:
`tools/receipts/20260919-workspace-ui-pre-exec-3.json`. It ranks board-first
highly but assigns low relevance to typed resume: relevance is advisory,
not permission to discard identity checks. Earlier attempts failed: framework
Python lacked torch; Homebrew Python saw a stray empty NumPy dist-info. A
task-owned metadata overlay exposes the actual installed 2.4.4 metadata;
no shared package was changed or model downloaded. The final event-driven
binary completed a 300.197-second 120x32 macOS PTY probe with 0 idle output
bytes, 0.0% delta CPU, 7,680KiB sampled maximum RSS, 3.576ms p95 input and a
clean reaped exit. Receipt: `docs/evidence/workspace-final-metrics.json`.
These are local debug-build observations, not a performance proof for Ryzen,
other terminals or fleet use. The bounded post-exec oracle ranked the
terminal-IDE shell and truthful-colour alternatives 1.0 and interaction motion
0.934 (`tools/receipts/20260919-workspace-ui-post-exec.json`); it is advisory
only and changes no owner or acceptance requirement.

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

## Acceptance charter before implementation

Actor: returning human who has forgotten where work stopped. Starting state:
fresh terminal. Success: identify a next action, open work, understand whether
anything actually ran, leave a note, inspect context, return Home and recover
the same draft. A plausible mistake is interpreting synthetic Running as a
live process; labels and truth view must falsify that interpretation. Test
real launch, keyboard AND mouse, narrow/wide resize, wrong command, repeated
Continue, interruption, fresh restart and independent per-work drafts. Tests
must inspect rendered state, not merely internal method calls.

| Axis | Required invariant | Evidence required |
|---|---|---|
| Frontier | The selected design beats credible current alternatives on the actual problem without sacrificing correctness. | Dated primary research, named alternatives and matched capability/cost measurements. Familiarity or small size cannot establish superiority. |
| Hyperscale | More than 1,000,000 active concurrent users remain correct under heterogeneous compute, network conditions, tenant skew and failures. | Sustained load, burst, saturation and recovery results with workload definitions, resource bounds and raw telemetry. |
| Idiomatic Rust | Ownership, state transitions, errors and resource lifetimes are explicit and sound. | File-specific design review, compiler/lint results, applicable soundness checks and exercised failure paths. The existing Python hook does not satisfy this requirement. |
| Generalized | Behavior works across the declared input and deployment family without special-casing test examples, identities or topology. | Independently chosen cases, boundary/property tests and configuration variation; no hardcoded acceptance shortcuts. |
| Decoupled | Components have explicit owners and contracts and can be replaced independently without changing required behavior. | Replacement tests, dependency analysis, contract compatibility and version-skew results. |
| Ephemeral | Correctness survives process loss; durable state has an explicit owner outside disposable workers. | Crash/restart/replay tests proving state integrity, idempotency, cancellation and recovery. |
| Portable | The same required semantics hold on every declared OS, architecture and runtime target. | Reproducible builds and actual end-to-end results per target, with untested targets visible. |
| Multi-tenanted by default | Identity, data, caches, receipts, quotas and recovery remain isolated under concurrent use. | Cross-tenant negative tests, authenticated scope propagation and noisy-neighbor/failure-isolation measurements. |
| Fastest possible correct Rust | Performance comparisons preserve every required semantic and account for the complete execution path. | Matched CPU, allocation, memory, I/O, contention, throughput and p50/p95/p99 results against credible alternatives; justified lower bounds where applicable. |

Predeclared local targets (not substitutes for table): Frontier owner frontend,
compare old chat-first journey versus board->work, target one activation to
Continue, failure additional command required; cost/human superiority unknown.
Hyperscale owner runtime, target >1M active users under skew/failure, workload
and infrastructure unavailable; FAIL acceptance until measured. Idiomatic Rust
owner both: no unsafe, typed failures, all workspace fmt/test/clippy/dependency
gates exit zero. Generalized owner frontend: 50x12 through 160x48, Unicode,
empty state and repeated work switching; no panic or cross-work draft leak.
Decoupled owner backend: adapter replacement and refusal tests, zero fake
success without owner receipt; real version skew unknown. Ephemeral owner Moo/
Logical-DB: crash/restart exposes session loss honestly, never restores fake
admission; durable resume remains unimplemented. Portable owner both: actual
macOS terminal now; Linux/Windows/Ryzen results remain unknown. Multi-tenancy
owner backend: wrong-scope requests rejected before adapter, measured isolation
at scale unknown. Performance owner frontend: ≤512MiB steady/1GiB peak,
≤1% idle CPU over five minutes, p95 input≤100ms, no >50ms main-thread task,
cold launch≤5s on physical Ryzen3, cancel≤250ms/release≤2s, zero idle network,
finite storage with 5GiB reserve. No timer or background worker added; absolute
superiority and absent hardware measurements cannot be inferred from that.

## Work tracking and lifecycle

- Read/spec/source/oracle: completed within the bounded source inventory above.
- Backend reuse and typed Continue: one delegated implementation returned;
  parent integration is complete pending repository-wide verification.
- Frontend board/work/context and truth view: implemented; visual and sustained
  runtime receipts are being refreshed against the final source.
- Real journeys and local repository gates: completed; nine-axis production
  evidence and the verified PR endpoint remain outstanding.
- Required endpoint: verified open PR (live main unprotected and no rulesets
  observed this session); no merge inferred from local success.

Owned resources: one frontend process per test (finish/kill verified by PTY
harness), bounded session-only UI buffers, task-owned reranker metadata overlay
(agent removes by Trash after both post-exec queries), finite receipt files
retained with this revision. No service or port is introduced.
