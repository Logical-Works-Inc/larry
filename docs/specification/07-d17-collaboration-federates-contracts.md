---
type: Design
title: specification.md, part 7 of 18
description: Part 7 of /specification.md, in order; the index lists every part.
resource: /docs/specification/07-d17-collaboration-federates-contracts.md
tags: [larry, prd, architecture, continuity, efficiency, reinforcement-learning]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: bounded source inspection and external critique; not runtime verification}
status: final-proposed-v1.2-awaiting-architecture-approval
stale_after: 2026-10-15
sources:
  - Director requirements and corrections in conversations 2026-09-15 and 2026-09-16
  - /research-ledger.md
  - /Users/srinji/.okf/governance/contract/frontier-universal-assurance.md
---
### D17 — collaboration federates contracts, not private minds

Choose project membership and explicit shared spaces; each person retains a
private relationship context. Harness federation exchanges signed, versioned,
minimized task/result envelopes under mutually verified authority. A remote
harness is untrusted even if its text sounds like the user. Resolve simultaneous
human directives by resource ownership, role and approved conflict policy; no
last-speaker-wins or inferred social hierarchy. Both legitimate but incompatible
requests become an explicit conflict while unrelated work proceeds. E18; owner
collaboration layer using Secure-Authority, Logical-DB and Moo.

### D18 — three replay modes, never one misleading guarantee

Choose: (1) observation replay displays recorded inputs/results; (2) deterministic
control replay folds recorded events with pinned reducer/policy and virtual
clock/random inputs, issuing NO live effects; (3) a fresh experiment reruns models
or tools in an isolated environment and may differ. Pin adapters, workflow,
model identity/template and dataset versions. Preserve model responses where
permitted; do not require access to hidden model reasoning. Redacted, expired or
deleted artifacts make replay explicitly incomplete. An old grant proves past
authorization, not current permission. E19; owner event/replay layer.

### D19 — one protocol, multiple accessible surfaces

Choose TUI/REPL and scriptable CLI with the same task/attachment contract as
phone/app/web. Persisted cursors support reconnect, slow networks, sleep and
device loss. Presentation never owns task truth. Phone background notifications
are hints, not a durable tape; exact Apple transport/lifecycle design requires
local Apple corpus research before implementation. Windows/Linux/macOS core and
TUI plus iOS phone are candidate support targets, all untested here. Native
platform affordances use adapters; no forced lowest-common-denominator UX.
E20; owner presentation, no separate phone run engine.

Director-authorized visual pass 1, 2026-09-17: a native AppCUI terminal mockup
consuming the published Logical Works storefront, with quiet REPL default,
compact header/footer, optional folder drawer and narrow vertical agent rail,
conversation/events, fixture diffs and file/line/trace inspection. Anchored
feedback holds affected owner work; Continue remembers its anchor. Agents are
parent/owner plus clone delegates, not peers. Telemetry is contextual, never a
permanent dashboard. Every simulated path stays visibly DEMO; no backend,
provider execution or workspace mutation is authorized by a fixture action.
Navigation/input/toggles must be genuinely interactive. The simulation and
future-owner inventory is retained in `/docs/architecture/visual-pass.md`;
this visual authorization does not relax E20/E22 or runtime acceptance.

Director correction 2026-09-19: the entry surface is now a mini-Jira/Kanban
workspace overview of current and recent work, not the quiet REPL landing.
Opening a work/project card automatically invokes the equivalent of Continue
through the shared typed coordinator and restores its context. Dedicated chat
shares the focused workspace with separate context panels and state-selected
telemetry cards in stable positions. Moo powers the workspace. Frontend data may
be synthetic if the UI shows where/how and documents the path to real owners;
fake orchestration is forbidden. ADR-W01 through ADR-W03 in
[workspace experience](/docs/workspace-experience.md) elaborate this decision.
The authorized backend slice is bounded projection/Continue coordination and
selective source reuse, not permission to bypass admission or the memory hold.
ADHD-friendly orientation is design intent, not a measured human outcome.

### D20 — typed workflow IR; Rhai is a candidate authoring frontend

Choose a versioned bounded workflow graph with explicit inputs, effects, retry,
parallelism, compensation and termination, not arbitrary Python as the trusted
control plane. JSON/RON serialize the IR; RON is a data notation, not an execution
language. Candidate Rhai subset constructs or transforms the IR with fuel,
memory, depth and output limits; no eval/import/filesystem/network/native effect
escape. Every native callback also needs its own resource limits. Freeze and
validate the generated IR before admission; dynamic replanning creates a new
versioned segment. Reuse any equivalent existing Flow compiler before adding one.
General Python remains possible only as an admitted Nova workload, never a
security boundary. Choose Rhai vs direct typed builder by model success/cost and
soundness trials, not familiarity. E21; owner workflow compiler.

### D21 — reuse named siblings through narrow, tested contracts

Choose contracts first and exact revision/artifact pins after compatibility
testing. Foundation surfaces are `std`, `lgwks_std`, `lgwks_bot`, `lgwks_deps`,
with `lgwks_ast` as the existing parser. Third-party dependencies go through
the estate's admission/storefront process. No copied vendor tree or dual engine.
Moo's current SQLite implementation does not license Larry to introduce a second
SQLite store; consume Moo's workspace contract while Larry persists through
Logical-DB. Never call a sibling production-ready from its name or README.
Missing semantics are integration work in their owner, not an excuse to duplicate
them. E22; owner integration/reuse ledger.

### D22 — local simplicity and fleet scale share semantics, not topology

Choose per-tenant/task partitions, explicit resource admission and active-worker
leases. No global event order or million idle processes. Live execution authority
has one fenced owner per scope; disconnected devices may draft and perform
separately admitted local work, not silently become concurrent authorities over
the same external effects. Logical-DB CRDT convergence is not a distributed lock.
Fleet routing, fencing and revocation need owner-supported contracts and proof.
All nine axes and hardware budgets remain acceptance obligations. E23; owner
deployment/control-plane integration.
