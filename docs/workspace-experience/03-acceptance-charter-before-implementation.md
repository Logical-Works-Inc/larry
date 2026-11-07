---
type: ADR
title: docs/workspace-experience.md, part 3 of 3
description: Part 3 of /docs/workspace-experience.md, in order; the index lists every part.
resource: /docs/workspace-experience/03-acceptance-charter-before-implementation.md
tags: [architecture, experience, workspace, evidence]
generated: {by: Codex, at: 2026-09-19}
verified: {by: Codex, method: source inspection and final local PTY receipt}
status: implementation-verified-locally-owner-integration-pending
stale_after: 2026-10-19
sources: [/specification.md, /research-ledger.md, /log.md, Director instructions 2026-09-19]
---
## Acceptance charter before implementation

Actor: returning human who has forgotten where work stopped. Starting state:
fresh terminal. Success: identify a next action, open work, understand whether
anything ran, leave a note, inspect context, return Home and recover
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
- Frontend board/work/context and truth view: W06 implementation, native PTY
  journey and rendered fixture review completed locally; only task-owned demo
  data is exercised.
- Real repository gates: must be rerun after W05 and the query-tool repair.
  Nine-axis production evidence remains outstanding.
- Required endpoint: verified open PR (live main unprotected and no rulesets
  observed this session); no merge inferred from local success.

Owned resources: one frontend process per test (finish/kill verified by PTY
harness), bounded session-only UI buffers, finite receipt files retained with
this revision. The now-unneeded task-local metadata overlay was moved to Trash
after the query-tool repair verified no caller overlay is needed. No service or
port is introduced.
