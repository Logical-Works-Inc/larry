//! The projection backend: the runtime's view of the journal.
//!
//! This module is the seam the Director asked for. The runtime never reads
//! state directly; it reads through [`ProjectionBackend`], and the only
//! implementation is [`FoldProjection`]: `lgwks_bot`'s `RecordProjection`, one
//! ECS world entity per run, folded by [`RunFold`] (larry#79). Rows are read in
//! `RunId` order, never entity order, so no operation depends on hash or entity
//! iteration order.
//!
//! **The world is a view and holds no authority.** The journal is the truth.
//! Whether the world may ever be authoritative is the open decision recorded in
//! `docs/architecture/runtime.md`. Each event is folded as the runtime stages
//! it, before the group-commit sync, as an entry numbered by the view itself.
//! A fold that fails poisons the view until a rebuild, so the runtime asks
//! [`ProjectionBackend::admit`] before it journals a fact: a refused fact is
//! never written and never folded.

use std::collections::BTreeMap;

use lgwks_bot::journal::RecordEntry;
use lgwks_bot::{ProjectionError, RecordFold, RecordProjection};

use larry_contracts::effect::{
    EffectIntent, EffectOutcome, TerminalDisposition, TerminalReceipt, terminal_disposition,
};
use larry_contracts::event::{Event, EventKind};
use larry_contracts::evidence::ImproverContext;
use larry_contracts::harness::{HarnessAdoption, HarnessError, TenantHarness, Vetted};
use larry_contracts::ids::{ContentDigest, OperationId, RunId, TenantId};
use larry_contracts::learning::{Charge, LearningSpent};
use larry_contracts::recursion::{
    Label, PlanScope, Produced, RecursionError, RecursionLedger, RecursionPlan, RecursionStep,
    Stage, StepOutcome, StopError,
};
use larry_contracts::replay::ReplayRecord;
use larry_contracts::selection::{Selection, SelectionError, SelectionStep, prepare};
use larry_contracts::state::{RunState, TransitionEvent, transition};

use crate::error::RuntimeError;
use crate::journal::{Journal, JournalEntry};
use crate::payload::{
    CommandAcceptedPayload, IntentPayload, OutcomePayload, ReceiptPayload, TransitionPayload,
};

/// Where one effect stands, as far as the journal records.
///
/// There is deliberately no `Dispatched` variant. `contracts::event::EventKind`
/// is a closed vocabulary with no "dispatched" fact, and that is the point:
/// the dangerous window is between the durable reservation and the observed
/// outcome, and an effect that crashed inside it must read as *reserved* —
/// "may have happened" — rather than as dispatched-and-therefore-known. A
/// recovered run therefore cannot mistake a crash for a non-event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EffectPhase {
    /// A durable intent exists; the effect may or may not have reached the
    /// endpoint.
    Reserved,
    /// An outcome was recorded.
    Observed {
        /// What the owner reported.
        outcome: EffectOutcome,
    },
    /// The outcome resolved to a terminal disposition; a receipt exists.
    Reconciled {
        /// The recorded disposition.
        disposition: TerminalDisposition,
    },
}

/// The read-only view the runtime and its callers use.
pub trait ProjectionBackend {
    /// Discards the current view and re-derives it from the whole journal.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::Codec`] for an undecodable payload and
    /// [`RuntimeError::FoldDivergence`] when a recorded transition disagrees
    /// with the pure algebra.
    fn rebuild(&mut self, journal: &Journal) -> Result<(), RuntimeError>;

    /// Refuses, without changing the view, a fact that [`ProjectionBackend::apply`]
    /// would refuse. A refused fold poisons the view, so the runtime asks here
    /// before it writes the fact to the journal.
    ///
    /// # Errors
    ///
    /// The conditions of [`ProjectionBackend::apply`], and
    /// [`RuntimeError::RunLimitReached`] for a new run past the view's capacity.
    fn admit(&self, event: &Event) -> Result<(), RuntimeError>;

    /// Folds one recorded fact into the view.
    ///
    /// # Errors
    ///
    /// Same conditions as [`ProjectionBackend::rebuild`].
    fn apply(&mut self, event: &Event) -> Result<(), RuntimeError>;

    /// The current state of `run`.
    fn run_state(&self, run: RunId) -> Option<RunState>;

    /// The tenant `run` was recorded under, as the *event* carried it.
    ///
    /// This is the run's tenant for every purpose, including recovery: it is
    /// the value the runtime wrote into the run's own records, not a value
    /// inferred from an effect intent (whose `tenant` is declared by the
    /// submitting proposal). Restoration rebuilds the run table from this, so
    /// a run whose recorded history reserved no effect is still attributable
    /// to its tenant (D04, D42).
    fn run_tenant(&self, run: RunId) -> Option<TenantId>;

    /// How many runs the view tracks.
    fn run_count(&self) -> usize;

    /// Where `operation` stands.
    fn effect_phase(&self, operation: OperationId) -> Option<EffectPhase>;

    /// How many effects the view tracks for `run`.
    fn effect_count(&self, run: RunId) -> usize;

    /// Every published terminal receipt, in publication order.
    fn receipts(&self) -> &[TerminalReceipt];

    /// Every recorded authorization, in decision order. Replaying this slice
    /// through `replay::replay_log` re-derives each decision from its own
    /// pinned inputs (R71, E55).
    fn authorizations(&self) -> &[ReplayRecord];

    /// Every reserved intent, in reservation order. Restoration rebuilds the
    /// live effect table from this, which is why recovery needs no side
    /// channel beyond the journal.
    fn intents(&self) -> &[EffectIntent];

    /// Every tracked run. This allocates and is called on the recovery path,
    /// not per tick.
    fn run_ids(&self) -> Vec<RunId>;

    /// Refuses, without changing the view, an adoption the tenant's record would refuse. The runtime
    /// asks before it writes the command that carries it (larry#61).
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Harness`] naming the first refusing rule.
    fn vet_adoption(
        &self,
        tenant: TenantId,
        adoption: &HarnessAdoption,
    ) -> Result<(), RuntimeError>;

    /// Refuses, without changing the view, a recursion step the tenant's ledger would refuse: a
    /// duplicate label, rollout 4k + 1, session 3k + 1, an unsettled dependency, a full probe width, a
    /// probe naming the wrong candidate, or a plan past the tenant's bound (larry#62).
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Recursion`], [`RuntimeError::RecursionPlan`] or
    /// [`RuntimeError::RecursionPlanLimit`].
    fn vet_step(&self, tenant: TenantId, step: &RecursionStep) -> Result<(), RuntimeError>;

    /// Refuses, without changing the view, a selection the fold would refuse: a plan that has not
    /// settled, or one that already has its one selection (recorded or in flight) (larry#64).
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Selection`] or [`RuntimeError::RecursionPlan`].
    fn vet_pick(&self, tenant: TenantId, step: &SelectionStep) -> Result<(), RuntimeError>;

    /// The one selection `tenant` recorded for `plan`, once its selector run succeeded.
    fn selection(&self, tenant: TenantId, plan: &RecursionPlan) -> Option<&Selection>;

    /// Refuses, without changing the view, a stop that would change nothing: an unknown plan, an
    /// already stopped plan, or no running plan to stop (larry#66).
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Stop`].
    fn vet_stop(&self, tenant: TenantId, scope: PlanScope) -> Result<(), RuntimeError>;

    /// What `tenant`'s learning has reserved so far, across all its plans (larry#66).
    fn learning_spent(&self, tenant: TenantId) -> LearningSpent;

    /// The ledger `tenant` keeps for `plan`, if any step of it was recorded.
    fn recursion(&self, tenant: TenantId, plan: &RecursionPlan) -> Option<&RecursionLedger>;

    /// The harness record `tenant` holds, if it has any. The tenant is the run table's, never a
    /// proposal's declaration.
    fn harness(&self, tenant: TenantId) -> Option<&TenantHarness>;
}

/// The per-run view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunView {
    /// State re-derived from recorded transitions.
    state: RunState,
    /// Tenant recorded by the run's events.
    tenant: Option<TenantId>,
    /// Last folded event sequence.
    last_sequence: Option<larry_contracts::event::EventSequence>,
    /// Effect phases keyed in deterministic operation order.
    effects: BTreeMap<OperationId, EffectPhase>,
    /// Count of accepted commands folded for this run.
    commands: usize,
}

impl Default for RunView {
    fn default() -> Self {
        Self {
            state: RunState::Draft,
            tenant: None,
            last_sequence: None,
            effects: BTreeMap::new(),
            commands: 0,
        }
    }
}

impl RunView {
    /// The run's current state.
    #[must_use]
    pub const fn state(&self) -> RunState {
        self.state
    }

    /// The tenant the run's recorded events carry, if any event named it.
    #[must_use]
    pub const fn tenant(&self) -> Option<TenantId> {
        self.tenant
    }

    /// The last recorded sequence for the run.
    #[must_use]
    pub const fn last_sequence(&self) -> Option<larry_contracts::event::EventSequence> {
        self.last_sequence
    }

    /// How many commands the run has accepted.
    #[must_use]
    pub const fn commands(&self) -> usize {
        self.commands
    }

    /// Every effect the run reserved.
    #[must_use]
    pub fn effects(&self) -> &BTreeMap<OperationId, EffectPhase> {
        &self.effects
    }
}

/// A record every refusal rule has passed, with what the fold then writes.
enum Checked {
    /// A command was accepted onto the run.
    Command(Option<PlanScope>),
    /// The run moves to `to`, re-derived from the pure algebra.
    Transition {
        /// The state the algebra produced.
        to: RunState,
    },
    /// A reservation, with the decision that admitted it.
    Intent(Box<IntentPayload>, Option<Vetted>, Option<Selection>),
    /// An observed outcome.
    Outcome(Box<OutcomePayload>),
    /// A published terminal receipt.
    Receipt(ReceiptPayload),
}

/// A recursion step whose effect has no receipt yet.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenStep {
    /// The run that carries it.
    run: RunId,
    /// The run's tenant.
    tenant: TenantId,
    /// The plan's identity.
    plan: ContentDigest,
    /// Which step.
    label: Label,
}

/// What an intent charges to learning: a recursion step or a selector run, by its authorized cost.
pub(crate) fn learning_charge(payload: &IntentPayload) -> Option<Charge> {
    let request = &payload.authorization.pinned.request;
    payload
        .recursion
        .as_ref()
        .map(|step| Charge::of_step(step.label.stage(), step.attempt, request.cost, request.unit))
        .or_else(|| {
            payload
                .selection
                .as_ref()
                .map(|_| Charge::of_selection(request.cost, request.unit))
        })
}

/// A plan's selection: in flight until its selector run succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pick {
    /// Reserved by an intent; not yet a fact.
    Pending(Selection),
    /// The selector run succeeded.
    Recorded(Selection),
}

/// The most recursion plans one tenant's view keeps.
pub(crate) const MAX_PLANS_PER_TENANT: usize = 16;

/// What the view keeps beside its rows: the three ordered lists no single run
/// owns, and the fold rules that fill them and the rows.
#[derive(Debug, Default)]
pub(crate) struct RunFold {
    /// Terminal receipts in recorded order.
    receipts: Vec<TerminalReceipt>,
    /// Pinned decisions in recorded order.
    authorizations: Vec<ReplayRecord>,
    /// Reservations in recorded order.
    intents: Vec<EffectIntent>,
    /// Each tenant's harness record, folded from intents and receipts (larry#61).
    harness: BTreeMap<TenantId, TenantHarness>,
    /// Each tenant's recursion ledgers by plan identity (larry#62).
    recursion: BTreeMap<(TenantId, ContentDigest), RecursionLedger>,
    /// Recursion steps with an intent and no receipt, by operation.
    open_steps: BTreeMap<OperationId, OpenStep>,
    /// What each tenant's learning has reserved (larry#66).
    learning: BTreeMap<TenantId, LearningSpent>,
    /// Each plan's one selection, in flight or recorded (larry#64).
    picks: BTreeMap<(TenantId, ContentDigest), Pick>,
    /// Selector runs with an intent and no receipt, by operation.
    open_picks: BTreeMap<OperationId, ((TenantId, ContentDigest), RunId)>,
}

impl RunFold {
    /// Every rule that can refuse a record, once, without touching the view:
    /// the payload decodes, a transition agrees with the pure algebra, the
    /// kind is one the view folds, and the record names a row. `apply` takes
    /// its decoded record from here, so the check and the fold cannot drift.
    fn check(
        &self,
        event: &Event,
        has_row: bool,
        run_tenant: Option<TenantId>,
    ) -> Result<Checked, RuntimeError> {
        let (checked, run) = match event.kind() {
            EventKind::CommandAccepted => {
                let payload: CommandAcceptedPayload = lgwks_std::json::from_slice(event.payload())?;
                (Checked::Command(payload.stop), payload.run)
            }
            EventKind::StateTransition => {
                // Re-derived from the pure algebra, never trusting the recorded target.
                let payload: TransitionPayload = lgwks_std::json::from_slice(event.payload())?;
                let rederived = transition(payload.from, payload.event)?;
                if rederived != payload.to {
                    return Err(RuntimeError::FoldDivergence {
                        run: event.run(),
                        recorded: payload.to,
                        rederived,
                    });
                }
                (Checked::Transition { to: rederived }, event.run())
            }
            EventKind::EffectIntentRecorded => {
                let payload: IntentPayload = lgwks_std::json::from_slice(event.payload())?;
                // The run's tenant, never the proposal's declaration (D42, E63).
                let tenant = run_tenant.unwrap_or_else(|| event.tenant());
                if let Some(ref step) = payload.recursion {
                    self.vet_recursion(tenant, step)?;
                }
                let picked = match payload.selection {
                    Some(ref pick) => Some(self.vet_selection(tenant, pick)?),
                    None => None,
                };
                let vetted = match payload.harness {
                    Some(ref adoption) => Some(self.vet(tenant, adoption)?),
                    None => None,
                };
                (
                    Checked::Intent(Box::new(payload), vetted, picked),
                    event.run(),
                )
            }
            EventKind::EffectOutcomeRecorded => {
                let payload: OutcomePayload = lgwks_std::json::from_slice(event.payload())?;
                (Checked::Outcome(Box::new(payload)), event.run())
            }
            EventKind::TerminalReceiptPublished => {
                let payload: ReceiptPayload = lgwks_std::json::from_slice(event.payload())?;
                let run = payload.receipt.run();
                (Checked::Receipt(payload), run)
            }
            other => return Err(RuntimeError::UnsupportedEvent(other)),
        };
        if has_row {
            Ok(checked)
        } else {
            Err(RuntimeError::RunNotTracked { run })
        }
    }

    /// The one rule set for a recursion step: the tenant's ledger vets it on a copy.
    fn vet_recursion(&self, tenant: TenantId, step: &RecursionStep) -> Result<(), RuntimeError> {
        let plan = step.plan.validated()?;
        let mut ledger = match self.recursion.get(&(tenant, plan.identity())) {
            Some(held) => held.clone(),
            None => {
                let held = self.recursion.keys().filter(|key| key.0 == tenant).count();
                if held >= MAX_PLANS_PER_TENANT {
                    return Err(RuntimeError::RecursionPlanLimit {
                        capacity: MAX_PLANS_PER_TENANT,
                    });
                }
                RecursionLedger::new(plan)
            }
        };
        let label = step.label;
        let probe = label.stage() == Stage::Probe;
        if step.candidate.is_some() != probe {
            return Err(RecursionError::ProducedMismatch { label }.into());
        }
        ledger.record_intent(label, step.attempt)?;
        if probe && ledger.candidate(label.round(), label.slot() >> 1) != step.candidate {
            return Err(RecursionError::CandidateMismatch { label }.into());
        }
        Ok(())
    }

    /// The one rule set for a selection: the plan's ledger must have settled and have no selection.
    /// The seed uses the run table's tenant. The result does not depend on the selector's context,
    /// so the fold prepares with an empty one.
    fn vet_selection(
        &self,
        tenant: TenantId,
        step: &SelectionStep,
    ) -> Result<Selection, RuntimeError> {
        let plan = step.plan.validated()?;
        let key = (tenant, plan.identity());
        if self.picks.contains_key(&key) {
            return Err(SelectionError::AlreadySelected.into());
        }
        let ledger = self.recursion.get(&key).ok_or(SelectionError::NotSettled)?;
        let context =
            ImproverContext::assemble(Vec::new()).map_err(|_| SelectionError::NotSettled)?;
        Ok(prepare(ledger, tenant, context)?.decide(&step.answer))
    }

    /// The one rule set for an adoption: the tenant's record vets it.
    fn vet(&self, tenant: TenantId, adoption: &HarnessAdoption) -> Result<Vetted, HarnessError> {
        match self.harness.get(&tenant) {
            Some(record) => record.vet(adoption),
            None => TenantHarness::default().vet(adoption),
        }
    }

    /// Records the event on the run's row. The tenant is first-wins: the one
    /// the run's earliest event carried. A later event cannot re-attribute a
    /// run that is already known, so a journal that names two tenants for one
    /// run keeps the one the run was created under.
    fn touch(view: &mut RunView, event: &Event) {
        view.tenant.get_or_insert(event.tenant());
        view.last_sequence = Some(event.sequence());
    }
}

impl RunFold {
    /// A succeeded selector run (not cancelling) makes its selection a fact; any other end frees the
    /// plan for another selection.
    fn settle_pick(&mut self, receipt: &TerminalReceipt, live: bool) {
        let Some((key, _)) = self.open_picks.remove(&receipt.operation()) else {
            return;
        };
        let won = live && receipt.disposition() == TerminalDisposition::Succeeded;
        match self.picks.remove(&key) {
            Some(Pick::Pending(chosen)) if won => {
                self.picks.insert(key, Pick::Recorded(chosen));
            }
            Some(other @ Pick::Recorded(_)) => {
                self.picks.insert(key, other);
            }
            Some(Pick::Pending(_)) | None => {}
        }
    }

    /// A run that ended without a receipt frees its plan's pending selection.
    fn abandon_picks(&mut self, run: RunId) {
        let held: Vec<OperationId> = self
            .open_picks
            .iter()
            .filter(|entry| entry.1.1 == run)
            .map(|entry| *entry.0)
            .collect();
        for operation in held {
            if let Some((key, _)) = self.open_picks.remove(&operation)
                && matches!(self.picks.get(&key), Some(Pick::Pending(_)))
            {
                self.picks.remove(&key);
            }
        }
    }

    /// Maps a terminal receipt to the recursion step's outcome.
    fn settle_step(&mut self, payload: &ReceiptPayload) -> Result<(), RuntimeError> {
        let receipt = &payload.receipt;
        let Some(open) = self.open_steps.remove(&receipt.operation()) else {
            return Ok(());
        };
        let outcome = match receipt.disposition() {
            TerminalDisposition::Succeeded => {
                match (open.label.stage(), payload.produced.clone()) {
                    (Stage::Probe, _) => StepOutcome::Succeeded { produced: None },
                    (_, Some(Produced::Harness(hash))) => StepOutcome::Succeeded {
                        produced: Some(hash),
                    },
                    (Stage::Propose, Some(Produced::Refused(error))) => {
                        // The proposal failed the harness reducer: one retry carrying the error, then a
                        // fallback to the round base.
                        if let Some(ledger) = self.recursion.get_mut(&(open.tenant, open.plan)) {
                            ledger.record_refusal(open.label, error)?;
                        }
                        return Ok(());
                    }
                    // A session that reports no harness, or a refused composition, produced nothing.
                    _ => StepOutcome::Failed,
                }
            }
            TerminalDisposition::Failed | TerminalDisposition::Compensated => StepOutcome::Failed,
            TerminalDisposition::UnknownPermanent => StepOutcome::Unknown,
            other => return Err(RuntimeError::UnsupportedDisposition(other)),
        };
        if let Some(ledger) = self.recursion.get_mut(&(open.tenant, open.plan)) {
            ledger.record_outcome(open.label, outcome)?;
        }
        Ok(())
    }

    /// The one rule set for a stop: it must change something.
    fn vet_stop(&self, tenant: TenantId, scope: PlanScope) -> Result<(), StopError> {
        match scope {
            PlanScope::One(identity) => match self.recursion.get(&(tenant, identity)) {
                None => Err(StopError::UnknownPlan),
                Some(ledger) if ledger.is_cancelled() => Err(StopError::AlreadyStopped),
                Some(_) => Ok(()),
            },
            PlanScope::All => {
                let running = self
                    .recursion
                    .iter()
                    .any(|entry| entry.0.0 == tenant && !entry.1.is_cancelled());
                if running {
                    Ok(())
                } else {
                    Err(StopError::NothingToStop)
                }
            }
            // The scope is non-exhaustive across crates: a scope this build does not know stops nothing.
            _ => Err(StopError::UnknownPlan),
        }
    }

    /// Stops the plans in scope for `tenant`, the run table's tenant. Steps in flight still settle.
    fn apply_stop(&mut self, tenant: TenantId, scope: PlanScope) {
        for entry in self.recursion.iter_mut() {
            let covered = entry.0.0 == tenant
                && match scope {
                    PlanScope::One(identity) => entry.0.1 == identity,
                    PlanScope::All => true,
                    _ => false,
                };
            if covered && !entry.1.is_cancelled() {
                entry.1.cancel();
            }
        }
    }

    /// A cancelling run stops its plan; a cancelled run, whose effects all settled, also releases the
    /// slots its steps held.
    fn release_cancelled(&mut self, run: RunId, cancelled: bool) -> Result<(), RuntimeError> {
        let held: Vec<OperationId> = self
            .open_steps
            .iter()
            .filter(|entry| entry.1.run == run)
            .map(|entry| *entry.0)
            .collect();
        for operation in held {
            let Some(open) = self.open_steps.get(&operation).cloned() else {
                continue;
            };
            if let Some(ledger) = self.recursion.get_mut(&(open.tenant, open.plan)) {
                if !ledger.is_cancelled() {
                    ledger.cancel();
                }
                if cancelled {
                    ledger.record_outcome(open.label, StepOutcome::Failed)?;
                }
            }
            if cancelled {
                self.open_steps.remove(&operation);
            }
        }
        Ok(())
    }
}

impl RecordFold<Event> for RunFold {
    type Key = RunId;
    type Row = RunView;
    type Error = RuntimeError;

    fn route(&self, record: &Event) -> Option<RunId> {
        // Two payloads name their own run. One that does not decode routes
        // nowhere, and `apply` then reports the decode failure.
        match record.kind() {
            EventKind::CommandAccepted => {
                let payload: CommandAcceptedPayload =
                    lgwks_std::json::from_slice(record.payload()).ok()?;
                Some(payload.run)
            }
            EventKind::TerminalReceiptPublished => {
                let payload: ReceiptPayload = lgwks_std::json::from_slice(record.payload()).ok()?;
                Some(payload.receipt.run())
            }
            EventKind::StateTransition
            | EventKind::EffectIntentRecorded
            | EventKind::EffectOutcomeRecorded => Some(record.run()),
            _ => None,
        }
    }

    fn apply(
        &mut self,
        entry: &RecordEntry<Event>,
        row: Option<&mut RunView>,
    ) -> Result<(), RuntimeError> {
        let event = entry.record();
        let checked = self.check(
            event,
            row.is_some(),
            row.as_deref().and_then(RunView::tenant),
        )?;
        let Some(view) = row else {
            return Err(RuntimeError::RunNotTracked { run: event.run() });
        };
        let previous = view.state;
        Self::touch(view, event);
        let tenant = view.tenant.unwrap_or_else(|| event.tenant());
        match checked {
            Checked::Command(stop) => {
                view.commands = view.commands.saturating_add(1);
                if let Some(scope) = stop {
                    self.apply_stop(tenant, scope);
                }
            }
            Checked::Transition { to } => {
                view.state = to;
                if to.is_terminal()
                    && let Some(record) = self.harness.get_mut(&tenant)
                {
                    // A run that ends without a succeeded receipt adopts nothing.
                    record.abandon(event.run());
                }
                if to.is_terminal() {
                    self.abandon_picks(event.run());
                }
                if to == RunState::Cancelling || to == RunState::Cancelled {
                    self.release_cancelled(event.run(), to == RunState::Cancelled)?;
                }
            }
            Checked::Intent(payload, vetted, picked) => {
                if let Some(charge) = learning_charge(&payload) {
                    let spent = self.learning.entry(tenant).or_default();
                    *spent = spent.plus(&charge);
                }
                if let (Some(pick), Some(chosen)) = (payload.selection.as_ref(), picked) {
                    let key = (tenant, pick.plan.identity());
                    self.picks.insert(key, Pick::Pending(chosen));
                    self.open_picks
                        .insert(payload.intent.operation, (key, event.run()));
                }
                if let Some(ref step) = payload.recursion {
                    let plan = step.plan;
                    self.recursion
                        .entry((tenant, plan.identity()))
                        .or_insert_with(|| RecursionLedger::new(plan))
                        .record_intent(step.label, step.attempt)?;
                    self.open_steps.insert(
                        payload.intent.operation,
                        OpenStep {
                            run: event.run(),
                            tenant,
                            plan: plan.identity(),
                            label: step.label,
                        },
                    );
                }
                if let Some(adopt) = vetted {
                    self.harness.entry(tenant).or_default().reserve(
                        payload.intent.operation,
                        event.run(),
                        adopt,
                    );
                }
                view.effects
                    .insert(payload.intent.operation, EffectPhase::Reserved);
                self.authorizations.push(payload.authorization);
                self.intents.push(payload.intent);
            }
            Checked::Outcome(payload) => {
                view.effects.insert(
                    payload.outcome.operation(),
                    EffectPhase::Observed {
                        outcome: payload.outcome,
                    },
                );
            }
            Checked::Receipt(payload) => {
                // Adoption is the effect's success and nothing earlier: a succeeded receipt on a
                // run that is not being cancelled. Every other ending settles it unadopted.
                if let Some(record) = self.harness.get_mut(&tenant) {
                    let adopted = payload.receipt.disposition() == TerminalDisposition::Succeeded
                        && previous != RunState::Cancelling;
                    record.settle(payload.receipt.operation(), adopted);
                }
                view.effects.insert(
                    payload.receipt.operation(),
                    EffectPhase::Reconciled {
                        disposition: payload.receipt.disposition(),
                    },
                );
                self.settle_step(&payload)?;
                self.settle_pick(&payload.receipt, previous != RunState::Cancelling);
                self.receipts.push(payload.receipt);
            }
        }
        Ok(())
    }
}

/// The [`ProjectionBackend`] on `lgwks_bot`'s ECS world: one world entity per
/// run, folded by [`RunFold`]. It is a view of the journal and holds no
/// authority; [`rebuild`](ProjectionBackend::rebuild) discards it and folds the
/// journal again.
#[derive(Debug)]
pub struct FoldProjection {
    /// The world, with one row per run in `RunId` order.
    view: RecordProjection<Event, RunFold>,
    /// Maximum number of tracked run views.
    capacity: usize,
}

impl Default for FoldProjection {
    fn default() -> Self {
        Self::new(crate::MAX_TRACKED_RUNS)
    }
}

impl FoldProjection {
    /// An empty view bounded to `capacity` runs.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            view: RecordProjection::empty(RunFold::default(), capacity),
            capacity,
        }
    }

    /// The per-run view, if the run is tracked.
    #[must_use]
    pub fn run(&self, run: RunId) -> Option<&RunView> {
        self.view.row(&run)
    }

    /// Every tracked run, in `RunId` order.
    pub fn runs(&self) -> impl Iterator<Item = (&RunId, &RunView)> {
        self.view.rows()
    }
}

impl ProjectionBackend for FoldProjection {
    fn rebuild(&mut self, journal: &Journal) -> Result<(), RuntimeError> {
        *self = Self::new(self.capacity);
        for entry in journal.entries() {
            if let JournalEntry::Event(ref event) = *entry {
                self.apply(event)?;
            }
        }
        Ok(())
    }

    fn admit(&self, event: &Event) -> Result<(), RuntimeError> {
        let key = self.view.fold().route(event);
        let known = key.is_some_and(|run| self.view.row(&run).is_some());
        if key.is_some() && !known && self.view.rows().count() >= self.capacity {
            return Err(RuntimeError::RunLimitReached {
                capacity: self.capacity,
            });
        }
        // A routed record always has a row once folded, so the check sees one.
        let tenant = key
            .and_then(|run| self.view.row(&run))
            .and_then(RunView::tenant);
        self.view
            .fold()
            .check(event, key.is_some(), tenant)
            .map(drop)
    }

    fn apply(&mut self, event: &Event) -> Result<(), RuntimeError> {
        // The view counts events only (commands have no row), contiguously
        // from one, so the sequence is the view's own and never the file's.
        let sequence = self.view.applied().map_or(1, |last| last.saturating_add(1));
        let entry = RecordEntry::staged(sequence, event.clone());
        self.view.fold_one(&entry).map_err(|refused| match refused {
            ProjectionError::Fold(cause) => cause,
            ProjectionError::RowLimit { limit } => {
                RuntimeError::RunLimitReached { capacity: limit }
            }
            ProjectionError::Journal(cause) => RuntimeError::Durable(cause),
            ProjectionError::Poisoned => RuntimeError::ProjectionPoisoned,
            other => RuntimeError::ProjectionRefused {
                reason: other.to_string(),
            },
        })
    }

    fn run_state(&self, run: RunId) -> Option<RunState> {
        self.run(run).map(RunView::state)
    }

    fn run_tenant(&self, run: RunId) -> Option<TenantId> {
        self.run(run).and_then(RunView::tenant)
    }

    fn run_count(&self) -> usize {
        self.view.rows().count()
    }

    fn effect_phase(&self, operation: OperationId) -> Option<EffectPhase> {
        self.view
            .rows()
            .find_map(|(_, view)| view.effects.get(&operation).copied())
    }

    fn effect_count(&self, run: RunId) -> usize {
        // A run the projection never saw has opened no effects.
        self.run(run).map_or(0, |view| view.effects.len())
    }

    fn receipts(&self) -> &[TerminalReceipt] {
        &self.view.fold().receipts
    }

    fn authorizations(&self) -> &[ReplayRecord] {
        &self.view.fold().authorizations
    }

    fn intents(&self) -> &[EffectIntent] {
        &self.view.fold().intents
    }

    fn run_ids(&self) -> Vec<RunId> {
        self.view.rows().map(|(run, _)| *run).collect()
    }

    fn vet_adoption(
        &self,
        tenant: TenantId,
        adoption: &HarnessAdoption,
    ) -> Result<(), RuntimeError> {
        self.view.fold().vet(tenant, adoption)?;
        Ok(())
    }

    fn harness(&self, tenant: TenantId) -> Option<&TenantHarness> {
        self.view.fold().harness.get(&tenant)
    }

    fn vet_step(&self, tenant: TenantId, step: &RecursionStep) -> Result<(), RuntimeError> {
        self.view.fold().vet_recursion(tenant, step)
    }

    fn recursion(&self, tenant: TenantId, plan: &RecursionPlan) -> Option<&RecursionLedger> {
        self.view.fold().recursion.get(&(tenant, plan.identity()))
    }

    fn vet_stop(&self, tenant: TenantId, scope: PlanScope) -> Result<(), RuntimeError> {
        Ok(self.view.fold().vet_stop(tenant, scope)?)
    }

    fn learning_spent(&self, tenant: TenantId) -> LearningSpent {
        self.view
            .fold()
            .learning
            .get(&tenant)
            .map_or_else(LearningSpent::default, |held| *held)
    }

    fn vet_pick(&self, tenant: TenantId, step: &SelectionStep) -> Result<(), RuntimeError> {
        self.view.fold().vet_selection(tenant, step).map(drop)
    }

    fn selection(&self, tenant: TenantId, plan: &RecursionPlan) -> Option<&Selection> {
        let pick = self.view.fold().picks.get(&(tenant, plan.identity()))?;
        match *pick {
            Pick::Recorded(ref chosen) => Some(chosen),
            Pick::Pending(_) => None,
        }
    }
}

/// Resolves an observed outcome against its declared strategy, refusing to
/// terminate an effect whose strategy leaves the unknown open (R72, E56).
///
/// # Errors
///
/// Returns [`RuntimeError::UnresolvableOutcome`] when the strategy requires
/// status query or bounded retry before any disposition may be recorded.
pub fn resolve(
    intent: &EffectIntent,
    outcome: EffectOutcome,
) -> Result<TerminalDisposition, RuntimeError> {
    terminal_disposition(outcome, intent.strategy).ok_or(RuntimeError::UnresolvableOutcome {
        operation: intent.operation,
    })
}

/// Re-derives every authorization the journal recorded and reports each
/// finding (R71, E55).
///
/// The returned report's `is_bit_identical` is the direct answer to "does
/// this journal replay identically?": it is `true` only when every decision
/// re-derived from its own pinned inputs equals the decision that was
/// recorded at the time.
#[must_use]
pub fn replay_authorizations(
    projection: &impl ProjectionBackend,
) -> larry_contracts::replay::ReplayReport {
    larry_contracts::replay::replay_log(projection.authorizations())
}

/// The transition event that carries a run out of `Reconciling` when an
/// effect's outcome is unknown and its strategy leaves it open.
#[must_use]
pub const fn unknown_outcome_event() -> TransitionEvent {
    TransitionEvent::ObserveUnknownOutcome
}

#[cfg(test)]
mod tests;
