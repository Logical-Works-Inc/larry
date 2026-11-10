---
type: ADR
title: "ADR 0032: blind selection and lessons"
description: Why the final harness is chosen blind from a seed the journal derives, why a lesson needs journal evidence, and what waits on Logical-DB.
resource: /docs/adr/0032-blind-selection-and-lessons.md
tags: [larry, adr, contracts, selection, lessons, epic-60]
generated: {by: claude-code, at: 2026-11-10}
verified: {by: claude-code, method: nextest run of crates/larry-contracts/tests/sim_selection.rs and the workspace}
status: accepted
stale_after: 2027-02-10
sources: [larry#64, larry#60, docs/adr/0030-grader-blind-learning.md, docs/adr/0031-recursion-ledger.md, AGENTS.md]
---

# 0032: blind selection and lessons

Status: accepted, 2026-11-10 (larry#64 part 1, epic larry#60)

## Context

After a task Larry chooses one final harness without knowing where each candidate came from, records
why it won and one transferable lesson, and later tasks in the same domain read those lessons.
Harness² gives candidates anonymous ids from a seeded shuffle, drops non-answers with the reason, and
falls back to a delivered candidate when the selector fails. Its library stores a lesson only with
cue, lesson and evidence, keeping the full adopted tree.

## Decision

- The seed is BLAKE3 over a domain tag, the task digest, the mode, k and the tenant
  (`selection_seed`). It reads no clock and no RNG, so replay picks the same winner. A
  deterministic Fisher-Yates driven by the seed's bytes numbers the candidates.
- Candidates are the delivered harnesses of a settled `RecursionLedger`, deduplicated in first-seen
  order: each composition in parallel mode; each round base and the last composition in sequential mode.
- The blind boundary is a type. `SelectorInput` holds an `ImproverContext` (grader-blind, ADR 0030)
  and `CandidateId`s. It has no hash field, and a test checks no candidate hash appears in it.
- The selector's answer is an input value; no model is called here. A missing answer, an unknown id,
  an empty or over-long reason, or a selector failure is a recorded `NonAnswer`. The candidate the
  seed put first wins, as a recorded fallback, and nothing raises. A recursion that has not settled
  is the one refusal (`NotSettled`), because nothing was delivered.
- `Lesson` carries tenant, domain, cue, lesson, evidence, confidence and the adopted `HarnessHash`.
  Evidence is a non-empty list of `CausalRef`s (run and sequence), never free text. `Lesson::new`
  refuses each missing or out-of-bounds field with its own `LessonError`. `Lesson::vet` takes a
  `RunIndex` and refuses a lesson citing a run the journal does not know. Confidence is a three-level
  enum, so no float.
- `LessonView::for_scope(tenant, domain, lessons)` is a pure filter that cannot return another
  tenant's lesson.

Rejected:
- Storing a summary of the adopted harness: the hash names the full tree, which is kept by #61.
- Free-text evidence: it cannot be checked against the journal.
- A JSON file or SQLite side store in place of Logical-DB (AGENTS.md).

## Part 2: the runtime (larry#64)

- A selection is an ordinary `Submit` run, the selector session. `WorkProposal` and `IntentPayload`
  gain an optional `SelectionStep` (the settled plan and the selector's answer); no new `CommandKind`
  or `EventKind`. Reading: the selector session is outside R = 4k and P = 3k. Neither the issue nor
  ADR 0031 counts it, and the ledger never sees it, so the identity is unchanged.
- The fold keeps one selection per (tenant, plan). The intent derives it with `prepare` and `decide`
  over the plan's ledger, using the run table's tenant for the seed, never the proposal's. It is
  reserved while the run is in flight and becomes a fact only on a succeeded receipt of a run that
  is not cancelling. A failed or cancelled selector run frees the plan for another.
- Admission asks the same rule set before the write: an unsettled plan is `NotSettled`, and a second
  selection (recorded or in flight) is `AlreadySelected`. The journal is unchanged on refusal. Replay,
  including a reopened journal file, gives the same `Selection`.
- `Selection`, `SelectionReason`, `NonAnswer`, `CandidateId` and `SelectorAnswer` have serde, with
  variant names as the tags. The widest record, a selector run with a 512-byte escaping reason, is
  13,862 of 65,524 bytes (MEASURED, `durable/fit.rs`).
- `Runtime::vet_lesson` checks a lesson's cited runs against the fold: a run counts only if the
  journal holds it for the lesson's own tenant, so another tenant's run is as unknown as a missing one.
  Nothing is stored.

## Consequences

- Waits on Logical-DB: the lesson store, retrieval ranking and decay, and the concurrent-write
  guarantee that two tenants in one domain never see each other's lessons under concurrent writes.
  This part proves only the pure scope filter over seeded interleavings. No store, file or table exists.
- Part 2 (below) wires the selection and the lesson vet; the lesson store itself stays unbuilt.
- Covered by `tests/sim_selection.rs`: 1,000 seeds of same seed, same selection; the seed changes with
  task, mode, k and tenant; the selector never sees a hash; every non-answer falls back; lesson
  refusals; a 300-seed two-tenant scope test.
