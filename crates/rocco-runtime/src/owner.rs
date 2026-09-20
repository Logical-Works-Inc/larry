//! The canonical-owner boundary (D21/D43).
//!
//! Rocco owns assistant-domain intent and run orchestration; it never itself
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

use std::collections::VecDeque;
use std::time::SystemTime;

use rocco_contracts::effect::{EffectIntent, EffectOutcome};
use rocco_contracts::ids::OperationId;

/// The owner boundary through which every external effect passes.
pub trait EffectOwner {
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
pub mod fixture {
    use super::{EffectIntent, EffectOutcome, EffectOwner, OperationId, SystemTime, VecDeque};

    /// An owner that returns scripted outcomes in order and records the
    /// operations it was asked about.
    #[derive(Debug, Clone, Default)]
    pub struct ScriptedOwner {
        scripted: VecDeque<EffectOutcome>,
        dispatched: Vec<OperationId>,
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

        fn next(&mut self, operation: OperationId) -> EffectOutcome {
            self.scripted
                .pop_front()
                .unwrap_or(EffectOutcome::Unknown { operation })
        }
    }

    impl EffectOwner for ScriptedOwner {
        fn dispatch(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
            self.dispatched.push(intent.operation);
            self.next(intent.operation)
        }

        fn reconcile(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
            self.reconciled.push(intent.operation);
            self.next(intent.operation)
        }
    }
}
