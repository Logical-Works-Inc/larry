---
type: ADR
title: "ADR 0147: a run lock that cannot be probed keeps its run and stops nothing else"
description: "One unopenable run lock keeps its journal and its group's row; the start goes on."
resource: /docs/adr/0147-an-unprobed-lock-keeps-its-run.md
tags: [larry, adr, executor, recovery, liveness]
generated: {by: claude-code, at: 2026-11-28}
verified: {by: claude-code, method: lgwks-box cargo nextest run, 2026-11-28}
status: accepted
stale_after: 2027-05-28
sources: [larry#619, larry#334, larry#394, larry#456, CWE-755, INVARIANTS.md]
---

## Status

Accepted.

## Context

Every start reaps the groups a killed executor left and recovers its journals.
Both ask a run's `<task>.lock` whether the run is live. Both passed a probe fault
up with `?`, so one lock that could not be opened (a directory in its place, or a
root-owned file left by a killed `sudo larry exec`) failed every later run on
that state directory.

## Decision

The sweep keeps that journal as a journal, with a typed `KeptCause::Lock`, and
goes on. The reap reads the run as held: nothing is signalled and the ledger row
stays. Each says so in one JSON line on standard error. A later start that can
open the lock reaps and recovers as before.

## Consequences

An unopenable lock leaves its group running and its journal unrecovered until a
person fixes the file; the two lines name it on every start.

Rejected: read an unprobeable run as dead. It would signal a group that may
belong to a live run (larry#334). Reversed by: a probe that proves the holder
dead without opening the lock.
