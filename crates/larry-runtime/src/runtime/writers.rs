//! The journal writers: every fact the runtime records goes through one of
//! these, and each one appends to the journal before it folds the projection.

use std::time::SystemTime;

use larry_contracts::command::CommandKind;
use larry_contracts::effect::{EffectIntent, EffectOutcome, TerminalDisposition, TerminalReceipt};
use larry_contracts::event::{Event, EventKind};
use larry_contracts::ids::{CommandId, RunId, TenantId};
use larry_contracts::replay::{BudgetSnapshot, EffectRequest, PinnedInputs};
use larry_contracts::state::{RunState, TransitionEvent};

use super::{RunLive, Runtime, TickReport};
use crate::error::RuntimeError;
use crate::journal::JournalEntry;
use crate::owner::EffectOwner;
use crate::payload::{
    CommandAcceptedPayload, CommandRefusal, CommandRefusedPayload, IntentPayload, OutcomePayload,
    ReceiptPayload, TransitionPayload, WorkProposal,
};
use crate::projection::ProjectionBackend;

impl<P, O> Runtime<P, O>
where
    P: ProjectionBackend + Default,
    O: EffectOwner,
{
    /// Establishes run ownership without exceeding the table's ceiling or the
    /// tenant's share of it.
    ///
    /// Each tenant holds at most `run_capacity / max_tenants` tracked runs, at
    /// least one, so one tenant cannot fill the table every tenant shares
    /// (larry#333). A terminal run whose receipt is sealed has left the table:
    /// it can record no further event, so its row is a slot the next run of its
    /// tenant takes.
    pub(super) fn track_run(&mut self, run: RunId, tenant: TenantId) -> Result<(), RuntimeError> {
        if self.runs.contains_key(&run) {
            self.ensure_run_tenant(run, tenant)?;
        } else {
            self.ensure_run_capacity(run, tenant)?;
            // A sealed terminal run is already known to the projection: it was
            // evicted from the table, and re-tracking it would spend a slot for
            // a run no work may enter.
            if self.terminal_in_view(run) {
                return Ok(());
            }
        }
        if let std::collections::btree_map::Entry::Vacant(slot) = self.runs.entry(run) {
            slot.insert(RunLive { tenant });
            let count = self.run_counts.entry(tenant).or_insert(0);
            *count = count.saturating_add(1);
        }
        // One budget per tenant (larry#208): a tenant enters the budget map
        // only through a tracked run, so the map holds at most the run table's
        // capacity of counters, and a restored tenant keeps the remainder the
        // fold gave it.
        self.budgets
            .entry(tenant)
            .or_insert(self.settings.initial_budget);
        Ok(())
    }

    /// Decides whether `run` may enter the table, and adds nothing.
    ///
    /// The check runs before a command's acceptance is recorded, so a journal
    /// never holds an accepted fact the table or the fold cannot take. A run
    /// the projection already records as terminal left the table and takes no
    /// row; a run it has not seen needs a view. The global ceiling refuses
    /// typed before any eviction, so the table never passes `run_capacity`,
    /// and this tenant's share may free only its own sealed terminal rows.
    pub(super) fn ensure_run_capacity(
        &mut self,
        run: RunId,
        tenant: TenantId,
    ) -> Result<(), RuntimeError> {
        self.ensure_run_tenant(run, tenant)?;
        if self.runs.contains_key(&run) || self.terminal_in_view(run) {
            return Ok(());
        }
        if self.runs.len() >= self.settings.run_capacity {
            return Err(RuntimeError::RunLimitReached {
                capacity: self.settings.run_capacity,
            });
        }
        // The fold takes this run's view at its acceptance record, so a new
        // view must have room too.
        if self.projection.run_state(run).is_none()
            && self.projection.run_count() >= self.settings.run_capacity
        {
            return Err(RuntimeError::RunLimitReached {
                capacity: self.settings.run_capacity,
            });
        }
        self.free_run_slots(tenant)
    }

    /// Whether the projection already records `run` as terminal.
    fn terminal_in_view(&self, run: RunId) -> bool {
        self.projection
            .run_state(run)
            .is_some_and(RunState::is_terminal)
    }

    /// Frees this tenant's slots past its share that a sealed terminal run can
    /// give back.
    ///
    /// The oldest row in identity order goes first, so the same journal frees
    /// the same rows on every replay. The global ceiling is refused by
    /// [`Self::ensure_run_capacity`] before this runs.
    fn free_run_slots(&mut self, tenant: TenantId) -> Result<(), RuntimeError> {
        let share = self.settings.tenant_share(self.settings.run_capacity);
        loop {
            let Some(held) = self.run_counts.get(&tenant).copied() else {
                return Ok(());
            };
            if held < share {
                return Ok(());
            }
            let victim = self.runs.iter().find_map(|(run, live)| {
                (live.tenant == tenant && self.sealed_terminal(*run)).then_some(*run)
            });
            let Some(victim) = victim else {
                return Err(RuntimeError::RunLimitReached { capacity: share });
            };
            self.runs.remove(&victim);
            if let Some(count) = self.run_counts.get_mut(&tenant) {
                *count = count.saturating_sub(1);
            }
        }
    }

    /// Whether `run` is terminal and its receipt is sealed, so the table may
    /// let it go.
    fn sealed_terminal(&self, run: RunId) -> bool {
        self.projection
            .run_state(run)
            .is_some_and(RunState::is_terminal)
            && self
                .projection
                .receipts()
                .iter()
                .any(|receipt| receipt.run() == run)
    }

    /// Refuses an envelope whose tenant differs from an established run owner.
    pub(super) fn ensure_run_tenant(
        &self,
        run: RunId,
        tenant: TenantId,
    ) -> Result<(), RuntimeError> {
        // An evicted sealed run keeps its owner in the projection, so another
        // tenant's command on it is still refused as trespass rather than
        // answered as a closed run (larry#333).
        let owner = self
            .runs
            .get(&run)
            .map(|live| live.tenant)
            .or_else(|| self.projection.run_tenant(run));
        if let Some(expected) = owner
            && expected != tenant
        {
            return Err(RuntimeError::RunTenantMismatch {
                run,
                expected,
                got: tenant,
            });
        }
        Ok(())
    }

    /// Captures exactly the inputs used by the authorization reducer.
    ///
    /// The budget snapshot is `tenant`'s own remainder (larry#208), so the
    /// decision bounds that tenant's cumulative spend and no other tenant's.
    pub(super) fn pinned_inputs(
        &self,
        tenant: TenantId,
        proposal: &WorkProposal,
        at: SystemTime,
    ) -> PinnedInputs {
        PinnedInputs::new(
            EffectRequest::new(
                proposal.operation(),
                proposal.required_scopes().clone(),
                proposal.cost(),
                proposal.unit(),
                at,
            ),
            self.settings.policy.clone(),
            BudgetSnapshot::new(self.remaining_budget(tenant), self.settings.unit),
            self.settings.capabilities.clone(),
            self.settings.validity,
        )
    }

    /// Records the accepted command's target run and kind.
    pub(super) fn record_command_accepted(
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

    /// Records why a durably accepted command never ran.
    ///
    /// The acceptance fact is already in the journal and stays there, so the
    /// refusal beside it is what a restart reads; the in-memory report alone
    /// dies with the process (larry#371, ADR 0078).
    pub(super) fn record_command_refused(
        &mut self,
        command: CommandId,
        tenant: TenantId,
        run: RunId,
        reason: CommandRefusal,
    ) -> Result<(), RuntimeError> {
        let payload = CommandRefusedPayload {
            command,
            run,
            reason,
        };
        let bytes = lgwks_std::json::to_vec(&payload)?;
        self.record_event(tenant, run, EventKind::CommandRefused, bytes)
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
    pub(super) fn record_intent(
        &mut self,
        run: RunId,
        intent: EffectIntent,
        authorization: larry_contracts::replay::ReplayRecord,
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
    pub(super) fn record_outcome(
        &mut self,
        run: RunId,
        intent: EffectIntent,
        outcome: EffectOutcome,
        dispatches: u8,
    ) -> Result<(), RuntimeError> {
        let tenant = self.tenant_of(run)?;
        let payload = OutcomePayload {
            intent,
            outcome,
            dispatches,
        };
        let bytes = lgwks_std::json::to_vec(&payload)?;
        self.record_event(tenant, run, EventKind::EffectOutcomeRecorded, bytes)
    }

    /// Publishes one immutable terminal receipt per operation.
    pub(super) fn publish_receipt(
        &mut self,
        run: RunId,
        intent: EffectIntent,
        disposition: TerminalDisposition,
        report: &mut TickReport,
    ) -> Result<(), RuntimeError> {
        if self.projection.has_receipt(intent.operation) {
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
    pub(super) fn advance(
        &mut self,
        run: RunId,
        event: TransitionEvent,
    ) -> Result<RunState, RuntimeError> {
        // A run with no recorded state is a run the algebra has not started:
        // `Draft` is its defined initial state, not a substitute for an error.
        let from = match self.projection.run_state(run) {
            Some(state) => state,
            None => RunState::Draft,
        };
        let to = larry_contracts::state::transition(from, event)?;
        self.record_transition(run, event, from, to)?;
        Ok(to)
    }

    /// Reads established ownership, refusing untracked runs.
    pub(super) fn tenant_of(&self, run: RunId) -> Result<TenantId, RuntimeError> {
        self.runs
            .get(&run)
            .map(|live| live.tenant)
            .or_else(|| self.projection.run_tenant(run))
            .ok_or(RuntimeError::RunNotTracked { run })
    }
}
