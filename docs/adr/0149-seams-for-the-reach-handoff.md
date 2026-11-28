---
type: ADR
title: "ADR 0149: the reach handoff and its offline count are private methods"
description: "A split of reach_owner lets a sim withdraw authority between the real mint and the real handoff."
resource: /docs/adr/0149-seams-for-the-reach-handoff.md
tags: [larry, adr, runtime, fence, tests]
generated: {by: claude-code, at: 2026-11-28}
verified: {by: claude-code, method: lgwks-box cargo nextest run, 2026-11-28}
status: accepted
stale_after: 2027-05-28
sources: [larry#612, larry#76, larry#197, ADR 0024, INVARIANTS.md]
---

## Status

Accepted. Number 0149: `main` took 0148 (larry#620) while this change was open.

## Context

The fence re-checks a warrant right before the owner, and a refused count is
reported as `Unspent`. The runtime holds the fence exclusively through one
reach, so no public call can land between those steps. No test ran either
refusal (CWE-367, CWE-613).

## Decision

`reach_owner` keeps the mint, the proof read and the group commit. The private
`hand_off` re-checks the warrant and calls the owner. The private `count_reach`
spends one offline unit. Their behaviour is unchanged. A sim mints, withdraws,
then calls them.

## Consequences

A `cfg(test)` hook field on `Runtime` was rejected: it adds a second path that
the shipped binary never runs. Both methods are visible to the runtime module
only. Reverse this if the broker gains a public way to withdraw during a
reach: the sims then move to the public interface and the split can go.
