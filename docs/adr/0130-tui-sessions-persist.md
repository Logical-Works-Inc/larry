---
type: ADR
title: "ADR 0130: TUI sessions persist in the state database"
description: "Each terminal session is a row in executor.db with /resume, /fork, /rename, /new, /export and /recap."
resource: /docs/adr/0130-tui-sessions-persist.md
tags: [larry, adr, tui, executor, ledger]
generated: {by: opencode, at: 2026-11-27}
verified: {by: opencode, method: lgwks-box cargo nextest run --locked, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#256, INVARIANTS.md, tests/e2e_journey.rs]
---

## Context

Larry kept no TUI session across a restart (larry#256, part of #255). Codex ships
`/resume`, `/fork`, `/rename`, `/new`, `/export` and `/recap`; Larry had four
slash commands and forgot everything on exit. The executor already owned a
SQLite store (`executor.db`), so session rows belong there, never in stray files
(SD-1, SD-3: state that outlives a run lives in a database).

## Decision

A `tui_sessions` table holds one row per terminal session: id, name, folder,
model, update stamp and the snapshot JSON (transcript, pane and every receipt
digest seen). Migration steps 9 and 10 add the table and its folder index; the
folder index makes one folder's listing a scan of its own rows. A create past
10,000 rows writes no row in one statement, and a duplicate id keeps the first
record, so two creators racing at the bound never both land (larry#397's shape).

The frame (`tui::app`) stays I/O-free: slash words dispatch to effects, and the
driver (`tui.rs`) owns the store, holds the frame after every turn, and adopts,
lists, resumes, forks, renames, exports and recaps. The update stamp is wall
clock; it orders a listing only, never a replay. Session ids are UUID version 7
from the estate, like run ids (ADR 0037). `/export` writes `<slug>.md` beside
the folder; `/recap` counts without compacting anything.

## Consequences

`/resume` lists this folder's sessions newest-first and continues one by number,
id prefix or name; `/fork` copies one row into a new session so the fork
diverges and the original is unchanged; `/export` names the transcript and
every digest. A refused settings row runs the frame unsaved and says so when a
session command runs. The ten-thousand-row sim prints list p50/p95/p99 and peak
RSS; the PTY journey kills -9 the terminal and resumes it.
