---
type: ADR
title: docs/architecture/runtime.md, part 2 of 3
description: Part 2 of /docs/architecture/runtime.md, in order; the index lists every part.
resource: /docs/architecture/runtime/02-adr-r05-the-ecs-seam-exists-the.md
tags: [larry, runtime, determinism, replay, ecs, bevy]
generated: {by: claude-code, at: 2026-09-20}
verified: {by: claude-code, method: cargo build/test/clippy/fmt, lgwks-deps check, check_governance.py, unittest, all local; no hosted CI}
status: implementation-verified-locally-owner-integration-pending
stale_after: 2026-10-20
sources: [/specification.md, /AGENTS.md, /contract/APPROVED.toml, /log.md, Director decisions 2026-09-20]
---
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
