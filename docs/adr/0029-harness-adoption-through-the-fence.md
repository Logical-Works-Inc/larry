---
type: ADR
title: "ADR 0029: harness adoption through the fence"
description: Why a harness version is adopted by an ordinary submitted run whose success makes the hash active for one tenant, and why that adoption is bounded by the journal record.
resource: /docs/adr/0029-harness-adoption-through-the-fence.md
tags: [larry, adr, runtime, harness, fence, epic-60]
generated: {by: claude-code, at: 2026-11-10}
verified: {by: claude-code, method: nextest run of crates/larry-runtime/tests/sim_harness_adoption.rs and the workspace}
status: accepted
stale_after: 2027-02-10
sources: [larry#61, larry#60, docs/adr/0028-harness-as-a-content-hashed-value.md, docs/adr/0024-fence-on-broker.md, specification R46]
---

# 0029: harness adoption through the fence

Status: accepted, 2026-11-10 (larry#61 part 2, epic larry#60)

## Context

ADR 0028 made the harness a hashed value. Part 2 must make adopting one an authorized effect: a
durable intent, a recorded authorization, current freshness, and a rollback that is an earlier
adoption. Two tenants with one tree must hold separate records. `CommandKind` and `EventKind` are
closed by the specification, so no new vocabulary is allowed.

## Decision

- An adoption is an ordinary `Submit` run. `WorkProposal` gains an optional `harness`
  (`HarnessAdoption`: a version and, for a new harness, the tree's canonical bytes; no tree means a
  rollback). The field is absent from the encoding when empty, so every old journal decodes as before.
- The adoption is recorded in the `EffectIntentRecorded` payload, after the gate authorized the work.
  A denied adoption therefore leaves no trace. The brief named `CommandAccepted`, but that payload
  carries no proposal body, and the fold reads events only.
- The fold keeps one `TenantHarness` per tenant inside `RunFold`, keyed by the run's tenant, never the
  proposal's declared one (D42, E63). The intent reserves the adoption as in flight. The hash becomes
  active only on a `Succeeded` terminal receipt for a run that is not cancelling. Failed, compensated,
  unknown-permanent, cancelled and denied runs leave the active hash where it was.
- `TenantHarness::vet` is the one rule set, asked before the command is written and again by the
  fold. It refuses: a tree that does not decode canonically, a tree that is not the version's hash, a
  second adoption in flight, a parent that is not the active hash, more than 64 distinct
  trees, and a rollback to a hash the tenant never adopted. A rollback is written against the harness
  it replaces: parent is the active hash, hash an earlier one.
- The proposal must declare `FileWrite`, the closest existing scope, so the gate decides it. A
  `harness.adopt` scope would be a specification change.
- The history keeps the last 256 adoptions. Replay and restore re-derive everything from the journal.

Rejected:
- A new `EventKind` or `CommandKind`: closed by the specification.
- A second harness store beside the journal: two truths, and a disk read at run time.
- Adopting at `CommandAccepted`: a denied or failed effect would already be active.

## Consequences

- The journal record bounds adoption. lgwks_bot refuses a record over 64 KiB and Larry's records spell
  bytes as JSON numbers, so one adoption carries at most `MAX_ADOPTED_TREE_BYTES` (2048) of tree. The
  widest record of a worst-case adoption is 38217 of 65524 bytes (MEASURED, `durable/fit.rs`). The
  durable journal also refuses any entry past `MAX_RECORD_BYTES` before the fold sees it.
- A larger harness needs chunked content records or a binary journal encoding. That is a follow-up;
  the per-record cap is lgwks_bot's.
- A tenant with an adoption left open (unknown outcome) cannot start another until it settles.
- No production owner is wired. The executor reading the active harness is #62's run-path work.
- Covered by `tests/sim_harness_adoption.rs` (32 seeds each).
