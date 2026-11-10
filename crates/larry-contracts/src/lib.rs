//! Typed contracts for the Larry runtime.
//!
//! Scope: the C02 work package of the Larry specification — the typed
//! command/event/effect/terminal contract, the finite run-state model, the
//! pure-authorization replay surface and the authority model — as governed
//! by this repository's `specification.md` (IDs R/D/E/C/A refer to its
//! inventory). The contract algebra is pure; the workspace
//! coordinator owns only bounded disposable projections and retry identities.
//! Persistence waits for Logical-DB readiness and never enters here as a
//! substitute store.
//!
//! Module map:
//!
//! - [`ids`] — newtype identifiers over lgwks_std UUIDs and BLAKE3 content
//!   digests; distinct kinds are not interchangeable.
//! - [`state`] — the finite run-state algebra and its single total, pure
//!   transition function (Cell 04).
//! - [`event`] — recorded events: gap-free per-run sequencing, causal
//!   parents across runs, bounded records (Cell 04 ordering, E35).
//! - [`command`] — command envelopes and identity-driven duplicate
//!   suppression: same identity + identical content is idempotent replay,
//!   same identity + different content is a typed conflict (E28).
//! - [`effect`] — effect intents, stable operation IDs, unified
//!   reconciliation strategies (R72), immutable terminal receipts and
//!   superseding resolutions.
//! - [`authority`] — grants, the attenuation equation
//!   `child = parent ∩ policy ∩ budget ∩ expiry` and the effect-boundary
//!   re-validation (Cell 04, D07; P1 remains a proof obligation).
//! - [`replay`] — the pure authorization reducer and recorded-log replay:
//!   pinned inputs replay bit-identically; mutated inputs diverge only as
//!   explicit findings (R71, D36, E55; P5 remains a proof obligation).
//! - [`evidence`] — evidence provenance as a type, so no grader output reaches an improver (larry#63).
//! - [`harness`] — the harness as a typed, content-hashed, validated value (larry#61).
//! - [`error`] — every typed failure variant the contract surface returns.
//! - [`workspace`] — bounded workspace projections and the shared Continue
//!   dispatch boundary, with an explicit unavailable Moo owner disposition.
//!
//! Serialization is available through the crate's `serde` cargo feature
//! (on by default), which rides the `lgwks_std/json` edge — the full serde
//! crate, derive macros included, re-exported by lgwks_std. No direct
//! serde dependency exists or is registered. With `--no-default-features`
//! the contracts compile without any serialization edge.

#![forbid(unsafe_code)]

pub mod authority;
pub mod binding;
pub mod command;
pub mod effect;
pub mod error;
pub mod event;
pub mod evidence;
#[cfg(test)]
mod fixture;
pub mod harness;
pub mod ids;
pub mod replay;
pub mod state;
pub mod workspace;
