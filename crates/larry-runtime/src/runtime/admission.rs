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

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::SystemTime;

use larry_contracts::binding::{BindingError, BindingField};
use larry_contracts::command::{Command, CommandAcceptance, CommandKind};
use larry_contracts::effect::TerminalDisposition;
use larry_contracts::event::EventKind;
use larry_contracts::ids::{ContentDigest, RunId, TenantId, WorkerId};
use larry_contracts::state::{RunState, TransitionEvent};

use super::display::ReadyCommand;
use super::instant::sample;
use super::live::LiveEffect;
use super::{DisplayedEffect, Runtime, TickItem, TickReport};
use crate::error::RuntimeError;
use crate::journal::{Journal, JournalEntry};
use crate::owner::EffectOwner;
use crate::payload::{CommandAcceptedPayload, CommandRefusal, WorkProposal};
use crate::projection::{EffectPhase, ProjectionBackend};
use crate::{MAX_EFFECTS_PER_RUN, MAX_READY_WORK};

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

impl Admitted {
    /// A pending entry for `command`, which targets `run`.
    pub(super) const fn new(
        run: RunId,
        command: Command,
        display: Option<DisplayedEffect>,
    ) -> Self {
        Self {
            run,
            command,
            display,
        }
    }
}

/// The run `command` names: its question's or answer's (E30), or its
/// proposal's.
fn target(command: &Command) -> Result<RunId, RuntimeError> {
    if let Some(run) = super::approval::run_of(command) {
        return run;
    }
    let proposal: WorkProposal = lgwks_std::json::from_slice(command.content().payload())?;
    Ok(proposal.run())
}

/// Why a pending command holds the later commands of its run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    /// A correction its run is not settled for (D25).
    Correction,
    /// Work whose question is pending (E30).
    Answer,
}

/// Whether a command of `kind` may pass a run held for an answer: the answer
/// itself, another question, or the stop control, which must stay real.
const fn passes_answer_hold(kind: CommandKind) -> bool {
    matches!(
        kind,
        CommandKind::Answer | CommandKind::Ask | CommandKind::Cancel
    )
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
    /// later, in journal order. Nothing in memory holds the command until the
    /// sync returns: the pending queue and the question book are filled only
    /// after it, so a caller that receives `Err` from the sync has had neither
    /// moved (larry#182). A `display` is checked against the command here and
    /// kept in memory with it: a restore admits the command without one, and
    /// its reach then refuses an owner that needs a binding. A retry under the
    /// same identity and content is acknowledged again as
    /// [`CommandAcceptance::DuplicateReplay`] and never reduced twice.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::Codec`] for a payload that is not a proposal,
    /// [`RuntimeError::RunTenantMismatch`] when the run belongs to another
    /// tenant, [`RuntimeError::ReadyWorkFull`] when `MAX_READY_WORK` commands
    /// are pending, [`RuntimeError::Command`] for a conflicting or foreign
    /// retry, the display check's refusal for a bound submission, and the
    /// journal's append and sync refusals. Nothing is written on any refusal
    /// but a sync refusal, whose entry stays on disk: no in-memory queue or
    /// question book holds it, and a restore is what admits it, once. After a
    /// sync refusal every later admit in this process, a retry included, is
    /// refused with the journal's typed error before the index moves (larry#209),
    /// so no path in this process admits the command.
    pub(super) fn admit(
        &mut self,
        command: Command,
        display: Option<DisplayedEffect>,
        principal: Option<WorkerId>,
    ) -> Result<CommandAcceptance, RuntimeError> {
        // The ceiling is checked before the payload is decoded or journalled:
        // an oversized command costs one length read and leaves nothing held
        // (larry#212).
        let bytes = command.content().payload().len();
        let ceiling = self.settings.max_payload_bytes;
        if bytes > ceiling {
            return Err(RuntimeError::PayloadTooLarge { bytes, ceiling });
        }
        let run = target(&command)?;
        if display.is_some() && command.content().kind() != CommandKind::Submit {
            // A display is only ever made for a submission (E65).
            return Err(RuntimeError::UnsupportedCommand(command.content().kind()));
        }
        if self.journal.command(command.id()).is_none() {
            self.ensure_admitted_tenant(run, command.tenant())?;
            // The pending set is bounded twice: the whole set by
            // `MAX_READY_WORK`, and one tenant by its share of that bound, so a
            // tenant past its share is refused while another still admits
            // (larry#333).
            let share = self.settings.tenant_share(MAX_READY_WORK);
            if self.tenant_pending(command.tenant()) >= share {
                return Err(RuntimeError::ReadyWorkFull { capacity: share });
            }
            if self.admitted.len() >= MAX_READY_WORK {
                return Err(RuntimeError::ReadyWorkFull {
                    capacity: MAX_READY_WORK,
                });
            }
            let kind = command.content().kind();
            if kind == CommandKind::Submit {
                let proposal: WorkProposal =
                    lgwks_std::json::from_slice(command.content().payload())?;
                let at = sample(&self.clock)?;
                self.validate_display(&command, &proposal, display.as_ref(), at)?;
            }
            if matches!(kind, CommandKind::Ask | CommandKind::Answer) {
                // A question or an answer counts as of its durable admission,
                // so it is judged against this instant before it is written.
                // An answer must come from the offer's authenticated principal
                // (E30, larry#331).
                let at = sample(&self.clock)?;
                self.check_question(&command, at, principal)?;
            }
        }
        let held = command.clone();
        let acceptance = self.journal.accept_command(command)?;
        // Nothing in memory moves until the entry is durable (larry#182). A
        // caller told its command went into the pending set must be able to
        // read it back after a crash, and a caller told its sync failed must
        // not have the next tick execute the command anyway. On a sync refusal
        // the entry stays on disk and no in-memory queue or question holds it:
        // a restore reads that entry and admits it exactly once (E28). The
        // journal latches on that refusal (larry#209), so this process's retry
        // is refused by `accept_command` and never reaches `DuplicateReplay`.
        self.journal.sync()?;
        if acceptance == CommandAcceptance::Initial {
            self.questions.fold(&held);
            self.admitted.push_back(Admitted {
                run,
                command: held,
                display,
            });
        }
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

    /// How many pending commands one tenant holds in the pending set.
    fn tenant_pending(&self, tenant: TenantId) -> usize {
        self.admitted
            .iter()
            .filter(|admitted| admitted.command.tenant() == tenant)
            .count()
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

    /// Why `command` must wait, if it must: a correction its run is not
    /// settled for, or work whose question is pending.
    fn hold(&self, run: RunId, command: &Command, at: SystemTime) -> Option<Hold> {
        match command.content().kind() {
            CommandKind::Steer if !self.settled(run) => Some(Hold::Correction),
            CommandKind::Submit if self.awaits_answer(run, command, at) => Some(Hold::Answer),
            _ => None,
        }
    }

    /// Whether a cancellation of `run` is admitted and not yet reduced.
    pub(super) fn cancel_pending(&self, run: RunId) -> bool {
        self.admitted.iter().any(|admitted| {
            admitted.run == run && admitted.command.content().kind() == CommandKind::Cancel
        })
    }

    /// Holds a just-accepted ready command pending when it must wait, or when
    /// its run already has a command pending that it may not pass. Returns
    /// the command back when it may be reduced now.
    pub(super) fn hold_or_pass(
        &mut self,
        run: RunId,
        command: Command,
        display: Option<&DisplayedEffect>,
        at: SystemTime,
    ) -> Option<Command> {
        let passes = passes_answer_hold(command.content().kind());
        let waiting = self.admitted.iter().any(|admitted| {
            admitted.run == run
                && !(passes && self.hold(run, &admitted.command, at) == Some(Hold::Answer))
        });
        if !waiting && self.hold(run, &command, at).is_none() {
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
        let mut blocked: BTreeMap<RunId, Hold> = BTreeMap::new();
        let mut index = 0;
        while left > 0 {
            let Some(next) = self.admitted.get(index) else {
                break;
            };
            let run = next.run;
            let behind = match blocked.get(&run) {
                Some(&Hold::Answer) => !passes_answer_hold(next.command.content().kind()),
                Some(&Hold::Correction) => true,
                None => false,
            };
            if behind {
                index = index.saturating_add(1);
                continue;
            }
            if let Some(hold) = self.hold(run, &next.command, at) {
                blocked.entry(run).or_insert(hold);
                index = index.saturating_add(1);
                continue;
            }
            // The tenant is checked while the command still sits in the set,
            // so a refusal here leaves it pending rather than dropped.
            if let Err(error) = self.ensure_run_tenant(run, next.command.tenant()) {
                // The command stays pending; the report names the refusal and
                // every other run's command still passes (larry#329).
                self.refuse(report, TickItem::Command(next.command.id()), error)?;
                index = index.saturating_add(1);
                continue;
            }
            let Some(next) = self.admitted.remove(index) else {
                break;
            };
            left = left.saturating_sub(1);
            let id = next.command.id();
            if let Err(error) = self.reduce(next, index, at, report) {
                self.refuse(report, TickItem::Command(id), error)?;
                // A failed acceptance record puts the command back at this
                // index, so the loop steps past it: one failure costs one slot
                // of the tick's budget, never the whole tick. A refusal after
                // the acceptance is durable leaves the set, so the command now
                // at this index is the next one and is reduced (larry#371).
                let put_back = self
                    .admitted
                    .get(index)
                    .is_some_and(|admitted| admitted.command.id() == id);
                if put_back {
                    index = index.saturating_add(1);
                }
            }
        }
        Ok(left)
    }

    /// Records `held`'s acceptance and folds it: work is gated and reserved, a
    /// cancellation requested, a question's run tracked for its tenant. A
    /// correction's or an answer's acceptance is the whole of its promotion:
    /// the journal is where its consumer reads it, and the answer was folded
    /// into the question book at admission (E30).
    ///
    /// The journal tells a reduced command from an unreduced one by its
    /// `CommandAccepted` fact, and a command journaled without that fact is
    /// pending (ADR 0025). When recording the acceptance fails, `held` goes
    /// back into the pending set at `place` before the error returns, so the
    /// next tick, or a restore, reduces it once; a retry of it is never a
    /// duplicate of a command nothing holds (larry#219, ADR 0041). The set
    /// keeps the slot the command left, so it stays within `MAX_READY_WORK`.
    /// A failure after the acceptance is recorded is the fold's own record:
    /// the command is reduced as far as the journal says.
    ///
    /// # Errors
    ///
    /// The acceptance record's refusal (a full journal, a latched one), or the
    /// fold's own refusal after it.
    pub(super) fn reduce(
        &mut self,
        held: Admitted,
        place: usize,
        at: SystemTime,
        report: &mut TickReport,
    ) -> Result<(), RuntimeError> {
        let Admitted {
            run,
            command,
            display,
        } = held;
        let (id, tenant, kind) = (command.id(), command.tenant(), command.content().kind());
        // A command that would open a run takes its table and view room before
        // its acceptance is journalled: an accepted fact the table or the fold
        // cannot take leaves a journal no restore can reopen (larry#333).
        let room = if matches!(
            kind,
            CommandKind::Submit | CommandKind::Ask | CommandKind::Cancel
        ) {
            self.ensure_run_capacity(run, tenant)
        } else {
            Ok(())
        };
        if let Err(error) = room.and_then(|()| self.record_command_accepted(id, tenant, kind, run))
        {
            let place = place.min(self.admitted.len());
            self.admitted.insert(
                place,
                Admitted {
                    run,
                    command,
                    display,
                },
            );
            return Err(error);
        }
        // Only an accepted command is reported as admitted: one whose
        // acceptance record failed stays pending and is named as refused
        // (larry#371).
        report.admitted.push(id);
        match kind {
            CommandKind::Submit => self.submit_work(&command, display.as_ref(), at, report),
            CommandKind::Cancel => self.request_cancellation(tenant, run),
            CommandKind::Ask => self.track_run(run, tenant),
            CommandKind::Steer | CommandKind::Answer => Ok(()),
            other => Err(RuntimeError::UnsupportedCommand(other)),
        }
    }

    /// Stage 2: admits queued commands in deterministic order within the tick
    /// bound. Durably admitted commands go first, in journal order, then the
    /// ready queue, under one budget (E28).
    ///
    /// One command's refusal is recorded in `report` and the stage goes on with
    /// the next command, unless it is the journal's own refusal (larry#329).
    pub(super) fn pump_ready(
        &mut self,
        at: SystemTime,
        report: &mut TickReport,
    ) -> Result<(), RuntimeError> {
        let budget = self.promote_admitted(self.settings.max_runs_per_tick, at, report)?;
        let share = self.settings.tenant_share(MAX_READY_WORK);
        let mut attempts = 0_usize;
        let mut passed_over = 0_usize;
        while attempts < budget {
            // Every command ahead was passed over once in this pass, so no
            // tenant with room is waiting behind a full one.
            if passed_over >= self.ready.len() {
                break;
            }
            let Some(queued) = self.ready.pop_front() else {
                break;
            };
            if self.tenant_pending(queued.command.tenant()) >= share {
                // A tenant at its pending share waits behind the others instead
                // of holding the head of the queue (larry#333).
                self.ready.push_back(queued);
                passed_over = passed_over.saturating_add(1);
                continue;
            }
            passed_over = 0;
            attempts = attempts.saturating_add(1);
            let id = queued.command.id();
            match self.admit_ready(queued, at, report) {
                Ok(true) => {}
                Ok(false) => return Ok(()),
                Err(error) => self.refuse(report, TickItem::Command(id), error)?,
            }
        }
        Ok(())
    }

    /// Admits one queued command. Returns `false` when the command has to wait
    /// for room in the pending set and was pushed back at the queue's front.
    fn admit_ready(
        &mut self,
        queued: ReadyCommand,
        at: SystemTime,
        report: &mut TickReport,
    ) -> Result<bool, RuntimeError> {
        let ReadyCommand { command, display } = queued;
        let id = command.id();
        let tenant = command.tenant();
        let kind = command.content().kind();
        let payload = command.content().payload().to_vec();
        let proposal: WorkProposal = lgwks_std::json::from_slice(&payload)?;
        self.ensure_run_tenant(proposal.run(), tenant)?;

        if kind == CommandKind::Submit && self.journal.command(id).is_none() {
            self.validate_display(&command, &proposal, display.as_ref(), at)?;
        }
        if self.journal.command(id).is_none() && self.admitted.len() >= MAX_READY_WORK {
            // A command that may have to wait has no room to: it stays
            // queued for the next tick rather than being dropped.
            self.ready.push_front(ReadyCommand { command, display });
            return Ok(false);
        }
        let kept = command.clone();
        let acceptance = self.journal.accept_command(command)?;
        if acceptance == CommandAcceptance::DuplicateReplay {
            report.duplicates.push(id);
            return Ok(true);
        }
        if let Some(command) = self.hold_or_pass(proposal.run(), kept, display.as_ref(), at) {
            let held = Admitted::new(proposal.run(), command, display);
            // A command that fails to record its acceptance returns to the
            // back of the pending set: it is journaled, so it must be held
            // (larry#219).
            self.reduce(held, self.admitted.len(), at, report)?;
        }
        Ok(true)
    }

    /// Gates and durably reserves a proposal before reaching its owner.
    pub(super) fn admit_work(
        &mut self,
        command: &Command,
        proposal: &WorkProposal,
        content: ContentDigest,
        display: Option<&DisplayedEffect>,
        at: SystemTime,
        report: &mut TickReport,
    ) -> Result<(), RuntimeError> {
        let (tenant, command_id) = (command.tenant(), command.id());
        let run = proposal.run();
        // A display approved at an earlier epoch is not a standing entitlement.
        // The command may have waited in the pending set since it was shown,
        // and a revocation since then withdraws the generation it approved
        // (larry#332). A ready command is checked at admission already; this
        // re-check is what covers the held one when its reduce finally comes.
        if let Some(shown) = display
            && shown.epoch != self.fence.epoch()?
        {
            // The command's acceptance is already durable, so the refusal is
            // recorded beside it: the report dies with the process, and a
            // restart must still say why the command never ran (larry#371).
            self.record_command_refused(
                command_id,
                tenant,
                run,
                CommandRefusal::StaleDisplayEpoch,
            )?;
            return Err(BindingError::Changed {
                field: BindingField::SecurityEpoch,
            }
            .into());
        }
        // Admissibility is decided from the runtime's own run table, never
        // inferred from the projection alone: recording the command already
        // created a run view, so a projection-based check would read `Draft`
        // and skip the admission step. A tracked run still in `Draft` was
        // tracked by a question asked before its work (E30) and was never
        // admitted.
        let is_new = !self.runs.contains_key(&run)
            || matches!(self.projection.run_state(run), None | Some(RunState::Draft));
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
        if self.live.contains_key(&proposal.operation()) {
            return Err(RuntimeError::DuplicateOperation {
                operation: proposal.operation(),
            });
        }
        // A question bound to this effect is the only way past it: an
        // approval covering its exact content and scopes, never a timeout
        // (D25, E30).
        if let Err(reason) = self.approved(tenant, proposal, content, at) {
            report.denied.push((proposal.operation(), reason));
            self.advance(run, TransitionEvent::Fail)?;
            return Ok(());
        }

        // The admission epoch comes from the fence and the tenant from the run
        // table, never from the proposal: a command cannot declare the security
        // generation it is admitted under or the tenant it is admitted for, so
        // a hostile generator cannot name a generation that outlives a
        // revocation, or record an effect — or re-attribute a run after a
        // restart — to a tenant it does not own (D42, E63). The pinned budget
        // snapshot is that tenant's own remainder, so the decision bounds one
        // tenant's spend and never another's (larry#208).
        let tenant = self.tenant_of(run)?;
        let mut pinned = self.pinned_inputs(tenant, proposal, at);
        if let Some(shown) = display {
            shown.constrain(&mut pinned);
        }
        let admitted_at = pinned.request.observed_at();
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
        let intent = proposal.intent(tenant, self.fence.epoch()?);
        let intent = match display {
            Some(shown) => intent.with_binding(*shown.binding()),
            None => intent,
        };
        let proof = crate::gate::Proof::issue(&pinned, &self.clock);
        self.record_intent(run, intent, authorization)?;
        // One budget per tenant (larry#208): the cost comes out of this run's
        // own tenant's counter, never a pool the other tenants share.
        let remaining = self
            .budgets
            .entry(tenant)
            .or_insert(self.settings.initial_budget);
        *remaining = remaining.saturating_sub(proposal.cost());
        report.authorized.push(intent.operation);
        self.live.insert(
            intent.operation,
            LiveEffect {
                intent,
                proof,
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
}
