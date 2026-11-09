---
type: Design
title: Larry final proposed specification and completion notebook
description: Continuity-first assistant runtime PRD, candidate decisions, architecture, evidence gates and implementation sequence.
resource: /specification.md
tags: [larry, prd, architecture, continuity, efficiency, reinforcement-learning]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: bounded source inspection and external critique; not runtime verification}
status: final-proposed-v1.2-awaiting-architecture-approval
stale_after: 2026-10-15
updated: {by: claude-code, at: 2026-11-09, note: the Rocco rename finished (ADR 0005) and the runtime falsifier line made current}
sources:
  - Director requirements and corrections in conversations 2026-09-15 and 2026-09-16
  - /research-ledger.md
  - /Users/srinji/.okf/governance/contract/frontier-universal-assurance.md
---

# Larry specification v1.2 — frontload the cognitive work

**Approval: pending. Runtime falsifiers: those /INVARIANTS.md names run in CI; the rest NOT RUN. Human participants: 0.**
This is a concrete decision package, not a collection of equally ranked options.
Except for the expressly settled Director decisions in Cell 12, decisions below
are recommendations until ratified. Metrics marked TARGET
are acceptance proposals, not measurements or promises. Source claims and source
inspection are distinguished in the [research ledger](research-ledger.md).

Larry is the Director-selected name; the repository began as the private Rocco,
was renamed Larry on 2026-11-06 (ADR 0005) and is the public alpha
`Logical-Works-Inc/larry`. Forge was the earlier scratchpad name, not a second runtime. The existing
`forge-harness` and `forge-sdk` remain historical
source repositories only. Final means the current requirements, dispositions and
execution plan are assembled; it does not mean every proof or product test passed.

## Parts

1. [Cell 00 — strategy and PRD](/docs/specification/01-cell-00-strategy-and-prd.md)
2. [Harness-review requirements](/docs/specification/02-harness-review-requirements.md)
3. [Cell 02 — candidate architecture graph](/docs/specification/03-cell-02-candidate-architecture-graph.md)
4. [Effect state machine: interruption is not rollback](/docs/specification/04-effect-state-machine-interruption-is-not.md)
5. [D04 — event history is authoritative; graph and context are projections](/docs/specification/05-d04-event-history-is-authoritative-graph.md)
6. [D09 — coordinator delegates work contracts, not cloned conversations](/docs/specification/06-d09-coordinator-delegates-work-contracts.md)
7. [D17 — collaboration federates contracts, not private minds](/docs/specification/07-d17-collaboration-federates-contracts.md)
8. [D23 — evolve at contracts, not through permanent generic machinery](/docs/specification/08-d23-evolve-at-contracts-not-through.md)
9. [D28 — adapters, not harness clones or ambient extensions](/docs/specification/09-d28-adapters-not-harness-clones-or.md)
10. [D33 — user controls are real, and memory waits for its owner](/docs/specification/10-d33-user-controls-are-real-and-memory.md)
11. [D39 — budget reservations are leases](/docs/specification/11-d39-budget-reservations-are-leases.md)
12. [D50 — consent appears only at meaningful boundary changes](/docs/specification/12-d50-consent-appears-only-at-meaningful.md)
13. [Cell 05 — resource model and initial targets](/docs/specification/13-cell-05-resource-model-and-initial.md)
14. [Cell 08 — human outcomes and experience contract](/docs/specification/14-cell-08-human-outcomes-and-experience.md)
15. [Cell 09 — initial falsifier inventory](/docs/specification/15-cell-09-initial-falsifier-inventory.md)
16. [Cell 10 — weak-model and truthful persona evaluation](/docs/specification/16-cell-10-weak-model-and-truthful-persona.md)
17. [Cell 11 — completion sequence and experiment outputs](/docs/specification/17-cell-11-completion-sequence-and.md)
18. [Open architecture question recorded 2026-09-20](/docs/specification/18-open-architecture-question-recorded-2026.md)
