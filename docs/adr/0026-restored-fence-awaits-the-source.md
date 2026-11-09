---
type: ADR
title: "ADR 0026: a restored fence awaits the authority source"
description: Why a runtime rebuilt from its journal counts its whole offline allowance spent until the host reaches the authority source, so a restart neither recharges offline authority nor revives what old settings still name.
resource: /docs/adr/0026-restored-fence-awaits-the-source.md
tags: [larry, adr, runtime, fence, e71, recovery]
generated: {by: claude-code, at: 2026-11-09}
verified: {by: claude-code, method: nextest run of fence::tests, tests/fence_recovery.rs and the workspace}
status: accepted
stale_after: 2027-02-09
sources: [larry#99, larry#22, specification R87, D48, E71, docs/adr/0024-fence-on-broker.md]
---

# 0026: a restored fence awaits the authority source

Status: accepted, 2026-11-09 (larry#99, the E71 port of PR #22)

## Context

The fence is one `lgwks_bot::broker` environment (ADR 0024). The broker is live and unjournaled. Its own
docs say a broker built after a restart has spent nothing. `restore` built the fence from
`settings.security_epoch` with nothing spent, which left two holes in E71 ("revoke during queue, dispatch,
partition, sleep and clock rollback; stale evidence never independently authorizes execution"):
- A host partitioned from its authority source could kill and restart the runtime to get a fresh offline
  allowance. A count cannot be recharged by sleep or a clock, but a restart did it.
- A host restoring from settings older than the source would dispatch at the old epoch. Withdrawals made by
  name before the crash were gone too, since the revoked set lives only in the broker.

## Decision

- `restore` of a journal that holds history builds the fence with `RevocationFence::restored`. An empty
  journal is a first start, with nothing to resume, and gets the fresh fence `new` gives. A restored fence's whole offline allowance counts as spent,
  and every mutating reach is refused with `OfflineExposureExhausted` naming the configured bound. Observation
  is never gated, so status queries still resolve what was sent.
- The first `authenticate` is the host reaching the source. It raises the fence to the epoch the source
  reports and takes up the configured bound with nothing spent. Withdrawals are re-delivered at that
  contact. A zero bound stays a kill switch.
- `Runtime::new` is unchanged: a fresh runtime has reached nothing, so it has spent nothing.

Rejected:
- PR #22's `AuthoritySource` trait consulted at every reach. It was written before ADR 0024, and it would
  be a second freshness owner beside the broker, with a new parameter on every constructor. The broker
  already refuses stale epochs, withdrawals and spent exposure, so the hole was only the restart.
- Deriving the spent count from the journal. `EventKind` has no dispatched fact (INVARIANTS), and an outcome
  does not say whether it came from a reach or a query, so a derived count could undercount.
- Journaling withdrawals. A withdrawal is a fact about authority, and the source owns it. A journaled copy
  would be stale evidence the next restart trusts.

## Consequences

- Every host that restores history must reach its authority source (`fence_mut().authenticate`) before any
  mutating reach. Executor recovery restores with an owner that reaches nothing and never authenticates, so
  it stays fenced as it should. The restore sims that dispatch after a restart (`sim_crash_restore`,
  `sim_e65_retry_after_restore_keeps_binding`) now authenticate first, as a host must.
- Covered by `fence::tests` (`e71_a_restored_fence_reaches_nothing_until_the_source_is_contacted`,
  `e71_a_restored_zero_bound_stays_shut_after_contact`) and `tests/fence_recovery.rs`
  (`e71_a_restart_recharges_no_offline_authority_and_old_settings_revive_nothing`).
