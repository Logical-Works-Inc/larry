---
type: Design
title: Larry evaluation structure and falsifier organization
description: Evaluation lanes, portfolio sources and falsifier inventory layout; all results NOT RUN.
resource: /docs/evaluation/README.md
tags: [larry, evaluation, falsifiers, benchmarks]
generated: {by: opencode, at: 2026-09-16}
verified: {by: opencode, method: structural gate and source review of /specification.md Cell 09/10 and /research-ledger.md; nothing executed}
status: final-structure-v1.1
stale_after: 2026-10-16
sources: [Director conversation 2026-09-16, /specification.md, /research-ledger.md]
---

# Evaluation structure

Canonical targets: /specification.md Cells 05–10; canonical source claims:
/research-ledger.md. This file organizes how the inventory is navigated.
Every benchmark, persona run and human study is NOT RUN; nothing here is a
score or an acceptance claim.

## Lanes

- **Lane A — same-model harness comparison** on frozen tasks, seeds,
  budgets and tool access, on a contamination-controlled private suite
  (D15/D41, E61). Public-benchmark contamination is a declared limitation,
  never averaged away.
- **Lane B — product baseline** against Claude Code with all differences
  reported; a stronger-teacher-assisted variant is separately named and
  fully costed (D15).
- **Lane C — persona and human experience**: MatrAIx as one external
  source (D34), simulator fidelity kept separate from assistant adherence,
  adherence kept separate from task reward, consenting-human pilots (D15,
  Cell 10).

## Falsifier inventory layout

- E01–E27 bind the original requirement inventory (R01–R43).
- E28–E54 bind the harness-review requirements (R44–R70); nine of them are
  planned crash/recovery journeys.
- E55–E62 bind the 2026-09-16 SOTA-gap requirements (R71–R78): reducer
  replay identity, permanent reconciliation dispositions, ecosystem
  adapters, telemetry cost attribution, budget-lease reclaim, memory edge
  semantics, contamination control and adversarial honesty/revocation.
- Allocation to execution cells C00–C12 is recorded in Cell 11 and checked
  mechanically; every case must run before full product acceptance.

## Evaluation hardening (2026-09-16)

Process-level recovery scoring (localize and repair, not only final
outcome), a revocation-enforcement falsifier, a post-failure honesty
falsifier and interacting-fault combined journeys enter through D41. The
inventory carries a derivation note so its size is auditable rather than
asserted (R77/R78, D41).

## File-specific excellence record

Requirements R19/R43/R65/R77/R78 and all E anchors. Axes: generalized and
frontier evaluation obligations. Predicted outcome: auditable evaluation
structure with zero invented results. Verification: structural gate, Cell 11
allocation check, crash-fraction check. Unresolved: every measurement; the
inventory is planned, not executed.
