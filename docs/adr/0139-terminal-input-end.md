---
type: ADR
title: "ADR 0139: the terminal frontend ends with its terminal"
description: "Bare larry refuses a stdin that is no terminal, and one watcher ends the frame on SIGTERM, SIGHUP or end of input."
resource: /docs/adr/0139-terminal-input-end.md
tags: [larry, adr, tui, signals, orphans]
generated: {by: claude, at: 2026-11-27}
verified: {by: claude, method: lgwks-box cargo nextest run --locked on toronto, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#573, INVARIANTS.md, src/tui/stop.rs, tests/support/terminal_cases.rs]
---

## Context

Number 0139: main and the open pull requests hold 0133 to 0138.

Thirty-five `larry` processes with parent 1 were found spinning on the build box.
SIGTERM did not end them. The reader in crossterm 0.29.0 (`event/source/unix/mio.rs`,
`try_read`) loops on a read that returns zero bytes, so `event::poll` never returns
on a hung-up terminal and the frame never reads its stop flag. The orphans came from
terminal journeys that killed only `script`, the parent of `larry`.

## Decision

`larry` with no command refuses a standard input that is not a terminal, with one
line, before it paints. While the frame runs, one watcher waits for SIGTERM and
SIGHUP, and a thread it owns blocks in `poll(2)` for a hang-up on a duplicate of
stdin, with no timeout. On any of these it sets the stop flag and puts a quiet
nonblocking socket over stdin, so the reader sees "no data yet" and `event::poll`
returns inside its own timeout. Stdin is put back before the screen is restored.
A signal exits as before. An ended input is a typed error and exit 1.

Every `larry` a test starts is held by a guard that kills and reaps it on drop, and
the terminal guard kills the binary before it kills `script`.

## Rejected

- crossterm's `use-dev-tty` feature: it adds the `filedescriptor` crate, and its
  reader still loops on an ended input until each poll timeout.
- Checking the descriptor before each `event::poll`: a hang-up between the check and
  the poll still spins.
- `process::exit` from the watcher: it skips the kill of a running command's group.

## Consequences

`larry` needs a terminal on stdin; a script uses `larry exec` or `larry help`. Larry
takes the `event` and `stdio` features of `rustix`; no crate is added. The estate
runtime has no wait for a descriptor to hang up and no safe `dup2`. On macOS,
`poll` can return early while a key is still unread; the watch then asks again.
Reverse this when a crossterm release returns from `poll` on end of input: the
socket swap is then dead code.

## Proof

The journeys in `tests/support/terminal_cases.rs` close the terminal under a running
frame, with and without a SIGHUP, and wait for the process to be gone.
`a_bare_larry_without_a_terminal_is_refused` covers a file, `/dev/null` and a pipe.

## Traceability

Issue #573, part of #423. The invariant lines are in INVARIANTS.md.
