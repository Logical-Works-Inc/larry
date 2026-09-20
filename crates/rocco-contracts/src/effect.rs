//! Effect intents, outcomes, reconciliation strategies and immutable
//! terminal receipts (spec Cell 04 effect protocol, R72, D36; work
//! package C02).
//!
//! The effect protocol is: validate/authorize -> reserve budget and write
//! intent -> dispatch with a stable operation ID -> observe/reconcile ->
//! persist outcome -> verify -> integrate/finalize. This module carries the
//! typed records for that protocol; dispatch, persistence and observation
//! belong to later work packages and Logical-DB, not here.
//!
//! R72 invariants encoded:
//!
//! - One unified metadata field — [`ReconciliationStrategy`] — declares
//!   for every effect how its outcome is reconciled:
//!   `Queryable`, `IdempotentRetrySafe`, `RequiresHuman`,
//!   `AlwaysCompensate`.
//! - **`OutcomeUnknown` is a declared terminal disposition, never an
//!   implicit retry.** [`terminal_disposition`] maps an unknown outcome to
//!   a terminal receipt only when the declared strategy makes the unknown
//!   permanent (`RequiresHuman`, `AlwaysCompensate`); for queryable and
//!   idempotent-retry-safe effects an unknown outcome is *not yet*
//!   terminal — it must be resolved by status query or a bounded,
//!   declared retry, never by blind resend (E56). Compensation under
//!   `AlwaysCompensate` is a new authorized effect, not time travel.
//! - **Terminal receipts are immutable once constructed.** The receipt
//!   exposes no mutation API; later evidence appends a
//!   [`SupersedingResolution`] that references the original receipt's
//!   digest (Cell 04: superseding resolution, not an in-place rewrite).

use crate::authority::SecurityEpoch;
use crate::ids::{ContentDigest, EffectId, OperationId, RunId, TenantId};

#[cfg(feature = "serde")]
use lgwks_std::json::serde::{Deserialize, Serialize};

/// Maximum dispatch attempts for an eligible effect (Cell 05: retry <= 3).
pub const MAX_EFFECT_ATTEMPTS: u8 = 3;

/// How an effect's outcome is reconciled (R72, D36 unified metadata).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub enum ReconciliationStrategy {
    /// The endpoint supports status queries: resolve unknowns by querying.
    Queryable,
    /// The endpoint accepts idempotent retries under the stable operation
    /// ID: retries are bounded and declared, never blind.
    IdempotentRetrySafe,
    /// No query, no idempotency: an unknown outcome is permanent and
    /// requires a human decision.
    RequiresHuman,
    /// No query, no idempotency: an unknown outcome is permanent and is
    /// discharged through a new authorized compensation effect.
    AlwaysCompensate,
}

impl ReconciliationStrategy {
    /// The declared retry policy for this strategy.
    ///
    /// `RequiresHuman` and `AlwaysCompensate` never retry: an unknown
    /// outcome for them is permanent, and retry eligibility is impossible
    /// for non-idempotent effects whose outcome is unknown (Cell 04, E56).
    #[must_use]
    pub const fn retry_policy(self) -> RetryPolicy {
        match self {
            Self::Queryable => RetryPolicy::QueryUntilResolved,
            Self::IdempotentRetrySafe => RetryPolicy::Bounded {
                max_attempts: MAX_EFFECT_ATTEMPTS,
            },
            Self::RequiresHuman | Self::AlwaysCompensate => RetryPolicy::None,
        }
    }

    /// Whether an unknown outcome under this strategy is a declared
    /// permanent disposition (R72: OutcomeUnknown can be permanent).
    #[must_use]
    pub const fn unknown_outcome_is_permanent(self) -> bool {
        matches!(self, Self::RequiresHuman | Self::AlwaysCompensate)
    }
}

/// The retry policy derived from a reconciliation strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub enum RetryPolicy {
    /// No retry is ever authorized for this effect.
    None,
    /// At most `max_attempts` dispatches, reusing the stable operation ID.
    Bounded {
        /// The inclusive attempt ceiling (Cell 05 default: 3).
        max_attempts: u8,
    },
    /// Retry is replaced by status query until the endpoint resolves.
    QueryUntilResolved,
}

/// A reserved, recorded intent to perform one external effect.
///
/// Written before dispatch (Cell 04: reserve budget and write intent) so
/// that a crash after reservation leaves durable evidence that the effect
/// may have happened — which is exactly why an ambiguous crash can never
/// be treated as "did not happen".
///
/// The intent is also the *recovery contract* D48 requires to be persisted
/// before dispatch: [`EffectIntent::admission_epoch`] records the authority
/// generation the reservation was admitted under, which is what a restored
/// runtime needs in order to know whether that reservation still stands.
/// The epoch is supplied by the runtime's dispatch fence, never by the
/// proposal being admitted — a command cannot declare the security facts of
/// its own admission (D42, E63).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub struct EffectIntent {
    /// The local effect record identity.
    pub id: EffectId,
    /// The stable operation ID presented to the external endpoint; reused
    /// verbatim on every reconciliation attempt.
    pub operation: OperationId,
    /// The run the effect belongs to.
    pub run: RunId,
    /// The tenant the effect belongs to.
    pub tenant: TenantId,
    /// The declared unified reconciliation metadata (R72).
    pub strategy: ReconciliationStrategy,
    /// The authority generation this reservation was admitted under
    /// (R87, D48). An effect is dispatchable only while it is still at that
    /// generation; see `rocco_runtime::fence`.
    pub admission_epoch: SecurityEpoch,
}

/// The observed outcome of a dispatched effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub enum EffectOutcome {
    /// The endpoint confirmed success.
    Succeeded {
        /// The stable operation ID of the effect.
        operation: OperationId,
    },
    /// The endpoint confirmed failure (a definitive no, not an unknown).
    Failed {
        /// The stable operation ID of the effect.
        operation: OperationId,
    },
    /// The outcome is ambiguous: ambiguous timeout, crash or unsupported
    /// status. Never blindly retried; its disposition follows the declared
    /// strategy (R72, E56).
    Unknown {
        /// The stable operation ID of the effect.
        operation: OperationId,
    },
}

impl EffectOutcome {
    /// The stable operation ID this outcome refers to.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        match self {
            Self::Succeeded { operation }
            | Self::Failed { operation }
            | Self::Unknown { operation } => *operation,
        }
    }
}

/// The disposition recorded on a terminal receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub enum TerminalDisposition {
    /// The effect succeeded.
    Succeeded,
    /// The effect definitively failed.
    Failed,
    /// The outcome is unknown and the declared strategy makes the unknown
    /// permanent (requires-human or always-compensate) (R72, E56).
    UnknownPermanent,
    /// The effect was compensated through a new authorized effect; the
    /// original external state was reversed at the endpoint.
    Compensated,
}

/// Resolves an observed outcome against the declared strategy into a
/// terminal disposition.
///
/// Returns `Some` only when the outcome may legitimately terminate the
/// effect's lifecycle:
///
/// - `Succeeded` and `Failed` outcomes always resolve.
/// - An `Unknown` outcome resolves only when the declared strategy makes
///   the unknown permanent (`RequiresHuman`, `AlwaysCompensate`).
/// - For `Queryable` and `IdempotentRetrySafe` strategies an unknown
///   outcome yields `None`: the effect stays open and must be resolved by
///   status query or a bounded retry — never by declaring victory or
///   failure (R72, E56).
#[must_use]
pub const fn terminal_disposition(
    outcome: EffectOutcome,
    strategy: ReconciliationStrategy,
) -> Option<TerminalDisposition> {
    match outcome {
        EffectOutcome::Succeeded { .. } => Some(TerminalDisposition::Succeeded),
        EffectOutcome::Failed { .. } => Some(TerminalDisposition::Failed),
        EffectOutcome::Unknown { .. } => {
            if strategy.unknown_outcome_is_permanent() {
                Some(TerminalDisposition::UnknownPermanent)
            } else {
                None
            }
        }
    }
}

/// The immutable terminal record of one effect's disposition.
///
/// Once constructed the receipt has no mutation API. Later evidence
/// produces a [`SupersedingResolution`] through [`TerminalReceipt::supersede`],
/// which reads the original and returns a new record that references it —
/// the original receipt's disposition is never rewritten in place
/// (Cell 04).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub struct TerminalReceipt {
    operation: OperationId,
    run: RunId,
    tenant: TenantId,
    disposition: TerminalDisposition,
    digest: ContentDigest,
}

impl TerminalReceipt {
    /// Publishes the durable terminal receipt for one effect.
    ///
    /// The receipt digest is computed over the operation ID, run, tenant
    /// and disposition (BLAKE3, deterministic), giving the superseding
    /// mechanism a stable reference to the exact record being superseded.
    #[must_use]
    pub fn publish(
        operation: OperationId,
        run: RunId,
        tenant: TenantId,
        disposition: TerminalDisposition,
    ) -> Self {
        let digest = ContentDigest::over(&[
            operation.as_bytes(),
            run.as_bytes(),
            tenant.as_bytes(),
            &[disposition as u8],
        ]);
        Self {
            operation,
            run,
            tenant,
            disposition,
            digest,
        }
    }

    /// The stable operation ID the receipt closes out.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }

    /// The run the effect belonged to.
    #[must_use]
    pub const fn run(&self) -> RunId {
        self.run
    }

    /// The tenant the effect belonged to.
    #[must_use]
    pub const fn tenant(&self) -> TenantId {
        self.tenant
    }

    /// The recorded disposition.
    #[must_use]
    pub const fn disposition(&self) -> TerminalDisposition {
        self.disposition
    }

    /// The receipt's content digest — the reference a superseding
    /// resolution cites.
    #[must_use]
    pub const fn digest(&self) -> ContentDigest {
        self.digest
    }

    /// Appends a superseding resolution for later evidence.
    ///
    /// This is an append, not a rewrite: the original receipt is only
    /// read, and the returned record cites its digest. A success is never
    /// rewritten into a failure (or back) in place (Cell 04).
    #[must_use]
    pub fn supersede(&self, disposition: TerminalDisposition) -> SupersedingResolution {
        SupersedingResolution {
            supersedes: self.digest,
            operation: self.operation,
            disposition,
        }
    }
}

/// A resolution record appended after a terminal receipt, citing the
/// receipt it supersedes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub struct SupersedingResolution {
    supersedes: ContentDigest,
    operation: OperationId,
    disposition: TerminalDisposition,
}

impl SupersedingResolution {
    /// The digest of the receipt this resolution supersedes.
    #[must_use]
    pub const fn supersedes(&self) -> ContentDigest {
        self.supersedes
    }

    /// The stable operation ID of the superseded receipt.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }

    /// The superseding disposition.
    #[must_use]
    pub const fn disposition(&self) -> TerminalDisposition {
        self.disposition
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(byte: u8) -> OperationId {
        OperationId::from_bytes([byte; 16])
    }

    fn run() -> RunId {
        RunId::from_bytes([0x11; 16])
    }

    fn tenant() -> TenantId {
        TenantId::from_bytes([0x22; 16])
    }

    fn intent(strategy: ReconciliationStrategy) -> EffectIntent {
        EffectIntent {
            id: EffectId::from_bytes([0x33; 16]),
            operation: op(9),
            run: run(),
            tenant: tenant(),
            strategy,
            admission_epoch: SecurityEpoch::INITIAL,
        }
    }

    #[test]
    fn intents_are_reserved_before_dispatch_with_a_stable_operation_id() {
        let reserved = intent(ReconciliationStrategy::Queryable);
        assert_eq!(reserved.operation, op(9));
        assert_eq!(reserved.run, run());
        assert_eq!(reserved.tenant, tenant());
    }

    #[test]
    fn r72_every_strategy_declares_its_retry_policy_and_unknown_permanence() {
        let cases = [
            (
                ReconciliationStrategy::Queryable,
                RetryPolicy::QueryUntilResolved,
                false,
            ),
            (
                ReconciliationStrategy::IdempotentRetrySafe,
                RetryPolicy::Bounded {
                    max_attempts: MAX_EFFECT_ATTEMPTS,
                },
                false,
            ),
            (
                ReconciliationStrategy::RequiresHuman,
                RetryPolicy::None,
                true,
            ),
            (
                ReconciliationStrategy::AlwaysCompensate,
                RetryPolicy::None,
                true,
            ),
        ];
        for (strategy, retry, permanent) in cases {
            assert_eq!(strategy.retry_policy(), retry, "{strategy:?}");
            assert_eq!(
                strategy.unknown_outcome_is_permanent(),
                permanent,
                "{strategy:?}"
            );
        }
    }

    #[test]
    fn r72_unknown_under_requires_human_is_a_declared_permanent_disposition() {
        let outcome = EffectOutcome::Unknown { operation: op(1) };
        assert_eq!(
            terminal_disposition(outcome, ReconciliationStrategy::RequiresHuman),
            Some(TerminalDisposition::UnknownPermanent)
        );
        // It is a terminal disposition, not a retry authorization.
        assert_eq!(
            ReconciliationStrategy::RequiresHuman.retry_policy(),
            RetryPolicy::None
        );
    }

    #[test]
    fn r72_unknown_under_always_compensate_is_permanent_and_compensation_is_a_new_effect() {
        let outcome = EffectOutcome::Unknown { operation: op(1) };
        assert_eq!(
            terminal_disposition(outcome, ReconciliationStrategy::AlwaysCompensate),
            Some(TerminalDisposition::UnknownPermanent)
        );
        // Compensation would be recorded as a new intent with its own
        // operation ID, never as a rewrite of the original effect.
        let compensation = intent(ReconciliationStrategy::AlwaysCompensate);
        assert_ne!(compensation.operation, outcome.operation());
    }

    #[test]
    fn r72_unknown_under_queryable_is_not_terminal_yet() {
        let outcome = EffectOutcome::Unknown { operation: op(1) };
        assert_eq!(
            terminal_disposition(outcome, ReconciliationStrategy::Queryable),
            None,
            "a queryable unknown must stay open for status query"
        );
        assert_eq!(
            terminal_disposition(outcome, ReconciliationStrategy::IdempotentRetrySafe),
            None,
            "a retry-safe unknown must stay open for a bounded retry"
        );
    }

    #[test]
    fn definitive_outcomes_resolve_under_every_strategy() {
        for strategy in [
            ReconciliationStrategy::Queryable,
            ReconciliationStrategy::IdempotentRetrySafe,
            ReconciliationStrategy::RequiresHuman,
            ReconciliationStrategy::AlwaysCompensate,
        ] {
            assert_eq!(
                terminal_disposition(EffectOutcome::Succeeded { operation: op(1) }, strategy),
                Some(TerminalDisposition::Succeeded)
            );
            assert_eq!(
                terminal_disposition(EffectOutcome::Failed { operation: op(1) }, strategy),
                Some(TerminalDisposition::Failed)
            );
        }
    }

    #[test]
    fn terminal_receipts_are_immutable_and_supersede_by_append() {
        let receipt =
            TerminalReceipt::publish(op(1), run(), tenant(), TerminalDisposition::Succeeded);
        let original_digest = receipt.digest();
        let resolution = receipt.supersede(TerminalDisposition::Failed);
        // The original receipt is untouched: no in-place rewrite.
        assert_eq!(receipt.disposition(), TerminalDisposition::Succeeded);
        assert_eq!(receipt.digest(), original_digest);
        // The resolution cites exactly the record it supersedes.
        assert_eq!(resolution.supersedes(), original_digest);
        assert_eq!(resolution.operation(), op(1));
        assert_eq!(resolution.disposition(), TerminalDisposition::Failed);
        // Superseding twice chains references, not mutations.
        let second = receipt.supersede(TerminalDisposition::Compensated);
        assert_eq!(second.supersedes(), original_digest);
        assert_eq!(receipt.disposition(), TerminalDisposition::Succeeded);
    }
}
