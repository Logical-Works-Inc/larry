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
use larry_contracts::ids::{OperationId, RunId, TenantId};
use larry_contracts::replay::ReplayRecord;
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
    Command,
    /// The run moves to `to`, re-derived from the pure algebra.
    Transition {
        /// The state the algebra produced.
        to: RunState,
    },
    /// A reservation, with the decision that admitted it.
    Intent(Box<IntentPayload>),
    /// An observed outcome.
    Outcome(Box<OutcomePayload>),
    /// A published terminal receipt.
    Receipt(ReceiptPayload),
}

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
}

impl RunFold {
    /// Every rule that can refuse a record, once, without touching the view:
    /// the payload decodes, a transition agrees with the pure algebra, the
    /// kind is one the view folds, and the record names a row. `apply` takes
    /// its decoded record from here, so the check and the fold cannot drift.
    fn check(event: &Event, has_row: bool) -> Result<Checked, RuntimeError> {
        let (checked, run) = match event.kind() {
            EventKind::CommandAccepted => {
                let payload: CommandAcceptedPayload = lgwks_std::json::from_slice(event.payload())?;
                (Checked::Command, payload.run)
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
                (Checked::Intent(Box::new(payload)), event.run())
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

    /// Records the event on the run's row. The tenant is first-wins: the one
    /// the run's earliest event carried. A later event cannot re-attribute a
    /// run that is already known, so a journal that names two tenants for one
    /// run keeps the one the run was created under.
    fn touch(view: &mut RunView, event: &Event) {
        view.tenant.get_or_insert(event.tenant());
        view.last_sequence = Some(event.sequence());
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
        let checked = Self::check(event, row.is_some())?;
        let Some(view) = row else {
            return Err(RuntimeError::RunNotTracked { run: event.run() });
        };
        Self::touch(view, event);
        match checked {
            Checked::Command => view.commands = view.commands.saturating_add(1),
            Checked::Transition { to } => view.state = to,
            Checked::Intent(payload) => {
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
                view.effects.insert(
                    payload.receipt.operation(),
                    EffectPhase::Reconciled {
                        disposition: payload.receipt.disposition(),
                    },
                );
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
        RunFold::check(event, key.is_some()).map(drop)
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
