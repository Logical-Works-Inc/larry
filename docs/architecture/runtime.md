---
type: ADR
title: Rocco runtime — the lowest invariant, made executable
description: The journal-authoritative deterministic runtime, its ECS seam, the deferred authority question and the pending Bevy dependency admission.
resource: /docs/architecture/runtime.md
tags: [rocco, runtime, determinism, replay, ecs, bevy]
generated: {by: claude-code, at: 2026-09-20}
verified: {by: claude-code, method: cargo build/test/clippy/fmt, lgwks-deps check, check_governance.py, unittest, all local; no hosted CI}
status: implementation-verified-locally-owner-integration-pending
stale_after: 2026-10-20
sources: [/specification.md, /AGENTS.md, /contract/APPROVED.toml, /log.md, Director decisions 2026-09-20]
---

# Outcome

`crates/rocco-runtime` is the first code in this repository that *drives* the
C02 contract algebra rather than describing it. Before this change the finite
run-state model, the pure authorization reducer, the effect intents and the
terminal receipts all existed in `rocco-contracts` as pure functions and values
with nothing to sequence them; the specification's runtime control flow was a
diagram, not a program.

The runtime is the lowest invariant rather than a feature: every higher claim in
the specification — that a commitment survives a crash, that an effect is never
dispatched without authority, that an outcome is never invented — is only as
true as this layer's ability to refuse the alternative.

# ADR-R01 — The journal is authoritative; everything else is a fold

Decision: an append-only, bounded journal is the only source of truth. The
run-state view, the effect phases and the receipt list are a fold over it.
`runtime::replay` rebuilds a projection from a journal alone and consults no
live state, so recovery is a re-fold and a projection is discardable at any
moment.

This is D04 taken literally — "event history is authoritative; graph and context
are projections" — and it is the property that makes every determinism argument
below cheap. A test discards the live projection, re-folds it from the journal,
and asserts the two agree.

Rejected: mutable world state with the journal as an audit trail. That inverts
D04 and makes replay correctness depend on the world's iteration order rather
than on the log.

# ADR-R02 — The tick is a fixed, bounded stage sequence

Decision: one tick samples the observed instant exactly once and then runs four
stages in a fixed order — admit, reconcile, cancel, sample. Every stage is
bounded by an explicit constant; every collection is a `BTreeMap` or an
insertion-ordered `Vec`; no stage consults the wall clock except the first.

The single pinned instant is E30/D36: expiry is a pinned input, so an expired
approval is a typed denial and never consent by timeout. One instant per tick
also means every authorization made in a tick is comparable, because they were
all decided against the same observed time.

Rejected: reading `SystemTime::now()` at each decision site. That is
unreplayable by construction — the same journal would re-derive different
decisions on a different day.

# ADR-R03 — No dispatch without a durable intent carrying its own replay record

Decision: the intent is written to the journal *before* the owner is called, and
it carries the pinned authorization inputs and the decision they produced. A
crash inside that window therefore leaves the effect reading as `Reserved` —
"may have happened" — never as a non-event.

`contracts::event::EventKind` is a closed five-variant vocabulary owned by the
specification, and it has no "dispatched" variant. That absence is load-bearing
and this work package did **not** extend the vocabulary to get a more convenient
picture. An effect that crashed mid-dispatch is genuinely ambiguous, and the
journal's silence about dispatch is what encodes that.

Rejected: adding an `EffectDispatched` event kind. It would let a resumed run
distinguish "dispatched" from "not yet dispatched", which is exactly the
distinction the runtime cannot actually make across a crash.

# ADR-R04 — Reconciliation is not retry, and neither is unbounded

Decision: an effect's *dispatch* budget and its *reconciliation* budget are
tracked separately. A strategy that declares retry safe (`IdempotentRetrySafe`)
re-dispatches up to the bounded ceiling, reusing the stable operation ID.
`Queryable` asks what happened. `RequiresHuman` and `AlwaysCompensate` can
neither be queried nor retried, so the only truthful record is `Unknown`, which
resolves to a permanent disposition — not an invented success and not an
invented failure.

Restoration spends the dispatch budget of any effect the journal leaves
`Reserved`, because the journal proves reservation and cannot prove the effect
never ran. Reconciliation budget is untouched, because asking what happened is
not retrying. Once a budget is spent the effect is left *visibly unresolved*:
no receipt, no status rewriting. The escalation policy above that is a later
work package's, and this runtime deliberately does not paper over its absence.

Rejected: treating an unknown outcome under a non-queryable strategy as a
failure. That is the single most dangerous simplification available here — it
turns "we do not know whether this happened" into a false claim that it did not.

# ADR-R05 — The ECS seam exists; the authority question is deliberately open

Decision: the runtime reads state only through `ProjectionBackend`. The sole
implementation today is `FoldProjection`. **This is not a decision that an ECS
world may ever hold authority**, and it is not a decision that it may not. The
Director deferred that call on 2026-09-20, and this ADR records it as open
rather than settling it by implementation accident.

What the seam buys is that the answer stays cheap: a `bevy_ecs` backend can be
admitted behind the trait without the reducer, the journal, the gate or the
effect lifecycle noticing, because none of them reference the world.

Constraints on any such backend, verified against Bevy 0.19.1 on 2026-09-20 and
recorded here so they are not rediscovered later:

- **Bevy's own contract does not guarantee ordering.** Its
  `nondeterministic_system_order` example states that unless order is explicitly
  specified, relative order is nondeterministic, and `Query::iter` documents
  that "iteration order is not guaranteed". Bevy's determinism is real but
  *emergent*, not promised.
- **`bevy_ecs` standalone is nonetheless deterministic run-to-run by default**,
  because `multi_threaded` is not in its default feature set — so the
  single-threaded executor runs a cached `Vec` topological order over a
  `FixedHasher` collection, not `RandomState`. This corrects a natural
  assumption that Bevy is nondeterministic out of the box; it is not.
- **But ambiguity detection is off by default** (`LogLevel::Ignore`). Bevy will
  run order-ambiguous conflicting systems and say nothing. Raising
  `ScheduleBuildSettings::ambiguity_detection` to `LogLevel::Error` turns that
  silent hazard into a schedule-build failure, and is the single highest-value
  thing Bevy offers this runtime.
- **`FixedUpdate` cannot be the tick.** It is driven by `Time<Virtual>`, is
  wall-clock derived, and "may run 0, 1 or more times during a single update".
- **Entity identities are explicitly not stable across runs**, and the world
  serialization path remaps them and pulls in the asset, transform and camera
  crates. There is no ECS-only, byte-exact world snapshot.
- **Hidden state exists that replay cannot see**: `Local<T>` — including the
  `MessageCursor` behind the 0.19 rename of `EventReader` to `MessageReader` —
  change-detection ticks, and in-flight `Commands` buffers.
- **Parallel query iteration is not orderable, and the ECS documentation says so
  obliquely.** `Query::iter` states that "iteration order is not guaranteed";
  `par_iter`/`par_iter_mut` turn that from an unspecified-but-stable archetype
  order into genuine thread-race order. The Bevy cheatbook page on parallel
  iteration, checked 2026-09-20, states no determinism rule for queries at all —
  its only ordering remark is that `par_read` makes event order "undefined".
  A backend that folded an aggregate in a parallel query loop would break replay
  in precisely the way this runtime exists to prevent.

The guard that makes this a non-issue today is that the journal's order is
*total and checked*, not merely conventional: an append that skips a sequence
number is refused, and a falsifier in `tests/determinism.rs` proves it. The fold
therefore consumes a totally ordered log rather than a scheduler's iteration
order. Any admitted ECS backend must preserve that property, and must not
parallel-iterate the fold.

A `bevy_ecs` projection would therefore have to be a single-threaded,
ambiguity-erroring, explicitly ordered view whose contents are always
re-derivable from the journal — which is precisely the shape ADR-R01 already
requires. If it cannot be that, it cannot hold authority.

Because Bevy's determinism is emergent rather than contractual, admission
carries a standing obligation: a replay-digest regression test that fails the
build if a `bevy_ecs` bump changes the fold. Bevy ships a major breaking release
roughly every four months, and `bevy_ecs` 0.19.1 moved both the event API and
the scene API. A runtime that must replay old logs cannot absorb that silently.

# Foundation facts for the pending admission, measured 2026-09-20

These exist so the Director signs or refuses with numbers rather than
impressions. Measured on this machine, macOS/darwin, dev profile, fresh target
directories; version and dependency figures read from crates.io and the Bevy
source at tag `v0.19.1`.

| Fact | Value |
|---|---|
| Bevy stable | 0.19.1 (2026-08-13); 0.20.0-rc.1 published 2026-09-15 |
| `bevy_ecs` 0.19.1 | `edition = "2024"`, `rust-version = "1.95.0"` — compatible with this workspace |
| `bevy_ecs` standalone, `default-features = false` | **117 crates in tree, 13.6 s cold** |
| Full `bevy` 0.19.1 default | 672 crates in tree, 130 s cold |
| `bevy_ecs` default features | `std`, `bevy_reflect`, `async_executor`, `backtrace` — **`multi_threaded` absent** |
| `bevy_world_serialization` 0.19.1 | drags in `bevy_asset`, `bevy_transform`, `bevy_camera` |
| Ecosystem lag | `bevy_roll_safe` 0.7.0 still pins `bevy ^0.18`; `bevy_ggrs` 0.22.0 is on `^0.19` |

The weight objection to Bevy is largely false at the `bevy_ecs` layer: 117
crates is an ordinary-service footprint, not a game engine, and it is roughly
5.7× smaller and 9.5× faster to build than the facade. The real costs are the
release cadence and the unspecified ordering contract, not the compile time.

This corroborates the split this work package already implements. The four
problems the runtime spends its complexity on — durable intent before dispatch,
authority recorded with its decision, budget spent as a reservation, and an
unknown outcome that is never rewritten into a success or a failure — are
exactly the four that no ECS layer addresses at any dependency count.

# ADR-R06 — `bevy_ecs` is not admitted yet, and this change adds no dependency edge

Status: **pending Director sign-off.** The runtime compiles against Rust `std`,
`lgwks_std` and `rocco-contracts` only. `lgwks-deps check .` reports one
semantic approval and every authored external edge owned, with no new row.

The dependency law in `/AGENTS.md` admits a new third-party crate only with an
owner, a reason and a human sign-off, and the default answer is no. `bevy_ecs`
has no row and was not given one by this change: an unsigned row carrying a
Director's name would be a fabricated approval, which is the exact failure this
register exists to prevent. The row text, ready to sign or to refuse, is in
`/contract/APPROVED.toml` as a comment.

`thiserror` is likewise not used here. Its register row names `rocco-contracts`
as the single `allowed_consumers` entry, so this crate hand-writes `Display` and
`std::error::Error` rather than widening a signed row as a local convenience.

# Evidence

Local gates, run from the repository root on 2026-09-20:

- `cargo build --bins --locked` — passed.
- `cargo test --workspace --all-targets --locked` — 131 passed, 0 failed
  (57 `rocco-contracts` unit, 12 `rocco-contracts` workspace, 30 `rocco-demo`
  unit, 6 `rocco-demo` integration, 2 `rocco-runtime` unit, 8 `rocco-runtime`
  determinism, 16 `rocco-runtime` lifecycle).
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — passed.
- `cargo fmt --all -- --check` — passed.
- `lgwks-deps check .` — `OK . — 1 semantic approvals, every authored external
  edge is owned`.
- `python3 tools/check_governance.py` — PASS, inventory 96/53/80/13/9.
- `python3 -m unittest discover -s tests` — 47 passed.
- `git diff --check` — clean.
- Reranker receipts: `tools/receipts/20260920-runtime-core-pre-exec.json` and
  `tools/receipts/20260920-runtime-core-post-exec.json`.

Hosted CI has not run for this change.

# What this does not claim

- **No canonical owner is integrated.** Effects pass through
  `owner::EffectOwner`; the only implementations in the crate are the
  deterministic `owner::fixture` doubles. Nothing contacts Moo, Sentinel, Nova,
  Logical-DB, Secure-Authority or a network.
- **No durability across process death.** The journal is bounded and in memory.
  Persistence is Logical-DB's and the memory hold is intact; this crate claims
  recovery *from a journal in hand*, not survival of a power loss.
- **No proof.** P1–P8 remain proof obligations. In particular P2 (atomic
  event+reservation and fenced effect ownership) and P5 (deterministic replay
  equivalence) are falsified by tests here, not discharged by proof; the
  contract module's own note that P5 awaits property tests and model checking
  still stands.
- **No compensation path.** `TerminalDisposition::Compensated` is handled but
  this slice never produces it; compensation is a new authorized effect and
  belongs with the C04 work.
- **No policy activation.** The policy, capability and validity snapshots are
  pinned for the runtime's lifetime. D26's explainable, atomically activated
  effective policy is unimplemented.
- **No frontend.** The existing AppCUI surface is a mockup and is expected to
  change; this work package is headless and touches no renderer.

# Unresolved

- ADR-R05's authority question, for the Director.
- ADR-R06's dependency admission, for the Director's signature.
- Retry and reconciliation exhaustion currently leave an effect visibly
  unresolved with no escalation. That is fail-closed and honest, but it is not
  a policy, and it needs one.
- The `Cancelling` run state completes as soon as its owned effects stop; there
  is no compensation-before-cancel path, so `RolledBack` remains unreachable in
  this slice.
- A denial records the run's `Fail` transition but not the denial *reason*: the
  closed `EventKind` vocabulary has no place for it, and inventing one is a
  specification change. The reason is returned in the tick report instead.
