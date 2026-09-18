//! Typed contracts for the Rocco runtime.
//!
//! Scope: the C02 work package of the Rocco specification — the typed
//! command/event/effect/terminal contract, the finite run-state model, the
//! pure-authorization replay surface and the authority model — as governed
//! by the private specification repository `srinji-kaggss/Rocco` (IDs R/D/E/C/A
//! refer to that inventory). This crate is stateless: persistence waits for
//! Logical-DB readiness and never enters here as a substitute store.
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
//! - [`error`] — every typed failure variant the contract surface returns.
//!
//! Serialization is available through the crate's `serde` cargo feature
//! (on by default), which rides the `lgwks_std/json` edge — the full serde
//! crate, derive macros included, re-exported by lgwks_std. No direct
//! serde dependency exists or is registered. With `--no-default-features`
//! the contracts compile without any serialization edge.

#![forbid(unsafe_code)]

pub mod authority;
pub mod command;
pub mod effect;
pub mod error;
pub mod event;
pub mod ids;
pub mod replay;
pub mod state;

/// Crate version as declared in Cargo.toml.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
pub(crate) mod test_support {
    //! Shared test helpers. The workspace lints deny `unwrap`, `expect`
    //! and `panic` everywhere, so test code surfaces unexpected
    //! construction failures through this explicit helper instead.

    /// Extracts the `Ok` value or fails the test with the error text.
    ///
    /// The scoped `allow` covers only this test-only failure path;
    /// production code has no equivalent escape hatch.
    #[allow(clippy::panic)]
    pub(crate) fn expect_ok<T, E: core::fmt::Display>(result: Result<T, E>) -> T {
        match result {
            Ok(value) => value,
            Err(err) => panic!("unexpected error: {err}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_exposed() {
        assert!(!VERSION.is_empty());
    }
}
