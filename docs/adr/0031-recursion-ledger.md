---
type: ADR
title: "ADR 0031: the recursion ledger"
description: Why the recursion budget identity R = 4k rollouts and P = 3k sessions is a pure reducer in larry-contracts that refuses the step that would break it, and what part 2 wires.
resource: /docs/adr/0031-recursion-ledger.md
tags: [larry, adr, contracts, recursion, epic-60]
generated: {by: claude-code, at: 2026-11-10}
verified: {by: claude-code, method: nextest run of crates/larry-contracts/tests/sim_recursion.rs and the workspace}
status: accepted
stale_after: 2027-02-10
sources: [larry#62, larry#60, docs/adr/0028-harness-as-a-content-hashed-value.md, docs/adr/0030-grader-blind-learning.md]
---

# 0031: the recursion ledger

Status: accepted, 2026-11-10 (larry#62 part 1, epic larry#60)

## Context

Harness² checks its budget identity (R = 4k rollouts, P = 3k sessions) once, at the end, with an
exception, and a crash mid-task loses the round. Larry runs recursion as journalled steps, so the
identity must be a property that holds at every step, and a restart must resume without a repeated probe.

## Decision

- Larry's reading. One round is two proposer sessions, two candidates, four rollouts (each candidate
  probed twice on the pinned task) and one composer session. Parallel mode runs k independent rounds
  from the base harness (2k candidates, k compositions). Sequential mode runs k nested rounds, and
  round i + 1 starts from round i's composition. Both total R = 4k and P = 3k, for k in 1 to 8.
- `RecursionLedger` in `larry-contracts::recursion` is a pure fold of `RecursionFact`s
  (`IntentRecorded`, `OutcomeRecorded`, `ProposalRefused`, `FellBack`, `Cancelled`). Live recording and
  `RecursionLedger::replay` use one rule set, so a replayed ledger equals the live one.
- An intent is refused, before it is counted, in this order: cancelled, duplicate label, rollout
  4k + 1, session 3k + 1, a step the plan lacks, an unsettled dependency, a full probe width. The
  idempotency key is task, mode, k and label (`RecursionPlan::step_key`).
- Dependencies: a probe waits for its candidate, a composer for its four probes, and a sequential
  round for the previous composer. A proposal or composition that fails falls back to the round base.
- `Unknown` is ambiguous. It is never counted as success, never rescheduled, it holds its probe slot
  and blocks its dependents until a reconciliation settles it.
- At most `probe_width` (1 to 16) probes are open. `next_steps` offers only the free width, so overflow
  waits and is never dropped. Cancel stops new steps; open ones settle.
- An invalid proposal gets one retry carrying the typed `HarnessError` from the #61 reducer, then a
  recorded `FellBack` to the round base. The retry is the same session asked again, so it counts once.
- The composer's input door is #63's `ImproverContext` (`compose_input`). No composition logic is here.

Rejected:
- A final assert on the totals: it is the Harness² weakness, and a crash loses the round.
- Counting a retry as a session: it would break P = 3k for the cases the identity must cover.
- Treating `Unknown` as failure and rescheduling: it could run a probe twice.

## Consequences

- Part 2 wires it: the facts journalled through the runtime fold, each probe an effect through the
  fence to the executor owner with a durable intent, the kill -9 test, and p50, p95, p99 and peak RSS.
  The facts need a canonical encoding there (`HarnessError` has no serde yet).
- Part 1 touches no runtime and dispatches nothing. #62 stays unticked until part 2.
- Covered by `tests/sim_recursion.rs`: the identity for every k and both modes, a 1,200-seed driver
  (same seed, same trace hash; replay equals live), width, duplicate, retry, ambiguity and a crash cut
  at every fact position. Measured: about 1,150 seed runs a second in a debug build.
