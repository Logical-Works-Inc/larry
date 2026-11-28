---
type: ADR
title: "ADR 0133: TUI sessions persist in the state database"
description: "Each terminal session is a row in executor.db, held after every turn that heard an event."
resource: /docs/adr/0133-tui-sessions-persist.md
tags: [larry, adr, tui, executor, ledger]
generated: {by: claude-code, at: 2026-11-27}
verified: {by: claude-code, method: lgwks-box cargo nextest run --locked, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#256, larry#578, INVARIANTS.md, tests/e2e_journey/sessions.rs]
---

## Context

Larry kept no TUI session across a restart (larry#256, part of #255): closing or
killing the terminal lost the transcript, the pane and every receipt digest. The
executor already owns a SQLite store (`executor.db`), so session rows belong
there, never in stray files (SD-1, SD-3: state that outlives a run lives in a
database).

## Decision

A `tui_sessions` table holds one row per terminal session, keyed by tenant and
id: name, folder, model, update stamp and the snapshot JSON (transcript, pane and
receipt digests). Migration step 12 adds it. The tenant is the one the run
executes as (`LARRY_TENANT`, else the store's default, ADR 0132), and every write
names it. A create past a tenant's 10,000 rows writes no row in one statement,
and a duplicate id keeps the first record, so two creators racing at the bound
never both land (larry#397's shape). A name is 1 to 64 characters with no control
character, a snapshot is at most 4 MiB and keeps the newest 256 receipts; each
refusal is a typed `SessionStoreError`. A frame holds up to 8 MiB, so the driver
leaves out the oldest entries until the snapshot fits the row.

The frame (`tui::app`) stays I/O-free. The driver (`tui.rs`) owns the store,
adopts one row at start and saves it after each turn that heard a key, a paste, a
resize or a session event, the turn that quits included; an idle tick writes
nothing. A refused database or row leaves one `sessions are not saved` notice and
the frame runs unsaved. Session ids are UUID version 7 from the estate, like run
ids (ADR 0037). The update stamp is wall clock and orders rows only, never a
replay.

Rejected: a file per session (a second store with its own crash rules), a store
with no caller (dead code under `-D warnings`, SD-1), and one table shared by
every tenant (a resume would list another tenant's transcripts, and one tenant
could fill the cap). Reversed if a save is measured to push the probe journey's
input round trip past one frame.

## Consequences

A kill -9 loses at most the turn in flight. Reading a row back in the terminal
(`/resume` and the other session commands) lands with the next parts of larry#256.
Proof: `tui_journey_a_killed_terminal_keeps_its_session_row` and
`src/tui/sim_sessions.rs`.
