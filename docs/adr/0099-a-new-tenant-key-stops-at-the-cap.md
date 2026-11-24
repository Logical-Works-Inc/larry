---
type: ADR
title: "ADR 0099: Larry's serve key stops a new tenant at the cap"
description: Why larry serve key refuses a key for a new tenant when the store holds 65,536, in the same SQL statement as the insert.
resource: /docs/adr/0099-a-new-tenant-key-stops-at-the-cap.md
tags: [larry, adr, serve, keys]
generated: {by: claude-code, at: 2026-11-23}
verified: {by: claude-code, method: sim_serve_key_refuses_a_new_tenant_at_the_cap_and_the_store_still_loads red then green on Oracle, 2026-11-23}
status: accepted
stale_after: 2027-05-23
sources: [larry#397, larry#321, ADR 0040]
---

# 0099: A new tenant's key stops at the cap

Status: accepted, 2026-11-23 (larry#397). Amends ADR 0040.

## Context

`larry serve` refuses startup and a SIGHUP reload past 65,536 keys (ADR 0040), and a failed
reload admits no one. `larry serve key` inserted with no count, so key 65,537 locked every
tenant out at the next reload.

## Decision

- `Ledger::set_tenant_key` writes only when the tenant already holds a key or the store holds
  fewer than the cap: `INSERT ... SELECT ... WHERE EXISTS (...) OR COUNT(*) < cap ON CONFLICT
  DO UPDATE`, the same shape as the route table's upsert.
- Zero rows written is `ServeError::KeysFull` (exit 1), naming the cap and the revoke that frees
  a slot. A rotation of a keyed tenant still succeeds at the cap.

Rejected: a count, then an insert, in two statements. Two issuers racing at 65,535 both pass
the count, and the store lands past the cap.

## Consequences

- The store cannot grow past the cap through `larry serve key`. A row written by another tool
  is still refused at load (ADR 0040).
- Falsifier: `sim_serve_key_refuses_a_new_tenant_at_the_cap_and_the_store_still_loads`.
