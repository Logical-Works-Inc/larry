---
type: Design
title: Larry final structure — components, flow and ownership
description: Navigable architecture summary of the proposed runtime; canonical decisions remain in the specification.
resource: /docs/architecture/README.md
tags: [larry, architecture, structure, ownership]
generated: {by: opencode, at: 2026-09-16}
verified: {by: opencode, method: source inspection of /specification.md; structural gate; diagrams NOT RUN in this file}
status: final-structure-v1.1
stale_after: 2026-10-16
sources: [Director conversation 2026-09-16, /specification.md, /docs/dependencies/README.md]
---

# Final structure

**Status: proposed architecture, approval pending. Nothing here is
implemented; all runtime acceptance is NOT RUN.** The binding versions of
every diagram, decision and target live in /specification.md (Cells 02–07);
this file is the navigable map the Director asked to be finalized.

## Fixed layers and owners

| Layer | Owner | Contract anchor |
|---|---|---|
| Assistant intent, commitments, context selection, run orchestration | Larry control plane (MD reducer; MI proposes) | D01/D02/D09 |
| Persistence, replication, durable records | Logical-DB | D21/D33 (memory blocked until ready) |
| Authenticated authority, grants, revocation | Secure-Authority | D07/D21 |
| Process/compute containment, leases | Nova | D11/D21 |
| Workspace claims, editor integration | Moo | D09/D21 |
| Internet egress, browser research | Sentinel | D21/D10 |
| Foundation crates and dependency root | Logicalworks-Crates | D21, /docs/dependencies/README.md |

Ownership rule: references to pre-existing sibling identities and receipt
protocols enter only through an explicit adapter decision (D21/D28); Larry
never becomes a second authority for a sibling's responsibility.

## Request lifecycle (summary)

Admission is durable and acknowledged, not successful: authenticate, persist
request identity and digest, queue or steer at a safe boundary, compile a
bounded context manifest, propose typed effects, gate authority/budget/
version at the reducer, dispatch to the canonical owner, observe or
reconcile OutcomeUnknown, verify domain obligations, close lifecycle and
publish a terminal receipt. The full flowchart and effect state machine are
canonical in /specification.md Cell 02. Cancellation enters reconciliation;
it does not rewrite an accepted external effect into a non-event.

## Determinism and reconciliation invariants (2026-09-16 additions)

- D36: authorization is a pure function of the declared effect, policy
  snapshot, capability state and reserved budget snapshot; replaying the
  recorded effect log yields bit-identical authorization decisions (E55).
  One effect-metadata schema carries
  `reconciliation_strategy ∈ {queryable, idempotent-retry-safe,
  requires-human, always-compensate}`; OutcomeUnknown can be permanent
  (E56). Budget leases (D39) expire through the same reconciliation
  machinery instead of a separate janitor authority.
- D40: superseded memory records freeze their decay clock; pinned
  commitments close only by an authorized transition; compacted nodes leave
  cryptographic tombstones and the compaction reversibility claim is
  declared explicitly (E60).

## File-specific excellence record

Requirements R01–R78 referenced by anchor only. Axes: decoupled ownership
and idiomatic structure presentation. Predicted outcome: one final navigable
map with zero competing definitions. Verification: structural gate, link and
traceability checks. Unresolved: D01–D41 ratification, P1–P8 proofs and all
runtime results remain NOT RUN.
