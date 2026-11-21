---
type: ADR
title: "ADR 0072: a held display re-checks its epoch, cancellation stops a retry, a restart spends the recorded budgets"
description: Why a held Submit is checked again at reduce, a Cancelling run makes no mutating reach, and restore charges the dispatch and reconciliation budgets the journal recorded.
resource: /docs/adr/0072-held-display-cancel-and-crash-budgets.md
tags: [larry, adr, runtime, dispatch, cancellation, recovery]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: larry#332 failing tests on main, then green with the three rules, 2026-11-21}
status: accepted
stale_after: 2027-02-21
sources: [larry#332, /INVARIANTS.md, docs/adr/0036-dispatch-budget-is-a-fold.md, docs/adr/0067-fenced-unknown-spends-reconciliation-budget.md]
---

# 0072: a held display re-checks its epoch, cancellation stops a retry, a restart spends the recorded budgets

Status: accepted, 2026-11-21 (larry#332).

## Context

Three runtime rules failed on main.

`validate_display` compares the display's epoch with the fence, but only at
admission. A `Submit` held behind a correction or a question is reduced later,
and `admit_work` stamps its intent with the epoch the fence stands at then. A
display approved at epoch N was dispatched after a revocation to N+1.

`reconcile_one`'s `RetryBounded` arm reached the owner with no run-state check,
so a run the person put in `Cancelling` kept making mutating reaches.

`reach_owner` increments `dispatches` before the owner call, and the count is
journaled only with the outcome. A crash inside that call restores one attempt
short, so every crash bought an extra attempt past `max_effect_attempts`.
`reconciliations` was not journaled at all, so each restart bought a fresh
budget of `Unknown` records for an effect left open.

## Decision

- `admit_work` re-checks the display's epoch against `self.fence.epoch()` and
  refuses `BindingError::Changed { field: SecurityEpoch }` before any run-state
  move or reservation.
- `reconcile_one` makes no `RetryBounded` reach for a `Cancelling` run. The
  refusal records one `Unknown` and spends `dispatches` to `max_attempts`, as
  the estate rule's own refusal does.
- `restore` adds one to the recorded dispatch count of an observed, unresolved
  `IdempotentRetrySafe` effect, capped at `max_effect_attempts`, and sets
  `reconciliations` from the journal's recorded `Unknown` outcomes for that
  operation.
- Tests: `e65_a_held_submit_refuses_a_revocation_since_its_display`,
  `sim_cancel_a_retry_safe_effect_never_retries_while_cancelling` and
  `sim_a_crash_at_every_point_never_spends_past_the_budget`.

## Consequences

- A held command shown before a revocation dispatches nothing and returns the
  typed binding refusal.
- A cancelled run's retry-safe effect stays visibly unresolved, and its hold is
  bounded by the retry budget.
- Across any number of crashes, the owner sees at most `max_effect_attempts`
  reaches and the journal holds at most that many `Unknown` records per effect.
- Recovery stays a fold over the journal; no record kind is added.

## Rejected

- Checking the epoch at the reach. The intent already recorded the later epoch,
  so the journal would claim admission under a generation the display never
  approved.
- Leaving cancellation to the fence. The fence answers authority, not the stop
  control; a retry-safe effect allowed while `Cancelling` is the defect.
- Journaling a `Dispatched` fact. The closed `EventKind` vocabulary has none
  (ADR 0036), and the fail-closed restore direction is the established rule.

Reversal: if the journal gains a synced dispatch fact written before the owner
call, that fact replaces the +1 rule.
