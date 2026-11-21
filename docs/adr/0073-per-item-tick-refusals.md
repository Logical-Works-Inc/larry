---
type: ADR
title: "ADR 0073: one item's refusal is recorded in the tick report, and the tick goes on"
description: Why every stage records a per-command, per-effect or per-run refusal as a typed TickItem entry and only journal and clock refusals end a tick.
resource: /docs/adr/0073-per-item-tick-refusals.md
tags: [larry, adr, runtime, multi-tenant, tick]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: larry#329 failing tests on main, then green with the classified refusals, 2026-11-21}
status: accepted
stale_after: 2027-02-21
sources: [larry#329, larry#332, /INVARIANTS.md, docs/adr/0072-held-display-cancel-and-crash-budgets.md]
---

# 0073: one item's refusal is recorded in the tick report, and the tick goes on

Status: accepted, 2026-11-21 (larry#329).

## Context

Every stage of the tick ended in `?`. A bound retry whose owner target changed,
a second tenant's command for a run another tenant has pending, a second
`Cancel`, a `Submit` on a non-draft run, and a held `Submit` whose display epoch
went stale each returned `Err` from `Runtime::tick`. One tenant's refusal
therefore skipped reconciliation and cancellation for every tenant in that
tick, and the caller saw only the first refusal.

## Decision

- `RuntimeError::ends_the_tick` classifies every variant once, with no wildcard
  arm: `Durable`, `JournalFull`, `DurableRepeat`, `DurabilityShort`,
  `ClockOutOfRange`, `ClockBeforeEpoch`, `BadSetting` and `SettingUnreadable`
  end a tick; every other variant is one item's refusal.
- `TickReport::refused` holds `(TickItem, RuntimeError)` in refusal order, where
  `TickItem` names the command, effect or run. The stages record one and go on.
- `admission` owns stage 2 (`pump_ready`, `admit_ready`, `admit_work`).
  `promote_admitted` leaves a refused pending command in the set;
  `pump_reconciliation` records a refused effect; `pump_cancellation` records a
  refused completion by run.
- The held `Submit` with a stale display epoch is such an entry, recorded once
  because its `CommandAccepted` fact is already durable. No new journal record
  kind exists; the journal carries only facts that changed.
- Tests: `a_per_item_refusal_is_a_typed_entry_and_the_tick_goes_on`,
  `a_foreign_command_is_a_typed_denial_and_reconciliation_goes_on`,
  `a_foreign_command_after_restore_is_a_typed_denial_and_the_tick_goes_on`,
  `a_second_cancel_is_a_typed_denial_and_completion_goes_on`,
  `a_submit_on_a_non_draft_run_does_not_stop_another_tenants_effect`,
  `sim_a_tenants_bad_command_never_stops_another_tenants_effect`, and
  `e65_a_held_submit_refuses_a_revocation_since_its_display`.

## Consequences

- One tenant's refused command or effect never stops another tenant's work in
  the same tick.
- A refusal is visible to the caller with its item identity, not only as the
  tick's error.
- A journal that refuses still ends the tick: the caller is told no further
  fact is durable.
- A refused pending command stays pending and is refused again on the next
  tick; it is never silently dropped.

## Rejected

- Returning the first refusal from `tick` and stopping. That is the defect:
  every tenant loses a tick.
- A second error type for the report. Reusing `RuntimeError` keeps one
  classification and one set of messages.
- Journaling a denial record. A refusal that changed no fact is not a fact; the
  report is where the caller reads it.

Reversal: if a denial must outlive the process for an auditor, it becomes a
recorded event with its own `EventKind`, which is a specification change.
