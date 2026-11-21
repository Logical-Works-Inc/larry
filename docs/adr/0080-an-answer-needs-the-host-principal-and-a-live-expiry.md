---
type: ADR
title: "ADR 0080: an answer needs the host principal and a live expiry"
description: Why a question answer counts only from the offer's host-authenticated principal, never from the requester, and only while its question is live.
resource: /docs/adr/0080-an-answer-needs-the-host-principal-and-a-live-expiry.md
tags: [larry, adr, runtime, approval, security]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: e30_a_client_that_asks_and_answers_itself_is_refused_and_nothing_is_reserved and e30_an_approval_used_after_expiry_is_not_approved red before the change and green after, 2026-11-21}
status: accepted
stale_after: 2027-02-21
sources: [larry#331, larry#98, larry#321, /INVARIANTS.md]
---

# 0080: an answer needs the host principal and a live expiry

Status: accepted, 2026-11-21 (larry#331, found by the 2026-11-19 review).

## Context

E30 lets an `Ask` hold a `Submit` until a person answers. Two fields decided who
counted, and the client wrote both:

- `Question.respondent` named the worker whose answer counts, and
  `Answer.respondent` repeated it. Admission compared the two payload fields, so
  any client could ask a question and answer it itself.
- `Asked::status` returned `Approved` as soon as a decision existed. It read no
  time, so an answer taken before `expires_at` approved a `Submit` reduced after
  it, and expiry became consent by delay.

The offer at the ingress carried two booleans a client declared. A boolean names
no one, so the runtime could not tell the person from the process that asked.

## Decision

- `ProtocolOffer` carries an optional `WorkerId` principal that the host sets
  with `ProtocolOffer::authenticated`. It is a typed identity, never a boolean
  and never a payload field.
- `ProtocolOffer::serves_questions` refuses an `Ask` or an `Answer` from an
  offer with no principal, as `ProtocolError::NoPrincipal`, after the existing
  capability and headless checks. The executor's offer binds the worker it
  holds (`authenticated(holder)`) and still asks no question.
- `check_ask` refuses a question whose requester is its respondent, as
  `QuestionError::RequesterIsRespondent`.
- `check_answer` takes the offer's principal. The answer's `respondent` must
  equal both the question's respondent and the principal, so a client cannot
  name an identity it does not hold; a principal that is the question's
  requester is refused. Both refusals write and reserve nothing.
- `Asked::status` returns `Approved` only for a use before `expires_at`. An
  approval used at or past expiry is `TimedOut`, and its `Submit` is denied
  `NotApproved`.
- `Book::fold` applies the same binding when it reads an answer from the
  journal, so a restore re-derives the verdict. The journaled `respondent` is
  the principal admission authenticated, because admission refused any other.
- Tests: `e30_a_client_that_asks_and_answers_itself_is_refused_and_nothing_is_reserved`
  and `e30_an_approval_used_after_expiry_is_not_approved`, and the existing
  E30 cases keep their assertions.

## Consequences

- Asking and answering now needs two identities: the worker that asks and the
  host-authenticated principal that answers.
- An approval has the lifetime of its question. A long wait needs a new
  question, not an old answer.
- `ProtocolOffer::new` keeps its shape, and an offer built with it can submit
  commands but can never raise or answer a question.

## Rejected

- Comparing the two declared `respondent` fields, as before. The client writes
  both, so the check proves nothing.
- A client-declared `is_human` boolean. It names no identity and any client can
  set it.
- Requiring the principal in `ProtocolOffer::new`. It rewrites the executor's
  command-only offer and three unrelated test helpers, and buys no safety over
  the fail-closed `serves_questions` check.
- Refusing the answer at admission when it is already expired. The answer
  arrives before expiry; the use, the reduction of the `Submit`, is later.

Reversal: none expected. If a host must raise questions without a worker
identity, it gets one from the authority layer first.
