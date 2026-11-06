---
type: ADR
title: "ADR 0004: the executor runs each command in its own process group and ledgers it in SQLite"
description: Why `larry exec` (larry#85) clones, how it bounds and kills commands, and how a kill -9 of it is reaped.
resource: /docs/adr/0004-executor-process-groups-and-ledger.md
tags: [larry, adr, executor, process, sqlite]
generated: {by: claude-code, at: 2026-11-06}
verified: {by: claude-code, method: tests/executor_journey.rs on the shipped larry binary, unit sims in src/executor}
status: accepted
stale_after: 2027-02-06
sources: [larry#85, larry#71, logicalworks-crates#846, lgwks_std 4.0.2 src/process.rs and src/process/identity.rs, lgwks_deps 6.1.0 src/sqlite.rs]
---

# ADR 0004: executor process groups and ledger

## Status

Accepted, 2026-11-06, with larry#85.

## Context

The executor runs a task's acceptance commands against a repository. A command may be hostile, may spawn
children, and may never exit; the executor itself may be killed with SIGKILL, which runs no destructor.
lgwks_std 4.0.2 owns process-group kill, group existence, and pid-plus-start-instant identity with orphan
reaping; lgwks_deps 6.1.0 owns the estate's one SQLite. The rivals were `Child::kill` with `read_to_end`
(kills the leaf only, and inherited pipes block the read forever) and kill-on-drop containers such as
processkit (a second owner of process lifetime, and a destructor never runs under kill -9).

## Decision

- Workspace: `git clone --local --no-hardlinks --separate-git-dir` into `<LARRY_STATE_DIR>/ws/<task_id>/`
  (`tree/` for the commands, `git/` beside it), then a detached checkout of the base. The executor's own git
  calls name `--git-dir` and `--work-tree`, so a command that rewrites `tree/.git` to point at the origin
  never redirects them. Never a worktree: it shares the origin's `.git`. The workspace is removed after every
  run; a start sweeps workspaces whose executor pid is gone and no ledger row names.
- Each acceptance argv runs directly (no shell) with only `PATH`, `HOME` (the workspace) and `LANG=C`, stdin
  null, in its own process group. stdout and stderr are read on scoped threads, 1 MiB each, the rest drained
  and marked `truncated`. The deadline is `recv_timeout` on the waiter's channel; no sleep, no polling.
- On timeout, and again after every leader exit, the whole group is killed and its absence confirmed.
- Before the wait, the leader's `ProcessIdentity` is written to `<LARRY_STATE_DIR>/executor.db`
  (`live_groups`); the row goes once the group is confirmed gone. Every start reaps each row with
  `reap_orphaned_group` and removes workspaces no live row names.
- Model-emitted commands are refused with `IsolationUnavailable` until Nova isolation lands
  (logicalworks-crates#846); there is no host fallback.
- Bounds: a 64 KiB task, 16 commands, 64 argv items of 4 KiB, a base of 256 bytes that cannot start with
  `-`, a 1 MiB diff, a timeout of 1 ms to 24 h (`LARRY_EXEC_TIMEOUT_MS`, default 10 min).

## Consequences

- The root crate gains lgwks_std `process` and `hash` and lgwks_deps `sqlite` (bundled SQLite).
- A group whose leader exited before the reap is not reachable by its record (`OrphanReap::LeaderGone`);
  the kill after every leader exit keeps that window to a kill -9 landing between the exit and the kill.
- tests/executor_journey.rs proves the hostile, timeout, kill -9, concurrency and refusal journeys on the
  shipped binary.
