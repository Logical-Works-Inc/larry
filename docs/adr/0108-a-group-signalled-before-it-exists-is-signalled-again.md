---
type: ADR
title: "ADR 0108: A kill that lands before the command's group exists is repeated until the leader's status arrives"
description: Why a cancel or deadline kill is repeated each slice until the leader reports, instead of settling on the first answer that the group is gone.
resource: /docs/adr/0108-a-group-signalled-before-it-exists-is-signalled-again.md
tags: [larry, adr, executor, process-group, cancel, determinism]
generated: {by: claude-code, at: 2026-11-24}
verified: {by: claude-code, method: cargo nextest run --workspace --locked on toronto, all tests passed, 2026-11-24}
status: accepted
stale_after: 2027-05-24
sources: [larry#442, larry#242, larry#268, larry#334, larry#427]
---

# 0108: A kill that lands before the command's group exists is repeated until the leader's status arrives

Status: accepted, 2026-11-24 (larry#442). Extends ADR 0105 and the larry#242 cancel.

## Context

- On Linux a command starts under util-linux `setsid`, which makes the process group with
  `setsid(2)` after `spawn` has returned. A cancel or a deadline that signals the group in
  that window finds no group: `ESRCH`, which `settle_with` reads as "gone".
- The wait then did `receiver.recv()` with nothing left to end the leader, so the run lasted
  as long as the command: `sim_esc_before_during_after_command` took 30.8 s and saw exit
  `Some(0)` where a killed command was expected, on seeds 2, 18, 22 and 26 under load.
- The issue proposed holding the test's "during" command on a fifo. That hides a product
  race: an Esc in the first milliseconds of a real command waited out the whole command.

## Decision

- `group::status_after_kill` takes the leader's status after the first signal. Each 25 ms
  slice with no status it signals the group again, and past `GROUP_PROBE_DEADLINE` on the
  injected clock with the leader still running it is `ExecError::GroupSurvived`. A waiter
  that ended without a status is reported as its fault and nothing is signalled.
- Both kill paths of `wait_for_status` (cancel and deadline) take their status through it.

Rejected: holding the command on a fifo in the test. It leaves the product race in place.
Also rejected: a longer first poll, which only moves the race.

## Consequences

- Two sims prove the wait on a virtual clock: 64 seeds of a group that appears after 0 to 99
  signals, each landing the kill or giving up at the first reading past the bound.
- A leader that survives SIGKILL for 5 s now fails the run typed, where it waited forever.
