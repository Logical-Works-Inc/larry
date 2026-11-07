---
type: Design
title: specification.md, part 13 of 18
description: Part 13 of /specification.md, in order; the index lists every part.
resource: /docs/specification/13-cell-05-resource-model-and-initial.md
tags: [rocco, prd, architecture, continuity, efficiency, reinforcement-learning]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: bounded source inspection and external critique; not runtime verification}
status: final-proposed-v1.2-awaiting-architecture-approval
stale_after: 2026-10-15
sources:
  - Director requirements and corrections in conversations 2026-09-15 and 2026-09-16
  - /research-ledger.md
  - /Users/srinji/.okf/governance/contract/frontier-universal-assurance.md
---
## Cell 05 — resource model and initial targets

Targets are proposed BEFORE experimentation; failures reopen decisions. Bounds
are defaults to validate, not claims of optimality or substitutions for stricter
existing limits. A per-instance bound and aggregate tenant bound both apply.

| Resource | Candidate bound / overflow behavior | Measurement |
|---|---|---|
| Ready work | 256 descriptors or 8 MiB per client; admission backpressure, no silent drop | queue bytes, wait time, rejection reasons |
| Workers | 4 runnable local workers default; 2 on constrained profile; ≤8 per run including children | actual process/task tree, total quotas |
| Delegation | depth ≤3; no cyclic task dependencies | graph validation + adversarial cycle |
| Context | ≤min(32k tokens, 50% declared context limit), with separately reserved output/tool budget; oversized obligations require split or explicit error | actual provider/tokenizer usage; obligation coverage |
| Graph retrieval | ≤256 candidate records and 2 MiB decoded per compilation; bounded expansion/cursors | bytes, nodes, p99, misses |
| Tool results | 256 KiB inline; larger output streamed to scoped artifacts with quota | memory/IO and truncation visibility |
| Retry | ≤3 attempts per eligible effect, deadline and jitter; unknown non-idempotent outcome not eligible | attempt ledger, load under outages |
| Speculation | ≤2 branches and ≤10% admitted run compute/token budget, all costs included | discarded work, net latency and quality |
| Workflow | ≤256 nodes, loop maximum declared, candidate ≤100k frontend operations and 16 MiB frontend heap | exhaustion/host-callback cancellation tests |
| Disposable cache | ≤256 MiB client; bounded eviction, no eviction of authoritative data | 1000 repeated lifecycle cycles |
| Retained data | explicit per-workspace quota; warn at 80%, stop admission before reserve exhausted; reserve terminal evidence capacity | disk-full and retention tests |

**Hardware acceptance:** Ryzen 3 4300U 4c/4t, 8 GiB RAM, integrated graphics,
256 GB SSD; physical 4 GiB/two-core constrained and slow-storage profiles;
5 Mbit/s down, 1 Mbit/s up, 150ms RTT, 1% loss plus outages. Retain separate
Ryzen 5 measurements, never substitute for Ryzen 3. Shared graphics ≤2GB counts
against physical RAM. Control-plane attributable processes ≤512MiB steady and
≤1GiB peak; idle CPU ≤1% of one logical CPU over 5 minutes; cold usable p95 ≤5s;
input acknowledgement p95 ≤100ms; cancel acknowledgement p95 ≤250ms, stop new
work immediately and release owned resources ≤2s except declared durable commit.
Zero unsolicited idle model/crawl traffic. Future web initial usable payload
≤1MiB compressed. Maintain operation with 5GiB free storage.

**Local model accounting:** report control-plane, model process, KV/cache and
whole-system memory separately. A selected model profile must fit with OS and
background applications; model allocation failure preserves user state and
returns a clear choice, never downloads/falls back automatically. No generic 8B
model-fit claim on 4GiB. CPU path is mandatory; exact model candidates require
measured quality, residency and latency.

**Complexity expectations (not speed proof):** fold O(events since snapshot),
bounded graph traversal O(visited vertices+edges), DAG scheduling O(V+E), context
render O(selected bytes). No replay of entire history on each message. Compare
incremental projections against on-demand queries, bandit against fixed routing,
Rhai frontend against typed builder, and speculative vs sequential execution on
the same semantics. Provider and remote-service costs remain inside the ledger.

## Cell 06 — nine-axis acceptance declaration

The required invariant/evidence wording is retained. IDs A1–A9 provide the
measurement bindings below. All runtime results are UNKNOWN today.

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

| ID / owner | Metric, domain, baseline and target | Procedure / failure / uncertainty |
|---|---|---|
| A1 Evaluation | Same-model success difference >0 with paired 95% interval lower bound >0; total cost per successful task ≤baseline; quality/safety constraints unchanged | E16, preregister tasks/seeds; report inconclusive power honestly; no universal-best claim |
| A2 Deployment | 1,000,001 simultaneously active virtual users sustaining admitted tasks, not idle identities; fleet throughput ≥offered admissible load, zero incorrect effects or cross-tenant disclosure | Cell 07; measured virtual load distinct from real humans; overload must bound queues and visibly reject |
| A3 Core owners | zero unowned effects/tasks, typed errors/terminal states, no unexplained unsafe code; 100% bounded assertion inventory mapped and exercised | compiler/lint/soundness + branch/condition coverage; gaps named, no test-count proxy |
| A4 Evaluation | all declared task families, input ranges, locales and constrained profiles exercised; unseen cases retain obligations | E01–E54, independent hidden cases; failure in one family not averaged away |
| A5 Integration | swap each adapter with a second compatible real implementation/configuration; zero outcome-semantic drift | contract/version skew suite; doubles alone insufficient, missing second real path stays unknown |
| A6 Lifecycle | zero lost acknowledged commitments, zero duplicated reconciled effects; ≥10% e2e cases actual panic/process crash with recovery | 9 of 54 planned E cases below; repeated 1000 cycles; real kill in owned fixture only |
| A7 Platform | same core outcomes on maintained Linux/Windows x86_64 and macOS arm64; iOS surface end-to-end separately | exact OS/toolchain/artifact recorded; no Mac resource-limit proxy for physical ThinkPad |
| A8 Security | zero unauthorized cross-scope reads/writes/cache hits; tenant quotas cannot be exceeded; admitted quiet tenant p99 ≤2x isolated baseline under noisy neighbor | concurrent two-tenant negatives plus fleet skew; threshold is candidate target, not measured |
| A9 Performance | Cell 05 budgets and E16 cost target; full-path p50/p95/p99, CPU, allocations, RSS, IO, contention, transfer and energy/cost | matched hardware/workload alternatives; record raw telemetry, uncertainty and failures |

## Cell 07 — fleet-scale workload, not a million-persona shortcut

Candidate workload: 1,000,001 active simulated users, each with an admitted
multi-step task for the sustained phase, 20s mean user command interval and
10s mean progress/event interval. This is approximately 50,000 commands/s plus
100,000 progress events/s before tool/model traffic (arithmetic estimate, not
capacity evidence). Use 100,000 tenants with Zipf 1.1 skew and seeds 7/23/101;
task mix 40% research, 30% coding, 20% workflow, 10% collaboration. Track all
worker/tool/model traffic separately; synthetic providers are protocol-load
results, never inference-capacity or real-task-success proof.

Declare fixed fleet hardware/count/cost and placement BEFORE the run; size using
measured per-shard capacity and headroom, not an invented node count now. Model
inference gets a separate admitted queue and real-provider/local-host capacity
receipts. Connection/device distribution: 50% normal, 30% weak network, 20%
intermittently disconnected; CPU/memory distribution uses the named device
profiles and separately labeled server hardware. Exercise 30min ramp, 60min
sustained, 2x offered burst for 5min, worker/shard loss and 30min recovery.
Candidate control-plane acknowledgement p99 ≤1s on normal network, ≤3s weak
network; task completion targets are frozen per benchmark family after pilot
measurement and before candidate comparison, not substituted by fast ack.
Correctness and resource bounds must hold at overload even if admission rejects.
These are required planned trials, not claims of millions of real deployed users.
