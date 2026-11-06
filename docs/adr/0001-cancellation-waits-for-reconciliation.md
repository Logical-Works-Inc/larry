---
type: ADR
title: Cancellation waits for reconciliation
description: A run reaches Cancelled only once every owned effect is reconciled or was never sent.
resource: /docs/adr/0001-cancellation-waits-for-reconciliation.md
tags: [larry, runtime, cancellation, adr]
generated: {by: claude-code, at: 2026-11-06}
verified: {by: claude-code, method: cargo nextest run --workspace --locked, 544 passed}
status: accepted
stale_after: 2027-03-21
sources: [larry#119, INVARIANTS.md, specification.md E30 R72]
---

# 0001 Cancellation waits for reconciliation

## Status

Accepted 2026-11-06 (larry#119).

## Context

`pump_cancellation` held a run only while an owned effect was `Reserved`. A `Queryable` effect whose
dispatch reported `Unknown` is `Observed` with no receipt, so the run became `Cancelled` while an external
effect that may have happened stayed unlabelled. Three cancel sims asserted that behaviour.

## Decision

A run stays `Cancelling` while any owned effect is `Reserved` or `Observed` without a `Reconciled`
disposition. Reconciliation keeps querying within `MAX_EFFECT_ATTEMPTS`; a resolved effect keeps its own
receipt and cancellation then completes. An owner that never answers leaves the run visibly `Cancelling`,
and a spent hold is an idle tick that records nothing. The hold is a fold over the journal, so it survives
restore.

## Consequences

- `Cancelled` now means no owned effect is open; operators read an unresolved hold as `Cancelling`.
- No disposition is invented at cancel time: `UnknownPermanent` stays the strategy's answer, not cancel's.
- Escalating a spent hold to a human is a later work package; this decision does not paper over it.
- Enforced by `sim_cancel_unresolved_observed_effect_holds_cancellation_open` and
  `sim_cancel_seeded_delays_hold_unresolved_open` in `crates/rocco-runtime/tests/sim_cancel.rs`.
