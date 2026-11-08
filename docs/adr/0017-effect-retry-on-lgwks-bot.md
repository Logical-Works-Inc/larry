---
type: ADR
title: "ADR 0017: the effect retry decision runs on lgwks_bot"
description: Why larry-runtime retries an open effect only when lgwks_bot::retry::retry_admissible admits it, over an EffectKey derived from the intent, bounded by the effect's authority lease.
resource: /docs/adr/0017-effect-retry-on-lgwks-bot.md
tags: [larry, adr, runtime, retry, effect, idempotency]
generated: {by: claude-code, at: 2026-11-08}
verified: {by: claude-code, method: full nextest run before and after on the same replay seeds}
status: accepted
stale_after: 2027-02-08
sources: [larry#78, larry#68, logicalworks-crates#845, lgwks_bot 7.1.1 effect and retry, docs.stripe.com/api/idempotent_requests]
---

# 0017: the effect retry decision runs on lgwks_bot

Status: accepted, 2026-11-08 (larry#78, larry#68 row 5)

## Context

`reconcile_one` re-dispatched an `IdempotentRetrySafe` effect while `dispatches < max_attempts`: a second
copy of the retry rule beside its estate owner, which larry#68 forbids. `lgwks_bot::retry::retry_admissible`
is RQ-009 as one pure function (budget, deadline, live authority, unchanged intent, and proof of non-application
or a deduplication contract) and reports every conjunct that failed. `lgwks_bot::effect::EffectKey` is the
seven-field identity a settlement is about. The old rule never looked at the authority lease, so a resend could
leave after the approval behind it had expired, against E30. An idempotency key protects a resend only inside
the remote's retention window (Stripe keeps keys 24 hours) and only for the same request.

## Decision

- `runtime::retry::decide` builds `RetryFacts` and asks `retry_admissible`; Larry keeps only the
  strategy-to-action policy (`ReconciliationStrategy::reconciliation_action`, larry-contracts).
- The key is derived from the intent, never stored: run, operation as the action, tenant as the environment
  (until #76 hands broker environments), the admission epoch plus one, a constant flow revision (Larry admits
  no flow document) and a BLAKE3 action digest over operation, run, tenant, strategy and binding. An all-zero
  id is the typed `RuntimeError::EffectIdentity`.
- An open effect is `DispatchCertainty::Unsettled`, so its class is `RequiresEvidence`. `IdempotentRetrySafe`
  is recorded as a `DeduplicationContract` with `LateArrival::Unspecified` whose retention is the effect's
  authority lease (pinned `observed_at` to pinned `valid_until`); the same lease is the policy deadline. Larry
  attests no retention beyond its own authority.
- Live authority is the fence's answer. A retry refused for authority alone still goes through `reach_owner`,
  so the fence stays the one enforcement point and reports `fenced`. Any other refusal records one `Unknown`
  and spends the dispatch budget, so the effect stays visibly open and leaves reconciliation.
- `restore` reads each lease from the recorded authorization; an intent without one restores already expired.

## Consequences

- While the lease holds, every decision equals the old budget rule, so replay traces are unchanged; a retry
  after `valid_until` is refused, dispatches nothing and leaves no receipt.
- No new crate: `effect` and `retry` compile with lgwks_bot's defaults off.
- When #845 publishes a supported surface for `effect` and `retry`, only `runtime::retry` changes.
- `runtime::retry::tests::larry_keeps_no_retry_rule_of_its_own` fails if the inline rule returns.
