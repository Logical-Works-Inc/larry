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
    CommandAcceptedPayload, IntentPayload, OutcomePayload, ReceiptPayload, TransitionPayload,
    WorkProposal,
};
use crate::projection::ProjectionBackend;

impl<P, O, C> Runtime<P, O, C>
where
    P: ProjectionBackend + Default,
    O: EffectOwner,
    C: crate::clock::Clock,
{
    /// Establishes run ownership without exceeding the run-table bound.
    pub(super) fn track_run(&mut self, run: RunId, tenant: TenantId) -> Result<(), RuntimeError> {
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
    pub(super) fn ensure_run_tenant(
        &self,
        run: RunId,
        tenant: TenantId,
    ) -> Result<(), RuntimeError> {
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
    pub(super) fn pinned_inputs(&self, proposal: &WorkProposal, at: SystemTime) -> PinnedInputs {
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
    ) -> Result<(), RuntimeError> {
        let tenant = self.tenant_of(run)?;
        let payload = OutcomePayload { intent, outcome };
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
            .ok_or(RuntimeError::RunNotTracked { run })
    }
}
