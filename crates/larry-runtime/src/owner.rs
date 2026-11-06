//! The canonical-owner boundary (D21/D43).
//!
//! Larry owns assistant-domain intent and run orchestration; it never itself
//! performs an external effect. An effect is handed to the canonical owner of
//! that responsibility — Moo for a workspace, Sentinel for egress, Nova for
//! compute — through this trait, and the runtime records what it observes.
//!
//! The trait is deliberately synchronous and total: it returns an
//! [`EffectOutcome`], not a `Result`. An owner that cannot say what happened
//! reports [`EffectOutcome::Unknown`] rather than an error, because "we do not
//! know" is a real, recordable state that drives reconciliation. An owner
//! that fails to *reach* the endpoint is indistinguishable from one whose
//! reply was lost, and the correct record for both is the same: unknown.

use std::time::SystemTime;

use larry_contracts::effect::{EffectIntent, EffectOutcome};

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
        _at: SystemTime,
    ) -> Result<EffectOutcome, larry_contracts::binding::BindingError> {
        Err(larry_contracts::binding::BindingError::UnsupportedOwner)
    }

    /// Performs the effect for the first time.
    fn dispatch(&mut self, intent: &EffectIntent, at: SystemTime) -> EffectOutcome;

    /// Asks the endpoint what happened to an effect whose outcome is still
    /// unknown. Implementations must reuse `intent.operation` verbatim so a
    /// supporting endpoint can deduplicate.
    fn reconcile(&mut self, intent: &EffectIntent, at: SystemTime) -> EffectOutcome;
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

    use super::{EffectIntent, EffectOutcome, EffectOwner, SystemTime};

    /// An owner that returns scripted outcomes in order and records the
    /// operations it was asked about.
    #[derive(Debug, Clone, Default)]
    pub struct ScriptedOwner {
        /// Outcomes consumed in fixture order.
        scripted: VecDeque<EffectOutcome>,
        /// Mutating calls observed by this fixture.
        dispatched: Vec<OperationId>,
        /// Status queries observed by this fixture.
        reconciled: Vec<OperationId>,
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
            Self::default()
        }

        /// An owner that returns `outcomes` in order and then reports
        /// unknown forever after.
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

        fn dispatch(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
            self.dispatched.push(intent.operation);
            self.next(intent.operation)
        }

        fn reconcile(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
            self.reconciled.push(intent.operation);
            self.next(intent.operation)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{EffectOutcome, OperationId, ScriptedOwner};

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
    }
}
