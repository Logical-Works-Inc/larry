---
type: ADR
title: "ADR 0023: the runtime journal on lgwks_bot RecordJournal, group-committed"
description: How Larry's journal survives a kill -9 on lgwks_bot's RecordJournal, why it syncs per owner reach and per tick, and how larry exec recovers a dead task without re-running it.
resource: /docs/adr/0023-journal-on-record-journal.md
tags: [larry, adr, runtime, journal, durability, lgwks-bot, track-1]
generated: {by: claude-code, at: 2026-11-09}
verified: {by: claude-code, method: nextest run of the sims and the kill -9 binary journey, larry exec before and after}
status: accepted
stale_after: 2027-02-09
sources: [larry#75, larry#68, logicalworks-crates#875, docs/adr/0019-executor-tasks-pass-the-runtime-gate.md]
---

# 0023: the runtime journal on lgwks_bot RecordJournal, group-committed

Status: accepted, 2026-11-09 (larry#75, row 2 of larry#68)

## Context

Larry's journal was memory only, so a kill -9 of `larry exec` lost every fact of the task it held. lgwks_bot
8.1 ships `journal::RecordJournal` (crates#875). It takes any record, chains each frame to the one before,
earns a `ProcessCrash` promise on every append, refuses a second opener with `Locked`, and takes a torn tail
back on open. It refuses committed bytes that break the chain rather than trimming them. Larry's own copy of
framing, chaining or locking would be a second owner of all that.

## Decision

- `Journal::open(path, capacity)` backs the in-memory journal with one `RecordJournal<Vec<u8>>`. A record
  holds an entry's parts (ids, tenant, parents, kind, payload) as JSON and never its digests. Replay rebuilds
  each entry through `Command::new` and `Event::new` and the journal's own acceptance rules, so a digest is
  always derived again, a command recorded twice is `DurableRepeat`, and a gap is a `Sequence` refusal.
- Group commit. Each entry is staged as the index takes it. `Journal::sync` writes every staged record in
  one write and one sync (`compare_and_append_all`). The runtime syncs before every owner reach, so the
  intent is on disk before the owner can act, and before every tick returns, whether a stage failed or not.
  Nothing an owner or a caller saw is ever only in memory. A store refusal leaves the records staged, so
  every later sync refuses too.
- `larry exec` gives each task `journals/<task>.journal` under the state directory and removes it when the
  task returns. It keeps a journal the store refused, as evidence.
- Every start, after the reap, recovers at most 1024 leftover journals in name order. It skips one whose
  task's executor is still alive (`owner_alive`) or whose lock is held. Each recovered journal is restored
  with an owner that reaches nothing, ticked once (`RequiresHuman`, so the reservation settles as
  `OutcomeUnknown`), and kept as `<task>.recovered`. A journal whose bytes are refused is kept as
  `<task>.refused`.

Rejected:
- One sync per entry. It costs `larry exec` 42 ms at p50 (65.4 to 107.0 ms, 32 runs, debug), against 8 ms
  for group commit.
- `FileJournal`. It persists only lgwks_bot's own effect ladder, not Larry's commands and events.
- One journal for every task. It would put every task behind one lock, and a damaged file would stop them
  all.
- Re-running a recovered task. Its body may have acted already, and an ambiguous reach is never retried
  blind.

## Consequences

- `larry exec` measured p50 70.0 to 78.1 ms and p99 96.3 to 98.3 ms (32 runs, debug), with peak RSS 12.5 to
  13.2 MiB.
- A `.recovered` journal is the dead task's whole history and still opens with `Journal::open`. A person
  settles its unknown effect; #90's runtime API serves those journals.
- Row 2 of larry#68 is held by `sim_a_second_opener_is_refused_while_the_first_holds_the_file`. Larry keeps
  no framing, chain or lock code.
