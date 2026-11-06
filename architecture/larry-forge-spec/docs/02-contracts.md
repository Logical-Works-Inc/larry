---
type: Specification
title: "Larry machine contracts — churn-resistant semantic waist"
description: "Machine-focused contract catalogue refining Larry PR #1 without changing canonical product scope."
resource: /architecture/larry-forge-spec/docs/02-contracts.md
tags: [larry, forge, architecture, contracts, hyperscale]
generated: {by: ChatGPT, at: 2026-09-16}
verified: {by: ChatGPT, method: "Pinned source inspection; runtime NOT RUN"}
status: proposed-not-ratified-runtime-not-run
stale_after: 2026-10-16
sources:
  - "https://github.com/srinji-kaggss/Larry/pull/1"
---

# Machine-focused implementation contracts

Larry PR #1 remains canonical. These MC identifiers refine existing requirements, decisions and falsifiers. They are designed to make future implementations replaceable without reopening semantics.

Each contract defines: **owner, preconditions, linearization point, postconditions, failure shape, forbidden shortcuts, falsifier and evolution seam**.

## MC01 — One owner for each meaning
**Owner:** Larry task semantics; existing sibling owners remain canonical.
- Preconditions: owner and protocol revision known before mutation.
- Linearization: owner-approved contract revision enters the compatibility manifest.
- Postconditions: all adapters preserve the same semantics; no duplicate store/grant/sandbox/workspace authority.
- Failure: `OwnerUnavailable` or `UnsupportedContract`; no substitute implementation.
- Falsifier: second real compatible adapter passes the same conformance suite.
- Evolution: topology/process boundaries may change; semantic ownership may not change implicitly.

## MC02 — Durable request identity
**Owner:** Larry admission; durability by Logical-DB.
- Request ID, immutable digest, attachments, delivery mode, tenant/principal/environment and protocol version are canonicalized before acknowledgement.
- Acknowledgement linearizes only after durable admission and resource reservation.
- Same ID/same digest is idempotent; same ID/different digest is conflict.
- Steering is a new durable input promoted only at a declared safe boundary.
- Falsifier: crash after durable admission but before ACK followed by replay produces one accepted identity and no duplicate work.

## MC03 — Effect identity is separate from task identity
- Every externally visible mutation has a stable effect ID, effect class, owner, target identity, authority digest and idempotency semantics.
- Task retries never mint a fresh logical effect silently.
- Compensation uses a new effect ID linked to the original failed effect.
- Falsifier: timeout after receiver-side success cannot produce a blind duplicate retry.

## MC04 — Typed effect state machine
Canonical states: `Proposed`, `AwaitingAuthority`, `Reserved`, `Dispatched`, `Observed`, `OutcomeUnknown`, `Verified`, `Failed`, `Cancelled`, `Finalized`.
- Payload bytes, stdout, provider text and model prose can never directly select these states.
- Unknown external outcome is not success, failure or retry permission.
- Falsifier: tool output containing a sentinel word cannot alter lifecycle state.

## MC05 — Authority is rechecked at the effect boundary
**Owner:** Secure-Authority.
- Model classification can propose intent but cannot enlarge authority.
- Child capability <= parent capability intersected with current resource policy, budget and expiry.
- Revocation applies at the next effect boundary even if context/prompt caches are stale.
- Falsifier: stale cached context containing an earlier grant cannot authorize a revoked effect.

## MC06 — Resource conservation
For every run tree: `sum(child reservations + retries + speculation + background work) <= admitted parent envelope`.
- Budgets include CPU, memory, storage, network, tokens, money, wall time and concurrency where applicable.
- Retries consume the original operation budget; nested layers cannot reset counters.
- Falsifier: outage across provider/tool/scheduler layers cannot amplify attempts beyond declared run budget.

## MC07 — Child admission and tracking are atomic semantics
- A child cannot begin effect-capable work before it is registered to an owning scope and budget reservation.
- Closing a scope first prevents new child admission, then drains/reconciles existing children.
- Failure to observe a child is explicit unresolved state, not silent disappearance.
- Falsifier: race last-slot admission versus scope close produces either admitted+tracked or refused, never spawned+untracked.

## MC08 — Cancellation is lifecycle settlement
- Cancel stops new dispatch immediately.
- Already dispatched effects enter observation/reconciliation.
- Descendants, leases and reservations settle before terminal cancellation.
- Surface-level task abort is not a substitute for domain cancellation.
- Falsifier: TUI, CLI and remote API cancellation yield equivalent domain disposition on the same run trace.

## MC09 — Completion is obligation discharge
- Admission freezes typed obligations and verifier classes.
- Exit code, provider finish reason, model statement and task output are evidence inputs only.
- Required obligations must be `Satisfied`, `Failed`, `Inconclusive`, `Unavailable` or explicitly policy-waived; only accepted states discharge completion according to contract.
- Falsifier: successful process exit with failed domain verification cannot finalize `Completed`.

## MC10 — Evidence is first-class and provenance-bound
- Evidence records source identity, version, observation time, capture completeness, digest, scope and verifier.
- Preview truncation and producer capture loss are distinct.
- Missing evidence stays unknown; no reconstructed guess becomes a receipt fact.
- Falsifier: expired artifact cannot be silently regenerated and cited as original evidence.

## MC11 — Durable event history; projections are disposable
- Canonical commitments, corrections, effects, authority decisions and outcomes are durable typed records.
- Graphs, summaries, prompts, embeddings and caches are projections.
- Rebuilding a projection may change ranking but cannot erase unfinished obligations.
- Falsifier: compaction crash with pending approval and incomplete tool call resumes without orphaning or replaying effects.

## MC12 — Context epochs bind semantics
- Each model turn declares instruction, policy, tool schema, source and context-manifest revisions.
- Prompt cache reuse cannot keep superseded authority or policy alive.
- A correction acknowledged as globally/shared-visible creates a revision fence for subsequent turns.
- Falsifier: projection refresh failure after acknowledged correction forces visible wait/refusal rather than silent stale execution.

## MC13 — Configuration is effective state, not file precedence folklore
- Every effective setting records source, scope, revision and override reason.
- Preference precedence cannot widen authority.
- Candidate config validates completely before atomic activation at a declared checkpoint.
- Failure preserves last-known-good effective revision.
- Falsifier: malformed project config cannot silently disable security or expand tool access.

## MC14 — Tool identity is version-bound
- Tool advertisement binds executable identity, schema hash, environment, tenant and lifecycle generation.
- Stale registrations are rejected after replacement.
- Untrusted extensions run only through Nova with no ambient host credentials.
- Falsifier: replacement after advertisement cannot execute an already approved call against a different binary/schema.

Continued in [02-contracts-2.md](02-contracts-2.md).
