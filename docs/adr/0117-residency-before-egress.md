---
type: ADR
title: "ADR 0117: Tenant residency is checked before model egress"
description: Why a tenant's country allowlist is stored beside its routes and checked before a route's key is read.
resource: /docs/adr/0117-residency-before-egress.md
tags: [larry, adr, model, residency]
generated: {by: codex, at: 2026-11-26}
verified: {by: cargo nextest, method: focused box run, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#203, larry#587, ADR 0039, ADR 0131, OWASP ASVS 4.0.3]
---

# ADR 0117: Tenant residency is checked before model egress

## Context

A route declares its provider's country, but a tenant could not say which
countries it allows. `LARRY_RESIDENCY` filters ranked selection only, and it is
process state: two tenants on one host cannot differ.

## Decision

`larry route residency <CC,...>` stores at most 64 sorted, uppercase ISO 3166-1
alpha-2 codes for the tenant in `executor.db` beside its routes (migration 11).
`--clear` removes the row. `route list` shows the policy.

With a stored policy, a route must declare one of those codes. `resolve_for`
(`route resolve` and terminal sessions) and `select_for` (briefed tasks) refuse
any other route typed, `ResidencyNotAllowed`, before its key is read and before
a byte is sent. The choice code moved to `executor::route_select` so that
`routes.rs` stays under the size law.

Rejected: the environment variable alone. It is not tenant state.

## Consequences

A tenant that sets a policy must declare matching residency on every route it
uses, the `env` route included (`LARRY_MODEL_RESIDENCY`).

Known limit: the stored policy is checked after the ranked choice. When the best
ranked route is outside it, the task is refused and does not move to the next
route. `LARRY_RESIDENCY` filters before ranking.

## Proof

`route_residency_policy_is_stored_and_filters_model_egress` drives the shipped
binary: the policy is stored and listed, a briefed task and `route resolve` are
refused, the listener counts no connection, and an admitting or cleared policy
resolves the route. `select_for_refuses_a_route_outside_the_stored_policy` and
`sim_residency_allows_a_matching_country_only` hold the two checks, and
`residency_policy_is_bounded_at_sixty_four_codes` the bound.
