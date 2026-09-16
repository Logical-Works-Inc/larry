---
type: Specification
title: "Rocco machine contracts — churn-resistant semantic waist"
description: "Machine-focused contract catalogue refining Rocco PR #1 without changing canonical product scope."
resource: /architecture/rocco-forge-spec/docs/02-contracts.md
tags: [rocco, forge, architecture, contracts, hyperscale]
generated: {by: ChatGPT, at: 2026-09-16}
verified: {by: ChatGPT, method: "Pinned source inspection; runtime NOT RUN"}
status: proposed-not-ratified-runtime-not-run
stale_after: 2026-10-16
sources:
  - "https://github.com/srinji-kaggss/Rocco/pull/1"
---

# Machine-focused implementation contracts

Rocco PR #1 remains canonical. These MC identifiers refine existing requirements, decisions and falsifiers. They are designed to make future implementations replaceable without reopening semantics.

Each contract defines: **owner, preconditions, linearization point, postconditions, failure shape, forbidden shortcuts, falsifier and evolution seam**.

## MC01 — One owner for each meaning
**Owner:** Rocco task semantics; existing sibling owners remain canonical.
- Preconditions: owner and protocol revision known before mutation.
- Linearization: owner-approved contract revision enters the compatibility manifest.
- Postconditions: all adapters preserve the same semantics; no duplicate store/grant/sandbox/workspace authority.
- Failure: `OwnerUnavailable` or `UnsupportedContract`; no substitute implementation.
- Falsifier: second real compatible adapter passes the same conformance suite.
- Evolution: topology/process boundaries may change; semantic ownership may not change implicitly.

## MC02 — Durable request identity
**Owner:** Rocco admission; durability by Logical-DB.
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

## MC15 — Provider neutrality preserves transcript semantics
- Canonical transcript maintains tool-call/result pairing and provider-independent message semantics.
- Provider-native opaque metadata is adapter-local unless explicitly standardized.
- Provider-executed tools are not replayed locally.
- Retry cannot silently switch provider/account/region/model/data recipient.
- Falsifier: stop reason plus outstanding tool calls cannot be interpreted as task completion.

## MC16 — Workspace mutation is owned by Moo
- Rocco proposes/adjudicates work; Moo owns workspace transaction/conflict semantics.
- Preview/apply/undo are revision-aware and preserve unrelated human changes.
- Raw filesystem mutation that bypasses the owner contract is forbidden for effectful product paths.
- Falsifier: human edit between preview and apply yields conflict or safe rebase, never overwrite.

## MC17 — Compute containment is owned by Nova
- Execution declares profile, CPU/RAM/storage/network/time/spend bounds and secret scope.
- Unsupported isolation returns explicit refusal; no fallback to less-confined host execution.
- Local model serving and task sandbox are independently admitted resources.
- Falsifier: unavailable VM/Wasm profile cannot silently execute natively.

## MC18 — Internet access is owned by Sentinel
- Network reads are effects with confidentiality, observability, cost, freshness and idempotency metadata.
- Speculation defaults to local/pure work; external speculation requires standing capability.
- Falsifier: speculative GET that could leak or charge is blocked without policy even if HTTP method is GET.

## MC19 — Model architecture is replaceable
- Core semantics do not depend on textual chain-of-thought, specific tokenizer, OpenAI-compatible transport or autoregressive decoding.
- Model interface exposes typed observations/proposals/tool requests, capabilities, cancellation, context/resource limits and accounting.
- Future continuous-latent, multimodal, symbolic/neural or learned planners fit behind this seam.
- Falsifier: replacing model family does not alter authority/effect/receipt state machines.

## MC20 — Learning cannot rewrite its grader or authority
- Corrections, deterministic guards, bandits, skills and parametric training are separate mechanisms.
- Learned candidates carry provenance, exposure, policy version, held-out evaluation and rollback.
- Promotion is a new governed event; training code cannot mutate evaluator truth or grant scope.
- Falsifier: reward improvement accompanied by authority widening or benchmark-oracle mutation is rejected.

## MC21 — Overload and retry are whole-path policies
- Every ingress/fan-out queue has descriptor and byte bounds.
- Backpressure/rejection happens before memory growth becomes unbounded.
- Retry budgets are end-to-end, not per layer.
- Notification streams may drop hints but durable catch-up cannot have silent gaps.
- Falsifier: slow consumer and provider outage remain bounded without acknowledged work loss.

## MC22 — Performance claims require full-path semantics
- Benchmark comparisons include admission, context compilation, model/tool calls, authority checks, workspace/sandbox operations, verification, persistence and cleanup.
- p50/p95/p99, CPU, allocations, RSS, I/O, contention, network, token/cost and energy where meaningful are reported.
- Removing semantic gates invalidates comparison.
- Falsifier: candidate that is faster only because verification/isolation is disabled cannot satisfy performance acceptance.

## MC23 — Scope identity follows every derivative
- Tenant/principal/project/environment/security-domain identity follows caches, embeddings, artifacts, summaries, skills, learning records and receipts.
- Cross-tenant cache/learning is opt-in and separately authorized.
- Falsifier: identical content in two tenants never causes private cache/evidence sharing absent explicit shared scope.

## MC24 — Human questions are durable typed waits
- Question binds requester, authorized respondent, run/effect digest, schema, expiry and answer scope.
- Timeout is not consent.
- Stale/replayed answer cannot resolve another question.
- Headless client receives actionable `AwaitingAuthority`, not implicit approval.
- Falsifier: concurrent questions answered out of order remain correctly bound.

## MC25 — Diagnostics report observation age and uncertainty
- Status distinguishes durable state, last observation time/source, live reachability and inferred health.
- Watchdogs cannot convert stale/unreachable state into “running”.
- Recovery actions are safe for current effect uncertainty.
- Falsifier: unknown external outcome diagnostic never recommends blind resend.

## MC26 — Machine change packets prevent architectural rediscovery
Every architecture-affecting change packet MUST state:
`owner`, `semantic_surface`, `old_revision`, `new_revision`, `compatibility`, `migration`, `rollback_boundary`, `resource_delta`, `security_delta`, `falsifiers`, `real_entrypoint_reachability`, `evidence`, `unknowns`.
- A PR that changes semantics without this packet fails architecture governance.
- Falsifier: reviewer can identify exactly which invariant changed without reverse-engineering implementation diff.

## MC27 — Frontier claims are independently falsifiable
- Same-model harness comparison, product comparison and human/persona evaluation are separate lanes.
- Security violation cannot be averaged away by quality/persona gains.
- Missing judge/trial remains in denominator with disposition.
- Falsifier: candidate superiority disappears when matched tools/budgets/evidence are enforced => claim is rejected or narrowed.

## MC28 — Background memory progress means durably applied progress
**Blocked until Logical-DB owner readiness.**
- Track accepted, scheduled and applied cursors separately.
- Applied position advances atomically with durable memory result or explicit failed/skipped disposition.
- No substitute store is permitted.
- Falsifier: crash after memory result commit but before acknowledgement neither loses nor duplicates processing.

## Acceptance principle

A contract is not “implemented” because a matching type exists. Promotion requires evidence from the **real supported entrypoint**, relevant crash/failure cases, owner compatibility and the applicable Rocco PR #1 acceptance axes.
