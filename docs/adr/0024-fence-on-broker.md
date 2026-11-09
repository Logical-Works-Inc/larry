---
type: ADR
title: "ADR 0024: the dispatch fence on lgwks_bot broker"
description: How Larry's freshness fence moved onto one lgwks_bot broker environment, why the warrant is re-checked at the handoff and counted after the owner, and why the top epoch closes the fence.
resource: /docs/adr/0024-fence-on-broker.md
tags: [larry, adr, runtime, fence, broker, lgwks-bot, track-1]
generated: {by: claude-code, at: 2026-11-09}
verified: {by: claude-code, method: nextest run of fence, sim_fence, sim_epochs and the fence unit tests; larry exec before and after}
status: accepted
stale_after: 2027-02-09
sources: [larry#76, larry#68, logicalworks-crates#876, docs/adr/0017-effect-retry-on-lgwks-bot.md, docs/adr/0018-gate-on-lgwks-bot-grants.md]
---

# 0024: the dispatch fence on lgwks_bot broker

Status: accepted, 2026-11-09 (larry#76, row 3 of larry#68)

## Context

`RevocationFence` kept its own security epoch, revoked set and offline count, and its own refusal order.
lgwks_bot 8.1 `broker` owns all four for one environment (crates#876, closed): `authenticate` raises to a
reported generation and absorbs an older one, `revoke_operation` withdraws by name (at most 65,536), and
`set_offline_bound` with `spend` counts reaches between contacts. `authorize` mints a warrant, and
`revalidate` checks it again at the handoff. A second copy in Larry would drift from that owner.

## Decision

- The fence is one broker environment per runtime. The host surface stays (`revoke`, `advance_epoch`,
  `authenticate`, the readers), and each call is one broker call. Larry keeps no epoch, set or counter.
- Security epoch `n` is broker generation `n + 1`. `u64::MAX` has none: a runtime cannot start there,
  `advance_epoch` stops at `u64::MAX - 1`, and an authority source reporting `u64::MAX` closes the fence for
  good (`FenceRefusal::Closed`). It does not stand below what the source reported.
- `fence::effect_key` is the one key, used both for the warrant and for the retry rule (ADR 0017). Its
  environment is the fence's, and its tenant, run, strategy and binding are in the action digest. The old key
  saturated `admission_epoch + 1`, so epochs `u64::MAX - 1` and `u64::MAX` shared a generation. The key now
  refuses an epoch with no generation (`EffectIdentity { field: "admission_epoch" }`).
- `reach_owner` mints the warrant first, then checks the proof and the binding and syncs the journal. It
  `revalidate`s the warrant immediately before the owner, so a withdrawal since the mint still refuses.
  It `spend`s one unit once the owner returns an outcome. An owner's typed refusal guarantees nothing was
  attempted, so it spends nothing (E65).
- A journal restored under an epoch below the one that admitted it is refused as `NeverIssued`, not admitted.
- Revocations past 65,536 are `FenceError::RevocationsFull`. Advancing the epoch still fences the rest.

Rejected:
- `dist_lease`. It is the cross-host lease for distributed CI. Its epochs saturate, which is the aliasing
  this row removes, and its run state is a `TenantStore` beside the journal.
- `spend` before the owner, as the broker suggests. It would charge an owner's atomic refusal one unit,
  which E65 pins at none. The runtime holds `&mut self` from the handoff to the count, so nothing can move
  the fence between them.
- One environment per tenant. Offline exposure and the epoch are one authority's facts. Splitting them per
  tenant would change E71's bound, which is a specification change, not a port.

## Consequences

- `larry exec` on main 5b779db against this change, two rounds of 32 runs each (debug): p50 75.1 and 75.4 ms,
  p99 79.8 and 80.9 ms in the steady round (133.6 and 111.8 ms in the first, an outlier on both binaries),
  peak RSS 13.2 and 13.1 MiB.
- Row 3 of larry#68 is held by `fence::tests::larry_keeps_no_fence_rule_of_its_own`.
- Readers and `revoke` return `Result`, so a host sees `FenceError::Unowned` instead of a default.
