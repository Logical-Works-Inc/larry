---
type: ADR
title: "ADR 0119: Provider adapters per route kind, a prompt template and structured replies"
description: Why each route kind maps its own path, auth, body and reply shape, temperature is sent only when pinned, and the route template is stored per tenant and sealed by hash.
resource: /docs/adr/0119-provider-adapters.md
tags: [larry, adr, model, routing]
generated: {by: codex, at: 2026-11-27}
verified: {by: codex, method: lgwks-box cargo nextest and clippy on toronto, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#205]
---

# ADR 0119

## Context

Only the OpenAI-compatible chat shape worked. `client.rs` fixed the
`/chat/completions` path and the bearer header, `wire.rs` fixed the body
shape and read only `prompt_tokens` and `completion_tokens`, and the
temperature rode on every request. A model that rejects the field, such as
the one the #230 benchmark met, answered every request with a 4xx, which is
terminal. The system prompts were constants, so no tenant or route could tune
them, and the reply was always bare text.

## Decision

- One adapter per provider kind, chosen by the route: OpenAI chat
  (`openai-chat`), Anthropic Messages (`anthropic-messages`) and OpenAI
  Responses (`openai-responses`). The adapter maps the request path, the
  credential header, the body and the reply: chat and responses send the
  bearer token, messages sends `x-api-key` with `anthropic-version
  2023-06-01`, and every kind maps its usage to the one `Usage` shape with
  the impossible-report rule kept.
- The route's temperature is optional end to end and is sent only when the
  route pins it. The per-route context bound stays `request_bytes`, enforced
  before any byte leaves. Messages carries `max_tokens` because the API
  requires one; the value is a wire constant, not a second budget.
- A route holds an optional prompt template, stored per tenant in
  `route_prompts` (migration 8) beside the route row. The template rides
  before the standing instruction of every brief and session prompt on that
  route, after which the partition still separates tenants. Only its BLAKE3
  hash is sealed, in the route seal and every authored record.
- Replies on the two new kinds are structured: the brief asks for exactly
  `{"content": "..."}`, and the author writes the `content` string. Anything
  else is refused, never written. Chat keeps the fence-stripped text.

## Consequences

- `larry route add` takes `--kind` (`openai-chat` unless named) and
  `--prompt`; `list` shows the kind and `resolve` seals the kind and the
  prompt hash. The `env` route names its kind with `LARRY_MODEL_KIND`.
- The mock endpoint serves all three paths with kind-shaped replies and
  records the credential header of each, so a swapped scheme fails a journey.

## Proof

- `sim_each_kind_sends_its_own_members`,
  `sim_each_new_kind_reads_its_reply_to_one_shape`,
  `sim_each_new_kind_refuses_an_unfinished_or_torn_reply`,
  `a_kind_and_prompt_are_checked_and_sealed` (unit and simulation).
- `tests/provider_kinds_journey.rs`: the same task through a mock of each
  kind names its kind and maps usage; no temperature sends none; the template
  seals by hash; a fault on a new kind is typed and served once.
