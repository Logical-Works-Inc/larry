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

mod display;
mod instant;
mod live;
mod reconcile;
mod retry;
mod writers;

use instant::sample;
use live::{LiveEffect, OwnerReach, RunLive};

pub use display::DisplayedEffect;
use display::ReadyCommand;

use std::collections::{BTreeMap, VecDeque};
use std::time::SystemTime;

use lgwks_bot::clock::Clock;

use larry_contracts::authority::{BudgetUnit, SecurityEpoch};
use larry_contracts::command::{Command, CommandAcceptance, CommandKind};
use larry_contracts::effect::{EffectIntent, EffectOutcome, TerminalDisposition, TerminalReceipt};
use larry_contracts::ids::{CommandId, OperationId, RunId, TenantId};
use larry_contracts::replay::{CapabilitySnapshot, DenialReason, PolicySnapshot, ValiditySnapshot};
use larry_contracts::state::{RunState, TransitionEvent};

use crate::error::RuntimeError;
use crate::fence::{FenceRefusal, RevocationFence};
use crate::journal::Journal;
use crate::owner::EffectOwner;
use crate::payload::WorkProposal;
use crate::projection::{EffectPhase, ProjectionBackend};
use crate::{MAX_EFFECTS_PER_RUN, MAX_READY_WORK, MAX_RUNS_PER_TICK, MAX_TRACKED_RUNS};

/// The runtime's configuration: the pinned snapshots and the bounds.
///
/// The policy, capability and validity snapshots are host-supplied and pinned
/// for the runtime's lifetime in this slice. A later work package replaces
/// them with live, versioned snapshots activated at safe boundaries (D26);
/// the runtime already threads them as inputs rather than reading them from
/// ambient state, so that change is a wiring change and not a rewrite.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RuntimeSettings {
    /// The pinned policy snapshot.
    pub policy: PolicySnapshot,
    /// The pinned capability snapshot.
    pub capabilities: CapabilitySnapshot,
    /// The pinned authority-validity snapshot.
    pub validity: ValiditySnapshot,
    /// The unit the run budget is denominated in.
    pub unit: BudgetUnit,
    /// The initially reserved budget.
    pub initial_budget: u64,
    /// The security generation the fence starts at (R87, D48). Host-supplied
    /// like the snapshots above, and for the same reason: it is a fact about
    /// authority, not about this process.
    pub security_epoch: SecurityEpoch,
    /// How many mutating reaches the fence permits before the authority
    /// source must be re-contacted.
    pub offline_dispatch_bound: u32,
    /// The journal entry ceiling.
    pub journal_capacity: usize,
    /// The run-table ceiling.
    pub run_capacity: usize,
    /// How many ready commands one tick may admit.
    pub max_runs_per_tick: usize,
}

impl RuntimeSettings {
    /// Settings with the specification's Cell 05 default bounds.
    #[must_use]
    pub fn with_defaults(
        policy: PolicySnapshot,
        capabilities: CapabilitySnapshot,
        validity: ValiditySnapshot,
        unit: BudgetUnit,
        initial_budget: u64,
    ) -> Self {
        Self {
            policy,
            capabilities,
            validity,
            unit,
            initial_budget,
            security_epoch: SecurityEpoch::INITIAL,
            offline_dispatch_bound: crate::MAX_OFFLINE_DISPATCHES,
            journal_capacity: crate::MAX_JOURNAL_ENTRIES,
            run_capacity: MAX_TRACKED_RUNS,
            max_runs_per_tick: MAX_RUNS_PER_TICK,
        }
    }
}

/// What one tick did, in stage order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

    /// Whether the tick changed nothing.
    ///
    /// A refused reach is a non-event: it wrote no journal entry, dispatched
    /// nothing and published nothing. A tick whose only activity was fence
    /// refusals is therefore idle, and [`TickReport::fenced`] is where the
    /// refusal is read.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.admitted.is_empty()
            && self.authorized.is_empty()
            && self.denied.is_empty()
            && self.reconciled.is_empty()
            && self.receipts.is_empty()
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
    /// Host-supplied snapshots and resource bounds.
    settings: RuntimeSettings,
    /// Budget not yet reserved by recorded intents.
    remaining_budget: u64,
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
    /// Returns [`RuntimeError::RunLimitReached`] when the run capacity is
    /// zero, which would make every run untrackable.
    pub fn new(settings: RuntimeSettings, owner: O, clock: Clock) -> Result<Self, RuntimeError> {
        let projection = P::default();
        let journal = Journal::new(settings.journal_capacity)?;
        if settings.run_capacity == 0 {
            return Err(RuntimeError::RunLimitReached { capacity: 0 });
        }
        let remaining_budget = settings.initial_budget;
        let fence = RevocationFence::new(settings.security_epoch, settings.offline_dispatch_bound);
        Ok(Self {
            journal,
            projection,
            owner,
            clock,
            fence,
            ready: VecDeque::new(),
            settings,
            remaining_budget,
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
    /// a submission (Cell 05).
    pub fn submit(&mut self, command: Command) -> Result<(), RuntimeError> {
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
    /// This is the hand-off point to a durable store: the journal is the whole
    /// state, so persisting it is persisting the runtime. Durable persistence
    /// is Logical-DB's and the memory hold is intact — this method moves bytes
    /// between two in-memory values and claims nothing about surviving a
    /// process.
    #[must_use]
    pub fn into_journal(self) -> Journal {
        self.journal
    }

    /// How many ticks have run.
    #[must_use]
    pub const fn tick_count(&self) -> u64 {
        self.tick_count
    }

    /// The budget still unreserved.
    #[must_use]
    pub const fn remaining_budget(&self) -> u64 {
        self.remaining_budget
    }

    /// Runs one bounded, deterministic tick.
    ///
    /// # Errors
    ///
    /// Propagates every typed refusal from the stages below.
    pub fn tick(&mut self) -> Result<TickReport, RuntimeError> {
        // The tick counter advances before any stage runs, so the stages agree
        // on which tick they are in and the reconciliation gate below can
        // compare against it.
        self.tick_count = self.tick_count.saturating_add(1);
        let at = sample(&self.clock)?;
        let mut report = TickReport::default();
        self.pump_ready(at, &mut report)?;
        self.pump_reconciliation(at, &mut report)?;
        self.pump_cancellation()?;
        Ok(report)
    }

    // ── stage 2: admit, gate, reserve, dispatch, observe ────────────────────

    /// Admits queued commands in deterministic order within the tick bound.
    fn pump_ready(&mut self, at: SystemTime, report: &mut TickReport) -> Result<(), RuntimeError> {
        let budget = self.settings.max_runs_per_tick;
        for _ in 0..budget {
            let Some(queued) = self.ready.pop_front() else {
                return Ok(());
            };
            let ReadyCommand { command, display } = queued;
            let id = command.id();
            let tenant = command.tenant();
            let kind = command.content().kind();
            let payload = command.content().payload().to_vec();
            let proposal: WorkProposal = lgwks_std::json::from_slice(&payload)?;
            self.ensure_run_tenant(proposal.run, tenant)?;

            if kind == CommandKind::Submit && self.journal.command(id).is_none() {
                self.validate_display(&command, &proposal, display.as_ref(), at)?;
            }
            let acceptance = self.journal.accept_command(command)?;
            if acceptance == CommandAcceptance::DuplicateReplay {
                report.duplicates.push(id);
                continue;
            }
            report.admitted.push(id);
            self.record_command_accepted(id, tenant, kind, proposal.run)?;

            match kind {
                CommandKind::Submit => {
                    self.admit_work(tenant, &proposal, display.as_ref(), at, report)?
                }
                CommandKind::Cancel => self.request_cancellation(tenant, proposal.run)?,
                CommandKind::Steer => {
                    // Steering is recorded at a safe boundary; the command's
                    // acceptance is the durable fact this slice owns.
                }
                other => return Err(RuntimeError::UnsupportedCommand(other)),
            }
        }
        Ok(())
    }

    /// Gates and durably reserves a proposal before reaching its owner.
    fn admit_work(
        &mut self,
        tenant: TenantId,
        proposal: &WorkProposal,
        display: Option<&DisplayedEffect>,
        at: SystemTime,
        report: &mut TickReport,
    ) -> Result<(), RuntimeError> {
        let run = proposal.run;
        // Admissibility is decided from the runtime's own run table, never
        // inferred from the projection: recording the command already created
        // a run view, so a projection-based check would read `Draft` and skip
        // the admission step.
        let is_new = !self.runs.contains_key(&run);
        self.track_run(run, tenant)?;

        if is_new {
            self.advance(run, TransitionEvent::Admit)?;
        }
        self.advance(run, TransitionEvent::Start)?;
        self.advance(run, TransitionEvent::EnterReconciling)?;

        if self.projection.effect_count(run) >= MAX_EFFECTS_PER_RUN {
            return Err(RuntimeError::EffectLimitReached {
                run,
                capacity: MAX_EFFECTS_PER_RUN,
            });
        }
        if self.live.contains_key(&proposal.operation) {
            return Err(RuntimeError::DuplicateOperation {
                operation: proposal.operation,
            });
        }

        let mut pinned = self.pinned_inputs(proposal, at);
        if let Some(shown) = display {
            shown.constrain(&mut pinned);
        }
        let admitted_at = pinned.request.observed_at;
        let valid_until = pinned.validity.valid_until;
        let authorization = match crate::gate::authorize(&pinned) {
            Ok(record) => record,
            Err(RuntimeError::Denied { operation, reason }) => {
                report.denied.push((operation, reason));
                self.advance(run, TransitionEvent::Fail)?;
                return Ok(());
            }
            Err(other) => return Err(other),
        };

        // Reservation: the intent is durable *before* the effect can leave,
        // and the budget is deducted against the same pinned snapshot that was
        // just recorded, so replay re-derives the identical decision.
        //
        // The admission epoch comes from the fence and the tenant from the run
        // table, never from the proposal: a command cannot declare the security
        // generation it is admitted under or the tenant it is admitted for, so
        // a hostile generator cannot name a generation that outlives a
        // revocation, or record an effect — or re-attribute a run after a
        // restart — to a tenant it does not own (D42, E63).
        let tenant = self.tenant_of(run)?;
        let intent = proposal.intent(tenant, self.fence.epoch());
        let intent = match display {
            Some(shown) => intent.with_binding(*shown.binding()),
            None => intent,
        };
        self.record_intent(run, intent, authorization)?;
        self.remaining_budget = self.remaining_budget.saturating_sub(proposal.cost);
        report.authorized.push(intent.operation);
        self.live.insert(
            intent.operation,
            LiveEffect {
                intent,
                // Nothing has reached the owner yet: the reservation is
                // durable, and the reach that follows is what sets this.
                sent: false,
                dispatches: 0,
                reconciliations: 0,
                dispatched_at_tick: self.tick_count,
                admitted_at,
                valid_until,
            },
        );

        self.dispatch(run, intent, at, report)
    }

    /// Checks durability and freshness before the first mutating reach.
    fn dispatch(
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
            // Fenced. The ordering is the point: the reservation above is
            // already durable and the owner below was never called, so the
            // effect reads as reserved — reserved, and genuinely never sent.
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
    /// retry — so the fence has exactly one enforcement point and exactly one
    /// place to audit. `Mutate` is the only reach that can change anything
    /// outside this process, and it is the only one the fence gates.
    /// `Observe` must stay open: a status query is how an effect that was
    /// already dispatched when revocation landed gets resolved, and fencing
    /// it would leave an external effect permanently unlabelled — the
    /// "already-dispatched treatment" D48 requires, enforced by shape rather
    /// than by convention.
    ///
    /// Returns `None` when the fence refused, in which case no owner call
    /// happened and no outcome exists to record.
    fn reach_owner(
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
                self.validate_bound_reach(&intent, at)?;
                let previous = self.live.get(&intent.operation).copied();
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
        self.record_outcome(run, intent, outcome)?;
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
    let journal = journal.reopen(settings.journal_capacity)?;

    let mut projection = P::default();
    projection.rebuild(&journal)?;

    let mut runs: BTreeMap<RunId, RunLive> = BTreeMap::new();
    let mut live: BTreeMap<OperationId, LiveEffect> = BTreeMap::new();
    let mut reserved_budget: u64 = 0;

    for record in projection.authorizations() {
        reserved_budget = reserved_budget.saturating_add(record.pinned.request.cost);
    }
    let leases = retry::leases(projection.authorizations());
    for intent in projection.intents() {
        let (admitted_at, valid_until) = retry::leased(&leases, intent.operation);
        let phase = projection.effect_phase(intent.operation);
        // A reservation with no recorded outcome may already have reached the
        // endpoint, and the journal cannot prove otherwise, so restoration
        // spends the *dispatch* budget rather than resetting it. Reconciliation
        // budget is untouched: asking what happened is not retrying.
        let dispatches = match phase {
            Some(EffectPhase::Reserved) => larry_contracts::effect::MAX_EFFECT_ATTEMPTS,
            Some(EffectPhase::Observed { .. } | EffectPhase::Reconciled { .. }) | None => 0,
        };
        live.insert(
            intent.operation,
            LiveEffect {
                intent: *intent,
                // The journal proves the effect was reserved and cannot prove
                // it was never sent, so every restored effect counts as
                // possibly-sent — including one that merely read as `Reserved`
                // because a crash landed inside the dispatch window. The
                // alternative fails open, and the direction here is the same
                // one the dispatch budget below already takes.
                sent: true,
                dispatches,
                reconciliations: 0,
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
        fence: RevocationFence::new(settings.security_epoch, settings.offline_dispatch_bound),
        ready: VecDeque::new(),
        remaining_budget: settings.initial_budget.saturating_sub(reserved_budget),
        settings,
        runs,
        live,
        tick_count: 0,
    })
}
