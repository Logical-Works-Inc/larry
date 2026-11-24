---
type: ADR
title: "ADR 0100: Larry's ledger waits on the store alone"
description: Why the executor ledger drops its own executor.db.lock spin and lets the store's sleeping busy handler do every wait.
resource: /docs/adr/0100-the-ledger-waits-on-the-store-alone.md
tags: [larry, adr, executor, ledger, sqlite]
generated: {by: claude-code, at: 2026-11-24}
verified: {by: claude-code, method: sim_a_ledger_write_on_a_busy_store_waits_without_spinning red then green, and concurrent_first_opens_of_one_fresh_ledger_all_succeed, on Oracle 2026-11-24}
status: accepted
stale_after: 2027-05-24
sources: [larry#398, larry#321, larry#214, logicalworks-crates#872, logicalworks-crates#873]
---

# 0100: The ledger waits on the store alone

Status: accepted, 2026-11-24 (larry#398). Supersedes the `executor.db.lock` rule of larry#214.

## Context

`Ledger::open` took `executor.db.lock` with a `try_lock` and `yield_now` loop for up to 60 s.
A waiter ran a full core, and on a 1 to 2 vCPU host it starved the executor that held the lock.
The lock existed for two estate bugs: a first open switched to WAL before its busy timeout
(logicalworks-crates#873), and `migrate` read and wrote `user_version` outside one transaction
(#872). Both closed on 2026-11-08 and ship in the pinned `lgwks_deps` 8.0.0.

## Decision

- Delete the lock, its loop and `ExecError::LedgerLockTimeout`. `lgwks_deps::sqlite::open` sets
  the 60 s busy timeout first and takes WAL under a blocking kernel lock; `migrate` runs each
  step in one immediate transaction. Every wait left is SQLite's busy handler, which sleeps.
- `ExecError::LedgerLock` becomes `ExecError::StateDir`: the one fault left is a state
  directory that cannot be made.
- `ExecError` moves to `executor::error`, so `src/executor.rs` holds the run alone.

Rejected: keep the lock and back off between tries. No synchronous pause passes the
sleep-polling axis, and a second wait beside the store's own is a second copy of what
`lgwks_deps` owns. Reverse it if a measured first open on the pinned store fails "database is
locked" again.

## Consequences

- A store held past 60 s is the store's typed refusal (`ExecError::Ledger` or `Sql`), not a
  Larry timeout. An old `executor.db.lock` file stays on disk and is read by nothing.
- Falsifiers: `sim_a_ledger_write_on_a_busy_store_waits_without_spinning` (under 5% of the wait
  on the CPU), `concurrent_first_opens_of_one_fresh_ledger_all_succeed` (16 openers, 8 rounds).
