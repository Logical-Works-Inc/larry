---
type: ADR
title: "ADR 0129: bounded live command output reaches the artifact pane"
description: "The TUI streams bounded command output while preserving complete head and tail capture."
resource: /docs/adr/0129-pane-stream.md
tags: [larry, adr, tui, executor, process]
generated: {by: codex, at: 2026-11-26}
verified: {by: codex, method: lgwks-box cargo nextest run --locked -p larry-demo, 2026-11-26}
status: accepted
stale_after: 2027-05-26
sources: [larry#252, INVARIANTS.md, tests/e2e_journey.rs]
---

## Context

`run_command` captured stdout and stderr until the command ended. The artifact pane therefore
could not show a running command's output.

## Decision

The existing socket drain invokes an optional callback for each read, split into chunks of at most
4 KiB. The callback uses the existing worker event channel with `try_send`. A full channel drops a
chunk and increments a counter. The command never waits for the frame.

The TUI relay converts bytes lossily to text. The pane keeps streamed lines in its existing
512-row ring. It bounds an unfinished line to 512 characters and sanitizes every row before
paint. A dropped-chunk row is shown at command completion. Head and tail capture remains the
model and report source.

The TUI registers SIGTERM and SIGHUP before it enters the frame. Either signal ends the frame,
which finishes the owned worker. Worker cancellation reaches `run_command`, which settles and
waits for the command process group before Larry exits.

The header row moves from `finished` to `started`: the pane paints `$ <command>` when the
command starts so streamed lines carry their context while running, and `finished` skips the
duplicate when the stream already showed it. The seeded wire pin moves with it
(`3b908485…ecf58986`): the frame is intended, not drift.

Other observers keep the default no-op callback. No new dependency or durable state is needed.

## Consequences

The pane can show output before a command ends, and a slow frame cannot stall the command's
capture drain. A burst can lose live chunks, but the pane reports the count and the bounded
head-tail capture remains available to the model and sealed report. Invalid UTF-8 is replaced
before the pane's sanitization step. The streaming journey arms a Drop guard over its
workspaces, so any exit releases a still blocked FIFO instead of orphaning the `cat` that
holds the command; the signal journey ends Larry with SIGTERM mid-command and asserts no
child remains.

## Proof

- `sim_stream_callback_splits_each_read_at_its_bound` proves the callback chunk bound.
- `sim_worker_stream_bound_counts_shed_chunks` proves nonblocking shedding and its count.
- `sim_stream_callback_bounds_and_drop_count` proves pane bounds and sanitization.
- `tui_journey_pane_streams_first_line_before_fifo_release` proves the shipped PTY journey.
- `tui_journey_sigterm_kills_running_command_group` proves signal teardown leaves no child.

## Traceability

Issue #252. Invariants: bounded queues, sanitized paint, complete report capture, process-group
teardown, and the human frontend run path. The rejected design was a second blocking queue
because it could stall the command and duplicate the event bound.
