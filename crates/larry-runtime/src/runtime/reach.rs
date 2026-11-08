//! The one place the runtime reaches an owner: the first dispatch and every
//! retry funnel through `reach_owner`, so the fence and the authority proof
//! have exactly one enforcement point (larry#77).

use std::time::SystemTime;

use larry_contracts::effect::{EffectIntent, EffectOutcome};
use larry_contracts::ids::RunId;

use super::{OwnerReach, Runtime, TickReport};
use crate::error::RuntimeError;
use crate::owner::EffectOwner;
use crate::projection::{EffectPhase, ProjectionBackend};

impl<P, O> Runtime<P, O>
where
    P: ProjectionBackend + Default,
    O: EffectOwner,
{
    /// Checks durability and freshness before the first mutating reach.
    pub(super) fn dispatch(
        &mut self,
        run: RunId,
        intent: EffectIntent,
        at: SystemTime,
        report: &mut TickReport,
    ) -> Result<(), RuntimeError> {
        if self.projection.effect_phase(intent.operation) != Some(EffectPhase::Reserved) {
            return Err(RuntimeError::IntentNotDurable {
                operation: intent.operation,
            });
        }
        let Some(outcome) = self.reach_owner(intent, OwnerReach::Mutate, at, report)? else {
            // Fenced or unproven. The ordering is the point: the reservation
            // above is already durable and the owner below was never called, so
            // the effect reads as reserved — reserved, and genuinely never sent.
            // No outcome is invented and no run state moves, because the
            // effect did not fail; it did not happen.
            return Ok(());
        };
        report.outcomes.push((intent.operation, outcome));
        let sent = self
            .live
            .get(&intent.operation)
            .is_some_and(|live| live.sent);
        self.settle(run, intent, outcome, sent, report)
    }

    /// The single point at which the runtime reaches the owner.
    ///
    /// Both lifecycle paths funnel through here — a first dispatch and a
    /// retry — so the fence and the authority proof have exactly one
    /// enforcement point and exactly one place to audit. `Mutate` is the only
    /// reach that can change anything outside this process, and it is the only
    /// one the fence and the proof gate. `Observe` must stay open: a status
    /// query is how an effect that was already dispatched when revocation
    /// landed gets resolved, and fencing it would leave an external effect
    /// permanently unlabelled — the "already-dispatched treatment" D48
    /// requires, enforced by shape rather than by convention.
    ///
    /// Returns `None` when the fence or the proof refused, in which case no
    /// owner call happened and no outcome exists to record.
    pub(super) fn reach_owner(
        &mut self,
        intent: EffectIntent,
        reach: OwnerReach,
        at: SystemTime,
        report: &mut TickReport,
    ) -> Result<Option<EffectOutcome>, RuntimeError> {
        Ok(match reach {
            OwnerReach::Mutate => {
                if let Err(refusal) = self.fence.check(&intent) {
                    report.fenced.push((intent.operation, refusal));
                    return Ok(None);
                }
                // The proof the gate minted at admission must still hold at the
                // call: a grant that lapsed since is not spent (larry#77).
                if let Some(live) = self.live.get(&intent.operation)
                    && let Err(reason) = live.proof.check()
                {
                    report.denied.push((intent.operation, reason));
                    return Ok(None);
                }
                self.validate_bound_reach(&intent, at)?;
                let previous = self.live.get(&intent.operation).cloned();
                if let Some(live) = self.live.get_mut(&intent.operation) {
                    live.dispatches = live.dispatches.saturating_add(1);
                    // The reach below is about to happen, so this effect may
                    // now have reached its owner. This is the only place the
                    // flag is set for a live effect, and it is set before the
                    // call rather than after: a call that panicked or was lost
                    // still may have happened, and the answer to a later status
                    // query about it is therefore evidence.
                    live.sent = true;
                }
                // The reach happens without the authority source having been
                // re-contacted, so it consumes offline authority. Spending it
                // here — rather than at the call site — is what keeps the
                // accounting tied to an owner call that actually occurred.
                let outcome = if intent.binding().is_some() {
                    match self.owner.dispatch_bound(&intent, at) {
                        Ok(outcome) => outcome,
                        Err(error) => {
                            // Typed owner refusal guarantees no attempted effect.
                            // Restore the witness, including a previous successful reach.
                            if let Some(previous) = previous {
                                self.live.insert(intent.operation, previous);
                            }
                            return Err(error.into());
                        }
                    }
                } else {
                    self.owner.dispatch(&intent, at)
                };
                self.fence.spend_offline();
                Some(outcome)
            }
            OwnerReach::Observe => {
                if let Some(live) = self.live.get_mut(&intent.operation) {
                    live.reconciliations = live.reconciliations.saturating_add(1);
                }
                Some(self.owner.reconcile(&intent, at))
            }
        })
    }
}
