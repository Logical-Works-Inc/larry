---
type: ADR
title: "ADR 0128: Queue a turn while a task runs"
description: Why the frame queues eight turns while a session runs.
resource: /docs/adr/0128-queue-ps-stop.md
tags: [larry, adr, tui, frontend]
generated: {by: opencode, at: 2026-11-27}
verified: {by: opencode, method: lgwks-box Toronto nextest sims and PTY journey, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#262, larry#255, larry#325]
---

# ADR 0128: Queue a turn while a task runs

## Context

Codex queues a message with Tab while a turn runs. Larry refused
typing into a running task except Esc, so a thought that arrived
mid-run was lost.

## Decision

The frame owns a queue of eight turns, bounded and painted, in
`tui::pending`, owned by `tui::app`:

- Enter while a session runs queues the text for the next turn; the
  queue shows above the composer, one numbered row per waiting turn;
  the ninth is refused naming the bound; each `Ended` starts the next
  queued turn in order; quitting clears the queue so no turn sends
  after the person leaves.
- While a session runs, slash commands still run and other text
  queues; an unknown slash word is refused the same way as when idle.
- The queued row and the running status row show only the turn's
  first line through `sanitize_line`, which takes no flag so no dead
  branch is left for a mutant to flip.

## Evidence

Five seeded sims in `src/tui/sim_queue.rs` (order, the bound, the
numbering, the pending queue, the always answer) pass. One PTY
journey in `tests/support/tui_queue.rs`, included by
`tests/e2e_journey.rs`, drives the shipped binary: two turns queued
during a run both send in order (4 model calls, no paid call).

## Consequences

The frame answers while busy: slash commands still run, text queues.
A ninth turn is refused naming the bound, so no input is silently
dropped. Nothing sends after quit.
/ps and /stop for background commands land next (larry#556) and
extend this record.
