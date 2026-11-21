---
type: ADR
title: "ADR 0067: a fenced effect left unknown spends the reconciliation budget"
description: Why the LeaveUnknown action counts one reconciliation instead of recording no outcome, so a never-sent effect stops growing the journal.
resource: /docs/adr/0067-fenced-unknown-spends-reconciliation-budget.md
tags: [larry, adr, runtime, reconciliation]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: sim_reconcile_a_fenced_unknown_records_at_most_the_attempt_budget on the pre-fix tree (10000 records) and the fixed tree (3)}
status: accepted
stale_after: 2027-02-21
sources: [larry#330, /crates/larry-runtime/src/runtime/reconcile.rs, /INVARIANTS.md]
---

# 0067: a fenced effect left unknown spends the reconciliation budget

Status: accepted, 2026-11-21 (larry#330).

## Context

`reconcile_one`'s `LeaveUnknown` branch (`RequiresHuman`, `AlwaysCompensate`) recorded one
`Unknown` outcome per pass and never incremented `LiveEffect::reconciliations`. A fenced
effect was never sent, so `is_reconcilable` kept it open under `!live.sent`, and every tick
appended another `Unknown` to the journal until `JournalFull`. The comment above the budget
check claimed the reconciliation budget bounded this; on this branch nothing moved the count.

## Decision

The `LeaveUnknown` branch spends one reconciliation on every pass it takes. The reconciliation
budget (`max_effect_attempts`, Cell 05: 3) is the one bound on how long an effect left open
keeps producing that record, and now it binds this branch too. After the budget is spent the
effect is left visibly unresolved, exactly as a refused retry is, and the journal stops growing.

## Consequences

- A fenced effect never sent grows the journal by at most `max_effect_attempts` entries, then
  stays open with no receipt and no run-state change.
- The effect still reports as open every tick after the budget is spent, but writes nothing.
- `tests/sim_reconcile.rs` (`sim_reconcile_a_fenced_unknown_records_at_most_the_attempt_budget`)
  ticks both `LeaveUnknown` strategies 10,000 times and holds the bound.

## Rejected

- Record no outcome for an effect never sent. The effect stays `Reserved`, and
  `is_reconcilable` returns true for a `Reserved` effect without spending any budget, so the
  effect would be re-evaluated every tick forever and the budget would never bind. That trades
  an unbounded journal for an unbounded open effect and makes the budget comment false.

Reversal: if an escalation path must distinguish a never-sent effect from one that ran out of
reconciliation budget, this ADR reopens with the field that carries that distinction.
