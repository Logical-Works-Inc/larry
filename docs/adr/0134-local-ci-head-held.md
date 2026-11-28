---
type: ADR
title: "ADR 0134: local CI writes no receipt when the head moved during the lanes"
description: "The head is read before the first lane and again after the last; a difference is refused."
resource: /docs/adr/0134-local-ci-head-held.md
tags: [larry, adr, local-ci, receipt]
generated: {by: claude-code, at: 2026-11-27}
verified: {by: claude-code, method: lgwks-box cargo nextest run, cargo mutants and llvm-cov on toronto, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#574, larry#53, INVARIANTS.md, tests/local_ci_journey.rs]
---

## Context

`cargo local-ci` says its receipt names the head it attests, so it cannot vouch for a later
commit. It read the head only after the last lane. A commit made while the suite ran, by a
person or a second session, got a receipt for code no lane had tested (CWE-367).

## Decision

Read `HEAD` before the first lane and again after the last. When they differ, write no receipt
and exit 1 with `head moved from <a> to <b> while the lanes ran`. A head git cannot name
before the lanes is refused after them, so every lane still runs.

The dead per-lane `env` field is removed: no lane set it.

## Rejected

Attest the head read before the lanes. A later lane may have built the newer commit, so the
receipt would vouch for a mix that no one tested.

## Consequences

One more `git rev-parse` per run. A head that moves and moves back between the two reads is not
seen. An uncommitted edit made during the run is not seen either; the receipt names a commit,
not a working tree.

Reverse this when the lanes run in a checkout of the head that no other writer can move.

## Proof

- `sim_a_head_that_moves_while_a_lane_runs_writes_no_receipt` commits inside each of the ten
  lanes. Before the change the run exited 0.
- `a_head_unknown_before_the_lanes_writes_no_receipt` proves an unknown first head fails closed.

## Traceability

Part of the 95% coverage order (Director 2026-11-27), larry#574.
