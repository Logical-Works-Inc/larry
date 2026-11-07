---
type: ADR
title: "ADR 0011: the demo and native PTY journeys are Rust tests on the shipped binaries"
description: Why the Python demo, native PTY and probe suites moved into Rust tests driven through script(1).
resource: /docs/adr/0011-pty-journeys-in-rust.md
tags: [larry, adr, journeys, pty, python-ban]
generated: {by: claude-code, at: 2026-11-07}
verified: {by: claude-code, method: cargo nextest run --workspace --locked -E 'binary(demo_journey) | binary(e2e_journey)'}
status: accepted
stale_after: 2027-02-07
sources: [larry#53, larry#147, docs/evidence/workspace-final-metrics.json, ECMA-48, POSIX kill(2)]
---

# 0011: the demo and native PTY journeys are Rust tests on the shipped binaries

Status: accepted, 2026-11-07 (larry#53 step 2)

## Context

`tools/test_demo_cli.py`, `tools/test_native_cli.py` and `tools/probe_workspace.py` were Python, which
product repositories may not hold (Director 2026-10-13). They also named the pre-rename binaries, so no
lane had run them since larry#131.

## Decision

- `tests/demo_journey.rs` drives `larry-demo` over stdin and stdout: the four Python journeys, plus two
  seeded simulations. One holds every frame and refusal to the in-process `Demo` for 32 command streams.
  The other kills the driver with SIGKILL at a seeded command and checks the next session starts Ready.
- `tests/e2e_journey.rs` holds one `Pty` driver for the native view. `script(1)` supplies the terminal, as
  the existing journey already did; a reader thread is owned by the test's scope; the Python suite's screen
  model is ported rule for rule. A match counts only after the view has answered the last keys.
- The shell prints its own PID before it execs the binary, so the kill journey signals the view itself.
- The probe is a test that prints its figures as one JSON line: first paint, 20 timed round trips, a 5 s
  idle window (the Python minimum), sampled RSS and quit-to-exit. Rejected: a PTY crate, which needs a
  `gap` issue and adds a test-only edge, and keeping the Python suites behind a Rust wrapper.

## Consequences

- The ci rust job and the sim.yml e2e job run the journeys, and so does `cargo local-ci`. None of them
  runs Python for these journeys.
- `docs/evidence/workspace-final-metrics.json` stays as the Python baseline the probe is compared with.
- The figures are host-specific debug-build measurements, never fleet or human acceptance.
