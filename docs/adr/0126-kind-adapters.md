---
type: ADR
title: "ADR 0126: Provider kind names per route"
description: Why each route names OpenAI chat, Anthropic Messages or OpenAI Responses, how the name parses and seals, and where the wire adapter plugs in.
resource: /docs/adr/0126-kind-adapters.md
tags: [larry, adr, model, routing]
generated: {by: opencode, at: 2026-11-27}
verified: {by: opencode, method: lgwks-box cargo nextest and clippy on toronto, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#205, larry#539, larry#546]
---

# ADR 0126

## Context

Every route was an OpenAI-compatible chat route: `Kind` had one variant,
`larry route add` took no `--kind`, and the sealed record named no kind.
Claude (Anthropic Messages) could not even be named as a route, so the
adapter work had nowhere to plug in.

## Decision

- `Kind` names three provider kinds: OpenAI chat (`openai-chat`),
  Anthropic Messages (`anthropic-messages`) and OpenAI Responses
  (`openai-responses`). `Kind::parse` reads a stored name, `Kind::name`
  writes it back, and an unknown name is `RouteError::BadKind`.
- `larry route add` takes `--kind` (`openai-chat` unless named);
  `list` shows the kind and the sealed record names it. The client's
  per-kind path, auth, body and reply mapping plugs in at the call site.

## Consequences

- A route added without `--kind` is an OpenAI chat route, so every stored
  route keeps working and the seal of an old route gains one member.
- An unknown `--kind` is refused naming the field, and nothing is stored.
- PR #551 serves all three kinds and removes the refusal: every named kind
  adds and resolves, and `RouteError::KindNotServed` is deleted with its
  message, its `Display` arm and its tests (larry#546).

## Proof

- `model::route::tests::each_kind_parses_and_seals_with_its_name` (unit):
  every named kind parses back and seals with its name; an unknown kind
  parses to nothing.
- `model::route::tests::the_refusal_set_is_closed_with_no_dead_variant`
  (unit): every refusal is built and its `opens_with_route` answer pinned;
  the classifier lists every variant with no wildcard, so a resurrected
  refusal fails the build.
- `executor::routes::tests::every_route_refusal_names_route_once` (unit):
  every field refusal names `route` once; an unknown kind is refused
  naming the field, storing nothing.
- `tests/route_cli_journey.rs`: two tenants' receipts each name
  `openai-chat` by exact seal equality.
- `tests/route_kind_journey.rs::every_named_kind_is_served_and_seals_its_name`:
  every named kind adds, resolves and seals with its name; an unknown
  kind is refused with `route kind must be one of`, exit 1, and nothing
  is stored.
