---
type: Design
title: Larry evaluation structure and falsifier organization
description: Evaluation lanes, portfolio sources and falsifier inventory layout; all results NOT RUN.
resource: /docs/evaluation/README.md
tags: [larry, evaluation, falsifiers, benchmarks]
generated: {by: opencode, at: 2026-09-16}
verified: {by: opencode, method: structural gate and source review of /specification.md Cell 09/10 and /research-ledger.md; nothing executed}
status: final-structure-v1.2
stale_after: 2026-12-09
updated: {by: claude-code, at: 2026-11-11, note: larry acp, the Terminal-Bench 2.0 Harbor adapter (larry#93)}
sources: [Director conversation 2026-09-16, /specification.md, /research-ledger.md]
---

# Evaluation structure

Canonical targets: /specification.md Cells 05–10; canonical source claims:
/research-ledger.md. This file organizes how the inventory is navigated.
`larry eval` scores one arm over the matched set `eval/matched-v1.json`, run
so far only on the mock endpoint (larry#87, ADR 0009). The Harness-Bench runner
(#94) and the scored runs (#95) follow. Every scored benchmark, persona run and
human study is NOT RUN; nothing here is a score.

## Terminal-Bench 2.0 through Harbor (larry#93, ADR 0035)

`larry acp` is an Agent Client Protocol agent, the kind Harbor's generic `acp`
agent drives. Each `session/prompt` runs the task instruction as a terminal
session in the task's working directory. Harbor's agent installs agents from the
ACP Registry and passes settings with `--agent-env`, so a run sets
`LARRY_POLICY=file.read,file.write,process.spawn,model.invoke,model.command`,
`LARRY_STATE_DIR`, `LARRY_MODEL_URL` and `LARRY_MODEL_KEY`. `model.command` is
named only because Harbor's task container isolates the commands. Each prompt
result carries the sealed report under `_meta.larry.report`, and the task's
`tests/test.sh` decides the reward. The registry listing is a publication the
Director names (ADR 0035), so `tests/acp_harbor_journey.rs` proves the adapter
today: a Harbor-shaped task on the mock endpoint, MEASURED there, not scored.

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
- E63–E80 bind the 2026-09-17 hostile-model security requirements (R79–R96).
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
