//! Stages 3 and 4 of the tick: reconcile open effects, then complete each
//! cancellation whose owned effects have all reconciled.

use std::time::SystemTime;

use larry_contracts::effect::EffectOutcome;
use larry_contracts::ids::{OperationId, RunId, TenantId};
use larry_contracts::state::{RunState, TransitionEvent};

use super::{LiveEffect, OwnerReach, Runtime, TickReport, retry};
use crate::error::RuntimeError;
use crate::owner::EffectOwner;
use crate::projection::{EffectPhase, ProjectionBackend};
use lgwks_bot::retry::RetryRefusal;

impl<P, O> Runtime<P, O>
where
    P: ProjectionBackend + Default,
    O: EffectOwner,
{
    // ── stage 3: reconcile ──────────────────────────────────────────────────

    /// Reconciles open effects in stable operation order.
    pub(super) fn pump_reconciliation(
        &mut self,
        at: SystemTime,
        report: &mut TickReport,
    ) -> Result<(), RuntimeError> {
        let open: Vec<OperationId> = self
            .live
            .iter()
            .filter(|&(operation, live)| self.is_reconcilable(*operation, live))
            .map(|(operation, _)| *operation)
            .collect();

        for operation in open {
            let Some(live) = self.live.get(&operation).cloned() else {
                continue;
            };
            let Some(outcome) = self.reconcile_one(&live, operation, at, report)? else {
                // A fenced retry: no owner call and no recorded outcome, so
                // the effect is not counted as reconciled. It stays visibly
                // open — not resolved, not failed — exactly as an effect that
                // ran out of interaction budget does.
                continue;
            };
            report.reconciled.push(operation);
            let run = live.intent.run;
            // `live` was copied before the reach, so the witness is re-read
            // from the table the reach updated: a retry that actually happened
            // has set it, and a reach the fence refused has not.
            let sent = self.live.get(&operation).is_some_and(|live| live.sent);
            self.settle(run, live.intent, outcome, sent, report)?;
        }
        Ok(())
    }

    /// The one owner interaction an open effect is entitled to.
    ///
    /// The declared strategy decides it, and the choice is recorded in the
    /// journal like any other outcome:
    ///
    /// - `IdempotentRetrySafe` with budget left re-dispatches, reusing the
    ///   stable operation ID so a supporting endpoint can deduplicate;
    /// - `Queryable` asks what happened;
    /// - a strategy that authorizes neither — `RequiresHuman`,
    ///   `AlwaysCompensate` — cannot be queried and cannot be retried, so the
    ///   only truthful outcome is `Unknown`. The journal proves the effect was
    ///   reserved and cannot prove it never ran, so this resolves to a
    ///   permanent unknown rather than an invented success or failure (R72, E56).
    ///
    /// A retry is a mutating reach and the fence gates it; a status query is
    /// an observation and the fence leaves it open. A retry is decided by the
    /// estate rule (`retry::decide`, lgwks_bot RQ-009). A refusal for authority
    /// alone is left to the fence, which reports it. Any other refusal spends
    /// the dispatch budget and records one `Unknown`, so the effect leaves
    /// reconciliation rather than being re-asked every tick. Returns `None`
    /// only when the fence refused the retry.
    fn reconcile_one(
        &mut self,
        live: &LiveEffect,
        operation: OperationId,
        at: SystemTime,
        report: &mut TickReport,
    ) -> Result<Option<EffectOutcome>, RuntimeError> {
        Ok(match live.intent.strategy.reconciliation_action() {
            larry_contracts::effect::ReconciliationAction::RetryBounded { max_attempts } => {
                let authority_live =
                    self.fence.check(&live.intent).is_ok() && live.proof.check().is_ok();
                let decision = retry::decide(live, max_attempts, at, authority_live)?;
                let authority_only = decision.refusals() == [RetryRefusal::AuthorityNotLive];
                if decision.permits_retry() || authority_only {
                    self.reach_owner(live.intent, OwnerReach::Mutate, at, report)?
                } else {
                    // The estate rule refused the retry. The effect stays
                    // visibly unresolved, and the budget is spent so it leaves
                    // reconciliation rather than being re-asked every tick.
                    if let Some(open) = self.live.get_mut(&operation) {
                        open.dispatches = max_attempts;
                    }
                    Some(EffectOutcome::Unknown { operation })
                }
            }
            larry_contracts::effect::ReconciliationAction::Observe => {
                self.reach_owner(live.intent, OwnerReach::Observe, at, report)?
            }
            larry_contracts::effect::ReconciliationAction::LeaveUnknown => {
                Some(EffectOutcome::Unknown { operation })
            }
            other => return Err(RuntimeError::UnsupportedReconciliationAction(other)),
        })
    }

    /// Whether an effect is still open *and* still within its interaction
    /// budget.
    ///
    /// An effect is open when the journal leaves it `Reserved` — a crash, or a
    /// dispatch whose outcome was never recorded — or when it has an observed
    /// outcome that its declared strategy refuses to resolve (R72, E56).
    ///
    /// The budget is the bound that keeps reconciliation from becoming an
    /// unbounded loop (Cell 05). Once it is spent the effect is left visibly
    /// unresolved: no receipt, no invented success, no silent failure. The
    /// escalation policy above that is a later work package's, and this
    /// function deliberately does not paper over its absence.
    fn is_reconcilable(&self, operation: OperationId, live: &LiveEffect) -> bool {
        if live.dispatched_at_tick >= self.tick_count {
            return false;
        }
        if live.reconciliations >= larry_contracts::effect::MAX_EFFECT_ATTEMPTS {
            return false;
        }
        let open = match self.projection.effect_phase(operation) {
            Some(EffectPhase::Reserved) => true,
            Some(EffectPhase::Observed { outcome }) => {
                // An observation resolves an effect only when this runtime
                // actually sent it. When it did not — every mutating reach was
                // refused — the outcome resolves nothing, so the effect stays
                // open and keeps being reported rather than settling into a
                // disposition no reach earned. The reconciliation budget below
                // still bounds how long that can go on.
                crate::projection::resolve(&live.intent, outcome).is_err() || !live.sent
            }
            Some(EffectPhase::Reconciled { .. }) | None => false,
        };
        if !open {
            return false;
        }
        // A retry-based strategy stops interacting once its dispatch budget is
        // spent, rather than spinning on an effect it may no longer touch.
        //
        // The fence is deliberately *not* consulted here. Excluding a fenced
        // effect from reconciliation would make the refusal silent: the effect
        // would sit open forever with nothing in the tick report saying why.
        // Leaving it in costs one pure check per open effect per tick, calls
        // no owner and writes nothing, and keeps the refusal visible on every
        // tick until the host either re-authenticates or gives up on the
        // effect deliberately.
        match live.intent.strategy.reconciliation_action() {
            larry_contracts::effect::ReconciliationAction::RetryBounded { max_attempts } => {
                live.dispatches < max_attempts
            }
            larry_contracts::effect::ReconciliationAction::Observe
            | larry_contracts::effect::ReconciliationAction::LeaveUnknown => true,
            // Route unsupported policies to reconcile_one's typed refusal.
            _ => true,
        }
    }

    // ── stage 4: cancellation ───────────────────────────────────────────────

    /// Records cancellation for a nonterminal run.
    pub(super) fn request_cancellation(
        &mut self,
        tenant: TenantId,
        run: RunId,
    ) -> Result<(), RuntimeError> {
        self.track_run(run, tenant)?;
        if self
            .projection
            .run_state(run)
            .is_some_and(RunState::is_terminal)
        {
            return Ok(());
        }
        self.advance(run, TransitionEvent::RequestCancellation)?;
        Ok(())
    }

    /// Completes cancellation only once every owned effect has reconciled.
    ///
    /// An effect still `Reserved` may be in flight, and one `Observed` but not
    /// `Reconciled` has an outcome its strategy left unresolved and no
    /// receipt. Finishing over either would mark the run cancelled while an
    /// external effect that may have happened stays unlabelled (larry#119),
    /// so the run stays `Cancelling` until reconciliation resolves the effect.
    pub(super) fn pump_cancellation(&mut self) -> Result<(), RuntimeError> {
        let cancelling: Vec<RunId> = self
            .runs
            .keys()
            .copied()
            .filter(|run| self.projection.run_state(*run) == Some(RunState::Cancelling))
            .collect();

        for run in cancelling {
            let still_open = self.live.values().any(|live| {
                live.intent.run == run
                    && matches!(
                        self.projection.effect_phase(live.intent.operation),
                        Some(EffectPhase::Reserved | EffectPhase::Observed { .. })
                    )
            });
            if still_open {
                continue;
            }
            self.advance(run, TransitionEvent::FinishCancellation)?;
        }
        Ok(())
    }
}
