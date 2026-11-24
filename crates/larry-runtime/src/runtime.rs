//! The runtime: one deterministic tick, one authoritative journal.
//!
//! The tick is a fixed sequence of stages, in this order and no other:
//!
//! 1. **sample** — read the observed instant exactly once, and use that one
//!    value for every authorization made in the tick (E30);
//! 2. **admit** — duplicate-suppress each ready command, record its
//!    acceptance, drive the run-state algebra, then gate, reserve, dispatch
//!    and record the outcome;
//! 3. **reconcile** — for every effect whose journal position is still
//!    `Reserved`, ask its owner what happened. This is where a resumed run
//!    and an ambiguous outcome both land;
//! 4. **cancel** — complete a cancellation only once every owned effect has
//!    stopped or reconciled. Cancellation requested is not cancellation
//!    completed.
//!
//! Every stage is bounded by an explicit constant, every collection is a
//! `BTreeMap` or an insertion-ordered `Vec`, and nothing consults the wall
//! clock except stage 1.
//!
//! # The one place the fence is enforced
//!
//! Stage 2 reserves, then reaches the owner, and the reach is the only way
//! anything leaves this process. Both lifecycle paths — a first dispatch and
//! a retry — funnel through [`Runtime::reach_owner`], so the freshness fence
//! (R87, D48, E71) has exactly one enforcement point and exactly one place
//! to audit. It gates *mutating* reaches and only mutating reaches: a status
//! query is how an effect that was already dispatched when revocation landed
//! gets resolved, and fencing it would leave an external effect permanently
//! unlabelled. A refused reach calls no owner, records no outcome and
//! publishes no receipt — the effect stays visibly open, which is the only
//! truthful state for an effect that was reserved and never sent.
//!
//! Keeping the query open is not the same as believing its answer. The
//! journal cannot tell a refused first dispatch from a crash inside the
//! dispatch window, but the live effect table can — a refused reach calls no
//! owner, so `LiveEffect::sent` stays false — and it is what separates the
//! two: an endpoint's answer resolves an effect this runtime sent, and
//! resolves nothing about one it never sent. That is what makes the paragraph
//! above true for the effect's whole life rather than only for the tick that
//! refused it.

mod admission;
mod approval;
mod display;
mod ingress;
pub(crate) mod instant;
mod live;
mod reach;
mod reconcile;
mod retry;
mod settings;
mod writers;

pub use settings::RuntimeSettings;

use admission::Admitted;
use instant::sample;
use live::{LiveEffect, OwnerReach, RunLive};

pub use approval::{Answer, Decision, Question, QuestionError, Subject};
pub use display::DisplayedEffect;
use display::ReadyCommand;

use std::collections::{BTreeMap, VecDeque};

use lgwks_bot::kernel::clock::Clock;

use larry_contracts::command::{Command, CommandKind};
use larry_contracts::effect::{
    EffectIntent, EffectOutcome, ReconciliationStrategy, TerminalDisposition, TerminalReceipt,
};
use larry_contracts::event::EventKind;
use larry_contracts::ids::{CommandId, OperationId, RunId, TenantId};
use larry_contracts::replay::DenialReason;
use larry_contracts::state::{RunState, TransitionEvent};

use crate::MAX_READY_WORK;
use crate::error::RuntimeError;
use crate::fence::{FenceRefusal, RevocationFence};
use crate::journal::{Journal, JournalEntry};
use crate::owner::EffectOwner;
use crate::payload::OutcomePayload;
use crate::projection::{EffectPhase, ProjectionBackend};

/// What one tick did, in stage order.
#[derive(Debug, Default)]
pub struct TickReport {
    /// First-time command admissions in this tick.
    admitted: Vec<CommandId>,
    /// Duplicate commands excluded from execution.
    duplicates: Vec<CommandId>,
    /// Operations authorized in this tick.
    authorized: Vec<OperationId>,
    /// Authorization refusals with their deterministic reasons.
    denied: Vec<(OperationId, DenialReason)>,
    /// Owner outcomes observed in this tick.
    outcomes: Vec<(OperationId, EffectOutcome)>,
    /// Operations processed by reconciliation.
    reconciled: Vec<OperationId>,
    /// Terminal receipts published in this tick.
    receipts: Vec<TerminalReceipt>,
    /// Mutating reaches refused by the freshness fence.
    fenced: Vec<(OperationId, FenceRefusal)>,
    /// Reaches that happened but that the broker refused to count (larry#197).
    unspent: Vec<(OperationId, FenceRefusal)>,
    /// One command's, effect's or run's typed refusal. The tick recorded it
    /// and went on; only the journal's and the clock's own refusals end a tick
    /// (larry#329).
    refused: Vec<(TickItem, RuntimeError)>,
}

/// The one item a [`TickReport::refused`] entry belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum TickItem {
    /// A command being admitted or reduced.
    Command(CommandId),
    /// An effect being dispatched or reconciled.
    Effect(OperationId),
    /// A run's own lifecycle step, such as finishing a cancellation.
    Run(RunId),
}

impl TickReport {
    /// Commands admitted for the first time this tick.
    #[must_use]
    pub fn admitted(&self) -> &[CommandId] {
        &self.admitted
    }

    /// Commands recognised as idempotent replays; the reducer did not fold
    /// them twice (E28).
    #[must_use]
    pub fn duplicates(&self) -> &[CommandId] {
        &self.duplicates
    }

    /// Effects the gate authorized this tick.
    #[must_use]
    pub fn authorized(&self) -> &[OperationId] {
        &self.authorized
    }

    /// Effects the gate denied, with the deterministic first-failing reason.
    #[must_use]
    pub fn denied(&self) -> &[(OperationId, DenialReason)] {
        &self.denied
    }

    /// Outcomes observed this tick.
    #[must_use]
    pub fn outcomes(&self) -> &[(OperationId, EffectOutcome)] {
        &self.outcomes
    }

    /// Effects reconciled this tick.
    #[must_use]
    pub fn reconciled(&self) -> &[OperationId] {
        &self.reconciled
    }

    /// Terminal receipts published this tick.
    #[must_use]
    pub fn receipts(&self) -> &[TerminalReceipt] {
        &self.receipts
    }

    /// Mutating reaches the freshness fence refused this tick, with the
    /// first-failing reason (R87, E71).
    ///
    /// A fenced effect is not a failed effect. Nothing was sent, so nothing
    /// was recorded: no outcome, no receipt and no run-state change. The
    /// effect stays visibly open, which is the only truthful state for one
    /// that was reserved and never dispatched.
    #[must_use]
    pub fn fenced(&self) -> &[(OperationId, FenceRefusal)] {
        &self.fenced
    }

    /// Reaches the owner received whose offline spend the broker then refused,
    /// a broker defect (E65). Unlike [`TickReport::fenced`], the effect may
    /// have happened, and its outcome is recorded (larry#197).
    #[must_use]
    pub fn unspent(&self) -> &[(OperationId, FenceRefusal)] {
        &self.unspent
    }

    /// One command's, effect's or run's typed refusal, in the order the tick
    /// refused them.
    ///
    /// The tick recorded each one and went on. Only the journal's own refusal
    /// (a full or refused file) and a clock that cannot be placed end the
    /// tick, so one tenant's refused command never skips reconciliation or
    /// cancellation for another tenant (larry#329).
    #[must_use]
    pub fn refused(&self) -> &[(TickItem, RuntimeError)] {
        &self.refused
    }

    /// Whether the tick changed nothing.
    ///
    /// A refused reach is a non-event: it wrote no journal entry, dispatched
    /// nothing and published nothing. A tick whose only activity was fence
    /// refusals is therefore idle, and [`TickReport::fenced`] is where the
    /// refusal is read. A recorded per-item denial is not idle: it is an
    /// answer the operator must read.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.admitted.is_empty()
            && self.authorized.is_empty()
            && self.denied.is_empty()
            && self.reconciled.is_empty()
            && self.receipts.is_empty()
            && self.refused.is_empty()
    }
}

/// The Larry runtime.
#[derive(Debug)]
pub struct Runtime<P, O> {
    /// Authoritative append-only history.
    journal: Journal,
    /// Discardable deterministic fold of the history.
    projection: P,
    /// Injected boundary for external effects.
    owner: O,
    /// Injected source of tick time: the estate's declared clock, read as elapsed time since
    /// the Unix epoch (larry#74).
    clock: Clock,
    /// Current authority check for mutating reaches.
    fence: RevocationFence,
    /// Bounded queue of commands awaiting admission.
    ready: VecDeque<ReadyCommand>,
    /// Durably admitted commands not yet reduced, in journal order (E28).
    admitted: VecDeque<Admitted>,
    /// The journal's questions and their answers (E30).
    questions: approval::Book,
    /// Host-supplied snapshots and resource bounds.
    settings: RuntimeSettings,
    /// Budget not yet reserved by recorded intents, one counter per tenant
    /// (larry#208).
    ///
    /// A tenant enters when one of its runs is tracked, so the map holds no
    /// more tenants than the run table's capacity, and one tenant's spend
    /// never refuses another tenant's effect. `restore` seeds a counter for
    /// every tenant a recorded intent spent from.
    budgets: BTreeMap<TenantId, u64>,
    /// Tenant ownership of tracked runs.
    runs: BTreeMap<RunId, RunLive>,
    /// Bounded effect witnesses in operation identity order.
    live: BTreeMap<OperationId, LiveEffect>,
    /// Monotonic logical tick number.
    tick_count: u64,
}

impl<P, O> Runtime<P, O>
where
    P: ProjectionBackend + Default,
    O: EffectOwner,
{
    /// A runtime with an empty journal and empty projection.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::BadSetting`] when any bound is zero or out of range
    /// (larry#212), and [`RuntimeError::JournalFull`] when the journal entry
    /// ceiling is zero.
    pub fn new(settings: RuntimeSettings, owner: O, clock: Clock) -> Result<Self, RuntimeError> {
        let settings = settings.pinned();
        settings.validate()?;
        // The projection is built at the configured capacity, never its own
        // default: the run table refuses past `run_capacity`, so a projection
        // bounded below it would refuse a run the runtime already tracks
        // (larry#212).
        let projection = P::with_capacity(settings.run_capacity);
        let journal = Journal::new(settings.journal_capacity)?;
        let fence = RevocationFence::new(settings.security_epoch, settings.offline_dispatch_bound)?;
        Ok(Self {
            journal,
            projection,
            owner,
            clock,
            fence,
            ready: VecDeque::new(),
            admitted: VecDeque::new(),
            questions: approval::Book::default(),
            settings,
            budgets: BTreeMap::new(),
            runs: BTreeMap::new(),
            live: BTreeMap::new(),
            tick_count: 0,
        })
    }

    /// Queues a command for admission. Queuing is not acceptance: the command
    /// enters the journal only when a tick admits it.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::ReadyWorkFull`] when the ready queue is at its
    /// bound. The caller must drain a tick; the runtime never silently drops
    /// a submission (Cell 05). A question or an answer is refused
    /// [`RuntimeError::UnsupportedCommand`]: it enters only through
    /// [`Self::submit_protocol`], from a client that can serve a human (E30).
    pub fn submit(&mut self, command: Command) -> Result<(), RuntimeError> {
        let kind = command.content().kind();
        if matches!(kind, CommandKind::Ask | CommandKind::Answer) {
            return Err(RuntimeError::UnsupportedCommand(kind));
        }
        self.enqueue(ReadyCommand {
            command,
            display: None,
        })
    }

    /// Enqueues one descriptor under the shared bound.
    fn enqueue(&mut self, command: ReadyCommand) -> Result<(), RuntimeError> {
        if self.ready.len() >= MAX_READY_WORK {
            return Err(RuntimeError::ReadyWorkFull {
                capacity: MAX_READY_WORK,
            });
        }
        self.ready.push_back(command);
        Ok(())
    }

    /// The authoritative journal.
    #[must_use]
    pub const fn journal(&self) -> &Journal {
        &self.journal
    }

    /// The current projection.
    #[must_use]
    pub const fn projection(&self) -> &P {
        &self.projection
    }

    /// The owner boundary, for assertions in tests.
    #[must_use]
    pub const fn owner(&self) -> &O {
        &self.owner
    }

    /// The live dispatch fence (R87, D48).
    #[must_use]
    pub const fn fence(&self) -> &RevocationFence {
        &self.fence
    }

    /// The configuration this runtime was built with, for the host that built
    /// it: its lease, its protocol range and every bound it enforces.
    #[must_use]
    pub const fn settings(&self) -> &RuntimeSettings {
        &self.settings
    }

    /// The fence, mutably, for the authority layer that owns it.
    ///
    /// This is the host's control surface — withdraw an operation, advance
    /// the epoch, re-authenticate against the authority source. The runtime
    /// only ever *reads* the fence: withdrawal and freshness are not
    /// decisions it is entitled to make about itself, and the two mutating
    /// operations the runtime performs on a reach are crate-internal.
    pub const fn fence_mut(&mut self) -> &mut RevocationFence {
        &mut self.fence
    }

    /// Consumes the runtime and returns its journal.
    ///
    /// The journal is the whole state, so holding it is holding the runtime.
    /// A journal from [`Journal::open`] keeps its file and anything staged
    /// since the last tick; [`Journal::sync`] puts that in the file
    /// (larry#75). A memory-only journal moves between two in-memory values
    /// and claims nothing about surviving a process.
    #[must_use]
    pub fn into_journal(self) -> Journal {
        self.journal
    }

    /// How many ticks have run.
    #[must_use]
    pub const fn tick_count(&self) -> u64 {
        self.tick_count
    }

    /// The budget `tenant` still has unreserved (larry#208).
    ///
    /// A tenant no run has been tracked for has reserved nothing, so it reads
    /// the whole initial budget; no counter is invented for it.
    #[must_use]
    pub fn remaining_budget(&self, tenant: TenantId) -> u64 {
        match self.budgets.get(&tenant) {
            Some(&budget) => budget,
            None => self.settings.initial_budget,
        }
    }

    /// Runs one bounded, deterministic tick.
    ///
    /// Each command's, effect's or run's typed refusal is recorded in the
    /// report and the tick goes on; only a journal refusal or a clock that
    /// cannot be placed ends it (larry#329). Then the journal's sync refusal is
    /// returned (larry#75).
    pub fn tick(&mut self) -> Result<TickReport, RuntimeError> {
        // The tick counter advances before any stage runs, so the stages agree
        // on which tick they are in and the reconciliation gate below can
        // compare against it.
        self.tick_count = self.tick_count.saturating_add(1);
        let staged = self.stages();
        // Whatever the stages recorded is a fact, refused stage or not, so it
        // reaches the file before the tick returns: one sync per tick plus one
        // per owner reach, never one per entry (group commit, larry#75).
        let synced = self.journal.sync();
        let report = staged?;
        synced?;
        Ok(report)
    }

    /// The tick's three stages, in order.
    ///
    /// Each stage records one item's refusal in `report` and goes on with the
    /// next item; a journal or clock refusal ends the tick here (larry#329).
    fn stages(&mut self) -> Result<TickReport, RuntimeError> {
        let at = sample(&self.clock)?;
        let mut report = TickReport::default();
        self.pump_ready(at, &mut report)?;
        self.pump_reconciliation(at, &mut report)?;
        self.pump_cancellation(&mut report)?;
        Ok(report)
    }

    /// Records one item's refusal in the report, or ends the tick when the
    /// refusal is the journal's or the clock's own (larry#329).
    pub(super) fn refuse(
        &self,
        report: &mut TickReport,
        item: TickItem,
        error: RuntimeError,
    ) -> Result<(), RuntimeError> {
        if error.ends_the_tick() {
            return Err(error);
        }
        report.refused.push((item, error));
        Ok(())
    }

    // ── stage 2: admit, gate, reserve, dispatch, observe ────────────────────

    // `pump_ready`, `admit_ready` and `admit_work` live in `admission`, the
    // module that owns command admission, and record one command's refusal in
    // the report without ending the tick (larry#329).

    /// Records an observed outcome and, when it may terminate the effect,
    /// publishes the disposition.
    ///
    /// `sent` is the caller's answer to "may this effect have reached its
    /// owner" — the witness the journal cannot carry. An outcome observed for
    /// an effect that was never sent is recorded, because the owner did report
    /// it, but it resolves nothing: an endpoint's answer is evidence about an
    /// effect this runtime dispatched, and no answer about an effect whose
    /// only reach was refused can be evidence that the effect happened. The
    /// effect therefore stays visibly open rather than being handed a
    /// disposition it never earned (D42, R88).
    fn settle(
        &mut self,
        run: RunId,
        intent: EffectIntent,
        outcome: EffectOutcome,
        sent: bool,
        report: &mut TickReport,
    ) -> Result<(), RuntimeError> {
        // The dispatch count the reach leaves in the live table is recorded with
        // the outcome, so the budget a restart spends is a fold over the journal
        // rather than live state (larry#182).
        let dispatches = match self.live.get(&intent.operation) {
            Some(live) => live.dispatches,
            None => 0,
        };
        self.record_outcome(run, intent, outcome, dispatches)?;
        if !sent {
            return Ok(());
        }
        let Ok(disposition) = crate::projection::resolve(&intent, outcome) else {
            // Left open on purpose: the strategy requires a status query or a
            // bounded retry, and the effect stays in `Reconciling` (R72, E56).
            return Ok(());
        };
        // A cancelling run still records what its in-flight effects did — the
        // evidence is retained — but it may not walk on to `Completed`.
        // Cancellation drains owned work; it does not become a success.
        let cancelling = self.projection.run_state(run) == Some(RunState::Cancelling);
        match disposition {
            TerminalDisposition::Succeeded | TerminalDisposition::Compensated => {
                if cancelling {
                    self.publish_receipt(run, intent, disposition, report)?;
                    return Ok(());
                }
                self.advance(run, TransitionEvent::VerificationPassed)?;
                self.advance(run, TransitionEvent::EnterFinalizing)?;
                self.publish_receipt(run, intent, disposition, report)?;
                self.advance(run, TransitionEvent::Complete)?;
            }
            TerminalDisposition::Failed => {
                self.publish_receipt(run, intent, disposition, report)?;
                self.advance(run, TransitionEvent::Fail)?;
            }
            TerminalDisposition::UnknownPermanent => {
                self.publish_receipt(run, intent, disposition, report)?;
                self.advance(run, TransitionEvent::ObserveUnknownOutcome)?;
            }
            other => return Err(RuntimeError::UnsupportedDisposition(other)),
        }
        Ok(())
    }
}

/// Rebuilds a projection purely from a journal (D04).
///
/// This is the whole recovery story in this slice: state is a fold over
/// recorded facts, so restoring it is `rebuild` and nothing else. No live
/// runtime state is consulted.
///
/// # Errors
///
/// Returns [`RuntimeError::Codec`] for an undecodable payload and
/// [`RuntimeError::FoldDivergence`] when a recorded transition disagrees with
/// the pure algebra.
pub fn replay<P: ProjectionBackend>(
    projection: &mut P,
    journal: &Journal,
) -> Result<(), RuntimeError> {
    projection.rebuild(journal)
}

/// How many `Unknown` outcomes the journal records for each operation.
///
/// An `Unknown` record is one reconciliation pass already spent, so `restore`
/// reads the reconciliation budget from these rather than resetting it: a
/// strategy that can neither query nor retry would otherwise append another
/// whole budget of them after every restart (larry#332).
///
/// # Errors
///
/// [`RuntimeError::Codec`] for an outcome record that does not decode.
fn recorded_unknowns(journal: &Journal) -> Result<BTreeMap<OperationId, u8>, RuntimeError> {
    let mut unknowns: BTreeMap<OperationId, u8> = BTreeMap::new();
    for entry in journal.entries() {
        if let JournalEntry::Event(ref event) = *entry
            && event.kind() == EventKind::EffectOutcomeRecorded
        {
            let payload: OutcomePayload = lgwks_std::json::from_slice(event.payload())?;
            if matches!(payload.outcome, EffectOutcome::Unknown { .. }) {
                let spent = unknowns.entry(payload.outcome.operation()).or_insert(0_u8);
                *spent = spent.saturating_add(1);
            }
        }
    }
    Ok(unknowns)
}

/// Restores a runtime from a recorded journal, consulting no live state.
///
/// The projection is rebuilt by folding, and the live effect table is
/// reconstructed from the recorded intents. Every effect the journal leaves in
/// [`EffectPhase::Reserved`] is restored with its bounded retry budget already
/// spent: the journal proves the effect was reserved, and cannot prove it was
/// never dispatched, so a crash consumes the budget rather than resetting it.
/// That direction is deliberate — the alternative fails open, and a
/// reconciliation strategy that is *not* retry-based (status query, human, or
/// compensation) is unaffected either way.
///
/// Each restored effect keeps the admission epoch its intent recorded, and the
/// fence starts from `settings.security_epoch`, which the host supplies. That
/// pairing is the recovery contract D48 asks to be persisted before dispatch:
/// a host that has since moved its authority to a later generation restores
/// with `security_epoch` set to it, and every effect admitted below that
/// generation comes back already fenced rather than being re-dispatched on the
/// strength of a decision that no longer stands (E71).
///
/// # Errors
///
/// Returns [`RuntimeError::Codec`] or [`RuntimeError::FoldDivergence`] from
/// the rebuild, and [`RuntimeError::RunNotTracked`] for a run the projection
/// tracks but recorded no tenant for.
pub fn restore<P, O>(
    settings: RuntimeSettings,
    owner: O,
    clock: Clock,
    journal: Journal,
) -> Result<Runtime<P, O>, RuntimeError>
where
    P: ProjectionBackend + Default,
    O: EffectOwner,
{
    // The recorded history is kept as it is; only the ceiling moves, so a
    // journal that filled up mid-effect can still record the reconciliation it
    // exists to perform.
    let settings = settings.pinned();
    settings.validate()?;
    let journal = journal.reopen(settings.journal_capacity)?;

    // The projection is rebuilt at the configured capacity, not `P::default()`:
    // a host set above 1,024 runs writes a journal its own default projection
    // could not reopen (larry#212).
    let mut projection = P::with_capacity(settings.run_capacity);
    projection.rebuild(&journal)?;
    // Admitted commands the journal holds no acceptance for are pending again,
    // in journal order (E28): the record says so, never the dead process.
    let pending = admission::recover(&journal)?;
    // Questions and answers are commands, so the book is a fold over the
    // journal in order, never the dead process's memory (E30).
    let questions = approval::Book::rebuild(&journal);
    // The fence is live state the journal does not hold, so a runtime resuming
    // recorded history counts its whole offline allowance spent until the host
    // reaches the authority source (E71): neither a restart nor settings older
    // than the source can recharge or revive authority. An empty journal is
    // a first start, with nothing to resume and nothing spent.
    let (epoch, bound) = (settings.security_epoch, settings.offline_dispatch_bound);
    let fence = if journal.is_empty() {
        RevocationFence::new(epoch, bound)?
    } else {
        RevocationFence::restored(epoch, bound)?
    };

    let mut runs: BTreeMap<RunId, RunLive> = BTreeMap::new();
    let mut live: BTreeMap<OperationId, LiveEffect> = BTreeMap::new();
    // One counter per tenant (larry#208): every recorded intent's cost is spent
    // from the tenant its own event named, so a restart keeps each tenant's own
    // remainder instead of one pooled balance. A tenant the journal never let
    // reserve is absent, and reads its whole initial budget.
    let budgets: BTreeMap<TenantId, u64> = projection
        .reserved_by_tenant()
        .into_iter()
        .map(|(tenant, spent)| (tenant, settings.initial_budget.saturating_sub(spent)))
        .collect();
    let leases = retry::leases(projection.authorizations());
    // The reconciliation budget is not journaled either (larry#332): every
    // `Unknown` record is one pass a strategy that can neither query nor retry
    // has already spent, so the fold counts them and a restart cannot buy
    // another budget of `Unknown` records.
    let unknowns = recorded_unknowns(&journal)?;
    // `cap::Auth` never serializes, so each recorded authorization is re-issued
    // a proof for only the lease left on the clock now (larry#77); an intent
    // with no recorded authorization holds none.
    let proofs: BTreeMap<OperationId, crate::gate::Proof> = projection
        .authorizations()
        .iter()
        .map(|record| {
            let proof = crate::gate::Proof::issue(&record.pinned, &clock);
            (record.pinned.request.operation(), proof)
        })
        .collect();
    for intent in projection.intents() {
        let (admitted_at, valid_until) = retry::leased(&leases, intent.operation);
        let proof = match proofs.get(&intent.operation) {
            Some(proof) => proof.clone(),
            None => crate::gate::Proof::lapsed(),
        };
        let phase = projection.effect_phase(intent.operation);
        // The dispatch budget is a fold over the journal (larry#182): a
        // recorded outcome carries the attempt number it was observed at, so a
        // restart continues the count instead of recharging it. An effect the
        // journal leaves `Reserved` has no outcome record, and the journal
        // cannot prove it was never dispatched, so it restores with the whole
        // budget spent; one the journal never mentions at all also restores
        // with the whole budget spent, because nothing proves a lower count.
        let dispatches = match phase {
            Some(EffectPhase::Observed { outcome }) => {
                let recorded = projection.dispatches(intent.operation);
                // A retry may have reached the owner after the journal's last
                // outcome and before the crash that cut the process off. The
                // record cannot prove it did not, so an unresolved retry-safe
                // effect restores one attempt further along (larry#332).
                if intent.strategy == ReconciliationStrategy::IdempotentRetrySafe
                    && crate::projection::resolve(intent, outcome).is_err()
                {
                    recorded.saturating_add(1).min(settings.max_effect_attempts)
                } else {
                    recorded
                }
            }
            Some(EffectPhase::Reconciled { .. }) => projection.dispatches(intent.operation),
            Some(EffectPhase::Reserved) | None => settings.max_effect_attempts,
        };
        let reconciliations = match unknowns.get(&intent.operation) {
            Some(&spent) => spent,
            None => 0,
        };
        live.insert(
            intent.operation,
            LiveEffect {
                intent: *intent,
                proof,
                // The journal proves the effect was reserved and cannot prove
                // it was never sent, so every restored effect counts as
                // possibly-sent — including one that merely read as `Reserved`
                // because a crash landed inside the dispatch window. The
                // alternative fails open, and the direction here is the same
                // one the dispatch budget below already takes.
                sent: true,
                // The journal keeps no key; the retry rule reads the one this
                // intent names at its recorded attempt (larry#197).
                dispatched: None,
                dispatches,
                reconciliations,
                // Zero, so a restored effect is reconcilable on the first tick
                // rather than waiting a tick for a dispatch that already
                // happened before the crash.
                dispatched_at_tick: 0,
                admitted_at,
                valid_until,
            },
        );
    }
    for run in projection.run_ids() {
        // The tenant comes from the run's own recorded events, never from a
        // reserved intent. An intent's `tenant` is *declared* by the proposal
        // that reserved it, and inferring a run's tenant from it would both
        // re-attribute the run to a declaration and fail outright for a run
        // whose recorded history reserved nothing — a denied submission, a
        // steering command, or a cancellation that arrived before any work.
        // Both are wrong for the same reason: recovery is a re-fold of what
        // the runtime recorded (D04), not a reconstruction from what a
        // submitter claimed (D42).
        let tenant = projection
            .run_tenant(run)
            .ok_or(RuntimeError::RunNotTracked { run })?;
        runs.insert(run, RunLive { tenant });
    }

    Ok(Runtime {
        journal,
        projection,
        owner,
        clock,
        fence,
        ready: VecDeque::new(),
        admitted: pending,
        questions,
        budgets,
        settings,
        runs,
        live,
        tick_count: 0,
    })
}
