//! The Rocco runtime: the lowest invariant, made executable.
//!
//! The specification's C02 work package defines a typed command/event/effect
//! contract and a finite run-state model, and `rocco-contracts` implements
//! that algebra purely. This crate is the thing that *drives* it: the
//! deterministic tick, the bounded journal, the admission and authorization
//! gate, the effect lifecycle and replay.
//!
//! Three invariants hold the whole crate up, and each is falsifiable from
//! outside:
//!
//! 1. **The journal is authoritative (D04).** State is a fold over recorded
//!    facts. [`runtime::replay`] rebuilds a projection from a journal alone
//!    and consults no live state, so recovery is a re-fold and a projection is
//!    never a source of truth.
//! 2. **Nothing is dispatched without a durable intent and a recorded
//!    authorization.** The intent is written to the journal before the owner
//!    is called, and it carries the pinned authorization inputs, so a crash
//!    inside the dispatch window leaves the effect reading as *reserved* —
//!    "may have happened" — never as a non-event. [`EventKind`] has no
//!    "dispatched" variant and this crate does not invent one.
//! 3. **Replay is bit-identical (R71, E55).** Every recorded authorization
//!    carries its own pinned snapshots, so
//!    [`projection::replay_authorizations`] re-derives each decision from its
//!    own inputs and reports divergence explicitly rather than passing
//!    silently.
//!
//! ## Freshness is not reproducibility (R87, D48, E71)
//!
//! Recording a decision is not the same as being entitled to act on it
//! later, so authorization is deliberately two things. [`gate`] is the
//! reproducible half: pure, pinned, replayable. [`RevocationFence`] is the
//! fresh half: live host state holding the security epoch, the withdrawn
//! operations and the offline exposure bound, consulted at the single point
//! where an effect reaches its owner to change something. Replaying the
//! journal re-derives the decision bit-identically; it never re-authorizes
//! the reach. A stale epoch, a named revocation or a spent offline allowance
//! refuses the reach and leaves the effect visibly open rather than
//! inventing a disposition for it.
//!
//! ## What this crate does not claim
//!
//! It integrates **no** canonical owner. Effects go through
//! [`owner::EffectOwner`], whose only implementations here are the
//! deterministic doubles in [`owner::fixture`]. Nothing in this crate
//! contacts Moo, Sentinel, Nova, Logical-DB, Secure-Authority or a network.
//! Persistence is Logical-DB's and the memory hold is intact; the journal is
//! bounded and in-memory, and durability across process death is not claimed.
//! P1–P8 remain proof obligations.
//!
//! ## The ECS seam
//!
//! [`projection::ProjectionBackend`] is the seam for a `bevy_ecs` view. The
//! question of whether an ECS world may ever hold *authority* is left open by
//! the Director and is recorded in `docs/architecture/runtime.md`; this crate
//! does not decide it. Today the only implementation is
//! [`projection::FoldProjection`], which uses `BTreeMap` throughout so no
//! operation depends on hash iteration order.
//!
//! ## Module map
//!
//! - [`clock`] — the pinned time source; one instant per tick (E30).
//! - [`journal`] — the bounded, append-only authoritative record (D04).
//! - [`payload`] — the typed bodies of the closed event vocabulary.
//! - [`projection`] — the read view, the ECS seam and the replay report.
//! - [`gate`] — the pure authorization reducer and the boundary re-check.
//! - [`RevocationFence`] — dispatch-time freshness: epoch, revocation,
//!   offline bound. The module behind it is private and this crate re-exports
//!   the three types below rather than opening a fourth namespace; the host
//!   needs a control surface for the fence, not the ability to reach past it.
//! - [`owner`] — the canonical-owner trait and its deterministic doubles.
//! - [`runtime`] — the tick, its stages and the typed report.
//! - [`error`] — every refused operation, with structured context.

#![forbid(unsafe_code)]

pub mod clock;
pub mod consumer;
pub mod error;
pub mod gate;
pub mod journal;
pub mod owner;
pub mod payload;
pub mod projection;
pub mod runtime;

mod fence;

pub use fence::{FenceError, FenceRefusal, RevocationFence};

/// Maximum ready-work descriptors before admission applies backpressure
/// (Cell 05: 256 descriptors, no silent drop).
pub const MAX_READY_WORK: usize = 256;

/// Maximum journal entries before an append is refused (Cell 05: bounded
/// retained data, explicit overflow behaviour).
pub const MAX_JOURNAL_ENTRIES: usize = 65_536;

/// Maximum runs the projection tracks.
pub const MAX_TRACKED_RUNS: usize = 1_024;

/// Maximum ready commands one tick may admit, so a single tick is bounded
/// work regardless of queue depth.
pub const MAX_RUNS_PER_TICK: usize = 64;

/// Maximum recorded effects per run.
pub const MAX_EFFECTS_PER_RUN: usize = 64;

/// Maximum mutating reaches permitted between authentications (R87, D48,
/// E71).
///
/// The bound is deliberately "one tick's worth": it equals
/// [`MAX_RUNS_PER_TICK`], the most effects a single tick can admit, so a host
/// that authenticates once per tick never meets the bound, while a host that
/// stops authenticating gets exactly one more tick's worth of effects out and
/// then no more. Offline exposure is counted, never timed — a suspended
/// process resumes with a staler fence, not a fresher one, and a rolled-back
/// clock cannot recharge a count.
pub const MAX_OFFLINE_DISPATCHES: u32 = 64;

/// Crate version as declared in `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_exposed() {
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn bounds_match_the_specification_cell_05() {
        // Retyping the Cell 05 numbers here is deliberate: if a bound moves,
        // this test is the place the change has to be justified.
        assert_eq!(MAX_READY_WORK, 256);
        assert_eq!(MAX_RUNS_PER_TICK, 64);
        assert_eq!(
            usize::try_from(MAX_OFFLINE_DISPATCHES),
            Ok(MAX_RUNS_PER_TICK)
        );
        const { assert!(MAX_EFFECTS_PER_RUN < MAX_JOURNAL_ENTRIES) };
    }
}
