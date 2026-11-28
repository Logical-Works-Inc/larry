---
type: ADR
title: "ADR 0149: an unopenable run lock reads as a live run"
description: "A run lock that cannot be opened proves nothing: its groups are left alone, its journal is kept and the next run starts."
resource: /docs/adr/0149-unopenable-run-lock.md
tags: [larry, adr, executor, recovery]
generated: {by: claude-code, at: 2026-11-28}
verified: {by: claude-code, method: lgwks-box cargo nextest run, 2026-11-28}
status: accepted
stale_after: 2027-05-28
sources: [larry#591, larry#334, larry#456, larry#394, INVARIANTS.md]
---

## Status

Accepted. Number 0149: `main` took 0148 for larry#620 while this change was open.

## Context

Every start asks each dead run's `<task>.lock` whether that run is gone, once in
the reap and once in recovery. An open error left both as `ExecError::Journals`.
A directory in the lock's place, or a root-owned file left by one killed
`sudo larry exec`, refused every later run on that state directory until a person
found the file. A FIFO there held the start with no end, because an open to write
waits for a reader. This broke the rule of larry#334: one leftover never refuses
a later run.

## Decision

A lock that cannot be opened or tried proves nothing, so `executor::recovery`
reads its run as held. The reap signals none of its process groups and keeps its
ledger rows. Recovery keeps its journal, prints one `"recovery":"kept"` line that
names the journal and the open error, and goes on. A lock file is opened
non-blocking, so a FIFO is an open error like the others. A retry that derives
that run id is still refused typed, because it cannot take the lock. Once the
person clears the file, the next start reaps and recovers the run.

## Consequences

The dead run's command can outlive its executor until the file is cleared, and
the line on standard error is how the operator learns of it.

Rejected: delete or replace the faulty lock and recover. A permission another
user set can hide a live run, and the reap would kill its group and take its
journal. That is not reversible. A kept journal is.

Measurement that reverses this: a start that can prove the run is dead without
the lock, for example a process identity the store checks, makes the held answer
too careful.
