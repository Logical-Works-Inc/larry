---
type: ADR
title: "ADR 0009: larry eval scores one arm over one matched task set"
description: How the Claude-alone and Claude-plus-Flash arms are run, priced and compared on one set.
resource: /docs/adr/0009-matched-task-set-eval.md
tags: [larry, adr, eval, cost, benchmark]
generated: {by: claude-code, at: 2026-11-07}
verified: {by: claude-code, method: cargo nextest run --workspace --locked; tests/eval_journey.rs against the mock model}
status: accepted
stale_after: 2027-02-07
sources: [larry#87, larry#95, ADR 0008, SWE-bench, Artificial Analysis coding-agent method]
---

# 0009: larry eval scores one arm over one matched task set

Status: accepted, 2026-11-07 (larry#87)

## Context

Track 3 compares Claude alone with Claude plus Flash on one task set: cost per accepted task, acceptance
rate, p50/p95/p99 per task and peak RSS for both arms. It is developed against the mock endpoint; the scored
runs spend model money and are #95.

## Decision

- One door: `larry eval` reads a set (`name`, up to 256 `tasks` of `id`, origin `files`, `goal`, `write`,
  `acceptance`) on stdin and runs each task through `executor::execute`, as `larry_submit_task` does
  (ADR 0008). Each origin is written under `<state>/eval/origin-<pid>-<n>` and committed with background
  maintenance off (larry#138). Rejected: a harness of its own, which would score a second code path.
- An arm is the environment: `LARRY_MODEL_*` pins the model, `LARRY_EVAL_PRICE_IN` and `_OUT` give dollars
  per million tokens. The comparison is two runs over one set; each card names its set and model.
- Money is whole numbers: prices parse to micro-dollars (at most 6 decimals, at most 10,000 dollars), costs
  are picodollars in `u128`, printed exactly. Rejected: `f64`, which rounds a cost differently per target.
- Tokens are the gateway's `usage`, recorded per authored file in the sealed report. Cost per accepted task
  is all spend over accepted tasks, rejected runs included. A refused task's cost is unknown, not zero, so
  the card prints `cost_complete: false`.
- A set, a price or a missing key is checked before the first completion and refused with exit 1, so a
  mistake spends nothing. A task needs 1 to 16 acceptance commands, since a task no command checks would
  count as accepted. Only a gateway refusal is the arm's, and it is scored; a git, ledger or write failure
  stops the run.
- Latency is nearest-rank p50/p95/p99 of each task's wall time. Peak RSS is measured outside the process,
  under `/usr/bin/time`, because `lgwks_std` has no RSS primitive.

## Consequences

- `eval/matched-v1.json` ships 12 POSIX sh tasks (13 files): FizzBuzz, a bug fix, text filters, small
  maths, a default port and a two-file task. Each is checked by `cmp` or `test`.
- A kill -9 leaves its origin behind; the next run's sweep removes every origin whose pid is gone.
- #95 runs both arms on real models with the Director's eval key and publishes the two cards.
