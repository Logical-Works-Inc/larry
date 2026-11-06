---
type: Specification
title: "Larry fleet and efficiency contract"
description: "Hyperscale resource, isolation and performance obligations without changing single-client semantics."
resource: /architecture/larry-forge-spec/docs/04-fleet-efficiency.md
tags: [larry, hyperscale, efficiency, multi-tenant]
generated: {by: ChatGPT, at: 2026-09-16}
verified: {by: ChatGPT, method: "Specification only; runtime/load tests NOT RUN"}
status: proposed-not-ratified-runtime-not-run
stale_after: 2026-10-16
sources:
  - "https://github.com/srinji-kaggss/Larry/pull/1"
---

# Fleet and efficiency contract

Larry's semantics must remain invariant from one embedded client to >1,000,000 simultaneously active users. Scale changes topology, placement and batching—not authority, effect truth or completion semantics.

## Partitioning

- Partition by tenant/resource/run; avoid global total ordering.
- Causal lineage crosses partitions through explicit IDs, not shared mutable process state.
- Exclusive external effect scopes have one fenced live authority at a time.
- Disconnected clients may continue only work already permitted by their bounded leases; they do not become parallel authorities over the same exclusive effect.

## Admission and queue bounds

Each ingress/fan-out point declares both descriptor and byte ceilings. Admission refuses/backpressures before memory growth becomes unbounded. Queue occupancy, oldest age, rejection cause and tenant attribution are observable.

Retry, speculation, delegation and schedules share conserved budgets. A nested service cannot interpret a retry as a new run budget.

## Noisy-neighbor isolation

- tenant quotas are enforced before expensive work starts;
- cache, artifact and model-context use are tenant/security-domain scoped;
- hot tenants cannot consume quiet-tenant reserved concurrency/memory budgets;
- overload may reject new work but cannot lose acknowledged commitments or weaken security/evidence gates.

## Fleet lifecycle

Workers are disposable. Durable truth lives outside worker process memory. A worker lease includes owner, generation, scope, expiry and fenced authority. Stale generations cannot commit effects or terminal receipts.

Fleet routing can migrate across regions/providers if data residency, recipient and authority policy allow it. Such movement is explicit effective configuration—not hidden fallback.

## Resource accounting

At minimum account for:

- CPU time and runnable concurrency;
- resident/peak memory, model weights and KV/cache separately;
- local/remote storage and artifact bytes;
- network ingress/egress and request count;
- model uncached/cache-write/cache-read/output/reasoning tokens where applicable;
- external service/provider spend;
- speculative/discarded work;
- retries and recovery work;
- verification and learning cost.

Optimization target is **verified useful outcome per total resource**, not token count or latency alone.

## Client baseline

Retain PR #1's physical Ryzen 3 4300U / 8 GiB baseline plus 4 GiB/two-core constrained profile. Whole-system model residency counts; control-plane memory cannot exclude helper processes it requires. No mandatory GPU, hidden cloud fallback or unsolicited idle inference.

## Fleet acceptance

Retain PR #1's target of 1,000,001 simultaneously active virtual users with skew, weak/intermittent networks, burst, shard loss and recovery. Synthetic provider load demonstrates protocol/control-plane behavior only; it does not establish inference capacity or real task quality.

Acceptance requires zero unauthorized effects/cross-tenant disclosures, bounded queues under overload, no lost acknowledged commitment, and recovery to a stable admitted state after injected failures.

## Performance methodology

Every comparison preserves all required semantics. Report p50/p95/p99 for admission, context compile, effect admission, external operations, verification and terminalization, plus CPU/RSS/allocation/I/O/contention/network/cost. State workload, hardware, versions and confidence/variance.

A faster candidate that bypasses sandboxing, verification, persistence, authority checks or reconciliation is not a valid performance comparison.

## Scaling-law guardrails

No algorithm on a request-critical path may require scanning all tenant history, all fleet workers or all active runs. Expected complexity remains bounded by selected/recent state or partition-local indices. Cross-partition coordination is explicit and rare.

Background maintenance uses bounded batches, cursors and backpressure. No global periodic model call is required for liveness.
