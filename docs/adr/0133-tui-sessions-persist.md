---
type: ADR
title: "ADR 0133: TUI sessions persist in the state database"
description: "Each terminal session is a row in executor.db, held after every turn that heard an event."
resource: /docs/adr/0133-tui-sessions-persist.md
tags: [larry, adr, tui, executor, ledger]
generated: {by: claude-code, at: 2026-11-27}
verified: {by: claude-code, method: lgwks-box cargo nextest run --locked, 2026-11-28}
status: accepted
stale_after: 2027-05-27
sources: [larry#256, larry#578, larry#609, INVARIANTS.md, tests/e2e_journey/sessions.rs]
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

## Reading a row back (larry#603)

`/resume` lists this tenant's sessions of this folder, newest first, without the
one the terminal holds: the newest 20 by number, the rest counted. `/resume <t>`
continues one, where `t` is a listed number, an exact name or the start of an id;
two matches are refused. `/new` starts an empty session, `/fork [t]` copies this
frame or a stored one into a new row, `/rename <name>` names this one. A switch
brings back the transcript, pane and receipt digests and nothing else: no running
command, no `/ps` row, no queued turn and no "stop asking". While a task runs the
commands wait in the composer and never reach the model.

One row has one writer. A save and a rename name the stamp the terminal last
wrote, and the new stamp is the clock's or one past the old one. A terminal whose
row another one resumed is told once, writes nothing more and keeps its frame for
`/fork`. This is the version check against a lost update (RFC 9110 section
13.1.1); no migration is added.

Rejected: a lock or owner column (a killed terminal leaves it held), last write
wins (one terminal erases the other's turns in silence), and the first attempt's
driver (PR #567: no tenant key, a named fork wrote the frame on screen over the
copy, and a running task could swap its transcript). Reversed if two terminals on
one row must merge.

## Writing it out (larry#609)

`/export [file]` writes the frame on screen as Markdown inside the folder
Larry runs in, `<session name>.md` with no file named: who and where, each entry,
every kept receipt digest. Every person, model and command text sits in a fence
longer than any backtick run it holds (CommonMark 0.31 section 4.5), with control
and invisible characters shown as markers. The file is at most 4 MiB: the oldest
entries are left out first and counted in it. It is made through held directory
descriptors (`lgwks_std::fs::capability::Dir`), mode 0600, never over an existing
name, never through a link, never into `.git` or outside the folder (CWE-22,
CWE-59). It needs no row, so a refused database changes nothing. It waits while a
task runs.

The name holds the whole file or nothing. The bytes go to a new part file
`.larry-export-<UUIDv7>.part` beside the target, are synced, and take the name by
`linkat`, which never replaces (POSIX `link(2)`); the part file is then removed
and the folder synced. A failed or killed write leaves nothing under the name, so
the next `/export` succeeds. Each export first sweeps its target's folder: a
regular file with a part name that no running export holds locked is removed, from
the first 65,536 entries. A name of that shape is refused as a target. A part
file another terminal sweeps before its lock is made again, at most 3 tries. A
file system with no hard links refuses the export and writes nothing.

`/recap` answers in the frame, a task running or not: counts, the newest 5 tasks,
the answer to the newest one and the receipts. It calls no model and compacts
nothing.

Rejected: PR #567's export (written beside the folder, following links,
overwriting, model text as raw Markdown, no bound); `replace_file` (its rename
replaces, and it resolves paths through links); `std::fs::hard_link` (it resolves
the folders again, so a link swapped in after the walk is followed); one fixed
part name (two terminals would publish each other's bytes); and a model-written
recap (a paid call that can fail, ADR 0044). Reversed if a folder with no hard
links must take an export, or a recap must say what no receipt holds.

## Consequences

A kill -9 loses at most the turn in flight, and a fresh terminal continues the
session. A listing reads the tenant's rows, at most 10,000.
Proof: `tui_journey_export_writes_the_session_and_recap_sums_it` (shipped debug
binary, oracle, 20 samples: `/export` p50 111.8 ms, p99 140.8 ms, RSS 10,900 KiB),
`tui_journey_a_kill_mid_export_leaves_no_part_file_and_the_next_export_succeeds`,
`src/tui/sim_export.rs`, `tui_journey_a_killed_terminal_resumes_its_session_and_forks_it`,
`tui_journey_a_full_session_store_lists_and_reports_latency_and_rss` (shipped
debug binary, 10,000 rows, 20 samples on the Oracle ARM64 box: p50 44.6 ms, p99
49.7 ms, RSS 14,544 KiB), `tui_journey_a_killed_terminal_keeps_its_session_row`,
`src/tui/sim_resume.rs` and `src/tui/sim_sessions.rs`.
