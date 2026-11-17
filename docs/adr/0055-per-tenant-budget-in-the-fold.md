---
type: ADR
title: "ADR 0055: one budget per tenant in the fold, and the fold refuses a run two tenants name"
description: Why the runtime keeps one budget per TenantId, why restore re-derives it from recorded intents, why a run whose events name two tenants is refused, and why the contracts ledger variant is deleted.
resource: /docs/adr/0055-per-tenant-budget-in-the-fold.md
tags: [larry, adr, runtime, budget, multi-tenant]
generated: {by: claude-code, at: 2026-11-16}
verified: {by: claude-code, method: the diff read, and larry-oracle all on this branch, 2026-11-16}
status: accepted
stale_after: 2027-02-16
sources: [larry#208, /specification.md, /docs/specification/13-cell-05-resource-model-and-initial.md, /docs/specification/11-d39-budget-reservations-are-leases.md, /INVARIANTS.md]
---

# 0055: one budget per tenant in the fold, and the fold refuses a run two tenants name

Status: accepted, 2026-11-16 (larry#208)

## Context

The runtime held one `remaining_budget: u64` for every tenant, and
`INVARIANTS.md` line 18 claimed tenants never share a budget. Tenant A
spending its ceiling therefore denied tenant B's effect, which is the
noisy-neighbour failure the quota-isolation sources name, and the document
claimed what the code refuted.

The fold also admitted a run's tenant first-wins: `RunView::tenant` took the
earliest event's tenant and ignored a later event naming another. Live writes
refuse a foreign envelope through the run table, but a journal restored from
disk could carry one run under two tenants and restore under either.

`larry_contracts::authority::validate_at_effect_boundary_with_ledger` had no
caller. It re-checked a single effect's cost against a `Grant` ceiling and a
`remaining` balance the stateless crate cannot own.

## Decision

- One budget per `TenantId`: `Runtime::budgets` is a `BTreeMap<TenantId, u64>`.
  An entry enters in `track_run`, so the map holds at most the run table's
  capacity of counters. `remaining_budget(tenant)` reads that tenant's counter,
  and a tenant no run has been tracked for reads the whole initial budget.
- The pinned snapshot carries the run's own tenant: `pinned_inputs` takes the
  tenant the run table holds, and `BudgetSnapshot::remaining` is that tenant's
  remainder, so `replay::decide` bounds one tenant's cumulative spend only.
- Restore re-derives the counters from the journal: `ProjectionBackend::
  reserved_by_tenant` folds every recorded intent's cost under the tenant its
  event named, and `restore` subtracts each tenant's sum from the initial
  budget. A restart cannot recharge one tenant by spending another's.
- The fold refuses a split run: `FoldProjection::entry` returns
  `RuntimeError::RunTenantMismatch` when a later event names a tenant other
  than the run's first, live and at restore.
- The contracts ledger variant is deleted with its test. Cell 05 still needs a
  per-instance and an aggregate tenant bound, and the aggregate bound has one
  home: the pure reducer's `BudgetSnapshot::remaining`, which the runtime pins
  per tenant. D39 places the budget ledger with the owner scheduler, not with
  a stateless contracts crate.

## Consequences

- A tenant that spends its whole ceiling refuses only its own next effect.
  Tenant B's effects authorize and receipt while tenant A stays spent.
- Each admission costs one map lookup; restore allocates one map. The fold's
  recorded trace and its replay stay bit-identical, because the snapshot reads
  a value the journal re-derives.
- A hand-built or corrupted journal that names two tenants for one run is
  refused at restore with a typed error naming the run, the first tenant and
  the later one, instead of restoring a mixed attribution.
- `Runtime::remaining_budget` takes a tenant. Every caller in this repository
  is updated; a host that read one pooled number now reads one per tenant.
- Cumulative spend keeps one implementation. Removing the second one means a
  future ledger-capability change edits the reducer and its pinning, not two
  copies that can drift.

## Rejected

- Keep one shared counter and correct `INVARIANTS.md` instead: the issue and
  Cell 05 name an aggregate tenant bound, and a shared counter makes one
  tenant's spend a denial of another's effect.
- One budget per run: bounded by the run table and easy to restore, but a
  tenant with ten runs would be granted ten ceilings.
- Keep the contracts ledger variant and call it from `reach_owner`: the
  runtime has already reserved the effect's cost by the time a reach happens,
  so the only balance it could pass is one it fabricated, and the check would
  duplicate the reducer with a value no owner ledger supplied.
