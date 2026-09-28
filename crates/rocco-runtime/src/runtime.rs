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

use std::collections::{BTreeMap, VecDeque};
use std::time::SystemTime;

use rocco_contracts::authority::{BudgetUnit, SecurityEpoch};
use rocco_contracts::command::{Command, CommandAcceptance, CommandKind};
use rocco_contracts::effect::{EffectIntent, EffectOutcome, TerminalDisposition, TerminalReceipt};
use rocco_contracts::event::{Event, EventKind};
use rocco_contracts::ids::{CommandId, OperationId, RunId, TenantId};
use rocco_contracts::replay::{
    BudgetSnapshot, CapabilitySnapshot, DenialReason, EffectRequest, PinnedInputs, PolicySnapshot,
    ValiditySnapshot,
};
use rocco_contracts::state::{RunState, TransitionEvent};

use crate::authority::AuthoritySource;
use crate::error::RuntimeError;
use crate::fence::{FenceRefusal, RevocationFence};
use crate::journal::{Journal, JournalEntry};
use crate::owner::EffectOwner;
use crate::payload::{
    CommandAcceptedPayload, IntentPayload, OutcomePayload, ReceiptPayload, TransitionPayload,
    WorkProposal,
};
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
    /// Legacy fixture-provisioning hint. Runtime never trusts this value: the
    /// explicit authority source supplies the reservation and dispatch epochs.
    pub security_epoch: SecurityEpoch,
    /// How many mutating reaches the fence permits before the authority
    /// source must be re-contacted. This may restrict, never enlarge, the
    /// authority source's own ceiling.
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

/// A run the runtime is tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RunLive {
    /// Tenant established by command admission.
    tenant: TenantId,
}

/// An effect the runtime has reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LiveEffect {
    /// Durable reservation associated with this live witness.
    intent: EffectIntent,
    /// Index of this reservation's recorded authorization; rebuilt on restore.
    authorization_index: usize,
    /// Whether this effect may have reached its owner. Set when a mutating
    /// reach is actually taken, and set by [`restore`] for every recorded
    /// reservation, because the journal proves the effect was reserved and
    /// cannot prove it was never sent.
    ///
    /// This is the witness the journal does not carry. `EventKind` has no
    /// "dispatched" fact, so a journal alone cannot tell a refused reach from
    /// a crash inside the dispatch window; the live table can, because a
    /// refused reach calls no owner at all and this flag is set only on a call
    /// that happened. It is what keeps an endpoint's answer from being read as
    /// evidence about an effect that never left the process.
    sent: bool,
    /// How many times the owner has been asked to *perform* the effect. Bounded
    /// by `MAX_EFFECT_ATTEMPTS` for strategies that declare retry safe.
    dispatches: u8,
    /// How many times the owner has been asked *what happened*. Bounded
    /// separately, because reconciliation is not retry: a crash must not stop
    /// a queryable effect from ever being queried.
    reconciliations: u8,
    /// The tick the effect was dispatched in. Reconciliation may not run in
    /// that same tick: an effect is only `Unknown` because time passed, so
    /// querying an endpoint in the instant it was called would be a fabricated
    /// timeout rather than an observed one.
    dispatched_at_tick: u64,
}

/// Which way the runtime is about to reach the owner.
///
/// The distinction is the whole of the fence's scope, so it is a type rather
/// than a boolean argument at the call site: `Mutate` can change the world
/// outside this process and is fenced; `Observe` cannot and is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnerReach {
    /// Cause, or re-cause, an external change.
    Mutate,
    /// Ask what already happened.
    Observe,
}

/// The Rocco runtime.
#[derive(Debug)]
pub struct Runtime<P, O, C, A = crate::authority::fixture::FixtureAuthority> {
    /// Authoritative append-only history.
    journal: Journal,
    /// Discardable deterministic fold of the history.
    projection: P,
    /// Injected boundary for external effects.
    owner: O,
    /// Injected source of tick time.
    clock: C,
    /// Current authority check for mutating reaches.
    authority: A,
    /// Bounded queue of commands awaiting admission.
    ready: VecDeque<Command>,
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

impl<P, O, C, A> Runtime<P, O, C, A>
where
    P: ProjectionBackend + Default,
    O: EffectOwner,
    C: crate::clock::Clock,
    A: AuthoritySource,
{
    /// A runtime with an empty journal and empty projection.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::RunLimitReached`] when the run capacity is
    /// zero, which would make every run untrackable.
    pub fn new(
        settings: RuntimeSettings,
        owner: O,
        clock: C,
        mut authority: A,
    ) -> Result<Self, RuntimeError> {
        let projection = P::default();
        let journal = Journal::new(settings.journal_capacity)?;
        if settings.run_capacity == 0 {
            return Err(RuntimeError::RunLimitReached { capacity: 0 });
        }
        let remaining_budget = settings.initial_budget;
        authority.begin_session(settings.offline_dispatch_bound);
        Ok(Self {
            journal,
            projection,
            owner,
            clock,
            authority,
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

    /// The source-owned dispatch fence; callers cannot mutate runtime authority.
    #[must_use]
    pub fn fence(&self) -> &RevocationFence {
        self.authority.fence()
    }

    /// The injected authority owner. Only fixture owners expose simulation controls.
    pub const fn authority_mut(&mut self) -> &mut A {
        &mut self.authority
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
        let at = self.clock.now();
        let mut report = TickReport::default();
        self.pump_ready(at, &mut report)?;
        self.pump_reconciliation(at, &mut report)?;
        self.pump_cancellation(&mut report)?;
        Ok(report)
    }

    // ── stage 2: admit, gate, reserve, dispatch, observe ────────────────────

    /// Admits queued commands in deterministic order within the tick bound.
    fn pump_ready(&mut self, at: SystemTime, report: &mut TickReport) -> Result<(), RuntimeError> {
        let budget = self.settings.max_runs_per_tick;
        for _ in 0..budget {
            let Some(command) = self.ready.pop_front() else {
                return Ok(());
            };
            let id = command.id();
            let tenant = command.tenant();
            let kind = command.content().kind();
            let payload = command.content().payload().to_vec();
            let proposal: WorkProposal = lgwks_std::json::from_slice(&payload)?;
            self.ensure_run_tenant(proposal.run, tenant)?;

            let acceptance = self.journal.accept_command(command)?;
            if acceptance == CommandAcceptance::DuplicateReplay {
                report.duplicates.push(id);
                continue;
            }
            report.admitted.push(id);
            self.record_command_accepted(id, tenant, kind, proposal.run)?;

            match kind {
                CommandKind::Submit => self.admit_work(tenant, &proposal, at, report)?,
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

        let pinned = self.pinned_inputs(proposal, at);
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
        let intent = proposal.intent(tenant, self.authority.fence().epoch());
        self.record_intent(run, intent, authorization)?;
        self.remaining_budget = self.remaining_budget.saturating_sub(proposal.cost);
        report.authorized.push(intent.operation);
        self.live.insert(
            intent.operation,
            LiveEffect {
                intent,
                authorization_index: self.projection.authorizations().len().saturating_sub(1),
                // Nothing has reached the owner yet: the reservation is
                // durable, and the reach that follows is what sets this.
                sent: false,
                dispatches: 0,
                reconciliations: 0,
                dispatched_at_tick: self.tick_count,
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
        let Some(outcome) = self.reach_owner(intent, OwnerReach::Mutate, at, report) else {
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
    ) -> Option<EffectOutcome> {
        match reach {
            OwnerReach::Mutate => {
                if let Err(refusal) = self.authorize_reach(&intent) {
                    report.fenced.push((intent.operation, refusal));
                    return None;
                }
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
                Some(self.owner.dispatch(&intent, at))
            }
            OwnerReach::Observe => {
                if let Some(live) = self.live.get_mut(&intent.operation) {
                    live.reconciliations = live.reconciliations.saturating_add(1);
                }
                Some(self.owner.reconcile(&intent, at))
            }
        }
    }

    /// Revalidates the reservation's own snapshots through the source, then
    /// checks and spends its fence. Replay never supplies fresh authority.
    fn authorize_reach(&mut self, intent: &EffectIntent) -> Result<(), FenceRefusal> {
        let index = self
            .live
            .get(&intent.operation)
            .ok_or(FenceRefusal::StaleSnapshots)?
            .authorization_index;
        let record = self
            .projection
            .authorizations()
            .get(index)
            .filter(|record| record.pinned.request.operation == intent.operation)
            .ok_or(FenceRefusal::StaleSnapshots)?;
        let fence = self.authority.authenticate_reach(intent, &record.pinned)?;
        fence.check(intent)?;
        fence.spend_offline();
        Ok(())
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

    // ── stage 3: reconcile ──────────────────────────────────────────────────

    /// Reconciles open effects in stable operation order.
    fn pump_reconciliation(
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
            let Some(live) = self.live.get(&operation).copied() else {
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
    /// an observation and the fence leaves it open. Returns `None` only when
    /// the fence refused the retry.
    fn reconcile_one(
        &mut self,
        live: &LiveEffect,
        operation: OperationId,
        at: SystemTime,
        report: &mut TickReport,
    ) -> Result<Option<EffectOutcome>, RuntimeError> {
        Ok(match live.intent.strategy.reconciliation_action() {
            rocco_contracts::effect::ReconciliationAction::RetryBounded { max_attempts }
                if live.dispatches < max_attempts =>
            {
                self.reach_owner(live.intent, OwnerReach::Mutate, at, report)
            }
            rocco_contracts::effect::ReconciliationAction::RetryBounded { .. } => {
                // Budget spent. The effect stays visibly unresolved.
                Some(EffectOutcome::Unknown { operation })
            }
            rocco_contracts::effect::ReconciliationAction::Observe => {
                self.reach_owner(live.intent, OwnerReach::Observe, at, report)
            }
            rocco_contracts::effect::ReconciliationAction::LeaveUnknown => {
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
        if live.reconciliations >= rocco_contracts::effect::MAX_EFFECT_ATTEMPTS {
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
            rocco_contracts::effect::ReconciliationAction::RetryBounded { max_attempts } => {
                live.dispatches < max_attempts
            }
            rocco_contracts::effect::ReconciliationAction::Observe
            | rocco_contracts::effect::ReconciliationAction::LeaveUnknown => true,
            // Route unsupported policies to reconcile_one's typed refusal.
            _ => true,
        }
    }

    // ── stage 4: cancellation ───────────────────────────────────────────────

    /// Records cancellation for a nonterminal run.
    fn request_cancellation(&mut self, tenant: TenantId, run: RunId) -> Result<(), RuntimeError> {
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

    /// Completes cancellation only once reserved effects have drained.
    fn pump_cancellation(&mut self, report: &mut TickReport) -> Result<(), RuntimeError> {
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
                        Some(EffectPhase::Reserved)
                    )
            });
            if still_open {
                continue;
            }
            // Cancellation completes only with zero receipts for the run's
            // open work, so no partial external effect is left unlabelled.
            let _ = report;
            self.advance(run, TransitionEvent::FinishCancellation)?;
        }
        Ok(())
    }

    // ── journal writers ─────────────────────────────────────────────────────

    /// Establishes run ownership without exceeding the run-table bound.
    fn track_run(&mut self, run: RunId, tenant: TenantId) -> Result<(), RuntimeError> {
        self.ensure_run_tenant(run, tenant)?;
        if !self.runs.contains_key(&run) && self.runs.len() >= self.settings.run_capacity {
            return Err(RuntimeError::RunLimitReached {
                capacity: self.settings.run_capacity,
            });
        }
        self.runs.entry(run).or_insert(RunLive { tenant });
        Ok(())
    }

    /// Refuses an envelope whose tenant differs from an established run owner.
    fn ensure_run_tenant(&self, run: RunId, tenant: TenantId) -> Result<(), RuntimeError> {
        if let Some(live) = self.runs.get(&run)
            && live.tenant != tenant
        {
            return Err(RuntimeError::RunTenantMismatch {
                run,
                expected: live.tenant,
                got: tenant,
            });
        }
        Ok(())
    }

    /// Captures exactly the inputs used by the authorization reducer.
    fn pinned_inputs(&self, proposal: &WorkProposal, at: SystemTime) -> PinnedInputs {
        PinnedInputs::new(
            EffectRequest::new(
                proposal.operation,
                proposal.required_scopes().clone(),
                proposal.cost,
                proposal.unit,
                at,
            ),
            self.settings.policy.clone(),
            BudgetSnapshot::new(self.remaining_budget, self.settings.unit),
            self.settings.capabilities.clone(),
            self.settings.validity,
        )
    }

    /// Records the accepted command's target run and kind.
    fn record_command_accepted(
        &mut self,
        command: CommandId,
        tenant: TenantId,
        kind: CommandKind,
        run: RunId,
    ) -> Result<(), RuntimeError> {
        let payload = CommandAcceptedPayload { command, run, kind };
        let bytes = lgwks_std::json::to_vec(&payload)?;
        self.record_event(tenant, run, EventKind::CommandAccepted, bytes)
    }

    /// Records both the offered transition and its derived state.
    fn record_transition(
        &mut self,
        run: RunId,
        event: TransitionEvent,
        from: RunState,
        to: RunState,
    ) -> Result<(), RuntimeError> {
        let tenant = self.tenant_of(run)?;
        let payload = TransitionPayload { event, from, to };
        let bytes = lgwks_std::json::to_vec(&payload)?;
        self.record_event(tenant, run, EventKind::StateTransition, bytes)
    }

    /// Persists a reservation together with its pinned authorization.
    fn record_intent(
        &mut self,
        run: RunId,
        intent: EffectIntent,
        authorization: rocco_contracts::replay::ReplayRecord,
    ) -> Result<(), RuntimeError> {
        let tenant = self.tenant_of(run)?;
        let payload = IntentPayload {
            intent,
            authorization,
        };
        let bytes = lgwks_std::json::to_vec(&payload)?;
        self.record_event(tenant, run, EventKind::EffectIntentRecorded, bytes)
    }

    /// Persists an owner observation without inventing a disposition.
    fn record_outcome(
        &mut self,
        run: RunId,
        intent: EffectIntent,
        outcome: EffectOutcome,
    ) -> Result<(), RuntimeError> {
        let tenant = self.tenant_of(run)?;
        let payload = OutcomePayload { intent, outcome };
        let bytes = lgwks_std::json::to_vec(&payload)?;
        self.record_event(tenant, run, EventKind::EffectOutcomeRecorded, bytes)
    }

    /// Publishes one immutable terminal receipt per operation.
    fn publish_receipt(
        &mut self,
        run: RunId,
        intent: EffectIntent,
        disposition: TerminalDisposition,
        report: &mut TickReport,
    ) -> Result<(), RuntimeError> {
        if self
            .projection
            .receipts()
            .iter()
            .any(|receipt| receipt.operation() == intent.operation)
        {
            return Err(RuntimeError::ReceiptAlreadyPublished {
                operation: intent.operation,
            });
        }
        let receipt =
            TerminalReceipt::publish(intent.operation, run, self.tenant_of(run)?, disposition);
        let payload = ReceiptPayload { receipt };
        let bytes = lgwks_std::json::to_vec(&payload)?;
        self.record_event(
            self.tenant_of(run)?,
            run,
            EventKind::TerminalReceiptPublished,
            bytes,
        )?;
        report.receipts.push(receipt);
        Ok(())
    }

    /// Appends a sequenced event before applying it to the projection.
    fn record_event(
        &mut self,
        tenant: TenantId,
        run: RunId,
        kind: EventKind,
        payload: Vec<u8>,
    ) -> Result<(), RuntimeError> {
        let sequence = self.journal.next_sequence(run)?;
        let event = Event::new(
            tenant,
            run,
            sequence,
            Vec::new(),
            kind,
            payload.into_boxed_slice(),
        )?;
        self.journal.record(event)?;
        let Some(last) = self.journal.last() else {
            return Ok(());
        };
        let stored = match *last {
            JournalEntry::Event(ref recorded) => recorded,
            JournalEntry::Command(_) => return Ok(()),
        };
        self.projection.apply(stored)
    }

    /// Derives and records the next run state using the pure contract algebra.
    fn advance(&mut self, run: RunId, event: TransitionEvent) -> Result<RunState, RuntimeError> {
        let from = self.projection.run_state(run).unwrap_or(RunState::Draft);
        let to = rocco_contracts::state::transition(from, event)?;
        self.record_transition(run, event, from, to)?;
        Ok(to)
    }

    /// Reads established ownership, refusing untracked runs.
    fn tenant_of(&self, run: RunId) -> Result<TenantId, RuntimeError> {
        self.runs
            .get(&run)
            .map(|live| live.tenant)
            .ok_or(RuntimeError::RunNotTracked { run })
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
/// Each restored effect retains its recorded epoch and pinned snapshots. An
/// explicit authority source starts a new session and revalidates those exact
/// inputs before any retry. Host settings cannot refresh a restored decision;
/// disconnected recovery cannot recreate an authenticated session (E71).
///
/// # Errors
///
/// Returns [`RuntimeError::Codec`] or [`RuntimeError::FoldDivergence`] from
/// the rebuild, and [`RuntimeError::RunNotTracked`] for a run the projection
/// tracks but recorded no tenant for.
pub fn restore<P, O, C, A>(
    settings: RuntimeSettings,
    owner: O,
    clock: C,
    journal: Journal,
    mut authority: A,
) -> Result<Runtime<P, O, C, A>, RuntimeError>
where
    P: ProjectionBackend + Default,
    O: EffectOwner,
    C: crate::clock::Clock,
    A: AuthoritySource,
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
    let authorization_indices: BTreeMap<OperationId, usize> = projection
        .authorizations()
        .iter()
        .enumerate()
        .map(|(index, record)| (record.pinned.request.operation, index))
        .collect();
    for intent in projection.intents() {
        let phase = projection.effect_phase(intent.operation);
        // A reservation with no recorded outcome may already have reached the
        // endpoint, and the journal cannot prove otherwise, so restoration
        // spends the *dispatch* budget rather than resetting it. Reconciliation
        // budget is untouched: asking what happened is not retrying.
        let dispatches = match phase {
            Some(EffectPhase::Reserved) => rocco_contracts::effect::MAX_EFFECT_ATTEMPTS,
            Some(EffectPhase::Observed { .. } | EffectPhase::Reconciled { .. }) | None => 0,
        };
        live.insert(
            intent.operation,
            LiveEffect {
                intent: *intent,
                authorization_index: *authorization_indices.get(&intent.operation).ok_or(
                    RuntimeError::IntentNotDurable {
                        operation: intent.operation,
                    },
                )?,
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

    authority.begin_session(settings.offline_dispatch_bound);
    Ok(Runtime {
        journal,
        projection,
        owner,
        clock,
        authority,
        ready: VecDeque::new(),
        remaining_budget: settings.initial_budget.saturating_sub(reserved_budget),
        settings,
        runs,
        live,
        tick_count: 0,
    })
}
