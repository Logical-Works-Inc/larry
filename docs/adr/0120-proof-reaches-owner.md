---
type: ADR
title: "ADR 0120: The checked authority proof reaches the owner"
description: Why every mutating owner call receives and re-reads the checked cap::Auth proof before it can record or run an effect.
resource: /docs/adr/0120-proof-reaches-owner.md
tags: [larry, adr, runtime, authority, owner]
generated: {by: codex, at: 2026-11-26}
verified: {by: codex, method: seeded runtime simulation and executor journey on toronto, 2026-11-26}
status: accepted
stale_after: 2027-05-26
sources: [larry#461, larry#182, ADR 0018, ADR 0112]
---

# 0120: The checked authority proof reaches the owner

Status: accepted, 2026-11-26 (larry#461, #182 defect 6).

## Context

`runtime::reach` checked the live proof before calling an owner, but both
mutating owner methods received only the intent and time. An owner could not
re-check the credential at the action boundary. A lease could lapse between
the runtime read and the owner read that the endpoint needs to trust.

## Decision

- `EffectOwner::dispatch` and `dispatch_bound` receive the checked
  `lgwks_bot::kernel::cap::Auth` and the exact required capability slice by
  reference.
- The runtime passes a clone of the checked proof and restores the live witness
  if the owner returns the typed `BindingError::Expired` refusal.
- `TaskOwner`, `Recovered` and `ScriptedOwner` read the proof and required
  capabilities before any recording or body execution. `TaskOwner` also checks
  the intent tenant against its own tenant. The default unbound path validates
  the proof, then returns `UnsupportedOwner`; only owners with explicit
  unbound support implement that path.
- `Auth` names no tenant or holder. Larry therefore checks the intent tenant
  against the owner tenant, but cannot compare it with authenticated proof
  identity. The estate gap is [logicalworks-crates#1161](https://github.com/Logical-Works-Inc/logicalworks-crates/issues/1161).

Rejected: pass a boolean, tenant marker or the runtime's `Proof` wrapper, or
let unbound dispatch forward to bound dispatch. Those values do not let an
owner validate the sealed capability credential at the action boundary, and
the forwarding path would run a bound body without a display binding check.

## Consequences

`sim_owner_refuses_lapsed_authority_before_send` advances the shared virtual
clock after the runtime check and proves 32 seeded refusals with no sent
record. The seeded ScriptedOwner and TaskOwner cases refuse a missing
capability with `NoCoveringGrant`. The unbound TaskOwner case proves the body
does not run. `sim_a_foreign_tenant_is_refused_on_the_runtime_reach` drives
32 seeded foreign-tenant commands through display, admission and a real tick
against a `TaskOwner` and proves each is exactly one typed `Changed` refusal
with the body never run. `executor_owner_reads_proof_tenant_before_running_task` drives
the shipped executor binary through the owner path and proves the authorized
task still runs and seals its accepted report.
