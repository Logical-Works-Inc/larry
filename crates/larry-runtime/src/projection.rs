//! The projection backend: the runtime's view of the journal.
//!
//! This module is the seam the Director asked for. The runtime never reads
//! state directly; it reads through [`ProjectionBackend`], and the only
//! implementation today is [`FoldProjection`], a deterministic fold over the
//! journal using `BTreeMap` throughout so no operation depends on hash
//! iteration order.
//!
//! **This is not a claim that a `bevy_ecs::World` may hold authority.** The
//! question "may the ECS world ever be authoritative, or is it always a
//! view?" is deliberately left open by the Director and is recorded as an
//! open decision in `docs/architecture/runtime.md`. What this seam buys is
//! that the answer stays cheap: a `bevy_ecs` backend can be admitted and
//! swapped in behind this trait without the reducer, the journal or the
//! effect lifecycle noticing, because none of them reference the world.

use std::collections::{BTreeMap, BTreeSet};

use larry_contracts::effect::{
    EffectIntent, EffectOutcome, TerminalDisposition, TerminalReceipt, terminal_disposition,
};
use larry_contracts::event::{Event, EventKind};
use larry_contracts::ids::{CommandId, OperationId, RunId, TenantId};
use larry_contracts::replay::ReplayRecord;
use larry_contracts::state::{RunState, transition};

use crate::error::RuntimeError;
use crate::journal::{Journal, JournalEntry};
use crate::payload::{
    CommandAcceptedPayload, CommandRefusal, CommandRefusedPayload, IntentPayload, OutcomePayload,
    ReceiptPayload, TransitionPayload,
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
    /// An empty view bounded to `capacity` runs.
    ///
    /// The runtime builds its projection through this, never `Default::default()`,
    /// so the view's own ceiling is the run table's (larry#212). A backend that
    /// keeps no capacity answers with its empty view.
    fn with_capacity(capacity: usize) -> Self
    where
        Self: Sized;

    /// Discards the current view and re-derives it from the whole journal.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::Codec`] for an undecodable payload and
    /// [`RuntimeError::FoldDivergence`] when a recorded transition disagrees
    /// with the pure algebra.
    fn rebuild(&mut self, journal: &Journal) -> Result<(), RuntimeError>;

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

    /// The run that recorded `operation`, if the journal has an intent for it.
    fn effect_run(&self, operation: OperationId) -> Option<RunId>;

    /// How many dispatches the journal records for `operation`, or the whole
    /// budget when a record cannot prove a lower count (larry#182).
    ///
    /// This is the count `restore` spends, so a restart continues the budget
    /// the journal recorded rather than resetting it. An operation the journal
    /// never recorded an outcome for proves nothing, so the answer is the
    /// whole budget.
    fn dispatches(&self, operation: OperationId) -> u8;

    /// How many effects the view tracks for `run`.
    fn effect_count(&self, run: RunId) -> usize;

    /// Every published terminal receipt, in publication order.
    fn receipts(&self) -> &[TerminalReceipt];

    /// Whether the journal has published a receipt for `operation`.
    fn has_receipt(&self, operation: OperationId) -> bool;

    /// Every recorded authorization, in decision order. Replaying this slice
    /// through `replay::replay_log` re-derives each decision from its own
    /// pinned inputs (R71, E55).
    fn authorizations(&self) -> &[ReplayRecord];

    /// Every recorded intent's cost, summed for the tenant its recorded event
    /// named (larry#208).
    ///
    /// The runtime keeps one budget per tenant, so `restore` re-derives each
    /// counter from these sums rather than from one pooled total. The tenant
    /// is the one the run's event carried, never the one the intent declared.
    /// This allocates and is called on the recovery path only.
    fn reserved_by_tenant(&self) -> BTreeMap<TenantId, u64>;

    /// Every reserved intent, in reservation order. Restoration rebuilds the
    /// live effect table from this, which is why recovery needs no side
    /// channel beyond the journal.
    fn intents(&self) -> &[EffectIntent];

    /// Every tracked run. This allocates and is called on the recovery path,
    /// not per tick.
    fn run_ids(&self) -> Vec<RunId>;
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
    /// How many dispatches each effect's recorded outcomes prove, keyed like
    /// `effects` (larry#182).
    dispatches: BTreeMap<OperationId, u8>,
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
            dispatches: BTreeMap::new(),
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

    /// The dispatch count the run's recorded outcomes prove for `operation`.
    #[must_use]
    pub fn dispatches(&self, operation: OperationId) -> Option<u8> {
        self.dispatches.get(&operation).copied()
    }
}

/// The deterministic fold implementation of [`ProjectionBackend`].
#[derive(Debug, Clone)]
pub struct FoldProjection {
    /// Run views keyed in deterministic identity order.
    runs: BTreeMap<RunId, RunView>,
    /// The operation-to-run witness used by the tick's hot effect lookups.
    operation_runs: BTreeMap<OperationId, RunId>,
    /// Terminal receipts in recorded order.
    receipts: Vec<TerminalReceipt>,
    /// Receipt identities for constant-size hot duplicate checks.
    receipt_operations: BTreeSet<OperationId>,
    /// Pinned decisions in recorded order.
    authorizations: Vec<ReplayRecord>,
    /// Recorded spend per tenant, in deterministic tenant order (larry#208).
    reserved: BTreeMap<TenantId, u64>,
    /// Reservations in recorded order.
    intents: Vec<EffectIntent>,
    /// Refusals of durably accepted commands, in recorded order (larry#371).
    refusals: Vec<(CommandId, CommandRefusal)>,
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
            runs: BTreeMap::new(),
            operation_runs: BTreeMap::new(),
            receipts: Vec::new(),
            receipt_operations: BTreeSet::new(),
            authorizations: Vec::new(),
            reserved: BTreeMap::new(),
            intents: Vec::new(),
            refusals: Vec::new(),
            capacity,
        }
    }

    /// The per-run view, if the run is tracked.
    #[must_use]
    pub fn run(&self, run: RunId) -> Option<&RunView> {
        self.runs.get(&run)
    }

    /// Every tracked run, in `RunId` order.
    #[must_use]
    pub fn runs(&self) -> &BTreeMap<RunId, RunView> {
        &self.runs
    }

    /// Why a durably accepted command never ran, in recorded order.
    ///
    /// The report a caller reads dies with the process; this fact is folded
    /// from the journal, so a restart answers the same question (larry#371).
    #[must_use]
    pub fn refusals(&self) -> &[(CommandId, CommandRefusal)] {
        &self.refusals
    }

    /// Folds one command-accepted record.
    fn fold_command_accepted(&mut self, event: &Event) -> Result<(), RuntimeError> {
        let payload: CommandAcceptedPayload = lgwks_std::json::from_slice(event.payload())?;
        let view = self.entry(payload.run, event)?;
        view.commands = view.commands.saturating_add(1);
        Ok(())
    }

    /// Folds one state-transition record, re-deriving the transition from the
    /// pure algebra rather than trusting the recorded target.
    fn fold_state_transition(&mut self, event: &Event) -> Result<(), RuntimeError> {
        let payload: TransitionPayload = lgwks_std::json::from_slice(event.payload())?;
        let rederived = transition(payload.from, payload.event)?;
        if rederived != payload.to {
            return Err(RuntimeError::FoldDivergence {
                run: event.run(),
                recorded: payload.to,
                rederived,
            });
        }
        let view = self.entry(event.run(), event)?;
        view.state = rederived;
        Ok(())
    }

    /// Folds one effect-reservation record.
    fn fold_intent(&mut self, event: &Event) -> Result<(), RuntimeError> {
        let payload: IntentPayload = lgwks_std::json::from_slice(event.payload())?;
        let operation = payload.intent.operation;
        if self.operation_runs.contains_key(&operation) {
            return Err(RuntimeError::DuplicateOperation { operation });
        }
        let view = self.entry(event.run(), event)?;
        view.effects.insert(operation, EffectPhase::Reserved);
        self.operation_runs.insert(operation, event.run());
        // The cost is charged to the tenant the recorded event named, never to
        // the tenant the intent declared (D42, larry#208).
        let spent = self.reserved.entry(event.tenant()).or_insert(0);
        *spent = spent.saturating_add(payload.authorization.pinned.request.cost());
        self.authorizations.push(payload.authorization);
        self.intents.push(payload.intent);
        Ok(())
    }

    /// Folds one observed-outcome record.
    fn fold_outcome(&mut self, event: &Event) -> Result<(), RuntimeError> {
        let payload: OutcomePayload = lgwks_std::json::from_slice(event.payload())?;
        let operation = payload.outcome.operation();
        let view = self.entry(event.run(), event)?;
        // The attempt number travels with the outcome, so the count a restart
        // spends is a fold over recorded facts, never live state (larry#182).
        view.dispatches
            .entry(operation)
            .and_modify(|spent| *spent = (*spent).max(payload.dispatches))
            .or_insert(payload.dispatches);
        view.effects.insert(
            operation,
            EffectPhase::Observed {
                outcome: payload.outcome,
            },
        );
        Ok(())
    }

    /// Folds one receipt record.
    fn fold_receipt(&mut self, event: &Event) -> Result<(), RuntimeError> {
        let payload: ReceiptPayload = lgwks_std::json::from_slice(event.payload())?;
        let operation = payload.receipt.operation();
        if !self.receipt_operations.insert(operation) {
            return Err(RuntimeError::ReceiptAlreadyPublished { operation });
        }
        let view = self.entry(payload.receipt.run(), event)?;
        view.effects.insert(
            operation,
            EffectPhase::Reconciled {
                disposition: payload.receipt.disposition(),
            },
        );
        self.receipts.push(payload.receipt);
        Ok(())
    }

    /// Folds one refusal of a durably accepted command. Nothing about the run
    /// changed; the fact says why the command never ran, and a restart reads
    /// it from the journal (larry#371).
    fn fold_command_refused(&mut self, event: &Event) -> Result<(), RuntimeError> {
        let payload: CommandRefusedPayload = lgwks_std::json::from_slice(event.payload())?;
        self.entry(event.run(), event)?;
        self.refusals.push((payload.command, payload.reason));
        Ok(())
    }

    /// Finds or admits a bounded run view and validates event sequencing.
    fn entry(&mut self, run: RunId, event: &Event) -> Result<&mut RunView, RuntimeError> {
        if !self.runs.contains_key(&run) && self.runs.len() >= self.capacity {
            return Err(RuntimeError::RunLimitReached {
                capacity: self.capacity,
            });
        }
        let view = self.runs.entry(run).or_default();
        // A run's tenant is the one its earliest recorded event carried. A
        // later event naming another tenant is a forged or corrupted journal,
        // so the fold refuses it rather than keeping either answer: first-wins
        // would silently restore a run under one tenant while its later facts
        // claim another (D42, larry#208).
        match view.tenant {
            Some(expected) if expected != event.tenant() => {
                return Err(RuntimeError::RunTenantMismatch {
                    run,
                    expected,
                    got: event.tenant(),
                });
            }
            Some(_) => {}
            None => view.tenant = Some(event.tenant()),
        }
        view.last_sequence = Some(event.sequence());
        Ok(view)
    }
}

impl ProjectionBackend for FoldProjection {
    fn with_capacity(capacity: usize) -> Self {
        Self::new(capacity)
    }

    fn rebuild(&mut self, journal: &Journal) -> Result<(), RuntimeError> {
        self.runs.clear();
        self.operation_runs.clear();
        self.receipts.clear();
        self.receipt_operations.clear();
        self.authorizations.clear();
        self.reserved.clear();
        self.intents.clear();
        self.refusals.clear();
        for entry in journal.entries() {
            if let JournalEntry::Event(ref event) = *entry {
                self.apply(event)?;
            }
        }
        Ok(())
    }

    fn apply(&mut self, event: &Event) -> Result<(), RuntimeError> {
        match event.kind() {
            EventKind::CommandAccepted => self.fold_command_accepted(event),
            EventKind::StateTransition => self.fold_state_transition(event),
            EventKind::EffectIntentRecorded => self.fold_intent(event),
            EventKind::EffectOutcomeRecorded => self.fold_outcome(event),
            EventKind::TerminalReceiptPublished => self.fold_receipt(event),
            EventKind::CommandRefused => self.fold_command_refused(event),
            other => Err(RuntimeError::UnsupportedEvent(other)),
        }
    }

    fn run_state(&self, run: RunId) -> Option<RunState> {
        self.runs.get(&run).map(|view| view.state)
    }

    fn run_tenant(&self, run: RunId) -> Option<TenantId> {
        self.runs.get(&run).and_then(RunView::tenant)
    }

    fn run_count(&self) -> usize {
        self.runs.len()
    }

    fn effect_phase(&self, operation: OperationId) -> Option<EffectPhase> {
        self.operation_runs
            .get(&operation)
            .and_then(|run| self.runs.get(run))
            .and_then(|view| view.effects.get(&operation).copied())
    }

    fn effect_run(&self, operation: OperationId) -> Option<RunId> {
        self.operation_runs.get(&operation).copied()
    }

    fn dispatches(&self, operation: OperationId) -> u8 {
        match self.effect_run(operation).and_then(|run| {
            self.runs
                .get(&run)
                .and_then(|view| view.dispatches(operation))
        }) {
            Some(spent) => spent,
            // The journal proves no dispatch, so it cannot prove fewer than the
            // whole budget was spent (larry#182): fail closed, never recharge.
            None => larry_contracts::effect::MAX_EFFECT_ATTEMPTS,
        }
    }

    fn effect_count(&self, run: RunId) -> usize {
        // A run the projection never saw has opened no effects.
        match self.runs.get(&run) {
            Some(view) => view.effects.len(),
            None => 0,
        }
    }

    fn receipts(&self) -> &[TerminalReceipt] {
        &self.receipts
    }

    fn has_receipt(&self, operation: OperationId) -> bool {
        self.receipt_operations.contains(&operation)
    }

    fn authorizations(&self) -> &[ReplayRecord] {
        &self.authorizations
    }

    fn reserved_by_tenant(&self) -> BTreeMap<TenantId, u64> {
        self.reserved.clone()
    }

    fn intents(&self) -> &[EffectIntent] {
        &self.intents
    }

    fn run_ids(&self) -> Vec<RunId> {
        self.runs.keys().copied().collect()
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
