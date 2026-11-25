---
type: ADR
title: "ADR 0113: A reused operation id is refused before its run moves"
description: Why a second submit of an operation id the runtime holds is refused and recorded before any transition, and why operation ids stay one namespace across tenants.
resource: /docs/adr/0113-a-reused-operation-id-is-refused-first.md
tags: [larry, adr, runtime, admission, tenancy]
generated: {by: claude-code, at: 2026-11-24}
verified: {by: claude-code, method: cargo nextest run --workspace --locked on toronto, 2026-11-24}
status: accepted
stale_after: 2027-05-24
sources: [larry#198, larry#182, larry#371, ADR 0078]
---

# 0113: A reused operation id is refused before its run moves

Status: accepted, 2026-11-24 (larry#198, #182 defect 8).

## Context

`admit_work` refused `DuplicateOperation` only after it had journaled `Admit`, `Start` and
`EnterReconciling` for the second submit's run. That run stayed in `Reconciling` with no effect,
and no record said why. The live table, the journal fold and the fence's revocation by name
all key by operation id, and one broker environment serves every tenant.

## Decision

- `admit_work` checks the live table before it tracks the run or moves it. A hit records
  `CommandRefusal::DuplicateOperation` beside the durable acceptance and returns
  `DuplicateOperation`. The run never leaves `Draft`. A restart reads the refusal.
- Operation ids are one namespace across a runtime's tenants. With reuse refused, a
  revocation by name withdraws one tenant's one effect.

Rejected: key the live table, the fence and the journal by tenant and operation. The journal,
the projection and the broker action name the operation alone, so it is a format change for a
namespace no host shares on purpose. The stuck run also stays unless the check moves. A
measurement that would reverse this: a host whose tenants mint the same operation id on
purpose.

## Consequences

`tests/sim_duplicate_operation.rs` covers 96 seeds. A reuse from another tenant, from the same
tenant's other run and from the first run itself is refused and recorded. The first run
completes with one dispatch, and a restore reduces nothing again.
