---
type: ADR
title: "ADR 0041: a reduce failure is never a silent duplicate"
description: Why a command whose acceptance record failed returns to the pending set, so a retry or a restore reduces it once, and why no failed-command set or new error variant was added.
resource: /docs/adr/0041-a-reduce-failure-is-never-a-silent-duplicate.md
tags: [larry, adr, runtime, admission, idempotency, recovery]
generated: {by: claude-code, at: 2026-11-14}
verified: {by: claude-code, method: two seeded simulations in runtime::admission::tests::reduce, red on main on their assertion and green after}
status: accepted
stale_after: 2027-05-14
sources: [larry#219, larry#209, docs/adr/0017-effect-retry-on-lgwks-bot.md, docs/adr/0023-journal-on-record-journal.md, docs/adr/0025-durable-admission.md, docs/adr/0036-dispatch-budget-is-a-fold.md]
---

# 0041: a reduce failure is never a silent duplicate

Status: accepted, 2026-11-14 (larry#219, after larry#209)

## Context

`pump_ready` journals a command with `accept_command`, holds or passes it, and then reduces it. Reduction
begins by recording the `CommandAccepted` fact. When that record failed (a journal with room for the command
and none after it) the tick returned the error and the command was gone from memory: popped from the ready
queue, or removed from the pending set. The journal still held it, with no `CommandAccepted`.

A retry of it was answered `DuplicateReplay` and listed in `TickReport::duplicates`, so the caller was told
"already recorded" about a command nothing would ever reduce in that process. A restore did reduce it, because
`recover` rebuilds the pending set from the journal, so the live process and the restored one gave different
answers about one journal. larry#209 closed the same hole for a refused sync with a latch. A reduce failure
without a sync failure had no equivalent.

## Decision

The journal already tells a reduced command from an unreduced one: `CommandAccepted` (ADR 0025). The pending
set is, by definition, the journal's commands without it. The tick broke that equality; this change restores
it.

- `Runtime::reduce` takes the command as an `Admitted` entry and the place it came from. If recording the
  acceptance fails, the entry goes back into the pending set at that place before the error returns. The next
  tick, or a restore, reduces it once.
- Both callers (`pump_ready` for the ready queue, `promote_admitted` for the pending set) pass their place. The
  tenant check in `promote_admitted` runs before the entry leaves the set, so its refusal also keeps it.
- A retry then meets one of three states. Reduced: `DuplicateReplay`, which is true. Pending: the tick reduces
  it first, and a protocol retry gets `DuplicateReplay` while the set holds it for its one reduction. Failing
  still: the same typed error (`JournalFull`), never a duplicate.
- Bound: the entry takes the slot it left, so the pending set stays within `MAX_READY_WORK` (256) and the
  journal within its ceiling. No marker or set is added.
- A failure after the acceptance record is the fold's own record. The journal says the command was accepted
  and the transitions it recorded stand; the error went to the tick's caller. Reducing it a second time would
  record `CommandAccepted` twice and refuse `Admit` from `Reconciling`.

Rejected:
- A typed refusal for a retry of a failed command (a new `RuntimeError` variant naming the run). It needs a
  failed-command set beside the journal that a restart forgets, so a restored process would answer the same
  retry differently, and the command would still never be reduced in this process.
- A scan of the journal on each `DuplicateReplay` to find an orphan. A duplicate is the hot path of a retry,
  and the scan is O(journal) for a state the fix now makes unreachable.

## Consequences

- A persistent cause (a full journal) returns the same typed error on every tick, because the pending command
  is retried first. It is the journal's backpressure; a restore with a larger ceiling clears it. Another
  tenant's append fails for the same cause, so nothing else is held that was not already.
- `report.duplicates` never names a command that is journaled and held nowhere.
- Covered by `runtime::admission::tests::sim_reduce` (`sim_a_reduce_failure_then_retry_is_never_a_silent_duplicate`,
  `sim_a_reduce_failure_survives_restore_and_reduces_once`,
  `sim_a_reduce_failure_interleaved_with_a_second_tenants_retries_never_crosses_tenants`), the first red on main
  on its assertion.
