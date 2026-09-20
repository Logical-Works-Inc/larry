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

use std::collections::BTreeMap;

use rocco_contracts::effect::{
    EffectIntent, EffectOutcome, TerminalDisposition, TerminalReceipt, terminal_disposition,
};
use rocco_contracts::event::{Event, EventKind};
use rocco_contracts::ids::{OperationId, RunId, TenantId};
use rocco_contracts::replay::ReplayRecord;
use rocco_contracts::state::{RunState, TransitionEvent, transition};

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
}

/// The per-run view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunView {
    state: RunState,
    tenant: Option<TenantId>,
    last_sequence: Option<rocco_contracts::event::EventSequence>,
    effects: BTreeMap<OperationId, EffectPhase>,
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
    pub const fn last_sequence(&self) -> Option<rocco_contracts::event::EventSequence> {
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

/// The deterministic fold implementation of [`ProjectionBackend`].
#[derive(Debug, Clone)]
pub struct FoldProjection {
    runs: BTreeMap<RunId, RunView>,
    receipts: Vec<TerminalReceipt>,
    authorizations: Vec<ReplayRecord>,
    intents: Vec<EffectIntent>,
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
            receipts: Vec::new(),
            authorizations: Vec::new(),
            intents: Vec::new(),
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
        let view = self.entry(event.run(), event)?;
        view.effects.insert(operation, EffectPhase::Reserved);
        self.authorizations.push(payload.authorization);
        self.intents.push(payload.intent);
        Ok(())
    }

    /// Folds one observed-outcome record.
    fn fold_outcome(&mut self, event: &Event) -> Result<(), RuntimeError> {
        let payload: OutcomePayload = lgwks_std::json::from_slice(event.payload())?;
        let operation = payload.outcome.operation();
        let view = self.entry(event.run(), event)?;
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

    fn entry(&mut self, run: RunId, event: &Event) -> Result<&mut RunView, RuntimeError> {
        if !self.runs.contains_key(&run) && self.runs.len() >= self.capacity {
            return Err(RuntimeError::RunLimitReached {
                capacity: self.capacity,
            });
        }
        let view = self.runs.entry(run).or_default();
        // First-wins: the run's tenant is the one its earliest recorded event
        // carried. A later event cannot re-attribute a run that is already
        // known, so a journal that names two tenants for one run keeps the one
        // the run was created under rather than flipping to the newest claim.
        view.tenant.get_or_insert(event.tenant());
        view.last_sequence = Some(event.sequence());
        Ok(view)
    }
}

impl ProjectionBackend for FoldProjection {
    fn rebuild(&mut self, journal: &Journal) -> Result<(), RuntimeError> {
        self.runs.clear();
        self.receipts.clear();
        self.authorizations.clear();
        self.intents.clear();
        for entry in journal.entries() {
            if let JournalEntry::Event(event) = entry {
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
        self.runs
            .values()
            .find_map(|view| view.effects.get(&operation).copied())
    }

    fn effect_count(&self, run: RunId) -> usize {
        self.runs.get(&run).map_or(0, |view| view.effects.len())
    }

    fn receipts(&self) -> &[TerminalReceipt] {
        &self.receipts
    }

    fn authorizations(&self) -> &[ReplayRecord] {
        &self.authorizations
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
) -> rocco_contracts::replay::ReplayReport {
    rocco_contracts::replay::replay_log(projection.authorizations())
}

/// The transition event that carries a run out of `Reconciling` when an
/// effect's outcome is unknown and its strategy leaves it open.
#[must_use]
pub const fn unknown_outcome_event() -> TransitionEvent {
    TransitionEvent::ObserveUnknownOutcome
}
