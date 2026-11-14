---
type: ADR
title: "ADR 0042: orphan groups keep their row until gone, and every wait is bounded"
description: Why the executor keeps a group's ledger row until the group is gone, why a row cannot yet close the spawn-to-insert window, and why the ledger lock, group probes and origin liveness are bounded or lock-based.
resource: /docs/adr/0042-orphan-groups-and-bounded-probes.md
tags: [larry, adr, executor, process-groups, ledger, lock]
generated: {by: claude-code, at: 2026-11-14}
verified: {by: claude-code, method: tests/executor_journey.rs, src/executor/ledger.rs, src/executor/workspace.rs and src/executor/sim_ledger.rs run on Oracle, 2026-11-14}
status: accepted
stale_after: 2027-02-14
sources: [larry#214, larry#85, larry#131, larry#182, larry#87, docs/adr/0004-executor-process-groups-and-ledger.md, docs/adr/0037-run-identity-and-idempotency-keys.md]
---

# 0042: orphan groups keep their row until gone, and every wait is bounded

Status: accepted, 2026-11-14 (larry#214)

## Context

A `larry exec` command runs in its own process group, and its ledger row names that group. The reap at
the next start signals the group and removes the row. Five defects in that path, found in review:

1. The row is written after `spawn`, so a kill -9 between them leaves a group no row names.
2. The reap dropped the reap's outcome and deleted the row either way, though INVARIANTS says the row goes
   only once the group is gone.
3. The ledger lock was an unbounded `flock`, while its comment claimed a 60 s bound.
4. The group probes counted `yield_now()` calls, not time.
5. An eval origin's liveness read the pid in its name, which INVARIANTS line 41 had retired for runs.

## Decision

- Row kept until gone. The reap signals the group, then waits for it with the bounded probe. A group
  still present after the wait keeps its row, and the next start tries again. A row goes only once
  `process_group_exists` is false.
- Bounded probes. A group is probed until 5 s have elapsed or 10,000 probes have run, whichever is first.
  The probe count is the second cap; the elapsed time is the bound.
- Bounded ledger lock. `Ledger::open` tries the lock until 60 s on a monotonic clock, then refuses with
  `ExecError::LedgerLockTimeout { waited }`. A test holds the lock and sets a 30 ms bound.
- Origin liveness by lock. Each `larry eval` origin holds an exclusive lock on the sibling file
  `eval/<origin>.lock` for its whole life. A sweep keeps an origin whose lock a live holder keeps, and
  removes the origin and its unheld lock file otherwise. The pid in the name proves nothing.

## The spawn-to-insert window (not closed)

Rejected: a row written before `spawn`. Without a marker a pending row names no group, so the reap cannot
act on it and the window stays open. Each marker found was worse: an `argv[0]` marker changes what the
acceptance command sees as its name; an environment marker needs `/proc/<pid>/environ` on Linux and a
different `ps` flag on macOS, two readers with different failure modes.

So the row is written right after `spawn` and its identity read, in one insert. Measured on the Linux
oracle, debug build, 400 `run_command` calls of `true`, from `spawn` returning to the insert committing:
p50 4.40 ms, p99 14.25 ms, max 26.01 ms. The window is milliseconds, and most of it is the SQLite commit.
A kill inside it still orphans the group; it is recorded here, not closed.

## Consequences

- The sim `sim_a_kill_at_every_point_between_spawn_and_insert_leaves_no_group_after_restart` kills at
  seeded yield counts after the group is recorded. It cannot land inside the window from outside the
  process, so it proves the reap at varied points, not the window itself.
- A group that cannot be reaped in 5 s keeps its row across starts. A `larry eval` killed mid-run leaves
  its origin until the next run sweeps it; a crashed holder's lock is released by the kernel.
- Racing starts may settle one row twice: each reap is idempotent (SIGKILL to a dead group is a no-op),
  so the oracle asserts every row is settled at least once and none survives.
- Two starts could race between a lock-holder's `try_lock` and the sweep's probe; the probe takes and
  releases the lock, so a live holder can, in a microsecond window, see its `try_lock` refused. Recorded
  as a known gap, not measured.

## Rejected

- Keeping the old `_outcome` drop: it removed a row whose group lived, so the next start could not name it.
- A pid-liveness check for origins: a pid a container reuses proves nothing (larry#182 defect 3).
