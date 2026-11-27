//! The canonical-owner boundary (D21/D43).
//!
//! Larry owns assistant-domain intent and run orchestration; it never itself
//! performs an external effect. An effect is handed to the canonical owner of
//! that responsibility — Moo for a workspace, Sentinel for egress, Nova for
//! compute — through this trait, and the runtime records what it observes.
//!
//! The trait is deliberately synchronous. An owner that cannot say what
//! happened returns [`EffectOutcome::Unknown`] rather than an error, because
//! "we do not know" is a real, recordable state that drives reconciliation.
//! Typed binding errors are reserved for refusals before an effect is reached,
//! including a lapsed authority proof.

use std::time::{Duration, SystemTime};

use larry_contracts::binding::BindingError;
use larry_contracts::effect::{EffectIntent, EffectOutcome};
use lgwks_bot::error::KernelError;
use lgwks_bot::kernel::cap::Auth;
use lgwks_bot::kernel::cap::Cap;

/// Re-reads an owner-bound authority proof before any mutating effect.
///
/// The owner supplies the capabilities required by its concrete effect. A
/// lapsed proof and an incomplete proof are distinct typed refusals so the
/// runtime can record the exact boundary failure.
///
/// `Auth::check` today reports only expiry and coverage, but its return type
/// is the estate's whole [`KernelError`] vocabulary, so every other variant
/// is named here too: an estate refusal the owner cannot place still means the
/// endpoint was never reached, so it fails closed as [`BindingError::NoCoveringGrant`]
/// rather than through a wildcard that would hide the mapping.
pub fn validate_proof(proof: &Auth, required: &[Cap]) -> Result<(), BindingError> {
    match proof.check(required) {
        Ok(()) => Ok(()),
        Err(KernelError::CredentialExpired { .. }) => Err(BindingError::Expired),
        Err(KernelError::CapabilityDenied { .. })
        | Err(KernelError::Conflict { .. })
        | Err(KernelError::DomainError { .. })
        | Err(KernelError::EffectIndeterminate { .. })
        | Err(KernelError::ActionNotDeclared { .. })
        | Err(KernelError::EffectUnsettled { .. })
        | Err(KernelError::EffectRefused { .. }) => Err(BindingError::NoCoveringGrant),
        // Future estate variants fail closed: a refusal the owner cannot
        // place still means the endpoint was never reached. This arm is also
        // the wildcard `#[non_exhaustive]` requires outside its own crate.
        Err(_) => Err(BindingError::NoCoveringGrant),
    }
}

/// The owner boundary through which every external effect passes.
pub trait EffectOwner {
    /// Resolves stable target identities without performing the effect.
    /// `None` is reserved for disconnected, effect-free lifecycle fixtures.
    /// Real owners must return `Some` or refuse; the default is unsupported.
    ///
    /// # Errors
    /// Refuses when resolved identity cannot be established.
    fn resolve(
        &mut self,
        _intent: &EffectIntent,
    ) -> Result<
        Option<larry_contracts::binding::ResolvedEffect>,
        larry_contracts::binding::BindingError,
    > {
        Err(larry_contracts::binding::BindingError::UnsupportedOwner)
    }

    /// Atomically compares the intent binding with current owner state and
    /// executes exactly that resolved target, retaining handles or using an
    /// owner transaction. Never re-resolve a mutable name after the comparison.
    /// The default refuses: there is no fall-through to legacy dispatch.
    ///
    /// # Errors
    /// A binding error guarantees no effect was attempted. Once attempted,
    /// ambiguous results MUST return `Ok(EffectOutcome::Unknown { .. })`.
    fn dispatch_bound(
        &mut self,
        _intent: &EffectIntent,
        _proof: &Auth,
        _required: &[Cap],
        _at: SystemTime,
    ) -> Result<EffectOutcome, larry_contracts::binding::BindingError> {
        Err(larry_contracts::binding::BindingError::UnsupportedOwner)
    }

    /// Refuses an unbound reach after checking the proof's capability coverage.
    fn dispatch(
        &mut self,
        _intent: &EffectIntent,
        proof: &Auth,
        required: &[Cap],
        _at: SystemTime,
    ) -> Result<EffectOutcome, BindingError> {
        validate_proof(proof, required)?;
        Err(BindingError::UnsupportedOwner)
    }

    /// Asks the endpoint what happened to an effect whose outcome is still
    /// unknown. Implementations must reuse `intent.operation` verbatim so a
    /// supporting endpoint can deduplicate.
    fn reconcile(&mut self, intent: &EffectIntent, at: SystemTime) -> EffectOutcome;

    /// How long this owner's endpoint remembers `intent.operation` and refuses a
    /// second application of it. The submitter's `IdempotentRetrySafe` is only a
    /// claim; only the owner knows what its endpoint promises, so `None`, the
    /// default, means an effect that may have reached the owner is never sent
    /// again (lgwks_bot RQ-009, larry#197).
    fn deduplication_window(&self, _intent: &EffectIntent) -> Option<Duration> {
        None
    }
}

/// Declared, deterministic owner doubles for exercising the runtime.
///
/// **This module performs no external effect and is not an owner
/// integration.** Nothing here contacts Moo, Sentinel, Nova or any network.
/// It exists so the runtime's reconciliation, cancellation and receipt paths
/// can be driven deterministically by tests, and it is named `fixture` so no
/// reader can mistake it for a wired owner — the same rule the repository
/// applies to visibly-DEMO synthetic data.
///
/// It compiles only for this crate's own tests or under the opt-in `fixture`
/// feature, which the crate's dev-dependency on itself turns on for its
/// integration tests, so no build that ships carries a double whose
/// `resolve` answers `None` (larry#68).
#[cfg(any(test, feature = "fixture"))]
pub mod fixture {
    use std::collections::VecDeque;

    use larry_contracts::ids::OperationId;

    use super::{
        Auth, BindingError, Cap, Duration, EffectIntent, EffectOutcome, EffectOwner, SystemTime,
    };

    /// Reads the owner-bound credential before the fixture records a reach.
    fn read_proof(proof: &Auth, required: &[Cap]) -> Result<(), BindingError> {
        super::validate_proof(proof, required)
    }

    /// An owner that returns scripted outcomes in order and records the
    /// operations it was asked about.
    #[derive(Debug, Clone)]
    pub struct ScriptedOwner {
        /// Outcomes consumed in fixture order.
        scripted: VecDeque<EffectOutcome>,
        /// Mutating calls observed by this fixture.
        dispatched: Vec<OperationId>,
        /// Status queries observed by this fixture.
        reconciled: Vec<OperationId>,
    }

    impl Default for ScriptedOwner {
        fn default() -> Self {
            Self::new()
        }
    }

    impl ScriptedOwner {
        /// An owner with no scripted outcomes.
        ///
        /// With an empty script every call reports
        /// [`EffectOutcome::Unknown`], which is the fail-closed default: a
        /// fixture that was never told what happened must not invent a
        /// success.
        #[must_use]
        pub fn new() -> Self {
            Self::scripted([])
        }

        /// An owner that returns `outcomes` in order and then reports
        /// unknown forever after. Its endpoint is declared to deduplicate on
        /// the operation id for as long as any lease, so the retry rule's
        /// other conjuncts are what a test exercises (larry#197).
        #[must_use]
        pub fn scripted(outcomes: impl IntoIterator<Item = EffectOutcome>) -> Self {
            Self {
                scripted: outcomes.into_iter().collect(),
                dispatched: Vec::new(),
                reconciled: Vec::new(),
            }
        }

        /// The operations this owner was asked to dispatch, in order.
        #[must_use]
        pub fn dispatched(&self) -> &[OperationId] {
            &self.dispatched
        }

        /// The operations this owner was asked to reconcile, in order.
        #[must_use]
        pub fn reconciled(&self) -> &[OperationId] {
            &self.reconciled
        }

        /// Consumes a scripted outcome. An exhausted script is a real answer,
        /// not a missing one: the fixture was never told what happened, so it
        /// reports unknown and never invents a success.
        fn next(&mut self, operation: OperationId) -> EffectOutcome {
            match self.scripted.pop_front() {
                Some(outcome) => outcome,
                None => EffectOutcome::Unknown { operation },
            }
        }

        /// Checks the proof, then records and returns the next scripted outcome.
        fn send(
            &mut self,
            request: &EffectIntent,
            credential: &Auth,
            required: &[Cap],
        ) -> Result<EffectOutcome, BindingError> {
            read_proof(credential, required)?;
            self.dispatched.push(request.operation);
            Ok(self.next(request.operation))
        }
    }

    impl EffectOwner for ScriptedOwner {
        fn resolve(
            &mut self,
            _intent: &EffectIntent,
        ) -> Result<
            Option<larry_contracts::binding::ResolvedEffect>,
            larry_contracts::binding::BindingError,
        > {
            // This fixture has no external target and performs no effects.
            Ok(None)
        }

        fn dispatch_bound(
            &mut self,
            request: &EffectIntent,
            credential: &Auth,
            required: &[Cap],
            _observed_at: SystemTime,
        ) -> Result<EffectOutcome, BindingError> {
            self.send(request, credential, required)
        }

        fn dispatch(
            &mut self,
            intent: &EffectIntent,
            credential: &Auth,
            required: &[Cap],
            _observed_at: SystemTime,
        ) -> Result<EffectOutcome, BindingError> {
            // The fixture declares unbound support explicitly: it reads the
            // same proof for this raw intent rather than forwarding to the
            // bound path, so the two reaches stay distinct.
            self.send(intent, credential, required)
        }

        fn reconcile(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
            self.reconciled.push(intent.operation);
            self.next(intent.operation)
        }

        fn deduplication_window(&self, _intent: &EffectIntent) -> Option<Duration> {
            Some(Duration::MAX)
        }
    }

    #[cfg(test)]
    mod tests {
        use std::time::{Duration, SystemTime};

        use larry_contracts::authority::SecurityEpoch;
        use larry_contracts::binding::BindingError;
        use larry_contracts::effect::{EffectIntent, EffectOutcome, ReconciliationStrategy};
        use larry_contracts::ids::{EffectId, OperationId, RunId, TenantId};
        use lgwks_bot::kernel::cap::Cap;
        use lgwks_bot::kernel::clock::Clock;
        use lgwks_bot::kernel::gate::GrantSet;

        use super::{EffectOwner, ScriptedOwner};

        // Source: specification D21/D43 and ADR-R04: an owner that cannot say
        // what happened reports unknown, so reconciliation stays open instead
        // of a receipt being published for an effect nobody observed.
        #[test]
        fn an_exhausted_script_reports_unknown_never_success() {
            let first = OperationId::from_bytes([1; 16]);
            let second = OperationId::from_bytes([2; 16]);
            let mut owner =
                ScriptedOwner::scripted([EffectOutcome::Succeeded { operation: first }]);
            assert!(matches!(
                owner.next(first),
                EffectOutcome::Succeeded { operation } if operation == first
            ));
            for _ in 0..3 {
                assert!(matches!(
                    owner.next(second),
                    EffectOutcome::Unknown { operation } if operation == second
                ));
            }
            assert!(matches!(
                ScriptedOwner::new().next(first),
                EffectOutcome::Unknown { operation } if operation == first
            ));
        }

        #[test]
        fn a_lapsed_proof_is_refused_before_scripted_send() -> Result<(), Box<dyn std::error::Error>>
        {
            let clock = Clock::virtual_at(Duration::ZERO);
            let proof = GrantSet::empty()
                .grant_expiring(Cap::new("larry.test"), Duration::ZERO)
                .issue_at(&[Cap::new("larry.test")], &clock)?;
            let intent = EffectIntent::new(
                EffectId::from_bytes([3; 16]),
                OperationId::from_bytes([3; 16]),
                RunId::from_bytes([3; 16]),
                TenantId::from_bytes([3; 16]),
                ReconciliationStrategy::Queryable,
                SecurityEpoch::INITIAL,
            );
            let mut owner = ScriptedOwner::new();
            assert_eq!(
                owner.dispatch(&intent, &proof, &[], SystemTime::UNIX_EPOCH),
                Err(BindingError::Expired),
                "the scripted owner refuses a proof lapsed at its own read"
            );
            assert!(
                owner.dispatched.is_empty(),
                "a refused proof is never recorded as sent"
            );
            Ok(())
        }

        #[test]
        fn sim_scripted_owner_refuses_a_missing_required_capability()
        -> Result<(), Box<dyn std::error::Error>> {
            for seed in 0_u8..32 {
                let granted = Cap::new(format!("larry.granted.{seed}"));
                let missing = Cap::new(format!("larry.missing.{seed}"));
                let clock = Clock::virtual_at(Duration::ZERO);
                let proof = GrantSet::empty()
                    .grant_expiring(granted.clone(), Duration::from_secs(60))
                    .issue_at(std::slice::from_ref(&granted), &clock)?;
                let intent = EffectIntent::new(
                    EffectId::from_bytes([seed; 16]),
                    OperationId::from_bytes([seed; 16]),
                    RunId::from_bytes([seed; 16]),
                    TenantId::from_bytes([seed; 16]),
                    ReconciliationStrategy::Queryable,
                    SecurityEpoch::INITIAL,
                );
                let mut owner = ScriptedOwner::new();
                let required = [granted, missing];
                assert_eq!(
                    owner.dispatch_bound(&intent, &proof, &required, SystemTime::UNIX_EPOCH),
                    Err(BindingError::NoCoveringGrant),
                    "seed {seed}: the scripted owner must require every effect capability"
                );
                assert!(
                    owner.dispatched.is_empty(),
                    "seed {seed}: a missing capability must not be recorded as sent"
                );
            }
            Ok(())
        }
    }
}
