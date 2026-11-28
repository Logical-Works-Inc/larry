---
type: ADR
title: "ADR 0141: an abandoned leader is killed first"
description: "A command the executor cannot track is killed itself before its group is settled and reaped."
resource: /docs/adr/0141-an-abandoned-leader-is-killed-first.md
tags: [larry, adr, executor, processes]
generated: {by: claude, at: 2026-11-27}
verified: {by: claude, method: lgwks-box cargo nextest run --locked on toronto, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#590, larry#442, INVARIANTS.md, src/executor/run.rs]
---

## Context

Number 0141: main holds 0139 and the open pull request for larry#592 holds 0140.

A command whose group the executor cannot identify or record is dropped. The drop
signalled the group and then waited for the leader. Where the `setsid` wrapper makes
the group after `spawn` returns, an early signal finds no group (`ESRCH`), which reads
as settled. The wait then lasted as long as the command, with no timeout and no cancel.

## Decision

The drop kills the leader itself first, then settles the group, then reaps. A dead
leader forks no more, and its unreaped pid cannot be reused, so the group signal
that follows still names this command's group and no other. A leader that refused the
kill is never waited on; that failure is reported with the first refusal.

## Rejected

Repeating the group signal until the status arrives, as the cancel path does
(larry#442): the drop path has no waiter thread to watch, and one direct kill of
an unreaped child is exact.

## Consequences

A drop now ends in the time of one kill and one wait. A member the leader forked
before it died, and before its group existed, is not covered: no group named it.
Reverse this if the group is created before `spawn` returns on every host.

## Proof

`sim_an_abandoned_child_dies_before_its_group_exists` drops a child that leads no
group. Before the change it waited 30.0 s for the child to exit by itself and failed.
`sim_a_group_the_ledger_cannot_record_is_killed` refuses the ledger write and finds
the group gone.

## Traceability

Issue #590. The invariant line is in INVARIANTS.md.
