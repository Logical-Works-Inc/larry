---
type: ADR
title: "ADR 0053: the group wait ends on one deadline from an injected clock"
description: Why the executor's process-group waits read one deadline from an lgwks_bot clock instead of counting yields, and why journeys wait on the event itself with one failure deadline.
resource: /docs/adr/0053-group-wait-deadline.md
tags: [larry, adr, executor, process-groups, clock, tests]
generated: {by: claude-code, at: 2026-11-16}
verified: {by: claude-code, method: the diff read, and larry-oracle all on this branch, 2026-11-16}
status: accepted
stale_after: 2027-02-16
sources: [larry#268, larry#214, larry#131, docs/adr/0042-orphan-groups-and-bounded-probes.md, docs/adr/0016-tick-clock-on-lgwks-bot.md, docs/adr/0052-hermetic-test-host.md]
---

# 0053: the group wait ends on one deadline from an injected clock

Status: accepted, 2026-11-16 (larry#268)

## Context

ADR 0042 bounded the process-group wait by elapsed time and put a second cap of 10,000 probes behind
it. The cap was the defect. A count of yields measures the scheduler, not the wait: on an idle host the
10,000 probes ran in a fraction of the 5 s, so `ExecError::GroupSurvived` could be decided early, and on
a loaded host the count outlasted the time it was meant to back. The same shape ran through the tests:
`POLL_LIMIT` (2,000,000 yields) waited for a ledger row and for a group's exit, `WITNESS_PROBES` waited
for a cancel witness, and three journeys gated a wall-clock number, so a loaded host decided a pass.

## Decision

- One deadline from one clock. `settle_group` and `await_group_gone` take `&lgwks_bot::clock::Clock` and
  end the wait when one reading of it is `GROUP_PROBE_DEADLINE` past the reading taken at the start
  (`past_deadline`, src/executor/run.rs). `GROUP_PROBES` is deleted, so nothing counts yields.
- The production clock is the runtime's. `run_command` and `Ledger::reap` build it with
  `larry_runtime::live_clock`, the same constructor the gate uses, so the wait and the gate's lease read
  one monotonic timeline placed at the epoch (ADR 0016).
- The sim drives it. `executor::sim_group` places `Clock::virtual_at` at zero, advances it one drawn step
  per probe, and asserts the wait ends inside the bound when the group goes in time and at the first
  reading past the bound when it does not, for every seed. A virtual clock moves only when the sim moves
  it, so a wait decided by a yield count is visible as a wrong trace, not as a lucky pass.
- Journeys wait on the event, with one failure deadline. `tests/support/deadline.rs` holds `PATIENCE`
  (30 s), `await_until` and `await_fifo_writer`: the write end of a fifo opens only once the command
  opens the read end, so it is the event that says a command started, and the ledger row is the event
  that says a group is recorded. A wait that never gets its event fails with its own name and bound.
- Latency is printed, never gated. `esc_to_interrupted_ms`, the idle window's `idle_output_bytes` and
  the probe figures are MEASURED and printed; the deterministic sims, not a wall-clock bound, prove the
  behaviour they accompany.

## Supersedes

The "Bounded probes" bullet of ADR 0042. Its other decisions (the row kept until the group is gone, the
bounded ledger lock, origin liveness by lock) stand unchanged.

## Consequences

- A probe still spins between checks, because no portable waitable handle exists for another process's
  group. The bound is honest; the wake is a yield. Linux `pidfd` and Darwin `EVFILT_PROC` would wait on
  an event, but neither covers a group, and `unsafe` FFI is forbidden workspace-wide.
- A wait that fails now costs its failure deadline (30 s in a journey, 120 s for one worker event)
  instead of an arbitrary number of yields. `esc_to_interrupted_ms` no longer fails a journey.
- The during-schedule cancel witness in `executor::sim_session` stops when `run_command` returns, so its
  wait is bounded by the command's own timeout and the sim reads no clock at all.

## Rejected

- A clock built inside the wait: nothing could be injected, so the bound could not be driven to its end
  on a virtual clock, and no seeded sim could prove it.
- A count of probes behind the deadline, as ADR 0042 had it: the count decides on a loaded host.
- A wall-clock latency gate in a journey: a busy host moves it, so it fails on load rather than on a
  defect.
