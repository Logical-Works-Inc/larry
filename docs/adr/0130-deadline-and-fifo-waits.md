---
type: ADR
title: "ADR 0130: deterministic waits for the deadline and FIFO journeys"
description: "The deadline journey reads its group from the sealed report and the FIFO release blocks until the reader holds it."
resource: /docs/adr/0130-deadline-and-fifo-waits.md
tags: [larry, adr, executor, tui, determinism]
generated: {by: opencode, at: 2026-11-27}
verified: {by: opencode, method: lgwks-box cargo nextest run --locked -p larry-demo --stress-count 200 on toronto, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#552, INVARIANTS.md, tests/executor_journey/ceiling.rs, tests/e2e_journey/pane.rs]
---

## Context

The fallback workspace run caught two flakes once in 1467 tests on a loaded box.
`a_task_past_its_deadline_seals_rejected_with_its_group_reaped` failed with
`Os { code: 2, kind: NotFound }`: it read a pid file the killed shell writes, and
the deadline may kill the shell before `echo` flushes, so the read raced the kill.
`tui_journey_pane_streams_first_line_before_fifo_release` missed `second`: the
release opened the FIFO read-write, which never blocks, so a release before `cat`
opened it was lost and `cat` blocked past the wait.

## Decision

The deadline journey derives the reaped group from the sealed report's own `pgid`,
never from a pid file: the report names the group the executor recorded, reaped and
removed, so no filesystem read races the kill. The FIFO release opens write-only,
which blocks until the command holds the FIFO for reading, inside `PATIENCE`: the
reader's open is the event, never a sleep. Closing at once hands that reader EOF.
The drop guard keeps its non-blocking read-write release, so teardown never hangs.

## Consequences

A spent deadline still seals `timed_out: true` with `deadline_passed` beside the
results, exit 2, and the ledger holds no row; the only change is where the reaped
pgid comes from. A release now waits for the reader instead of racing it, so the
second line cannot be lost to an early release; a command that never opens the
FIFO fails the release inside the bound by name. No production code changed.

## Proof

- `ceiling::a_task_past_its_deadline_seals_rejected_with_its_group_reaped`,
  200/200 on toronto
  (`cargo nextest run --locked -p larry-demo --stress-count 200`, 364.4 s).
- `pane::tui_journey_pane_streams_first_line_before_fifo_release`, 200/200 on
  toronto (same form, 177.2 s).
- `cargo clippy --workspace --all-targets --locked -- -D warnings` clean.
- The leaked `larry` orphan the fallback run left on toronto (nextest "1 leaky",
  unreaped PTY child holding the box lock) was reaped; no leaky in either loop.

## Traceability

Issue #552. Invariants: every test wait ends on its event or on one deadline
(the reader's open; the sealed report's pgid). The rejected design was retrying
the pid-file read or lengthening a bound, which keeps the race instead of waiting
on the event the verdict is drawn from.
