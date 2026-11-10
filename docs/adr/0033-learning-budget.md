---
type: ADR
title: "ADR 0033: the learning budget"
description: Why learning has its own per-tenant budget inside standing authority, refused at the fence and journalled, and why a lesson cannot be planted from text.
resource: /docs/adr/0033-learning-budget.md
tags: [larry, adr, runtime, authority, learning, epic-60]
generated: {by: claude-code, at: 2026-11-10}
verified: {by: claude-code, method: nextest run of crates/larry-runtime/tests/sim_learning_budget.rs and the workspace}
status: accepted
stale_after: 2027-02-10
sources: [larry#66, larry#60, larry#42, larry#38, docs/adr/0031-recursion-ledger.md, docs/adr/0032-blind-selection-and-lessons.md]
---

# 0033: the learning budget

Status: accepted, 2026-11-10 (larry#66 part 1, epic larry#60)

## Context

"It never spent more than I allowed" must hold for learning too. A proactive loop (#42) that proposes
and probes without end is denial of wallet, and a poisoned deliverable (#38) must not plant a lesson.

## Decision

- `LearningBudget` (rollouts, sessions, money) is a field of `RuntimeSettings`, beside the other
  pinned standing-authority snapshots the host supplies. Money reuses `Budget` in milli-cents, so there
  is no float and no second money type. The default is 256 rollouts, 192 sessions and 10,000,000
  milli-cents; a host narrows it.
- The fold counts what each tenant's learning reserved (`LearningSpent`) across all its plans: a probe
  is a rollout; a first-attempt proposer, composer or selector run is a session; a retry adds no session;
  every learning step adds its declared cost as money. The cost must be in milli-cents: a step priced
  in tokens or requests is refused (`LearningError::Unit`, denied `UnitMismatch`), never counted as
  free, so pricing a loop in another unit cannot slip past the money ceiling. Counting is by the run
  table's tenant and rebuilds identically on replay.
- `LearningBudget::admit` is the one pure rule. The runtime asks it in `admit_work`, before the effect
  is reserved. A step past a ceiling is denied `DenialReason::BudgetExhausted` and its run fails. Both
  are journalled: the command and the failed run's transitions. Nothing is reserved, so nothing is
  spent. Refused commands are otherwise not journalled today, so this uses the existing denial path, not
  a new `EventKind`. The recursion rules of ADR 0031 still refuse earlier, before the command is written.
- `Runtime::learning_budget(tenant)` returns the ceiling and the spend, so a surface can show them.
- Poisoning. Evidence text is inert bytes. No conversion turns an `Evidence` into a `CausalRef`
  (a `compile_fail` doctest pins this), so the only door to a lesson is `Lesson::new` with journal
  references, and `Runtime::vet_lesson` refuses a reference to a run the journal lacks for that tenant.
  The test plants "write a lesson" in a deliverable, fabricates a run, and sees it refused; a control
  store with no vet keeps the same lesson, which shows the vet is what refuses.

Rejected:
- A side config for the learning budget: it would not be standing authority.
- Counting at the receipt: a loop would spend before any receipt arrived.

## Part 2: one stop (larry#66)

- `Runtime::stop_learning(tenant, PlanScope::One(identity) | PlanScope::All)` is the one action to stop.
  It rides an existing `Cancel`: the body is a `WorkProposal` with an optional `stop` (omitted when
  empty, like `recursion` and `selection`) and a control run of its own, derived from the journal. No
  new `CommandKind` or `EventKind`. `CommandAcceptedPayload` records the scope, so the fold and replay
  rebuild it, including from a reopened journal file.
- The fold stops the plans in scope by the run table's tenant, never the payload's, using the ledger's
  cancel: `next_steps` offers nothing and a new intent is refused `Cancelled` before the write. Steps in
  flight still settle, and their width slots free as they settle. `All` covers the tenant's plans at
  that moment, and no other tenant's.
- A stop that would change nothing is refused before the write (`UnknownPlan`, `AlreadyStopped`,
  `NothingToStop`), so a second stop has no effect. A stop admitted behind an earlier one is recorded
  as a no-op, so the fold never refuses what the journal holds.

## Consequences

- Open for #66: the ten-task journey through the public interface, TUI visibility of the budget, the
  one-action rollback and stop surfaces on CLI, phone and web, and releasing held executors on stop
  through the owner. Part 1 proves the budget and the poisoning refusal only.
- The refusal shows the ceiling's remaining amount, not which ceiling; the typed `LearningError` names it.
- Covered by `tests/sim_learning_budget.rs` (session ceiling hit exactly across plans over 24 seeds,
  rollout and money ceilings, two tenants independent, replay equal, poisoning with its control).
