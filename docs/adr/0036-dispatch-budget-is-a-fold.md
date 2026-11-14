---
type: ADR
title: "ADR 0036: the dispatch budget is a fold over the journal"
description: Why the dispatch count an effect restores with is read from the outcome record it already writes, and why no in-memory state moves before the journal's sync returns.
resource: /docs/adr/0036-dispatch-budget-is-a-fold.md
tags: [larry, adr, runtime, effect, retry, idempotency, durability]
generated: {by: claude-code, at: 2026-11-14}
verified: {by: claude-code, method: nextest run of the two falsifiers red on main then green, and the whole runtime suite}
status: accepted
stale_after: 2027-05-14
sources: [larry#182, larry#195, docs/adr/0017-effect-retry-on-lgwks-bot.md, docs/adr/0023-journal-on-record-journal.md, docs/adr/0025-durable-admission.md, docs.stripe.com/api/idempotent_requests]
---

# 0036: the dispatch budget is a fold over the journal

Status: accepted, 2026-11-14 (larry#182 defects 2 and 7, larry#195)

## Context

`restore` set an effect's dispatch count by phase: `Reserved` restores with the
whole budget spent, and `Observed` and `Reconciled` restored with zero. An
`IdempotentRetrySafe` effect that used all its dispatches and recorded `Unknown`
reads as `Observed`, so it restored with the full budget again on every restart,
and its attempt numbers restarted at 1: one `EffectKey` named two physical
dispatches. The journal already records an outcome for every reach that returned,
so the count it implies was there to read and was thrown away.

`Runtime::admit` recorded acceptance, folded the question book and pushed the
pending command before it synced. A caller whose sync refused still had its
command queued for the next tick, so a command the caller was told had failed to
become durable was executed anyway.

## Decision

- `OutcomePayload` carries `dispatches`: how many times the owner had been asked
  to perform the effect when it reported the outcome. `restore` reads it through
  `ProjectionBackend::dispatches`, so the budget is a fold over recorded facts,
  never live state a restart resets.
- A record with no `dispatches` (one written before this change) restores as the
  whole budget spent, and so does an effect the journal leaves `Reserved` or
  never mentions. The journal cannot prove a lower count, so the fail-closed
  answer is that every attempt may have been used.
- The attempt number is not a new event kind. It rides the outcome record the
  dispatch already writes, so the fold stays deterministic, one write per reach,
  and the journal's closed event vocabulary is unchanged.
- `Runtime::admit` syncs before it touches memory: the pending queue and the
  question book are filled only after the sync returns `Ok`, so a sync refusal
  leaves neither moved and the entry on disk stays for a restore to admit once.
- The executor's `drive` names a tick that fails the sync with `ExecError::Unsealed`
  instead of dropping the sealed report, and keeps that journal for recovery.

Rejected:
- A seventh `EventKind` for each dispatch. `larry-contracts` owns the closed
  five-variant vocabulary and this crate may not extend it, and a second write
  per reach is cost for a fact the outcome record can carry.
- A side file of live dispatch counts. It is a second record beside the durable
  journal, which D04 forbids, and a torn tail can leave it disagreeing with the
  journal.

## Consequences

- A retry-safe effect spends its `MAX_EFFECT_ATTEMPTS` dispatches once, across
  any number of restarts, and every attempt names one `EffectKey`.
- Larry#209: a refused sync latches the journal. The lgwks_bot store latches its
  own handle on any failed append (`owner/store.rs:856-867`), so a later sync
  never writes the unwritten bytes. Larry does not retry them in process: a
  retry is refused with `RuntimeError::Durable` until a restore replays the file.
- An acknowledged command is in memory only after its entry is durable, so a
  caller that got `Err` from the sync never has the command run for it.
- Covered by `tests/sim_budget_restore.rs` (`sim_a_restart_spends_the_recorded_dispatch_budget`)
  and `runtime::admission::tests` (`sim_a_sync_refusal_admits_nothing_and_a_restore_admits_once`),
  both red on `main` and green after.
- The `Journal::lost_acknowledgement` fence is test-only, compiled under `cfg(test)`
  alone, so no shipped build carries it.
