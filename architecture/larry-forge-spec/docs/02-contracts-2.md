---
type: Specification
title: "Machine-focused implementation contracts, part 2: MC15 to MC28"
description: "Machine-focused contract catalogue refining Larry PR #1 without changing canonical product scope."
resource: /architecture/larry-forge-spec/docs/02-contracts-2.md
tags: [larry, forge, architecture, contracts, hyperscale]
generated: {by: ChatGPT, at: 2026-09-16}
verified: {by: ChatGPT, method: "Pinned source inspection; runtime NOT RUN"}
status: proposed-not-ratified-runtime-not-run
stale_after: 2026-10-16
sources:
  - "https://github.com/srinji-kaggss/Larry/pull/1"
---

# Machine-focused implementation contracts, part 2: MC15 to MC28

Part 1 is [02-contracts.md](02-contracts.md).

## MC15 — Provider neutrality preserves transcript semantics
- Canonical transcript maintains tool-call/result pairing and provider-independent message semantics.
- Provider-native opaque metadata is adapter-local unless explicitly standardized.
- Provider-executed tools are not replayed locally.
- Retry cannot silently switch provider/account/region/model/data recipient.
- Falsifier: stop reason plus outstanding tool calls cannot be interpreted as task completion.

## MC16 — Workspace mutation is owned by Moo
- Larry proposes/adjudicates work; Moo owns workspace transaction/conflict semantics.
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

A contract is not “implemented” because a matching type exists. Promotion requires evidence from the **real supported entrypoint**, relevant crash/failure cases, owner compatibility and the applicable Larry PR #1 acceptance axes.
