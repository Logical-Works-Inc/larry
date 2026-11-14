---
type: ADR
title: "ADR 0039: a model route is data a tenant owns, not a gateway in the binary"
description: Why the model gateway and model id left the binary for a per-tenant route table in the state directory's store, what a route holds, how a task resolves one, and what a sealed record says about it.
resource: /docs/adr/0039-routes-are-tenant-data.md
tags: [larry, adr, model, routing, residency, multi-tenant, larry-201]
generated: {by: claude-code, at: 2026-11-14}
verified: {by: claude-code, method: nextest run of the route simulations and journeys on Oracle}
status: accepted
stale_after: 2027-02-14
sources: [larry#201, larry#200, docs/product/README.md, docs/adr/0003-model-client-on-lgwks-std-http.md, docs/adr/0004-executor-process-groups-and-ledger.md, docs/adr/0037-run-identity-and-idempotency-keys.md, ISO 3166-1 alpha-2, RFC 6750 section 2.1]
---

# 0039: a model route is data a tenant owns, not a gateway in the binary

Status: accepted, 2026-11-14 (larry#201, step 1 of model routing)

## Context

The model client read one gateway URL and one model id from the process environment and fell back to a
9router address and model compiled into the binary. Every tenant of a process shared them. The Director
(2026-11-14) called such choices semantic features of the end user, not constants. The product plan says
customer data must never take a path to a host outside Canada and that the second face picks a host per
tenant. A compiled-in default also sent a prompt somewhere nobody chose whenever the variables were absent.

## Decision

- A route is a row in the state directory's store, keyed by tenant and id: provider kind (`openai-chat`
  today), base URL, the NAME of the `LARRY_KEY_*` variable that holds the key, model id, optional
  temperature, a request byte bound (1 KiB to 16 MiB, default 1 MiB), a residency (ISO 3166-1 alpha-2) and
  a description of at most 1 KiB. The key is read from its variable only when a task reaches the model,
  and is never stored, logged or put in an error or a report.
- A stored route may name only `LARRY_KEY_` and at least one more byte (`KeyEnvOutsideNamespace`
  otherwise), because its variable's value is sent as the bearer token to the route's host: a tenant must
  not name `GH_TOKEN`, a cloud key or the operator's `LARRY_MODEL_KEY` (CWE-200). Only the `env` route
  names `LARRY_MODEL_KEY`. A stored row outside the namespace is `Damaged`, never read.
- A tenant holds at most 256 routes and one default. A full table refuses the next route as `TableFull`;
  the count and the insert are one statement, so concurrent adders cannot pass the bound.
- A task resolves a route in one pure order: the route it names, else the tenant's default, else the
  operator's `env` route (only when `LARRY_MODEL_URL`, `LARRY_MODEL` and `LARRY_MODEL_KEY` are all set),
  else `NoRoute`. A name no route has is `Unknown` and never falls through to the default.
- The model settings are parsed only when a task reaches a model, so a typo in a knob fails no task with no
  brief.
- Each sealed model record and session names the route id, the model, the gateway's host and port and the
  residency (`null` when undeclared), never a URL path or a key. `larry verify` covers them.
- `larry route add|list|remove|default|resolve` manages the table; the task document and
  `larry_submit_task` take an optional `route`.

Rejected:

- A TOML route file read at start. It needs no schema, but it is per process, not per tenant, it is not
  durable under the receipt seal, and it leaves no audit trail of a change.
- Keeping the compiled-in default for tasks that name nothing. It is one fewer refusal, but it keeps the
  hardcoded gateway and sends data where no tenant chose.

## Consequences

- Residency is stored and sealed here; refusing a route by it is larry#203. Choice by meaning is #202,
  budgets #204 and provider kinds #205, each plugging into `model::route`.
- The MCP, ACP and eval doors keep working through the `env` route; a deployment with neither a route nor the
  variables refuses a task with a goal as `executor_not_connected`.
- The store migrates to schema version 5 on the next open; older binaries ignore the new table.
- Covered by `tests/route_cli_journey.rs`, `tests/sim_route_table.rs`, `tests/sim_route_resolve.rs` and
  `executor::ledger::routes` tests.
