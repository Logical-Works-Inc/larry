---
type: ADR
title: "ADR 0083: a tenant holds a share of every shared cap"
description: Why the run table, the pending set and the pending questions are divided among a settable tenant count, and a sealed terminal run leaves the run table.
resource: /docs/adr/0083-a-tenant-holds-a-share-of-every-shared-cap.md
tags: [larry, adr, runtime, multitenant, capacity]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: tests/sim_tenant_caps.rs's sim_a_tenant_at_its_share_refuses_and_another_tenant_still_admits_runs_and_asks, sim_a_sealed_terminal_run_leaves_the_table_and_frees_its_slot, sim_three_tenants_at_one_share_hold_the_global_question_bound, sim_three_tenants_at_one_share_hold_the_global_run_bound and sim_an_evicted_runs_owner_still_refuses_a_foreign_tenant red before the change and green after, with executor::gate::tests::task_settings_name_the_tenants_the_executor_serves, 2026-11-21}
status: accepted
stale_after: 2027-02-21
sources: [larry#333, larry#295, larry#321, /INVARIANTS.md]
---

# 0083: a tenant holds a share of every shared cap

Status: accepted, 2026-11-21 (larry#333, found by the 2026-11-19 review).

## Context

Three bounds were global, and each refused every tenant once one tenant held it:

- the run table refused at `run_capacity` and never let a terminal run go, so
  one tenant's live runs refused every other tenant's run;
- the pending set refused at `MAX_READY_WORK`, so one tenant's held commands
  refused every other tenant's admission;
- the pending questions refused at `max_pending_questions`, so one tenant's
  unanswered questions refused every other tenant's `Ask`.

`journal_capacity` and `run_capacity` also accepted `usize::MAX`, so a setting
could name a bound the runtime cannot hold.

## Decision

- `RuntimeSettings` gains `max_tenants`, the tenant count each shared cap is
  divided by. A tenant holds at most `capacity / max_tenants`, never less than
  one; a capacity below the tenant count gives every tenant one.
- The run table (`track_run`), the pending set (`admit`, `pump_ready`) and the
  pending questions (`check_ask`) each compare that tenant's own count with its
  share. Past it the tenant is refused `RunLimitReached`, `ReadyWorkFull` or
  `PendingFull` naming the share, and another tenant still admits. A queued
  command of a tenant at its share passes behind the others in the ready queue
  instead of holding its head.
- The global bound stays first, beside the share, in every cap: the run table
  refuses past `run_capacity` before any eviction, a command that would open a
  run takes its room before its acceptance is journalled, and the question
  book refuses past `max_pending_questions` for every tenant together as well
  as past one tenant's share. The share is a second bound, never a replacement.
- A terminal run whose receipt is sealed leaves the run table when its tenant
  needs the slot, the first in identity order, so the same journal frees the
  same rows on every replay. The projection keeps the run's view, so
  `run_state` still answers, a restore rebuilds the same table, and the
  projection's tenant is the owner an evicted run's foreign command is still
  refused against.
- `journal_capacity` and `run_capacity` refuse values past the stated ceilings
  of 1,048,576 and 65,536, and `max_tenants` values outside 1..=65,536, in
  `Runtime::new` and in `restore`, as `max_pending_questions` does.
- `max_tenants` defaults to one, so a host that names none keeps the old
  behaviour and a journal recorded before this change replays the same refusals
  at the same instant. The executor names it explicitly: one state directory is
  one tenant, and the shipped task path sets the count rather than inheriting a
  default.

## Consequences

- A host serving several tenants sets `max_tenants` to its tenant count; a
  lower count shares more tightly, a higher one gives every tenant less.
- The projection's memory bound is still `run_capacity` run views; a table of
  sealed runs the projection has not folded still refuses typed, and no slot is
  spent for a view that cannot exist.
- Journal rotation stays with larry#216; this change frees run slots, not
  journal entries.

## Rejected

- Eviction alone, with one global cap. One tenant can still hold every live run
  while the others are refused, which is the reported defect.
- The share alone, dropping the global bound. With the default of one tenant
  every tenant gets the whole cap, so N tenants hold N times it, and the first
  tenant's foreign command on a closed run is answered as closed.
- A tenant count derived from the tenants seen so far. It moves every tenant's
  share when a new tenant appears, and the same journal replays differently.
- A default tenant count above one. It breaks a single tenant's existing bound
  of 256 pending commands, and changes refusals a recorded journal already
  expects.
- Evicting sealed run views from the projection in this change. The entry must
  still answer the state the journal recorded, and its global memory bound is a
  separate decision.

Reversal: none expected. A host that wants the old whole-cap tenant sets
`max_tenants` to one, which is already the default.
