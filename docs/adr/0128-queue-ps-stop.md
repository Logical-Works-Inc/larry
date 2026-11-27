---
type: ADR
title: "ADR 0128: Queue a turn while a task runs, /ps and /stop for its commands"
description: Why the frame queues eight turns and lists and stops background commands.
resource: /docs/adr/0128-queue-ps-stop.md
tags: [larry, adr, tui, frontend]
generated: {by: opencode, at: 2026-11-27}
verified: {by: opencode, method: lgwks-box Oracle nextest sims and PTY journeys, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#262, larry#255, larry#252, larry#242, larry#241]
---

# ADR 0128: Queue a turn while a task runs, /ps and /stop for its commands

## Context

Codex queues a message with Tab while a turn runs, and has `/ps`
(background terminals and output) and `/stop`. Larry refused typing
into a running task except Esc, so a thought that arrived mid-run was
lost, and a running command could only be ended with Esc, never named.

## Decision

The frame owns a queue of eight turns and a job list of sixteen
commands, both bounded and both painted, all in `tui::app`:

- Enter while a session runs queues the text for the next turn; the
  queue shows above the composer; the ninth is refused naming the
  bound; each `Ended` starts the next queued turn in order.
- Every `Started` command is tracked per frame with its last five
  output lines; `/ps` lists them; `/stop` ends the running ones
  through the session's cancel path, the same one Esc uses, so the
  process group dies (larry#242); with nothing running both only say so.
- `/quit` (and the second Ctrl+C) with running commands asks first,
  ends them, and quits when the session ends.

## Evidence

Fourteen seeded sims in `src/tui/sim_queue.rs` (order, `/ps` lines,
finish without a start, `/stop` interrupt, quit asks, the quit singular,
the end marks done, the queue bound, the jobs bound, the output tail, the
first line, queue numbering, the always answer) pass. Two PTY
journeys in `tests/support/tui_queue.rs`, included by `tests/e2e_journey.rs`,
drive the shipped binary: queued turns send in order (4 model calls), and
`/stop` kills the whole group (the background pid is gone, 1 model call).
Oracle nextest: 14 of 14 green.

## Consequences

The frame answers while busy: slash commands still run, text queues. A ninth
turn is refused naming the bound, so no input is silently dropped. `/ps` costs
one bounded list; `/stop` reuses the cancel path, so no second kill owner
exists. Quitting with running commands takes two steps, so a stray key never
strands work, and the queue clears on quit so no turn sends after the person
leaves.
