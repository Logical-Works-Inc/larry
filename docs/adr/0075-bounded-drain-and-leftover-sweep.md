---
type: ADR
title: "ADR 0075: a command's leftover is reported and skipped, and its output drain ends on its status"
description: Why a workspace is reset before removal, a refused sweep entry is one typed event, and each command stream is a socket pair sized by a read timeout.
resource: /docs/adr/0075-bounded-drain-and-leftover-sweep.md
tags: [larry, adr, executor, workspace, process, sweep]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: larry#334 journeys failing on main, then green with the reset, the typed sweep faults and the bounded drain, 2026-11-21}
status: accepted
stale_after: 2027-02-21
sources: [larry#334, larry#182, larry#214, /INVARIANTS.md, /docs/adr/0042-orphan-groups-and-bounded-probes.md]
---

# 0075: a command's leftover is reported and skipped, and its output drain ends on its status

Status: accepted, 2026-11-21 (larry#334).

## Context

Three leftovers from one run stopped the next one.

- A command could leave a directory with no permissions. `Workspace::drop`
  dropped its `remove_dir_all` error and counted the tree as stranded; the
  next start's `sweep` returned that error, and `prepare` refused every run.
- The output readers were scoped threads joined with no deadline. A child
  that called `setsid` and kept the command's stdout or stderr pipe open held
  the join forever: past the command timeout and past the person's Esc, with
  the process group already dead.
- `journal_live` read every probe that was not `Locked` as "not live" and
  `recovered_evidence` used `Path::exists`; a stat or open error became
  "dead" or "absent".

## Decision

- `remove_workspace` resets every directory beneath a workspace, the root
  included, to owner rwx on an explicit stack (never recursion), follows no
  symlink, then calls `remove_dir_all`. `Workspace::drop` uses it.
- `sweep`, `sweep_origins` and `remove_unheld_locks` return `Vec<SweepFault>`.
  An entry whose liveness probe errors is kept (an unknown owner may be live);
  an entry whose removal fails is kept. Each is one typed `SweepFault`, and
  `prepare` and `Settings::sweep_origins` write one JSON line per fault to
  standard error. A global listing failure still refuses the run.
- `journal_live` and `recovered_evidence` return `Result<bool, ExecError>`:
  `ExecError::Journals` for a stat error and the new `ExecError::Journal` for
  a lock probe error. `gate::admit_and_run` propagates the stat error.
- Each command stream is a `UnixStream` pair, not a pipe. The pair's read
  timeout is armed before the command spawns, because Darwin refuses
  `SO_RCVTIMEO` with `EINVAL` once the child that held the writing end has
  closed it, so arming it in the reader thread raced every fast command. The
  parent reads with that `CANCEL_POLL` timeout, and its copy of the child's end
  is dropped right after spawn. Once the command's group is confirmed gone
  (`settled`), a read issued after that observation that blocks raises the
  stream as open at once; a stream that keeps yielding bytes is raised open at
  `DRAIN_GRACE`. Either way the run ends with `ExecError::Spawn` naming the
  stream that stayed open.

## Consequences

- One command's leftover never refuses a later run. A sweep that cannot settle
  an entry keeps it and says so on stderr; the next start still runs.
- An unknown liveness is never read as dead, so a probe error cannot delete a
  live run's workspace or rerun a task whose evidence could not be read.
- A command whose child left the group ends inside the drain grace with a typed
  refusal, instead of hanging the executor with its group dead.
- The report's head, tail, dropped and total counts are unchanged: the bounded
  drain trims its tail exactly as the reference capture does.

## Rejected

- Detached reader threads awaited through a channel deadline. A long-lived host
  leaks one thread per escaped child, and a writer that keeps writing cannot be
  told from a stream about to close.
- Keeping pipes and closing the parent's end only. A pipe cannot carry a read
  timeout through the standard library, so the join still had no deadline.
- A short fixed read budget for every command. A slow command's whole output
  would be cut, and the spec names the command's deadline and the cancel input.
- Failing the sweep on the first entry that will not settle. That is the defect:
  every later run is refused.
- Treating a probe error as "dead". A live run's workspace would be removed.

Reversal: if `std` grows a timed pipe read on every target, the socket pair
returns to a pipe; the drain's rule does not change.
