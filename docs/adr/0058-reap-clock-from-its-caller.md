---
type: ADR
title: "ADR 0058: the ledger reap takes its clock from its caller"
description: Why Ledger::reap reads no clock of its own, and how its saboteur simulation drives the same group wait on a virtual clock.
resource: /docs/adr/0058-reap-clock-from-its-caller.md
tags: [larry, adr, executor, tests, clock]
generated: {by: claude-code, at: 2026-11-17}
verified: {by: claude-code, method: larry-oracle test on the larry#308 branch, 2026-11-17}
status: accepted
stale_after: 2027-02-17
sources: [larry#308, larry#268, larry#214, larry#131, /docs/adr/0053-one-deadline-per-wait.md, POSIX kill(2) and wait(2)]
---

# 0058: the ledger reap takes its clock from its caller

Status: accepted, 2026-11-17 (larry#308)

## Context

`Ledger::reap` settled the process groups a killed executor left. For each row it
signalled the group, then waited up to `GROUP_PROBE_DEADLINE` (5 s) for the group to be
gone. It built its own live clock inside the call.

The saboteur simulation spawns its own `sleep 30` children. A killed child stays a zombie
until its parent calls `wait()`, and a group holding a zombie still exists to the kernel
probe. So the probe answered still-there for the whole 5 s deadline on every trace: 4
seeds, 2 replays each, 120 s of real time in one test on every nextest run and in the
pre-commit mutation gate.

## Decision

- `reap` takes `clock: &Clock` from its caller and reads no clock of its own. The run
  path (`prepare`, which every task and session start reaches) passes
  `larry_runtime::live_clock()`, the same clock the gate and `run_command` build.
- The row loop moved to a private `reap_with` whose group-gone probe the caller supplies.
  `reap` passes the one wait rule, `await_gone` in `run.rs`, over `process_group_exists`.
- The simulation passes a probe that advances a `Clock::virtual_at` one step before each
  probe. The 5 s deadline now passes in about twenty probes with no real time and the
  same seed still prints the same trace.

## Consequences

- The saboteur test ends in under 2 s instead of 120 s: the last test of every nextest
  run and of the mutation gate no longer sets the wall time.
- The deadline rule stays in one place. `await_gone` is visible to the executor, and
  `reap` and the simulation both read it.
- Every assertion and seed of the simulation is unchanged, and so is the trace format.
- No dependency, no crate and no row format moves.

## Rejected

- A live clock read inside `reap`, as before: correct for the product, un-injectable for
  the test, so the simulation pays the full deadline twice per seed.
- The simulation driving `await_gone` around `reap`: the ledger's own row loop and its
  clock use would go unexercised, and the test would prove a wait the run path does not take.
- A sleeping helper thread that advances the clock: a scheduler race in a test that must
  replay from its seed.
