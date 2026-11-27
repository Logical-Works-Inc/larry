---
type: ADR
title: "ADR 0115: Repository roots and tenant policy are ledger state"
description: Why executor repository roots and remote policy are durable per-tenant ledger state and local LARRY_POLICY remains operator-only.
resource: /docs/adr/0115-repo-allowlist-and-tenant-policy.md
tags: [larry, adr, executor, tenancy, security]
generated: {by: codex, at: 2026-11-26}
verified: {by: codex, method: lgwks-box cargo nextest and clippy on oracle, 2026-11-26}
status: accepted
stale_after: 2027-05-26
sources: [larry#207, ADR 0115]
---

# ADR 0115

## Context

Larry accepts task documents from local and remote doors. The executor ledger
already owns durable tenant state, so these controls must be read from that
store and checked before the clone or model reach.

## Decision

The executor state ledger stores one bounded repository-root allowlist and one
policy record per tenant. A task repository must be absolute. Larry resolves it
before the admission gate, rejects paths inside the state directory, and then
requires the canonical path to be below a stored root. A refused path never
creates a workspace or calls git.

The local `larry exec` operator keeps `LARRY_POLICY`. Remote executor doors
load policy from the tenant ledger. A missing remote record is an empty policy.
The policy is resolved before the task enters the runtime gate, so two tenants
cannot inherit one process-wide scope set. Task repositories and session working
directories use the same ledger authorization before route or model resolution.
`model_commands` is a JSON boolean; another JSON type is a typed task refusal.
Evaluator-owned origins use a crate-private route restricted to canonical
`<state>/eval` paths; public task execution still refuses every state path.
ACP reloads the ledger policy before each prompt, so a live provision or
withdrawal is effective without trusting `LARRY_POLICY`.

## Why

The old parser accepted `.` and the workspace cloned it against the server's
working directory. The old gate read one permissive environment policy for all
callers. A remote caller could therefore read another repository or run with
the local operator's capabilities. Presence checks also turned
`"model_commands": false` into a request for model commands.

## Consequences

An operator must provision a tenant's ledger roots and policy before a remote
task can run. A missing remote policy refuses work. Root and policy changes are
durable and are shared by every executor process that opens the ledger.

The existing SQLite ledger is already the owner of executor state, tenant
routes and receipt keys. Adding these records there keeps recovery and reload
on one durable source. An environment-only allowlist or policy copy loses
tenant separation and survives neither reload nor concurrent doors.

## Proof

`exec_rejects_relative_and_disallowed_repositories_before_clone` drives the
binary and asserts the relative, outside-root and state-directory refusals,
with no workspace.
`sim_tenant_policy_and_repository_roots_survive_reopen` proves the seeded
ledger keeps policy and roots across reopen. `sim_remote_tenant_policies_do_not_cross` submits
the same task under two ledger policies and observes one run and one typed
refusal.
`model_commands_requires_boolean_value` proves false is false and a non-boolean
is refused.
