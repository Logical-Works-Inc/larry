---
type: ADR
title: "ADR 0101: A stopped eval arm keeps its card"
description: Why an infrastructure fault partway through larry eval prints the card of the outcomes before it, marked partial, and exits 1.
resource: /docs/adr/0101-a-stopped-eval-arm-keeps-its-card.md
tags: [larry, adr, eval, scorecard]
generated: {by: claude-code, at: 2026-11-24}
verified: {by: claude-code, method: an_infrastructure_fault_stops_the_arm_and_prints_the_outcomes_before_it red then green, 2026-11-24}
status: accepted
stale_after: 2027-05-24
sources: [larry#399, larry#321, larry#87, larry#396]
---

# 0101: A stopped eval arm keeps its card

Status: accepted, 2026-11-24 (larry#399). Extends ADR 0009.

## Context

`larry eval` stopped on the first fault that is not the arm's doing (a ledger, git, write or
process-group fault) and printed nothing. A fault on task 199 of 256 dropped 198 scored outcomes
and the tokens paid for them.

## Decision

- An infrastructure fault still ends the arm at its task. `EvalError::Stopped` carries the card
  of every outcome before it and the task's own error as its source.
- The card says `complete: false` and `stopped: {task, cause}`. A finished card says
  `complete: true` and `stopped: null`, so every card has one shape.
- `larry eval` prints the partial card on stdout, then the cause on stderr, and exits 1. When
  stdout refuses the card, both causes are kept (`EvalRun::Unprinted`).
- The `eval` command moves to `src/bin/larry/eval_cli.rs`; `src/bin/larry.rs` was at its size.

Rejected: score the fault as a refused outcome and run on. It charges the machine's fault to the
model, and the tasks after it spend money into the same fault. Rejected: one JSON line per task as
it ends. It breaks the one-line card every reader parses. Reverse it if an operator needs the
remaining tasks run past a fault: then a fault is a per-task ending with its own count.

## Consequences

- An operator reruns the set from the stopping task and adds the two cards; no paid outcome is lost.
- Falsifier: `an_infrastructure_fault_stops_the_arm_and_prints_the_outcomes_before_it`.
