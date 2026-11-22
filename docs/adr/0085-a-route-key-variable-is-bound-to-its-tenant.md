---
type: ADR
title: "ADR 0085: a route's key variable is bound to its tenant"
description: Why a stored route may name only LARRY_KEY_<TAG>_* under its own tenant's digest tag, checked at add and again at resolve.
resource: /docs/adr/0085-a-route-key-variable-is-bound-to-its-tenant.md
tags: [larry, adr, model, routing, multitenant, security]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: tests/route_cli_journey.rs's a_route_cannot_name_another_tenants_key_variable and a_route_stored_before_the_binding_is_refused_at_resolve red before the change (exit 0 where 1 was required) and green after, with sim_route_table's sim_a_row_stored_before_the_binding_is_refused_at_resolve and model::route::tests::a_key_variable_is_bound_to_its_tenants_tag, 2026-11-21}
status: accepted
stale_after: 2027-02-21
sources: [larry#328, larry#201, ADR 0039, CWE-200, /INVARIANTS.md]
---

# 0085: a route's key variable is bound to its tenant

Status: accepted, 2026-11-21 (larry#328, found by the 2026-11-19 review).

## Context

ADR 0039 made a route tenant data, but the namespace its key variable lives in was the process's: a
stored route could name any `LARRY_KEY_*` variable. `a_stored_route_cannot_name_a_variable_outside_its_namespace`
covered only names outside `LARRY_KEY_*` (`GH_TOKEN`, `LARRY_MODEL_KEY`, the bare prefix).

On one host every tenant's shell holds every tenant's key. So a tenant that knew another tenant's
variable name, or guessed it, added a route with that name and its own URL, and the next model call
sent the other tenant's key to this tenant's host (CWE-200). The route store, the sealed report and
the resolver all stayed silent.

## Decision

- The tag is 16 characters of the tenant's digest text in upper case: the first 64 bits. The
  namespace is `LARRY_KEY_<TAG>_`. A stored route's key variable must start with it and name at
  least one more byte, so two routes of one tenant differ by their own suffix.
- `larry route add` checks the binding before the store opens and refuses another tenant's variable
  as `RouteError::KeyEnvNotThisTenants`, naming the variable and this tenant's prefix. The output
  carries `key_env`: the exact variable name the operator exports, since the tag is not something a
  person is expected to compute.
- The resolver takes the tenant text and checks the binding again before the route is returned, so
  the key variable is never read for a route the tenant may not use. A row stored before this change
  is refused there, typed. The check never falls through to the `env` route: the tenant named a
  route it may not use, and that is a refusal, not a default.
- The store still reads an unbound row, and `larry route list` shows it. It is not `Damaged` (the
  shape is legal); it is refused where it would send. A person must be able to see the row to
  remove or replace it.
- The `env` route is unchanged: the operator names `LARRY_MODEL_KEY` themself. There is no tenant on
  that path.
- Nothing new is stored: the tag is a function of the tenant text the store already keys rows by, so
  no schema change, no migration and no new dependency.

## Consequences

- The tag is a namespace, not a secret: it appears in `larry route add` output and in the refusal.
  A refusal shows the variable name and the prefix, never a value.
- Two tenants of one host share a namespace with probability near 2^-33 at the 65,536-tenant bound
  (`n = 2^16` over a 64-bit tag). A 32-bit tag, the issue's example, would give about one collision
  in two at that count, and a collision is the defect again.
- An operator with a pre-binding route sees a typed refusal naming the new prefix on the next
  resolve, and one `larry route add` with the printed name replaces the row.
- Covered by `tests/route_cli_journey.rs` (both cases through the shipped binary, the mock counting
  zero requests), `tests/sim_route_table.rs` (`sim_a_row_stored_before_the_binding_is_refused_at_resolve`,
  `sim_key_variable_length_walk_matches_the_written_bounds`, `sim_monkey_edits_of_a_valid_key_variable_agree_with_the_reference`,
  `sim_hostile_and_honest_adders_at_once_leave_exactly_the_honest_rows`) and
  `model::route::tests::a_key_variable_is_bound_to_its_tenants_tag`.

## Rejected

- An operator table mapping each variable to the tenants and hosts allowed to use it. It is a second
  record outside the store: a second durable owner, a second thing to seal, and a row the operator
  must add for every tenant and key. A forgotten row refuses a tenant's own route, and a mistyped row
  admits another tenant's, silently. The tenant digest needs no operator action and cannot be
  forgotten.
- A 32-bit tag (the first 8 hex characters the issue names as an example). It is shorter, and at the
  supported tenant count the birthday bound makes a shared namespace likely.
- Refusing only at resolve. The tenant would store a route it cannot use and learn why at the first
  model call, after the prompt is built.
- Refusing only at add. A row already in the store, or one written by a process that did not check,
  would still send.
- Treating an unbound row as `Damaged`. `larry route list` would refuse the whole table after one old
  row, and the person could no longer see what to remove.
- Deriving the tag from the tenant's *name* (an operator-chosen word). Two tenants can choose one
  name, and the binding then binds nothing.

Reversal: none expected. A host that wants one namespace can name every tenant's state directory the
same, which is one tenant, not two.
