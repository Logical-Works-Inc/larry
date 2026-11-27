---
type: ADR
title: "ADR 0118: Model token and money budgets"
description: Tenant-scoped model token and micro-dollar budgets, route prices, bounded limiting and retry jitter.
resource: /docs/adr/0118-token-budget.md
tags: [larry, adr, model, budget, routing]
generated: {by: codex, at: 2026-11-26}
verified: {by: cargo nextest, method: focused box run, 2026-11-26}
status: accepted
stale_after: 2027-05-26
sources: [larry#204, ADR 0039, RFC 9110]
---

# ADR 0118: Model token and money budgets

## Context

There is no token or money budget, so "spend over budget is refused" cannot
hold. The executor counts one request per task (`TASK_COST`), the eval prices
one pair per run from the environment with no cached-input price, the client
has no limiter, and its retries draw no jitter, so many executors retry in
step against one gateway. The executor path never waits for a retry while the
default backoff promises 500 ms, and the client documents a cancel-aware wait
that does not exist.

## Decision

The model client owns the pre-send budget boundary. It checks the shared
tenant-route token and micro-dollar counters before `lgwks_std::http` runs,
and records only usage the route reports, priced from the resolved route's
versioned price table in whole micro-dollars (tokens times micro-dollars per
million tokens, divided back down: a products-scale count is picodollars, and
recording it as micro-dollars would spend a million budgets on one call). A
limit already reached is a typed refusal naming the exhausted unit.

Budget state is the executor's bounded tenant-route limiter: at most 256
keys, each the authenticated tenant and the resolved route, never only the
model name. Both the pre-send check and the usage record refuse a 257th key,
so one tenant's burst never starves another's entry. Each key holds settled
usage beside the pending hold of its calls in flight: `before` adds the
call's request size as pending tokens (and its price at the dearer route
rate as pending micros) under the same lock that checks spent plus pending
against both budgets, and `record` releases that hold and settles the usage
the gateway reported, zero on a failed call. A reply that reports no `usage`
settles at the reservation (the request size as tokens, its price at the
dearer route rate), never at zero: many OpenAI-compatible proxies omit
`usage`, and settling zero would let an untrusted gateway spend nothing
(larry#204, CWE-770). A check without the hold
admitted every call in flight before any of them recorded, spending N times
past the budget.

Jitter is the estate seeded stream (`lgwks_std::seeded::Seeded`), drawn
against the retry policy's bound and seeded from the tenant-route key's
digest, so one key replays one wait sequence on every host. An unkeyed client
draws zero entropy, which is what the injected-clock oracle reads. The pause
callback stays the cancellation seam: it sees the exact wait and answers
whether to retry.

The unset backoff is zero, like the executor, which never waits for a retry:
a wait it is asked for stops the retry at once, so the default asks for none.
Unmetered gateways spend their reservation, so a budget still bounds them.

The route's price table rides with it (migration 8, ADR 0127): revision 1 with
no rates at `add`, `larry route price` setting micro-dollar rates and bumping
the revision, re-adding keeping the table. A declared table prices the calls;
a route with none keeps the environment prices. No price is guessed for an
unset budget, and this change adds no migration: the limiter is process-local
and the budgets are environment.

## Rejected

The old request-count `TASK_COST` gate cannot account for reported usage or
provider prices. A separate mock-only limiter cannot protect the executor's
real model call. Wall-clock sleeping cannot be cancelled by this synchronous
executor path. OS-entropy jitter cannot replay: one seed must give one wait
sequence. An unset backoff that waits promises retries the executor refuses.

## Consequences

Every model call is checked against its tenant-route budget before any
gateway byte. Unset budgets stay unlimited, and a route with no rates keeps
the environment prices, so no existing arm changes price. The price columns
ride migration 8 (ADR 0127); this change adds no store migration. The unset
backoff is zero, like the executor that never waits.

## Proof

`sim_tenant_budget_refuses_before_call` and its money twin drive the model
client against the mock with a zero budget and assert the typed refusal and
zero served requests. `sim_reported_usage_is_priced_in_whole_micro_dollars`
fits two completions in one dollar at a million micro-dollars per million
tokens. `sim_a_non_zero_backoff_is_waited_then_cancelled` waits one 50 ms
backoff, cancels, and serves once. `the_unset_backoff_matches_the_executor`
pins the zero default. The limiter sims share one limiter between two
tenants and refuse the 257th key on both paths; `sim_concurrent_reservations_hold_one_budget`
runs 100 and 1,000 threads on one tenant-route and admits exactly the
budget's share, leaving the other tenant untouched. `sim_keyed_clients_replay_one_wait_sequence`
drives two same-key clients through one scripted fault script and asserts
equal waits inside the policy bound, different for a second tenant.
`sim_a_reply_without_usage_settles_at_its_reservation` answers every
completion with no `usage` on a 900-token budget and asserts the limiter
refuses with `BudgetExceeded` once the settled charge reaches it.
