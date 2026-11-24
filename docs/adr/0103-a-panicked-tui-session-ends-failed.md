---
type: ADR
title: "ADR 0103: A panicked TUI session ends its task as Failed"
description: Why the TUI worker joins a session thread whose channel closed with no Ended and ends the task as Failed naming the panic, and why tui::run reaps the worker on every return.
resource: /docs/adr/0103-a-panicked-tui-session-ends-failed.md
tags: [larry, adr, tui, worker, panic]
generated: {by: claude-code, at: 2026-11-24}
verified: {by: claude-code, method: sim_tui_worker_a_panicked_session_ends_failed_with_its_message red then green, 2026-11-24}
status: accepted
stale_after: 2027-05-24
sources: [larry#412, larry#239, larry#343]
---

# 0103: A panicked TUI session ends its task as Failed

Status: accepted, 2026-11-24 (larry#412). Extends ADR 0045.

## Context

- The worker sent `Ended` only when `run_session` returned. A session thread that panicked
  sent nothing more, so `Worker::next` read `None` forever and the frame showed a running
  task. Esc set a cancel flag that no running code read.
- `Drop` saw the panic only as `join().is_err()` and lost its message.
- Release builds unwind on panic, so the fault is live in the shipped binary.

## Decision

- `Worker::next` is `wait(Duration::ZERO)`, so the frame and the sims read through one path.
- A channel that closes with no `Ended` is a thread that panicked. The read joins it and ends
  the task as `Ended::Failed("the session stopped on a panic: <payload>")`, once. The payload
  text comes from `crate::panic_text`, the one reader of panic payloads.
- `tui::run` owns the worker and reaps it with `finish` on every return. A panic at quit is
  `TuiError::Session`, returned after the screen is restored. The drop then holds a thread
  only during the frame's own unwind; that panic is the failure the process reports, std's
  panic hook named the session's panic as it happened, and the drop reaps the thread.
- A private `start(job)` seam runs any job on the session thread; `spawn` is its production
  caller.

Rejected: a second reaper thread per session that joins the session thread and sends
`Ended`. It doubles the threads per task and still needs the same join and payload read.
`catch_unwind` around `run_session` is a banned panic synonym.

## Consequences

- A panic in a session never leaves a spinner. The person sees why the task stopped.
- Tests: `tui::sim_tui::worker_panics` (`sim_tui_worker_a_panicked_session_ends_failed_with_its_message`,
  `sim_tui_worker_drop_joins_the_session_before_it_returns`,
  `sim_tui_worker_finish_names_a_panic_and_only_a_panic`). A channel handshake fixes every
  channel state the reads see, so the sims read no clock.
- Reverse this if `run_session` gains a typed fault for every path that now panics, and the
  workspace forbids unwinding in release builds.
