---
type: ADR
title: "ADR 0130: per-tenant receipt key store, stored tenant, key ids"
description: "One receipt key per tenant with key ids, a stored default tenant and named selection."
resource: /docs/adr/0130-per-tenant-key-store.md
tags: [larry, adr, executor, tenant, receipt]
generated: {by: opencode, at: 2026-11-27}
verified: {by: opencode, method: lgwks-box cargo nextest run, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#211, larry#550, INVARIANTS.md, tests/tenant_keys_journey.rs]
---

## Context

One signing key sealed every tenant's receipts, and the tenant id was a hash of
the state directory's path spelling. Two spellings were two tenants, a move was
a new tenant, and a rotation re-keyed every tenant at once.

## Decision

Migration 10 adds `dir_tenant` and `receipt_keys(tenant, key_id)`, carrying the
old global key as id 1 under the empty placeholder. `Ledger::open` draws the
default once, rekeys legacy rows onto it, and backfills the placeholder.

`LARRY_TENANT` selects a named tenant, else the stored default. The run path
mints at most id 1 on first seal; reads never mint. Receipts name `kid`; old
ids verify, new seals use the active id. `larry key` reads, `rotate` appends.

Reports and live groups keep their shape here; their tenant columns, quota and
the 0700 tree land in part C.

## Consequences

Two tenants on one directory seal under disjoint keys and verify only their
own. A move keeps every id. A rotation keeps old receipts verifiable. No new
dependency; one migration; additive only.

## Proof

- `sim_two_tenants_hold_disjoint_keys_for_every_seed` proves isolation.
- `sim_a_thousand_concurrent_tenant_provisions_hold_distinct_keys` proves concurrency.
- `sim_a_truncated_key_is_refused_typed` proves damage fails closed.
- `sim_two_tenants_on_one_state_directory_verify_only_their_own` proves the journey.
- `a_rotated_key_verifies_old_receipts_by_key_id_and_seals_new_ones_under_the_new_id` proves rotation.

## Traceability

Part of #211, closes #550. Rejected: rebuilding reports and groups with quota
in the same part, which mixes key isolation with retention and spends the
mutant budget.
