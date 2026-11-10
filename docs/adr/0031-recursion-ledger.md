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

## Part 2: the runtime fold (larry#62)

- A step is an ordinary `Submit` run: `WorkProposal` and `IntentPayload` gain an optional
  `RecursionStep` (plan, label, attempt, and for a probe the candidate hash its proposer settled on). No
  new `CommandKind` or `EventKind`; the field is absent from the encoding when empty.
- `RunFold` keeps a `RecursionLedger` per tenant and plan identity (`RecursionPlan::identity`), at
  most 16 plans per tenant. The intent folds to `record_intent`; a terminal receipt folds to
  `record_outcome` (succeeded, failed or compensated, or unknown-permanent, which stays ambiguous).
- Admission asks the same rule set (`vet_recursion` on a copy) before the command is written, and
  again when the intent folds, so duplicate StepKey, rollout 4k + 1, session 3k + 1, an unsettled
  dependency, a full probe width, a probe naming the wrong candidate and a plan past the bound are
  refused and never journalled. The tenant is the run table's, never the proposal's.
- Cancel: a run entering `Cancelling` stops its plan; when it reaches `Cancelled` its held slot frees.
  An unknown effect keeps the run cancelling and the slot held until it settles.
- The facts use the journal's existing JSON payload encoding (serde on the pure types), not a
  hand-written binary form; variant names are the stable tags. A decoded plan is re-checked by
  `RecursionPlan::validated`. The widest record of a full step is 4445 of 65524 bytes (MEASURED).
- The produced harness comes from the outcome, never the intent. `EffectOwner::produced` is asked once
  when a succeeded receipt is published, and the answer rides `ReceiptPayload.produced` (omitted when
  empty): `Produced::Harness(hash)`, or `Produced::Refused(HarnessError)` when the proposer's owner ran
  the #61 reducer on its edit, the only place that sees the edit. A refused proposal folds to
  `ProposalRefused`: one retry (attempt 1, a new run) carrying the error, then `FellBack` to the
  round base. A succeeded proposer or composer that reports no harness is recorded as a failed step
  and falls back; the fold cannot refuse a receipt, so Failed is the choice.
- Cancel with an unknown effect: the runtime already reconciles unknown effects, so the owner's status
  answer publishes the receipt, the run reaches Cancelled and the held slot frees (tested).

## Consequences

- Open (part 3): no production executor owner is wired, so steps run on the scripted owner double;
  peak RSS is not measured; the executor reading the active harness is #62's run-path work.
- Covered by `tests/sim_recursion.rs`: the identity for every k and both modes, a 1,200-seed driver
  (same seed, same trace hash; replay equals live), width, duplicate, retry, ambiguity and a crash cut
  at every fact position. Measured: about 1,150 seed runs a second in a debug build.
  Part 2: `tests/sim_recursion_runtime.rs` (restart at every step and from the real journal file at
  every stage, refusals before the write, width, cancel, plan bound, retry and fallback). One step with restore, admission and fold took p50 2.0 to 3.1 ms and p99 5.7 to
  6.9 ms across the three stages in a debug build (MEASURED).
