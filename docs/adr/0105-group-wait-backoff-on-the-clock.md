---
type: ADR
title: "ADR 0105: The group waits pause on a clock backoff"
description: Why the executor's group-gone wait and EPERM retry sleep a bounded backoff on the injected clock instead of spinning with yield_now, and why no kernel event replaces the probe.
resource: /docs/adr/0105-group-wait-backoff-on-the-clock.md
tags: [larry, adr, executor, process-group, clock, cpu]
generated: {by: claude-code, at: 2026-11-24}
verified: {by: claude-code, method: cargo nextest run --workspace --locked on toronto, 1315 of 1315 passed, 2026-11-24}
status: accepted
stale_after: 2027-05-24
sources: [larry#427, larry#268, larry#398, larry#131]
---

# 0105: The group waits pause on a clock backoff

Status: accepted, 2026-11-24 (larry#427). Extends ADR 0100 and the larry#268 clock rule.

## Context

- `await_gone` and `settle_with` looped on `std::thread::yield_now` until the group was gone or
  5 s passed. A group that takes its whole grace burned one core for 5 s. On a 1 to 2 vCPU host
  that core competes with the processes the wait wants to see exit.
- A process group has no kernel exit event. A pidfd (Linux) or `NOTE_EXIT` (kqueue) watches one
  pid. The group outlives its leader, and the ledger reaps groups that are not our children.
- The estate owns timed waits: `Clock::sleep` (lgwks_bot `rt::time`), driven by the process
  driver `rt::runtime::block_on`. lgwks_bot's own job settle uses the same poll-and-sleep shape.

## Decision

- On a wall clock, each pause is `Clock::sleep` for 100 us, doubled per probe up to 10 ms and
  clipped to the time left in the bound. A group that never goes away costs at most 520 probes
  over 5 s. A gone group is seen at most 10 ms late, and never later than its own age plus 100 us.
- On a virtual clock the thread yields. The sims advance the clock inside the probe, so a virtual
  sleep would wait for a move that never comes.
- The group kill and wait move to `executor::run::group`: settling a group is its own job.

Rejected: an event wait on pidfd or kqueue. It watches one pid, not a group. Rejected too: a
Condvar or `park_timeout` that nothing notifies. That is a hidden sleep beside the clock.

## Consequences

- The bound is still one reading of the injected clock (larry#268). The sims pass unchanged.
- Reverse this when lgwks_std ships a group-exit event (a Linux subreaper with `waitid` on the
  group), and measure the waiter's CPU before and after.
