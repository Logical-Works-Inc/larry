---
type: Log
title: log.md, part 7 of 15
description: Part 7 of /log.md, in order; the index lists every part.
resource: /docs/log/07-2026-09-20-the-runtime-implemented-the.md
tags: [governance, decisions, evidence]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: explicit conversation decisions and source inspection}
status: active
stale_after: 2026-10-15
sources: [Director conversations 2026-09-15 through 2026-09-17, /specification.md, /research-ledger.md]
---
## 2026-09-20: the runtime, implemented — the lowest invariant

Director direction: work on the lowest invariant, the runtime, based on Bevy;
the existing frontend is a mockup and will change. Director decisions this turn:
build the projection seam now and admit a Bevy backend on later sign-off; defer
the question of whether an ECS world may ever hold authority; take the full C02
runtime slice; update specification, this log, a new runtime ADR and the
dependency register.

Semantic change: `crates/larry-runtime` is a new workspace member and the first
code in this repository that drives the C02 contract algebra rather than
describing it. It is a deterministic, bounded tick over an append-only journal:
admission with identity-based duplicate suppression, the pure authorization gate
of D36 with its pinned snapshots recorded alongside each decision, durable intent
written before dispatch, observe and reconcile with the permanent unknown of
R72, a bounded retry budget tracked separately from a bounded reconciliation
budget, cancellation that completes only once owned work has stopped, immutable
terminal receipts, and restoration by re-folding the journal.

Design decision recorded as ADR-R01–R06 in `docs/architecture/runtime.md`. The
load-bearing one: the journal is authoritative and the projection is a fold over
it (D04 taken literally), which is what makes replay cheap and recovery a
re-fold. The closed `EventKind` vocabulary was deliberately **not** extended with
an "EffectDispatched" fact, because a crash inside the dispatch window is
genuinely ambiguous and the vocabulary's silence encodes that.

Affected IDs: none added. R01–R96, D01–D53, E01–E80, C00–C12 and A1–A9 are
unchanged; the inventory stays 96/53/80/13/9. The runtime is recorded as an
implementation of C02, not a new decision, and the two open questions it raises
are recorded in Cell 12 as open rather than ratified.

Evidence: `cargo build --bins --locked` passed; `cargo test --workspace
--all-targets --locked` passed 131 tests (57 + 12 `larry-contracts`, 30 + 6
`larry-demo`, 2 + 8 + 16 `larry-runtime`); `cargo clippy --workspace
--all-targets --locked -- -D warnings` passed; `cargo fmt --all -- --check`
passed; `lgwks-deps check .` returned `OK . — 1 semantic approvals, every
authored external edge is owned`, with **no new registered edge**;
`python3 tools/check_governance.py` PASS on 96/53/80/13/9; `python3 -m unittest
discover -s tests` passed 47; `git diff --check` clean. Reranker receipts
`tools/receipts/20260920-runtime-core-pre-exec.json` and `-post-exec.json`.
Hosted CI has not run for this change.

The three invariants the runtime is built to refuse the alternative of: nothing
is dispatched without a durable intent and a recorded authorization; state is a
fold over recorded facts and never a source of truth; and every recorded
authorization re-derives bit-identically from its own pinned inputs, with a
decision-changing mutation surfacing an explicit divergence rather than passing
silently.

Unresolved, and not claimed: no canonical owner is integrated — the only
`EffectOwner` implementations are the deterministic `owner::fixture` doubles and
nothing contacts Moo, Sentinel, Nova, Logical-DB, Secure-Authority or a network;
the journal is in memory, so recovery-from-a-journal-in-hand is claimed and
survival of a process death is not; P1–P8 remain proof obligations and are
falsified by tests, not discharged by proof; compensation is handled but never
produced, so `RolledBack` is unreachable in this slice; policy, capability and
validity snapshots are pinned for the runtime's lifetime, so D26 activation is
unimplemented; a denial records the run's `Fail` transition but not its reason,
because the closed vocabulary has no place for it. Retry and reconciliation
exhaustion leave an effect visibly unresolved with no escalation policy — honest
and fail-closed, but a policy is owed.

Dependency: `bevy_ecs` is **not** admitted and is not an edge. The admission
request is recorded as a comment in `contract/APPROVED.toml` with the row text
ready to sign or refuse; an unsigned row carrying a Director's name would be a
fabricated approval, which is the exact failure that register prevents.
Independent research on Bevy 0.19.1 is folded into the ADR: `bevy_ecs` standalone
is 117 crates and 13.6 s cold rather than the facade's 672 and 130 s, and it is
deterministic run-to-run by default because `multi_threaded` is not in its
default features — but Bevy's own documentation declines to guarantee ordering,
ambiguity detection is off by default, `FixedUpdate` is wall-clock driven, entity
ids are explicitly not stable across runs, and a major breaking release lands
roughly every four months. That research independently reaches the same split
this work package implements. The ECS-authority question itself stays open.
