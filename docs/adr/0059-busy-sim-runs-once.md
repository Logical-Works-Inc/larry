---
type: ADR
title: "ADR 0059: the busy-ledger sim runs once, with no busy timeout"
description: Why the ledger reader's retry simulation moves into one test binary and turns SQLite's busy timeout off.
resource: /docs/adr/0059-busy-sim-runs-once.md
tags: [larry, adr, executor, tests, sqlite]
generated: {by: claude-code, at: 2026-11-17}
verified: {by: claude-code, method: larry-oracle nextest on the larry#309 branch, 2026-11-17}
status: accepted
stale_after: 2027-02-17
sources: [larry#309, larry#294, /docs/adr/0053-group-wait-deadline.md, SQLite sqlite3_busy_timeout]
---

# 0059: the busy-ledger sim runs once, with no busy timeout

Status: accepted, 2026-11-17 (larry#309)

## Context

`sim_live_group_pid_answers_after_two_busy_reads` proved that a ledger read refused
with `SQLITE_BUSY` is retried inside the one failure deadline. It lived in
`tests/support/exec_cli.rs`, which three journey binaries include by `#[path]`
(`executor_journey`, `executor_idempotency_journey`, `receipt_journey`).

Each seed's reader kept rusqlite's default 5 s busy timeout. So each of the two
refusals waited the whole timeout before it returned: 2 reads x 8 seeds x 5 s = 80 s.
The sim ran in three binaries, so one CI run spent 240 s of runner time on it.

## Decision

- The reader sets `Connection::busy_timeout(Duration::ZERO)`, which turns every busy
  handler off: each refusal returns at once and `deadline::PATIENCE` alone bounds the
  wait, as the invariant it proves says.
- The sim moves into `tests/executor_journey.rs`, a binary that already includes the
  fixture and already carries a seeded sim. It runs once.
- `live_group_pid_through` stays in `tests/support/exec_cli.rs` beside its one live
  caller and widens to `pub(crate)` so the moved test drives the same retry rule.
- The seeds, the assertions and the trace are unchanged. `INVARIANTS.md` names the new
  path for the test.

## Consequences

- The sim ends in 0.101 s on larry-oracle, down from 80 s x 3 on main at 91ee874.
- The retry rule the journeys run is the rule the sim proves; no second copy exists.
- No new test binary, no new compile unit, and no fourth copy of
  `tests/support/deadline.rs`'s own tests.
- No product code, dependency or crate moves.

## Rejected

- A dedicated `tests/sim_busy_ledger.rs`: it names the owner by file but adds an
  integration binary to compile and link in every lane.
- The sim kept in the fixture file and compiled off in two binaries by a cfg flag: a
  test compiled off is invisible to those runs, and no other support file does that.
- A shorter nonzero busy timeout: it still waits real seconds per refusal, so the
  runner's load would decide the sim's wall time.
