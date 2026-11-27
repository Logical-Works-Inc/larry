---
type: ADR
title: "ADR 0132: Transport is refused before model egress"
description: Why a model route requires HTTPS unless its host is loopback, and why pre-rule stored rows fail closed at resolve.
resource: /docs/adr/0132-transport-refused-before-egress.md
tags: [larry, adr, model, transport]
generated: {by: opencode, at: 2026-11-27}
verified: {by: cargo nextest, method: focused box run, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#203, larry#531, ADR 0039]
---

# ADR 0132: Transport is refused before model egress

## Context

A model route could use plaintext for a remote host, so the bearer key and
the request body left over `http://` when the URL said so. Rows stored before
any transport rule also stayed resolvable.

## Decision

`Route::new` and `Route::environment` refuse a non-loopback `http` URL with
the typed `InsecureTransport` refusal, which carries only the host. Loopback
(`localhost`, `127.0.0.0/8`, `::1`) stays permitted for local mocks.

`Route::stored` keeps pre-rule rows listable and removable, and `resolve`
refuses them with the same typed refusal before any key read. A stored row is
never upgraded silently: the operator re-adds it over `https`.

`authority_of` and `is_loopback` live in `model::route_transport`; the table
and the sealed view live in `model::route_table` and `model::route_view`.

## Consequences

Remote plaintext routes fail before the model client reads the key or opens a
connection. Local loopback HTTP remains for deterministic mocks and sidecars.

## Proof

`sim_plaintext_non_loopback_is_refused_before_send` and
`sim_plaintext_loopback_is_allowed` prove the refusal and its exception.
`a_legacy_plaintext_route_lists_and_removes_but_resolution_refuses` proves a
pre-rule row lists, removes, and never resolves.
