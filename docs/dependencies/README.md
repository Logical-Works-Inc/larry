---
type: Design
title: Rocco dependency root and sibling order
description: Logicalworks-Crates is the core starting point for dependencies; estate siblings follow through narrow adapters with pinned revisions.
resource: /docs/dependencies/README.md
tags: [rocco, dependencies, logicalworks-crates, adapters]
generated: {by: opencode, at: 2026-09-16}
verified: {by: opencode, method: Director decision 2026-09-16 and ledger revision pins; no integration executed}
status: final-structure-v1.1
stale_after: 2026-10-16
sources: [Director conversation 2026-09-16, /research-ledger.md, /specification.md]
---

# Dependency root: Logicalworks-Crates

**Director-settled 2026-09-16: Logicalworks-Crates is the core starting
point for dependencies.** Every Rocco dependency chain begins there; no
component, adapter or tool may root its dependency graph elsewhere. This
settles the starting point; it does not by itself ratify D21's reuse
mechanics or claim any sibling is production-ready from its name (D21).

## Foundation surfaces

`std`, `lgwks_std`, `lgwks_bot`, `lgwks_deps`, with `lgwks_ast` as the
existing parser. Third-party dependencies enter only through the estate's
admission/storefront process; no copied vendor trees and no dual engines.

## Sibling adapter order

| Order | Owner | Rocco consumes | Ledger-pinned inspection revision |
|---|---|---|---|
| 1 | Logicalworks-Crates | std/bot/deps foundation, lgwks_ast parser | `30ae69deca86eb25c95e113296eb2d08f50526a7` |
| 2 | Logical-DB | durable records, sync, restore; memory persistence | `e4e1ead86042cc917e959c3f5fb2206ff61f9319` |
| 3 | Secure-Authority | capability issue/verify (Biscuit, TTL ceiling) | `ab417c9f8f664271cf5966e24c51bdc337aa4bed` |
| 4 | Nova | compute leases, isolation profiles | `34fd34aa757c4d4fce941beb92cc297f075be80c` |
| 5 | Moo | workspace claims, editor integration | `a4e64252368226913f92a1453c8b435bd1efbede` |
| 6 | Sentinel | egress, pinned fetch, search boundary | `a3d62f765352636a4f930d0ad3d1f222c13be9f7` |

Revisions identify inspected reference snapshots; they are not production
readiness claims and are re-pinned through C01 compatibility testing before
any integration. Missing sibling semantics are integration work in the
sibling owner, never a reason to duplicate them inside Rocco.

## Gate

C01 owns the adapter/API/source-license manifest and conformance fixtures;
no implementation cell bypasses this dependency order, and the memory hold
(Logical-DB readiness) stays intact.

## File-specific excellence record

Requirement: one explicit dependency root with pinned reuse evidence.
Axes: decoupled, portable, multi-tenanted-by-default integration. Predicted
outcome: zero parallel authorities and zero vendored twins. Verification:
ledger anchors above, structural gate, C01 conformance fixtures when they
exist. Unresolved: sibling readiness proofs; all integration results NOT RUN.
