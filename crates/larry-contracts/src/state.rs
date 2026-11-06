//! The finite run-state algebra (spec Cell 04, work package C02).
//!
//! The happy path is `Draft -> Admitted -> Running -> Reconciling ->
//! Verifying -> Finalizing -> Completed`; the branch states
//! `AwaitingAuthority`, `AwaitingDependency`, `Conflict`, `Cancelling`,
//! `Failed`, `RolledBack`, `Cancelled` and `OutcomeUnknown` are part of the
//! same finite model. Exactly one total, pure function — [`transition`] —
//! maps every `(RunState, TransitionEvent)` pair to the next state or a
//! typed [`TransitionError`]. No other state change is expressible.
//!
//! Encoded invariants (Cell 04):
//!
//! - **Durable terminal receipt is immutable.** `Completed`, `Cancelled`,
//!   `Failed`, `RolledBack` and `OutcomeUnknown` refuse every event with
//!   [`TransitionError::Terminal`]. Later evidence appends a superseding
//!   resolution as a new record (see `effect::TerminalReceipt::supersede`);
//!   it never rewrites a terminal state in place.
//! - **Cancellation requested is not cancellation completed.**
//!   `RequestCancellation` lands in `Cancelling` from every live state;
//!   `Cancelled` is reachable only from `Cancelling` through
//!   `FinishCancellation`. No path leads from `Cancelling` to `Completed`.
//! - **Partial external effects are never labelled `RolledBack`.**
//!   [`TransitionEvent::MarkRolledBack`] carries [`RollbackEvidence`]; the
//!   `PartialExternalEffects` variant is refused in every state with
//!   [`TransitionError::PartialRollbackRefused`], and `FullyCompensated` is
//!   accepted only from `Cancelling`. The total function therefore cannot
//!   produce `RolledBack` from partial evidence: unowned external state
//!   stays visible instead of being rewritten into a rollback claim.

use crate::error::TransitionError;

#[cfg(feature = "serde")]
use lgwks_std::json::serde::{Deserialize, Serialize};

/// Every state a run can occupy (Cell 04 candidate algebra, made finite).
///
/// The set is closed by the specification: adding a state is a
/// specification change, not a local edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub enum RunState {
    /// Assembled but not yet admitted.
    Draft,
    /// Admission is durable and acknowledged — durable, not successful
    /// (architecture README request lifecycle).
    Admitted,
    /// Suspended until the required authority grant arrives.
    AwaitingAuthority,
    /// Suspended until an external dependency becomes ready.
    AwaitingDependency,
    /// Executing admitted work.
    Running,
    /// Dispatching, observing and reconciling effects.
    Reconciling,
    /// Verifying domain obligations against reconciled effects.
    Verifying,
    /// Closing the lifecycle and publishing the terminal receipt.
    Finalizing,
    /// Terminal: the durable terminal receipt is published and immutable.
    Completed,
    /// Two legitimate directives are incompatible; resolution follows the
    /// approved conflict policy while unrelated work proceeds (E18).
    Conflict,
    /// Cancellation has been requested and is being carried out. This is
    /// not `Cancelled`: owned effects must still be stopped and reconciled.
    Cancelling,
    /// Terminal: a definitive failure was observed.
    Failed,
    /// Terminal: every dispatched external effect is compensated or
    /// provably never started. Never reachable from partial evidence.
    RolledBack,
    /// Terminal: cancellation completed; owned resources are released.
    Cancelled,
    /// Terminal: an external effect ended in an ambiguous state and its
    /// declared reconciliation strategy makes the unknown permanent
    /// (R72, E56). Resolution, when it ever comes, is a superseding
    /// resolution record, not a state rewrite.
    OutcomeUnknown,
}

impl RunState {
    /// All fifteen states of the closed algebra, in declaration order.
    pub const ALL: [RunState; 15] = [
        RunState::Draft,
        RunState::Admitted,
        RunState::AwaitingAuthority,
        RunState::AwaitingDependency,
        RunState::Running,
        RunState::Reconciling,
        RunState::Verifying,
        RunState::Finalizing,
        RunState::Completed,
        RunState::Conflict,
        RunState::Cancelling,
        RunState::Failed,
        RunState::RolledBack,
        RunState::Cancelled,
        RunState::OutcomeUnknown,
    ];

    /// True for the five terminal dispositions, which refuse every event.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            RunState::Completed
                | RunState::Failed
                | RunState::RolledBack
                | RunState::Cancelled
                | RunState::OutcomeUnknown
        )
    }
}

impl core::fmt::Display for RunState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let name = match *self {
            RunState::Draft => "Draft",
            RunState::Admitted => "Admitted",
            RunState::AwaitingAuthority => "AwaitingAuthority",
            RunState::AwaitingDependency => "AwaitingDependency",
            RunState::Running => "Running",
            RunState::Reconciling => "Reconciling",
            RunState::Verifying => "Verifying",
            RunState::Finalizing => "Finalizing",
            RunState::Completed => "Completed",
            RunState::Conflict => "Conflict",
            RunState::Cancelling => "Cancelling",
            RunState::Failed => "Failed",
            RunState::RolledBack => "RolledBack",
            RunState::Cancelled => "Cancelled",
            RunState::OutcomeUnknown => "OutcomeUnknown",
        };
        f.write_str(name)
    }
}

/// Evidence accompanying a rollback claim.
///
/// The transition function uses this evidence to refuse labelling partial
/// external effects as rolled back (Cell 04).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub enum RollbackEvidence {
    /// Every dispatched external effect is compensated or provably never
    /// started. Only this evidence may produce `RolledBack`, and only from
    /// `Cancelling`.
    FullyCompensated,
    /// Some external effects remain uncompensated. Refused in every state.
    PartialExternalEffects,
}

/// An event offered to the run-state algebra.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub enum TransitionEvent {
    /// Admit a draft run; admission is durable, not successful.
    Admit,
    /// Suspend pending the required authority grant.
    AwaitAuthority,
    /// The awaited authority grant arrived.
    AuthorityGranted,
    /// Suspend pending an external dependency.
    AwaitDependency,
    /// The awaited dependency became ready.
    DependencyReady,
    /// Begin executing admitted work.
    Start,
    /// Enter effect reconciliation (dispatch, observe, reconcile).
    EnterReconciling,
    /// Verification of reconciled effects passed; proceed to finalization.
    VerificationPassed,
    /// Verification failed; return to reconciliation.
    VerificationFailed,
    /// Begin lifecycle finalization.
    EnterFinalizing,
    /// Publish the durable terminal receipt; the run completes.
    Complete,
    /// Two legitimate but incompatible directives were detected.
    DetectConflict,
    /// The approved conflict policy resolved the conflict.
    ResolveConflict,
    /// Cancellation was requested. Requested, not completed.
    RequestCancellation,
    /// Every owned effect is stopped, reconciled or compensated:
    /// cancellation is complete.
    FinishCancellation,
    /// A definitive failure was observed.
    Fail,
    /// A rollback completed; the evidence decides whether the claim is
    /// admissible at all.
    MarkRolledBack(RollbackEvidence),
    /// An external effect ended in an ambiguous state; the declared
    /// reconciliation strategy determines whether this is terminal (R72).
    ObserveUnknownOutcome,
}

/// The total, pure transition function of the run-state algebra.
///
/// Every `(RunState, TransitionEvent)` pair yields exactly one outcome:
/// the next state, or a typed [`TransitionError`]. The function is pure —
/// no clock, no entropy, no I/O — so the same pair always yields the same
/// result, which is what replay verification (P5, E55) requires of it.
///
/// # Errors
///
/// Returns [`TransitionError::Terminal`] for any event offered to a
/// terminal state, [`TransitionError::PartialRollbackRefused`] for a
/// rollback claim carrying partial external effects, and
/// [`TransitionError::NotAllowed`] for an event the state does not admit.
pub fn transition(state: RunState, event: TransitionEvent) -> Result<RunState, TransitionError> {
    match (state, event) {
        (RunState::Draft, TransitionEvent::Admit) => Ok(RunState::Admitted),
        (RunState::Draft, TransitionEvent::AwaitAuthority) => Ok(RunState::AwaitingAuthority),
        (RunState::Draft, TransitionEvent::AwaitDependency) => Ok(RunState::AwaitingDependency),
        (RunState::Draft, TransitionEvent::DetectConflict) => Ok(RunState::Conflict),
        (RunState::Draft, TransitionEvent::RequestCancellation) => Ok(RunState::Cancelling),

        (RunState::Admitted, TransitionEvent::Start) => Ok(RunState::Running),
        (RunState::Admitted, TransitionEvent::AwaitAuthority) => Ok(RunState::AwaitingAuthority),
        (RunState::Admitted, TransitionEvent::AwaitDependency) => Ok(RunState::AwaitingDependency),
        (RunState::Admitted, TransitionEvent::DetectConflict) => Ok(RunState::Conflict),
        (RunState::Admitted, TransitionEvent::RequestCancellation) => Ok(RunState::Cancelling),
        (RunState::Admitted, TransitionEvent::Fail) => Ok(RunState::Failed),

        (RunState::AwaitingAuthority, TransitionEvent::AuthorityGranted) => Ok(RunState::Admitted),
        (RunState::AwaitingAuthority, TransitionEvent::RequestCancellation) => {
            Ok(RunState::Cancelling)
        }
        (RunState::AwaitingAuthority, TransitionEvent::Fail) => Ok(RunState::Failed),

        (RunState::AwaitingDependency, TransitionEvent::DependencyReady) => Ok(RunState::Admitted),
        (RunState::AwaitingDependency, TransitionEvent::RequestCancellation) => {
            Ok(RunState::Cancelling)
        }
        (RunState::AwaitingDependency, TransitionEvent::Fail) => Ok(RunState::Failed),

        (RunState::Running, TransitionEvent::EnterReconciling) => Ok(RunState::Reconciling),
        (RunState::Running, TransitionEvent::AwaitAuthority) => Ok(RunState::AwaitingAuthority),
        (RunState::Running, TransitionEvent::AwaitDependency) => Ok(RunState::AwaitingDependency),
        (RunState::Running, TransitionEvent::DetectConflict) => Ok(RunState::Conflict),
        (RunState::Running, TransitionEvent::RequestCancellation) => Ok(RunState::Cancelling),
        (RunState::Running, TransitionEvent::Fail) => Ok(RunState::Failed),
        (RunState::Running, TransitionEvent::ObserveUnknownOutcome) => Ok(RunState::OutcomeUnknown),

        (RunState::Reconciling, TransitionEvent::VerificationPassed) => Ok(RunState::Verifying),
        (RunState::Reconciling, TransitionEvent::RequestCancellation) => Ok(RunState::Cancelling),
        (RunState::Reconciling, TransitionEvent::Fail) => Ok(RunState::Failed),
        (RunState::Reconciling, TransitionEvent::ObserveUnknownOutcome) => {
            Ok(RunState::OutcomeUnknown)
        }

        (RunState::Verifying, TransitionEvent::EnterFinalizing) => Ok(RunState::Finalizing),
        (RunState::Verifying, TransitionEvent::VerificationFailed) => Ok(RunState::Reconciling),
        (RunState::Verifying, TransitionEvent::RequestCancellation) => Ok(RunState::Cancelling),
        (RunState::Verifying, TransitionEvent::Fail) => Ok(RunState::Failed),

        (RunState::Finalizing, TransitionEvent::Complete) => Ok(RunState::Completed),
        (RunState::Finalizing, TransitionEvent::RequestCancellation) => Ok(RunState::Cancelling),
        (RunState::Finalizing, TransitionEvent::Fail) => Ok(RunState::Failed),

        (RunState::Conflict, TransitionEvent::ResolveConflict) => Ok(RunState::Reconciling),
        (RunState::Conflict, TransitionEvent::RequestCancellation) => Ok(RunState::Cancelling),
        (RunState::Conflict, TransitionEvent::Fail) => Ok(RunState::Failed),

        (RunState::Cancelling, TransitionEvent::FinishCancellation) => Ok(RunState::Cancelled),
        (
            RunState::Cancelling,
            TransitionEvent::MarkRolledBack(RollbackEvidence::FullyCompensated),
        ) => Ok(RunState::RolledBack),
        (RunState::Cancelling, TransitionEvent::ObserveUnknownOutcome) => {
            Ok(RunState::OutcomeUnknown)
        }

        // Partial external effects are never labelled rolled back: this
        // refusal takes precedence in every state, terminal or live.
        (_, TransitionEvent::MarkRolledBack(RollbackEvidence::PartialExternalEffects)) => {
            Err(TransitionError::PartialRollbackRefused { state })
        }
        // Terminal dispositions are immutable: no transition may leave them.
        (
            RunState::Completed
            | RunState::Failed
            | RunState::RolledBack
            | RunState::Cancelled
            | RunState::OutcomeUnknown,
            _,
        ) => Err(TransitionError::Terminal { state }),
        (state, event) => Err(TransitionError::NotAllowed { state, event }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TransitionError as Err;
    use TransitionEvent as Event;

    fn all_events() -> Vec<TransitionEvent> {
        vec![
            Event::Admit,
            Event::AwaitAuthority,
            Event::AuthorityGranted,
            Event::AwaitDependency,
            Event::DependencyReady,
            Event::Start,
            Event::EnterReconciling,
            Event::VerificationPassed,
            Event::VerificationFailed,
            Event::EnterFinalizing,
            Event::Complete,
            Event::DetectConflict,
            Event::ResolveConflict,
            Event::RequestCancellation,
            Event::FinishCancellation,
            Event::Fail,
            Event::MarkRolledBack(RollbackEvidence::FullyCompensated),
            Event::MarkRolledBack(RollbackEvidence::PartialExternalEffects),
            Event::ObserveUnknownOutcome,
        ]
    }

    #[test]
    fn happy_path_walks_every_stage_to_completed() {
        let steps = [
            (RunState::Draft, Event::Admit, RunState::Admitted),
            (RunState::Admitted, Event::Start, RunState::Running),
            (
                RunState::Running,
                Event::EnterReconciling,
                RunState::Reconciling,
            ),
            (
                RunState::Reconciling,
                Event::VerificationPassed,
                RunState::Verifying,
            ),
            (
                RunState::Verifying,
                Event::EnterFinalizing,
                RunState::Finalizing,
            ),
            (RunState::Finalizing, Event::Complete, RunState::Completed),
        ];
        for (from, event, to) in steps {
            assert_eq!(transition(from, event), Ok(to), "{from} + {event:?}");
        }
    }

    #[test]
    fn the_algebra_is_total_over_every_state_event_pair() {
        for state in RunState::ALL {
            for event in all_events() {
                // Totality: every pair maps to Ok or a typed error, never
                // to a panic or an undefined result.
                let _outcome = transition(state, event);
            }
        }
    }

    #[test]
    fn terminal_receipts_are_immutable_no_transition_leaves_completed() {
        let partial = Event::MarkRolledBack(RollbackEvidence::PartialExternalEffects);
        for event in all_events() {
            if event == partial {
                continue; // dedicated test: refused with PartialRollbackRefused everywhere
            }
            let outcome = transition(RunState::Completed, event);
            assert_eq!(
                outcome,
                Err(Err::Terminal {
                    state: RunState::Completed
                }),
                "Completed must refuse {event:?}"
            );
        }
    }

    #[test]
    fn every_terminal_state_refuses_every_event() {
        let terminal = [
            RunState::Completed,
            RunState::Failed,
            RunState::RolledBack,
            RunState::Cancelled,
            RunState::OutcomeUnknown,
        ];
        let partial = Event::MarkRolledBack(RollbackEvidence::PartialExternalEffects);
        for state in terminal {
            assert!(state.is_terminal());
            for event in all_events() {
                if event == partial {
                    continue; // dedicated test: refused with PartialRollbackRefused everywhere
                }
                let outcome = transition(state, event);
                assert_eq!(outcome, Err(Err::Terminal { state }), "{state} + {event:?}");
            }
        }
    }

    #[test]
    fn cancellation_requested_is_not_cancellation_completed() {
        assert_eq!(
            transition(RunState::Running, Event::RequestCancellation),
            Ok(RunState::Cancelling)
        );
        // Cancelling is not Cancelled, and cannot complete as if nothing
        // happened.
        assert_eq!(
            transition(RunState::Cancelling, Event::Complete),
            Err(Err::NotAllowed {
                state: RunState::Cancelling,
                event: Event::Complete
            })
        );
        // Only the finished-cancellation event lands Cancelled.
        assert_eq!(
            transition(RunState::Cancelling, Event::FinishCancellation),
            Ok(RunState::Cancelled)
        );
        // A live run cannot skip Cancelling and land Cancelled directly.
        assert!(matches!(
            transition(RunState::Running, Event::FinishCancellation),
            Err(Err::NotAllowed { .. })
        ));
        // Cancellation is requested from every live state.
        for state in RunState::ALL {
            if state.is_terminal() || state == RunState::Cancelling {
                continue;
            }
            let outcome = transition(state, Event::RequestCancellation);
            assert_eq!(
                outcome,
                Ok(RunState::Cancelling),
                "{state} must accept cancellation"
            );
        }
    }

    #[test]
    fn partial_external_effects_are_never_labelled_rolled_back() {
        for state in RunState::ALL {
            let outcome = transition(
                state,
                Event::MarkRolledBack(RollbackEvidence::PartialExternalEffects),
            );
            assert_eq!(
                outcome,
                Err(Err::PartialRollbackRefused { state }),
                "{state} must refuse a partial rollback label"
            );
        }
        // Fully-compensated evidence is admitted only from Cancelling.
        assert_eq!(
            transition(
                RunState::Cancelling,
                Event::MarkRolledBack(RollbackEvidence::FullyCompensated)
            ),
            Ok(RunState::RolledBack)
        );
        assert!(matches!(
            transition(
                RunState::Running,
                Event::MarkRolledBack(RollbackEvidence::FullyCompensated)
            ),
            Err(Err::NotAllowed { .. })
        ));
    }

    #[test]
    fn awaiting_states_return_to_admitted_when_their_gate_clears() {
        assert_eq!(
            transition(RunState::AwaitingAuthority, Event::AuthorityGranted),
            Ok(RunState::Admitted)
        );
        assert_eq!(
            transition(RunState::AwaitingDependency, Event::DependencyReady),
            Ok(RunState::Admitted)
        );
    }

    #[test]
    fn conflict_resumes_through_reconciliation() {
        assert_eq!(
            transition(RunState::Running, Event::DetectConflict),
            Ok(RunState::Conflict)
        );
        assert_eq!(
            transition(RunState::Conflict, Event::ResolveConflict),
            Ok(RunState::Reconciling)
        );
    }

    #[test]
    fn unknown_outcome_is_terminal_and_requires_a_superseding_resolution() {
        assert_eq!(
            transition(RunState::Reconciling, Event::ObserveUnknownOutcome),
            Ok(RunState::OutcomeUnknown)
        );
        assert!(RunState::OutcomeUnknown.is_terminal());
        assert_eq!(
            transition(RunState::OutcomeUnknown, Event::EnterReconciling),
            Err(Err::Terminal {
                state: RunState::OutcomeUnknown
            })
        );
    }

    #[test]
    fn verification_failure_returns_to_reconciliation() {
        assert_eq!(
            transition(RunState::Verifying, Event::VerificationFailed),
            Ok(RunState::Reconciling)
        );
    }
}
