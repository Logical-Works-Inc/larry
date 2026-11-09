//! E28 (larry#100): an input is durable before it is acknowledged, and a
//! correction is promoted only at a safe boundary.
//!
//! [`Runtime::admit`] journals a command and syncs the journal's file before
//! it returns, so an acknowledgement a caller holds survives a kill -9. The
//! command is then *admitted* but not yet *accepted*: acceptance is the
//! `CommandAccepted` fact a tick records when it reduces the command, and the
//! admitted commands without it are the pending set. The set is kept in
//! journal order, bounded by `MAX_READY_WORK`, and a restore rebuilds it from
//! the journal alone, never from process memory. A retry of an acknowledged
//! command under its identity and content is `DuplicateReplay`; other content
//! under that identity is `ConflictingRetry`, refused with nothing written.
//!
//! A `Steer` is a correction. It is promoted (its `CommandAccepted` recorded)
//! only once every effect of its run has a definite disposition: succeeded,
//! failed or compensated. While an effect is reserved, observed but
//! unresolved, or permanently unknown, the correction stays pending, and later
//! commands of the same run wait behind it, so one run's commands are reduced
//! in the order they were acknowledged. Other runs pass it. No committed
//! effect is rewritten.

use std::collections::{BTreeSet, VecDeque};
use std::time::SystemTime;

use larry_contracts::command::{Command, CommandAcceptance, CommandKind};
use larry_contracts::effect::TerminalDisposition;
use larry_contracts::event::EventKind;
use larry_contracts::ids::{RunId, TenantId};

use super::instant::sample;
use super::{DisplayedEffect, Runtime, TickReport};
use crate::MAX_READY_WORK;
use crate::error::RuntimeError;
use crate::journal::{Journal, JournalEntry};
use crate::owner::EffectOwner;
use crate::payload::{CommandAcceptedPayload, WorkProposal};
use crate::projection::{EffectPhase, ProjectionBackend};

#[cfg(test)]
mod tests;

/// One durably admitted command not yet reduced, with the run it targets.
#[derive(Debug, Clone)]
pub(super) struct Admitted {
    /// The run the command's proposal names.
    run: RunId,
    /// The command as the journal holds it.
    command: Command,
    /// The display it was queued with, kept in memory only: a restore admits
    /// the command without one, and its reach then refuses a bound owner.
    display: Option<DisplayedEffect>,
}

/// The run `command`'s proposal names.
fn target(command: &Command) -> Result<RunId, RuntimeError> {
    let proposal: WorkProposal = lgwks_std::json::from_slice(command.content().payload())?;
    Ok(proposal.run)
}

/// The admitted commands `journal` holds no `CommandAccepted` fact for, in
/// journal order.
///
/// # Errors
///
/// [`RuntimeError::Codec`] for an acceptance payload that does not decode.
pub(super) fn recover(journal: &Journal) -> Result<VecDeque<Admitted>, RuntimeError> {
    let mut accepted = BTreeSet::new();
    for entry in journal.entries() {
        if let JournalEntry::Event(ref event) = *entry
            && event.kind() == EventKind::CommandAccepted
        {
            let payload: CommandAcceptedPayload = lgwks_std::json::from_slice(event.payload())?;
            accepted.insert(payload.command);
        }
    }
    let mut pending = VecDeque::new();
    for entry in journal.entries() {
        if let JournalEntry::Command(ref command) = *entry
            && !accepted.contains(&command.id())
        {
            // Admission and the tick both decode the proposal before the
            // journal takes a command, so one that does not decode was never
            // acknowledged here: a hand-built or altered file holds it, the
            // consumer's digests expose it (E35), and nothing may reduce it.
            let Ok(run) = target(command) else {
                continue;
            };
            pending.push_back(Admitted {
                run,
                command: command.clone(),
                display: None,
            });
        }
    }
    Ok(pending)
}

impl<P, O> Runtime<P, O>
where
    P: ProjectionBackend + Default,
    O: EffectOwner,
{
    /// Durably admits `command` and acknowledges it (E28).
    ///
    /// The command is journaled and the journal's file synced before this
    /// returns, so the acknowledgement survives a kill -9; a tick reduces it
    /// later, in journal order. A `display` is checked against the command
    /// here and kept in memory with it: a restore admits the command without
    /// one, and its reach then refuses an owner that needs a binding. A retry under the same identity and content
    /// is acknowledged again as [`CommandAcceptance::DuplicateReplay`] and
    /// never reduced twice.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Codec`] for a payload that is not a proposal,
    /// [`RuntimeError::RunTenantMismatch`] when the run belongs to another
    /// tenant, [`RuntimeError::ReadyWorkFull`] when `MAX_READY_WORK` commands
    /// are pending, [`RuntimeError::Command`] for a conflicting or foreign
    /// retry, the display check's refusal for a bound submission, and the
    /// journal's append and sync refusals. Nothing is written on any refusal
    /// but a sync refusal, whose entry a restore then admits.
    pub(super) fn admit(
        &mut self,
        command: Command,
        display: Option<DisplayedEffect>,
    ) -> Result<CommandAcceptance, RuntimeError> {
        let run = target(&command)?;
        if display.is_some() && command.content().kind() != CommandKind::Submit {
            // A display is only ever made for a submission (E65).
            return Err(RuntimeError::UnsupportedCommand(command.content().kind()));
        }
        if self.journal.command(command.id()).is_none() {
            self.ensure_admitted_tenant(run, command.tenant())?;
            if self.admitted.len() >= MAX_READY_WORK {
                return Err(RuntimeError::ReadyWorkFull {
                    capacity: MAX_READY_WORK,
                });
            }
            if command.content().kind() == CommandKind::Submit {
                let proposal: WorkProposal =
                    lgwks_std::json::from_slice(command.content().payload())?;
                let at = sample(&self.clock)?;
                self.validate_display(&command, &proposal, display.as_ref(), at)?;
            }
        }
        let held = command.clone();
        let acceptance = self.journal.accept_command(command)?;
        if acceptance == CommandAcceptance::Initial {
            self.admitted.push_back(Admitted {
                run,
                command: held,
                display,
            });
        }
        self.journal.sync()?;
        Ok(acceptance)
    }

    /// Refuses a run another tenant owns, by the run table or by a command
    /// already pending for it.
    fn ensure_admitted_tenant(&self, run: RunId, tenant: TenantId) -> Result<(), RuntimeError> {
        self.ensure_run_tenant(run, tenant)?;
        if let Some(owner) = self
            .admitted
            .iter()
            .find(|admitted| admitted.run == run && admitted.command.tenant() != tenant)
        {
            return Err(RuntimeError::RunTenantMismatch {
                run,
                expected: owner.command.tenant(),
                got: tenant,
            });
        }
        Ok(())
    }

    /// Whether every effect of `run` has a definite disposition, so a
    /// correction may be promoted without contradicting one in flight.
    fn settled(&self, run: RunId) -> bool {
        self.live
            .values()
            .filter(|live| live.intent.run == run)
            .all(|live| {
                matches!(
                    self.projection.effect_phase(live.intent.operation),
                    Some(EffectPhase::Reconciled {
                        disposition: TerminalDisposition::Succeeded
                            | TerminalDisposition::Failed
                            | TerminalDisposition::Compensated
                    })
                )
            })
    }

    /// Whether `command` must wait: a correction its run is not settled for.
    fn unsafe_correction(&self, run: RunId, command: &Command) -> bool {
        command.content().kind() == CommandKind::Steer && !self.settled(run)
    }

    /// Holds a just-accepted ready command pending when it is a correction its
    /// run is not settled for, or when its run already has a command pending.
    /// Returns the command back when it may be reduced now.
    pub(super) fn hold_or_pass(
        &mut self,
        run: RunId,
        command: Command,
        display: Option<&DisplayedEffect>,
    ) -> Option<Command> {
        let waiting = self.admitted.iter().any(|admitted| admitted.run == run);
        if !waiting && !self.unsafe_correction(run, &command) {
            return Some(command);
        }
        self.admitted.push_back(Admitted {
            run,
            command,
            display: display.cloned(),
        });
        None
    }

    /// Reduces pending commands in journal order, at most `budget`, and
    /// returns the budget left.
    ///
    /// A correction its run is not settled for is held, and so is every later
    /// pending command of that run; other runs' commands pass it.
    pub(super) fn promote_admitted(
        &mut self,
        budget: usize,
        at: SystemTime,
        report: &mut TickReport,
    ) -> Result<usize, RuntimeError> {
        let mut left = budget;
        let mut blocked = BTreeSet::new();
        let mut index = 0;
        while left > 0 {
            let Some(next) = self.admitted.get(index) else {
                break;
            };
            let run = next.run;
            if blocked.contains(&run) || self.unsafe_correction(run, &next.command) {
                blocked.insert(run);
                index = index.saturating_add(1);
                continue;
            }
            let Some(next) = self.admitted.remove(index) else {
                break;
            };
            left = left.saturating_sub(1);
            let proposal: WorkProposal =
                lgwks_std::json::from_slice(next.command.content().payload())?;
            self.ensure_run_tenant(run, next.command.tenant())?;
            report.admitted.push(next.command.id());
            self.reduce(&next.command, &proposal, next.display.as_ref(), at, report)?;
        }
        Ok(left)
    }

    /// Records `command`'s acceptance and folds it: work is gated and
    /// reserved, a cancellation requested. A correction's acceptance is the
    /// whole of its promotion: the journal is where its consumer reads it.
    pub(super) fn reduce(
        &mut self,
        command: &Command,
        proposal: &WorkProposal,
        display: Option<&DisplayedEffect>,
        at: SystemTime,
        report: &mut TickReport,
    ) -> Result<(), RuntimeError> {
        let (id, tenant, kind) = (command.id(), command.tenant(), command.content().kind());
        self.record_command_accepted(id, tenant, kind, proposal.run)?;
        match kind {
            CommandKind::Submit => self.admit_work(tenant, proposal, display, at, report),
            CommandKind::Cancel => self.request_cancellation(tenant, proposal.run),
            CommandKind::Steer => Ok(()),
            other => Err(RuntimeError::UnsupportedCommand(other)),
        }
    }
}
