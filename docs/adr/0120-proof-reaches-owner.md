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
  `lgwks_bot::kernel::cap::Auth` by reference.
- The runtime passes a clone of the checked proof and restores the live witness
  if the owner returns the typed `BindingError::Expired` refusal.
- `TaskOwner`, `Recovered` and `ScriptedOwner` read the proof before any
  recording or body execution. `TaskOwner` also checks the intent tenant
  against its own tenant.

Rejected: pass a boolean, tenant marker or the runtime's `Proof` wrapper.
Those values do not let an owner validate the sealed capability credential at
the action boundary, and the private wrapper would couple the public owner
contract to runtime internals.

## Consequences

`sim_owner_refuses_lapsed_authority_before_send` advances the shared virtual
clock after the runtime check and proves 32 seeded refusals with no sent
record. `executor_owner_reads_proof_tenant_before_running_task` drives the
shipped executor binary through the owner path and proves the authorized task
still runs and seals its accepted report.
