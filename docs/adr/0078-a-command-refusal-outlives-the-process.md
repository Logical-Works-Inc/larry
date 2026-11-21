---
type: ADR
title: "ADR 0078: a durably accepted command's refusal is a recorded fact"
description: Why a held Submit refused for a stale display epoch writes its own event, why a refused reduce costs one tick slot, and why a fold divergence stays per-item.
resource: /docs/adr/0078-a-command-refusal-outlives-the-process.md
tags: [larry, adr, runtime, multi-tenant, journal]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: larry#371 failing tests on main, then green with the recorded refusal, 2026-11-21}
status: accepted
stale_after: 2027-02-21
sources: [larry#371, /docs/adr/0073-per-item-tick-refusals.md, /INVARIANTS.md]
---

# 0078: a durably accepted command's refusal is a recorded fact

Status: accepted, 2026-11-21 (larry#371, child of #321).

## Context

ADR 0073 made one item's refusal a `TickReport` entry and the tick go on,
and rejected journaling a denial because "a refusal that changed no fact is
not a fact". It named its reversal: if a denial must outlive the process for
an auditor, it becomes a recorded event with its own `EventKind`.

The review of #363 found that reversal applies. A `Submit` is durably
accepted before it is reduced, so its `CommandAccepted` fact is on disk. If
its display epoch goes stale while the command waits in the pending set, the
reduce dispatches nothing and records the refusal only in the report. After
a restart the journal says the command was accepted and never says why it
never ran.

The same review found two smaller gaps. `promote_admitted` re-inserts a
command whose `record_command_accepted` failed at the index it left, and the
loop then re-reduced that same slot until the tick's budget was spent, so
the commands behind it never ran in that tick. And `FoldDivergence` was
classified per item; the review asked whether a split between the journal
and the projection must end the tick.

## Decision

- `EventKind::CommandRefused` is a recorded fact, tag 5, appended after the
  existing vocabulary. Its payload names the command, its run and a typed
  `CommandRefusal` reason. The first reason is `StaleDisplayEpoch`.
- The fact is written at the one stale-epoch re-check in `admit_work`, after
  the `CommandAccepted` fact and before the typed error returns. The journal
  is where a restart reads it; the report keeps the same refusal as a
  `TickItem` entry for the caller that is still running.
- `FoldProjection` folds the fact into its refusal list and changes no run
  state, because no state changed. `restore` therefore carries it.
- `promote_admitted` steps past a command whose acceptance record failed:
  the command stays pending at its place, one slot of the tick's budget is
  spent, and the next command is reduced in the same tick. A command refused
  after its acceptance is durable has left the pending set, so the loop does
  not step: the command that took its slot is the next one, and it is reduced.
- A command is named in `TickReport::admitted` only once its acceptance
  record is durable; one that failed is named as refused, never both.
- `FoldDivergence` stays one item's refusal, as ADR 0073 decided: it is
  neither the journal file's refusal nor a clock that cannot be placed. A
  test now drives the verdict, and the tick goes on for the other items.
- Tests: `sim_a_command_whose_accept_fails_is_reduced_once_and_the_tick_goes_on`,
  `sim_a_stale_display_refusal_survives_a_restart_as_a_journal_fact`,
  `a_fold_divergence_is_a_recorded_refusal_and_the_tick_goes_on`,
  `sim_a_refusal_after_acceptance_never_skips_the_next_pending_command`,
  `canonical_kind_tags_preserve_existing_command_and_event_digests`.

## Consequences

- A restart answers why an accepted command never ran, from the journal
  alone, with no live state.
- One failed acceptance costs one tick slot, not the whole tick.
- The existing vocabulary keeps its tags; a journal written before this
  change replays unchanged because no older record carries the new kind.
- The refusal list is bounded by the journal's own entry ceiling, and every
  refusal is written once per reduce attempt that reaches it.

## Rejected

- Recording the refusal as an `EffectOutcomeRecorded` or a `StateTransition`.
  No effect was reserved and no run state changed; either would claim a fact
  that did not happen.
- Reusing `CommandAccepted`. It would say the command was reduced when the
  refusal says it never ran.
- Ending the tick at the stale-epoch refusal. That is the defect ADR 0073
  fixed: every other tenant loses the tick.
- Keeping the refusal in the report only. The process that held it is gone
  after a restart, which is exactly the operator's question.

Reversal: if a refusal need not outlive the process, the fact returns to the
report alone; if a fold divergence is found to be global corruption rather
than a bad record, it moves to `ends_the_tick` in a new ADR.
