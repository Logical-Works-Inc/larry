---
type: ADR
title: "ADR 0027: durable scoped questions"
description: Why an E30 question and its answer are journaled commands admitted through E28, and why expiry, cancellation or a headless client never becomes consent.
resource: /docs/adr/0027-durable-scoped-questions.md
tags: [larry, adr, runtime, approval, e30, questions]
generated: {by: claude-code, at: 2026-11-09}
verified: {by: claude-code, method: nextest run of tests/e30_questions.rs and the workspace}
status: accepted
stale_after: 2027-02-09
sources: [larry#98, larry#25, specification R46, D25, E30, Cell 02, docs/adr/0025-durable-admission.md]
---

# 0027: durable scoped questions

Status: accepted, 2026-11-09 (larry#98, the E30 port of PR #25)

## Context

E30 asks for concurrent questions, an expired approval, an answer after cancellation and no interactive
client, with exact respondent and effect binding and unrelated waits unaffected. Cell 02 says "Durable
scoped question; expiry is not consent", and D25 binds requester, respondent, effect digest, schema,
expiry and answer scope. PR #25 kept its question book in process memory, so a kill -9 lost every
pending question and any answer a human had already given.

## Decision

- `CommandKind` gains `Ask` (tag 3) and `Answer` (tag 4), and `DenialReason` gains `NotApproved`. Both
  enums are closed contracts, so this is a specification change.
- A question and an answer are commands. They enter only through `submit_protocol` (local `submit`
  refuses them), which first requires `HumanQuestion` from a client that is not headless. `Runtime::admit`
  then checks them at the instant of admission and journals and syncs them (E28). The question book
  (`runtime::approval::Book`) is a fold over the journal's commands, and `restore` rebuilds it.
- A question names one `Submit` by its run, operation, `CommandContent::digest` and schema. While the
  question is pending, that `Submit` is held, and only an `Ask`, `Answer` or `Cancel` of the same run
  passes it. Other runs are unaffected.
- An answer counts only from the question's tenant and respondent, about the question's exact subject,
  once, before expiry, on a run that is open with no cancellation pending, and only for scopes the
  question offered. Each refusal is a typed `QuestionError` and writes nothing.
- A `Submit` is admitted only when it has no question, or when it is approved with its exact digest
  and every scope inside the approved set. A rejection, a timeout, a cancellation or a different body
  is denied `NotApproved` and reserves nothing.
- At most `MAX_PENDING_QUESTIONS` (256) questions are pending at once. A scope set needs no bound of
  its own, because `Scope` is closed at five variants.

Rejected:
- PR #25's in-memory book: it breaks R46 and Cell 02 at the first restart.
- A new `EventKind` for answers. The command already is the durable fact, and a second record would be
  a second truth (INVARIANTS: `EventKind` is closed).
- Treating expiry as rejection-by-default written to the journal. `TimedOut` is derived from the
  question's expiry and the clock, so a restored runtime agrees without a timer fact.

## Consequences

- A host that needs approval must ask before submitting the work, and must have a human surface.
  `AwaitingAuthority` for a headless host is the typed protocol refusal.
- Covered by `tests/e30_questions.rs` (`e30_concurrent_questions_keep_exact_bindings_and_unrelated_waits`,
  `e30_expiry_and_cancellation_never_become_consent`, `e30_a_headless_client_is_refused_and_nothing_is_written`,
  `e30_an_approval_covers_only_its_exact_content`, `e30_pending_questions_refuse_past_their_bound_and_recover`,
  `sim_e30_a_question_and_its_answer_survive_a_crash`).
