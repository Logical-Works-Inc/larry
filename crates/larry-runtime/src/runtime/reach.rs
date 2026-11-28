//! The one place the runtime reaches an owner: the first dispatch and every
//! retry funnel through `reach_owner`, so the fence and the authority proof
//! have exactly one enforcement point (larry#77).

use std::time::SystemTime;

use larry_contracts::effect::{EffectIntent, EffectOutcome};
use larry_contracts::ids::RunId;
use larry_contracts::replay::DenialReason;
use lgwks_bot::kernel::broker::Authority;
use lgwks_bot::kernel::cap::{Auth, Cap};
use lgwks_bot::kernel::effect::EffectKey;

use super::{OwnerReach, Runtime, TickItem, TickReport};
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
                let attempts = match self.live.get(&intent.operation) {
                    Some(live) => live.dispatches.saturating_add(1),
                    None => 1,
                };
                let key = crate::fence::effect_key(&intent, attempts)?;
                let warrant = match self.fence.authorize(key, &intent) {
                    Ok(warrant) => warrant,
                    Err(refusal) => {
                        report.fenced.push((intent.operation, refusal));
                        return Ok(None);
                    }
                };
                // The proof the gate minted at admission must still hold at the
                // call: a grant that lapsed since is not spent (larry#77). A reach
                // with no live witness holds no proof at all, so it is refused
                // rather than let through unchecked (larry#197).
                let auth = match self.live.get(&intent.operation) {
                    Some(live) => match live.proof.checked_auth() {
                        Ok((auth, required)) => (auth, required),
                        Err(reason) => {
                            report.denied.push((intent.operation, reason));
                            return Ok(None);
                        }
                    },
                    None => {
                        report
                            .denied
                            .push((intent.operation, DenialReason::AuthorityExpired));
                        return Ok(None);
                    }
                };
                self.validate_bound_reach(&intent, at)?;
                // Group commit (larry#75): every fact recorded so far, the
                // intent among them, is on disk before the owner can act on
                // it. A file that refuses means the owner is never called.
                self.journal.sync()?;
                self.hand_off(intent, key, &warrant, auth, at, report)?
            }
            OwnerReach::Observe => {
                self.journal.sync()?;
                if let Some(live) = self.live.get_mut(&intent.operation) {
                    live.reconciliations = live.reconciliations.saturating_add(1);
                }
                Some(self.owner.reconcile(&intent, at))
            }
        })
    }

    /// Hands a warranted effect to its owner: the second half of a mutating
    /// reach, after the mint, the proof and the group commit.
    ///
    /// The handoff (larry#76): the broker re-checks the warrant right before
    /// the owner, so a withdrawal since the mint holds. `None` means it
    /// refused, the owner was never called and nothing was counted.
    pub(super) fn hand_off(
        &mut self,
        intent: EffectIntent,
        key: EffectKey,
        warrant: &Authority,
        (auth, required): (Auth, Vec<Cap>),
        at: SystemTime,
        report: &mut TickReport,
    ) -> Result<Option<EffectOutcome>, RuntimeError> {
        if let Err(refusal) = self.fence.revalidate(warrant, &intent) {
            report.fenced.push((intent.operation, refusal));
            return Ok(None);
        }
        let previous = self.live.get(&intent.operation).cloned();
        if let Some(live) = self.live.get_mut(&intent.operation) {
            live.dispatches = live.dispatches.saturating_add(1);
            live.dispatched = Some(key);
            // The reach below is about to happen, so this effect may now have
            // reached its owner. This is the only place the flag is set for a
            // live effect, and it is set before the call rather than after: a
            // call that panicked or was lost still may have happened, and the
            // answer to a later status query about it is therefore evidence.
            live.sent = true;
        }
        let outcome = if intent.binding().is_some() {
            self.owner.dispatch_bound(&intent, &auth, &required, at)
        } else {
            self.owner.dispatch(&intent, &auth, &required, at)
        };
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                // Typed owner refusal guarantees no attempted effect.
                // Restore the witness, including a previous successful reach.
                if let Some(previous) = previous {
                    self.live.insert(intent.operation, previous);
                }
                return Err(error.into());
            }
        };
        self.count_reach(&intent, warrant, report);
        Ok(Some(outcome))
    }

    /// Counts one reach that happened without the authority source.
    ///
    /// It spends one unit of offline authority; a typed owner refusal returns
    /// before this and spends none (E65). Nothing can move the fence between
    /// the handoff and here, as one call holds the runtime exclusively, so a
    /// refusal would be a broker defect: it is reported apart from the fenced
    /// reaches, which never happened, and the outcome of the reach that did
    /// happen is still recorded (larry#197).
    pub(super) fn count_reach(
        &mut self,
        intent: &EffectIntent,
        warrant: &Authority,
        report: &mut TickReport,
    ) {
        if let Err(refusal) = self.fence.spend(warrant, intent) {
            report.refused.push((
                TickItem::Effect(intent.operation),
                RuntimeError::Unspent {
                    operation: intent.operation,
                    refusal,
                },
            ));
        }
    }
}
