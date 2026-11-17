---
type: ADR
title: "ADR 0056: every runtime bound is a validated setting, and restore rebuilds at the run capacity"
description: Why the runtime's capacities, payload ceiling, skew, attempt bound, protocol range and lease are RuntimeSettings fields, why a bad value is refused rather than clamped, and why the executor keeps no lease, revision or epoch constant.
resource: /docs/adr/0056-runtime-settings-bounds.md
tags: [larry, adr, runtime, settings, restore]
generated: {by: claude-code, at: 2026-11-17}
verified: {by: claude-code, method: the diff read, and larry-oracle all on this branch, 2026-11-17}
status: accepted
stale_after: 2027-02-16
sources: [larry#212, /INVARIANTS.md, crates/larry-runtime/src/runtime/settings.rs]
---

# 0056: every runtime bound is a validated setting, and restore rebuilds at the run capacity

Status: accepted, 2026-11-17 (larry#212)

## Context

`RuntimeSettings` held the pinned snapshots, the journal and run capacities and the per-tick
budget, and every other bound was a constant: the pending-question ceiling in `runtime::approval`,
the 64-grant scan in `runtime::display`, the payload size in `runtime::display`, the skew in
`replay::decide` (fixed at zero), the dispatch-attempt bound in `runtime::reconcile`, the protocol
range in `protocol::negotiate`, and the executor's 60 s `GATE_LEASE`, its `POLICY_REVISION = 1` and
the `SecurityEpoch::INITIAL` it resolved. `restore` then rebuilt the projection with
`P::default()`, whose run table is 1,024 runs. A host that set `run_capacity` to 4,096 wrote a
journal its own default projection could not replay, and the failure surfaced only at the next
start, on the data that had to survive. No value was validated, so a zero capacity was clamped by
whichever check met it first.

## Decision

- Every bound is a field of `RuntimeSettings`: `journal_capacity`, `run_capacity`,
  `max_runs_per_tick`, `max_pending_questions`, `max_grants`, `max_payload_bytes`, `skew_bound`,
  `max_effect_attempts`, `min_protocol_version`, `max_protocol_version` and `lease`. The defaults
  are the Cell 05 values the crate shipped with, so a host that names none gets the old behaviour.
- `Runtime::new` and `restore` call one `validate` before they build anything. A zero or
  out-of-range value is `RuntimeError::BadSetting { name, value, min, max }`, naming the field and
  its range. Nothing is clamped: a host that sets zero is told, not quietly repaired.
- `restore` builds the projection with `P::with_capacity(settings.run_capacity)`, so the view's
  ceiling is the run table's and a journal this runtime wrote is one a restore can always reopen.
- `admit` checks `command.content().payload().len()` against `max_payload_bytes` before the payload
  is decoded, before it is journalled and before any queue or question book moves, and refuses with
  `RuntimeError::PayloadTooLarge { bytes, ceiling }`.
- The skew rides the decision, not the runtime: `Runtime::new` pins `skew_bound` into the
  `ValiditySnapshot` every decision carries, so `replay::decide` judges a lease with the configured
  allowance and a replay re-derives the identical decision.
- The executor keeps no copy of a runtime rule. `policy_revision` is a BLAKE3 digest over the policy
  record's sorted scope names, `with_expiry` sets the validity from `settings.lease`, `TaskOwner`
  resolves the `settings.security_epoch` the fence was built with, and `drive` offers
  `runtime.settings().protocol_range()`.

## Consequences

- A host sets one record and gets exactly what it set, in the run that built it and in every
  restore. The 2,000-run simulation fills 2,000 runs, drops the process and restores all of them on
  a 4,096 ceiling.
- A changed `LARRY_POLICY` yields a different revision, so a decision pinned under one policy is
  never comparable under another; the same policy always yields the same revision.
- `max_grants` may only lower the 64-grant scan, `max_payload_bytes` may only lower the
  `MAX_BOUND_FIELD_BYTES` the binding hashes, and `skew_bound` and `lease` have declared maxima, so
  a setting cannot widen a work bound the specification fixed.
- A bad value is refused at construction; it can no longer reach a tick.

## Rejected

- Clamping a bad value into range and keeping the constants: the host is not told its setting was
  ignored, and two sources for one bound is the drift this issue found.
- A durable policy-revision counter: it needs a store the executor does not have for one number, and
  a content digest gives the same guarantee from the record alone.
- Validating `run_capacity <= journal_capacity`: the journal holds several entries per run, so the
  relation the host needs is not a field comparison, and a test that sets a small journal with the
  default run capacity is legal.
