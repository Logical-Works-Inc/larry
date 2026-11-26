---
type: ADR
title: "ADR 0114: hot runtime lookups are indexed by open work"
description: "The tick, approval and receipt paths use bounded indexes instead of scanning journal history."
resource: /docs/adr/0114-open-work-cost.md
tags: [larry, adr, runtime, performance, open-work]
status: accepted
sources: [larry#216, INVARIANTS.md, crates/larry-runtime/tests/open_work_cost.rs]
---

# 0114: hot runtime lookups are indexed by open work

Status: accepted, 2026-11-26 (larry#216)

## Context

The runtime used three history scans on user paths. Reconciliation searched
every run for an operation. Ask and Submit searched every question. Receipt
publication searched every receipt. A completed journal therefore made open
work slower even when the user had one effect.

## Decision

- `FoldProjection` folds one operation-to-run `BTreeMap` and uses it for effect
  phase and dispatch lookups.
- `FoldProjection` folds a `BTreeSet<OperationId>` for receipt identity checks.
- `approval::Book` keeps exact question records cold, a subject index for
  settled binding checks, and a pending index keyed by tenant, run and
  operation. Answered questions leave the pending index.
- The runtime keeps cancellation run ids and per-tenant run counts. Tick and
  run-capacity admission do not recount unrelated history.

The indexes are deterministic, bounded by the journal or runtime capacities,
reconstructed from the journal, and do not change replay facts.

## Rejected

Keeping the scans preserves less code but makes cost depend on history. A
hash map would make iteration order a replay input. Evicting question records
would lose exact respondent and subject validation after restart.

## Consequences

Open-work decisions now pay for ordered index lookups and retain bounded cold
records needed for replay. Journal restore rebuilds the same indexes before a
runtime accepts work. The benchmark prints the cost and sampled resident set
so a later change can detect a regression.

## Proof

`open_work_cost.rs` drives the public runtime at 1, 1,000 and 60,000 recorded
runs. It prints 64-sample p50, p95, p99 and sampled peak RSS for tick, ask and
receipt. Baseline and after rows at 60,000 are below.

| metric | baseline p50/p95/p99 us | after p50/p95/p99 us | peak RSS KiB |
|---|---:|---:|---:|
| tick | 19.0 / 2,592.7 / 14,141.2 | 13.7 / 79.9 / 128.3 | 65,940 / 66,048 |
| ask | 105,914.4 / 380,481.7 / 654,577.5 | 219.3 / 309.1 / 925.5 | 228,000 / 235,408 |
| receipt | 780,757.2 / 2,045,731.7 / 3,289,614.4 | 477.8 / 720.9 / 28,194.3 | 301,540 / 311,108 |

Command: `PATH=/opt/homebrew/bin:/usr/bin:$PATH lgwks-box /usr/bin/time -v cargo nextest run --locked -p larry-runtime -E 'test(open_work_benchmark_prints_all_required_percentiles)' -- --nocapture`.

The seeded replay simulations compare trace digests and rebuilt projections.
The targeted benchmark and E30 journey are the user-path checks.
