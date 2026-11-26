//! E30 (larry#98): a durable scoped question, and the answer that is the only
//! way past it (Cell 02 "Missing human decision", D25).
//!
//! A question is an `Ask` command and an answer an `Answer` command, each
//! durably admitted through the protocol ingress (E28) from a client that can
//! serve a human (E47: `HumanQuestion`, never headless). The book of questions
//! is a fold over the journal's commands in journal order, so a restore
//! rebuilds it from the record alone, and an answer counts as of its durable
//! admission. A `Submit` whose effect a pending question names waits; once the
//! question is approved, rejected, timed out or cancelled, the submission is
//! admitted only when an approval covers its exact content and scopes, and is
//! denied [`DenialReason::NotApproved`] otherwise. Expiry is never consent, an
//! answer past expiry or after cancellation is refused with nothing written,
//! and one question's wait holds no other run's commands.

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use larry_contracts::authority::Scope;
use larry_contracts::command::{Command, CommandKind};
use larry_contracts::ids::{CommandId, ContentDigest, OperationId, RunId, TenantId, WorkerId};
use larry_contracts::replay::DenialReason;
use larry_contracts::state::RunState;
use lgwks_macros::Error;
use lgwks_std::json::serde::{Deserialize, Serialize};

use super::{DisplayedEffect, Runtime, TickReport};
use crate::error::RuntimeError;
use crate::journal::{Journal, JournalEntry};
use crate::owner::EffectOwner;
use crate::payload::WorkProposal;
use crate::projection::ProjectionBackend;

/// The exact effect a question is about: its run, its operation, the content
/// digest of the `Submit` that carries it, and the answer schema revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
pub struct Subject {
    /// The run the effect belongs to.
    run: RunId,
    /// The effect's stable operation identity.
    operation: OperationId,
    /// `CommandContent::digest` of the `Submit` the approval admits.
    effect: ContentDigest,
    /// The answer schema revision both sides render.
    schema: ContentDigest,
}

impl Subject {
    /// Names one effect exactly.
    #[must_use]
    pub const fn new(
        run: RunId,
        operation: OperationId,
        effect: ContentDigest,
        schema: ContentDigest,
    ) -> Self {
        Self {
            run,
            operation,
            effect,
            schema,
        }
    }
}

/// The body of an `Ask` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
pub struct Question {
    /// The effect in question.
    subject: Subject,
    /// Who asks; it can never be the respondent.
    requester: WorkerId,
    /// The only identity whose answer counts: the respondent must answer
    /// through an offer the host authenticated as this worker.
    respondent: WorkerId,
    /// The instant past which no answer is taken and nothing is approved.
    expires_at: SystemTime,
    /// The scopes an approval may grant.
    scopes: BTreeSet<Scope>,
}

impl Question {
    /// Asks `respondent`, on behalf of `requester`, about `subject` until
    /// `expires_at`, offering `scopes`. Admission refuses a question whose
    /// `requester` is its `respondent`: a worker cannot approve its own work.
    #[must_use]
    pub const fn new(
        subject: Subject,
        requester: WorkerId,
        respondent: WorkerId,
        expires_at: SystemTime,
        scopes: BTreeSet<Scope>,
    ) -> Self {
        Self {
            subject,
            requester,
            respondent,
            expires_at,
            scopes,
        }
    }
}

/// What a respondent decides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
#[non_exhaustive]
pub enum Decision {
    /// Approve exactly these scopes, each one the question offered.
    Approve(BTreeSet<Scope>),
    /// Refuse the effect.
    Reject,
}

/// The body of an `Answer` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
pub struct Answer {
    /// The `Ask` command this answers.
    question: CommandId,
    /// Who answers; it must be the question's respondent and the offer's
    /// host-authenticated principal.
    respondent: WorkerId,
    /// The effect the respondent saw; it must be the question's.
    subject: Subject,
    /// The decision.
    decision: Decision,
}

impl Answer {
    /// `respondent`'s `decision` on `question`, about the `subject` it showed.
    #[must_use]
    pub const fn new(
        question: CommandId,
        respondent: WorkerId,
        subject: Subject,
        decision: Decision,
    ) -> Self {
        Self {
            question,
            respondent,
            subject,
            decision,
        }
    }
}

/// Where one question stands at an instant. A caller sees it only through
/// what it does: a held `Submit`, a dispatch, a `NotApproved` denial, or a
/// typed refusal of a late answer.
#[derive(Debug, Clone, PartialEq, Eq)]
enum QuestionStatus {
    /// Waiting for its respondent.
    Pending,
    /// Approved for exactly these scopes.
    Approved(BTreeSet<Scope>),
    /// Refused by its respondent.
    Rejected,
    /// Expired unanswered; never consent.
    TimedOut,
    /// Its run was cancelled, or closed, unanswered.
    Cancelled,
}

/// Why a question or an answer was refused; nothing was written.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum QuestionError {
    /// The runtime's configured bound of pending questions is reached.
    #[error("{capacity} questions are pending; answer one first")]
    PendingFull {
        /// The bound.
        capacity: usize,
    },
    /// The effect already has a question.
    #[error("operation {operation} already has a question")]
    AlreadyAsked {
        /// The effect's operation.
        operation: OperationId,
    },
    /// The question expires no later than it is asked.
    #[error("the question expires before it is asked")]
    ExpiredWhenAsked,
    /// The question's respondent is its requester, so no answer can count.
    #[error("the question's responder is its requester; a worker cannot approve itself")]
    RequesterIsRespondent,
    /// The answer arrived from an offer with no host-authenticated principal.
    #[error("the answer to question {question} arrived with no authenticated principal")]
    UnboundPrincipal {
        /// The question.
        question: CommandId,
    },
    /// No question has this identity.
    #[error("no question {question}")]
    UnknownQuestion {
        /// The identity the answer named.
        question: CommandId,
    },
    /// The answer's tenant, respondent or effect is not the question's.
    #[error("the answer's tenant, respondent or effect is not question {question}'s")]
    BindingMismatch {
        /// The question.
        question: CommandId,
    },
    /// The question already has its answer.
    #[error("question {question} already has its answer")]
    AlreadyAnswered {
        /// The question.
        question: CommandId,
    },
    /// An approved scope the question did not offer.
    #[error("question {question} did not offer scope {scope:?}")]
    ScopeOutsideQuestion {
        /// The question.
        question: CommandId,
        /// The first such scope in order.
        scope: Scope,
    },
    /// The answer arrived at or past the question's expiry.
    #[error("question {question} expired; expiry is not consent, ask again")]
    Expired {
        /// The question.
        question: CommandId,
    },
    /// The run is cancelling, closed, or has a cancellation pending.
    #[error("run {run} is cancelled or closed")]
    RunClosed {
        /// The run.
        run: RunId,
    },
}

/// The run an `Ask` or `Answer` command names, or `None` for any other kind.
pub(super) fn run_of(command: &Command) -> Option<Result<RunId, RuntimeError>> {
    let payload = command.content().payload();
    match command.content().kind() {
        CommandKind::Ask => Some(
            lgwks_std::json::from_slice::<Question>(payload)
                .map(|question| question.subject.run)
                .map_err(RuntimeError::from),
        ),
        CommandKind::Answer => Some(
            lgwks_std::json::from_slice::<Answer>(payload)
                .map(|answer| answer.subject.run)
                .map_err(RuntimeError::from),
        ),
        _ => None,
    }
}

/// One asked question, with its tenant and its answer once admitted.
#[derive(Debug, Clone)]
struct Asked {
    /// The tenant the `Ask` envelope carried.
    tenant: TenantId,
    /// What was asked.
    question: Question,
    /// The admitted answer's decision.
    decision: Option<Decision>,
}

/// The tenant-scoped hot key for one effect's pending question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct QuestionKey {
    /// The tenant that owns the question.
    tenant: TenantId,
    /// The run the question names.
    run: RunId,
    /// The operation the question names.
    operation: OperationId,
}

impl QuestionKey {
    /// Builds the index key from the journal envelope and question subject.
    const fn new(tenant: TenantId, subject: Subject) -> Self {
        Self {
            tenant,
            run: subject.run,
            operation: subject.operation,
        }
    }
}

impl Asked {
    /// Where the question stands at `at`, its run `closed` or not.
    ///
    /// An approval is consent only for a use before `expires_at`: a `Submit`
    /// reduced at or past expiry is denied `NotApproved`, never dispatched on
    /// an answer taken earlier (E30, larry#331).
    fn status(&self, at: SystemTime, closed: bool) -> QuestionStatus {
        match self.decision {
            Some(Decision::Approve(ref scopes)) if at < self.question.expires_at => {
                QuestionStatus::Approved(scopes.clone())
            }
            Some(Decision::Approve(_)) => QuestionStatus::TimedOut,
            Some(Decision::Reject) => QuestionStatus::Rejected,
            None if closed => QuestionStatus::Cancelled,
            None if at >= self.question.expires_at => QuestionStatus::TimedOut,
            None => QuestionStatus::Pending,
        }
    }
}

/// Every question the journal holds, by its `Ask` command's identity.
#[derive(Debug, Default)]
pub(super) struct Book {
    /// Questions in identity order, retained for exact answer validation.
    asked: BTreeMap<CommandId, Asked>,
    /// All subject identities, including settled questions, for O(log n) binding.
    by_subject: BTreeMap<QuestionKey, CommandId>,
    /// Only questions that can still hold work, keyed by tenant, run and operation.
    pending: BTreeMap<QuestionKey, CommandId>,
}

impl Book {
    /// The book `journal`'s commands fold to, in journal order.
    pub(super) fn rebuild(journal: &Journal) -> Self {
        let mut book = Self::default();
        for entry in journal.entries() {
            if let JournalEntry::Command(ref command) = *entry {
                book.fold(command);
            }
        }
        book
    }

    /// Folds one admitted command. Admission checked it before the journal
    /// took it, so a body that does not decode was never acknowledged here,
    /// and the consumer's digests expose an altered file (E35).
    pub(super) fn fold(&mut self, command: &Command) {
        let payload = command.content().payload();
        match command.content().kind() {
            CommandKind::Ask => {
                if let Ok(question) = lgwks_std::json::from_slice::<Question>(payload) {
                    let key = QuestionKey::new(command.tenant(), question.subject);
                    self.asked.insert(
                        command.id(),
                        Asked {
                            tenant: command.tenant(),
                            question,
                            decision: None,
                        },
                    );
                    self.by_subject.insert(key, command.id());
                    self.pending.insert(key, command.id());
                }
            }
            CommandKind::Answer => {
                // The journal's answer counts only under the same binding the
                // admission enforced: the respondent is the question's
                // respondent and not its requester, and the effect is the
                // question's. A restore therefore re-derives the verdict from
                // the record alone (E30, larry#331).
                if let Ok(answer) = lgwks_std::json::from_slice::<Answer>(payload)
                    && let Some(asked) = self.asked.get_mut(&answer.question)
                    && asked.decision.is_none()
                    && answer.respondent == asked.question.respondent
                    && answer.respondent != asked.question.requester
                    && answer.subject == asked.question.subject
                {
                    let key = QuestionKey::new(asked.tenant, answer.subject);
                    asked.decision = Some(answer.decision);
                    self.pending.remove(&key);
                }
            }
            _ => {}
        }
    }

    /// The question bound to `tenant`'s effect `operation` on `run`.
    fn bound(&self, tenant: TenantId, run: RunId, operation: OperationId) -> Option<&Asked> {
        let key = QuestionKey {
            tenant,
            run,
            operation,
        };
        self.by_subject
            .get(&key)
            .and_then(|question| self.asked.get(question))
    }

    /// Removes questions that can no longer hold work from the hot index.
    fn prune_pending(&mut self, at: SystemTime, mut closed: impl FnMut(RunId) -> bool) {
        let settled: Vec<QuestionKey> = self
            .pending
            .iter()
            .filter_map(|(key, question)| {
                self.asked.get(question).and_then(|asked| {
                    (asked.status(at, closed(key.run)) != QuestionStatus::Pending).then_some(*key)
                })
            })
            .collect();
        for key in settled {
            self.pending.remove(&key);
        }
    }

    /// Counts pending questions without walking settled history.
    fn pending_count(&mut self, at: SystemTime, closed: impl FnMut(RunId) -> bool) -> usize {
        self.prune_pending(at, closed);
        self.pending.len()
    }

    /// Counts one tenant's pending questions without walking settled history.
    fn tenant_pending_count(
        &mut self,
        tenant: TenantId,
        at: SystemTime,
        closed: impl FnMut(RunId) -> bool,
    ) -> usize {
        self.prune_pending(at, closed);
        self.pending
            .keys()
            .filter(|key| key.tenant == tenant)
            .count()
    }
}

impl<P, O> Runtime<P, O>
where
    P: ProjectionBackend + Default,
    O: EffectOwner,
{
    /// Whether `run` takes no answer: cancelling or closed, or a cancellation
    /// already admitted for it.
    fn run_closed(&self, run: RunId) -> bool {
        self.projection
            .run_state(run)
            .is_some_and(|state| state == RunState::Cancelling || state.is_terminal())
            || self.cancel_pending(run)
    }

    /// How many questions are pending at `at`, every tenant together.
    ///
    /// This is the `max_pending_questions` bound: the share below is a second
    /// bound beside it, never a replacement (larry#295, larry#333).
    fn pending_questions(&mut self, at: SystemTime) -> usize {
        let projection = &self.projection;
        let cancel_pending = self.cancel_pending_runs();
        self.questions.pending_count(at, |run| {
            projection
                .run_state(run)
                .is_some_and(|state| state == RunState::Cancelling || state.is_terminal())
                || cancel_pending.contains(&run)
        })
    }

    /// How many of `tenant`'s questions are pending at `at`.
    ///
    /// The share is per tenant: one tenant's unanswered questions never refuse
    /// another tenant's ask (larry#333).
    fn tenant_pending_questions(&mut self, tenant: TenantId, at: SystemTime) -> usize {
        let projection = &self.projection;
        let cancel_pending = self.cancel_pending_runs();
        self.questions.tenant_pending_count(tenant, at, |run| {
            projection
                .run_state(run)
                .is_some_and(|state| state == RunState::Cancelling || state.is_terminal())
                || cancel_pending.contains(&run)
        })
    }

    /// Checks an `Ask` or `Answer` before the journal takes it. An answer is
    /// judged against `principal`, the host-authenticated worker the offer
    /// presents, so a client cannot name an identity it does not hold.
    pub(super) fn check_question(
        &mut self,
        command: &Command,
        at: SystemTime,
        principal: Option<WorkerId>,
    ) -> Result<(), RuntimeError> {
        let payload = command.content().payload();
        let kind = command.content().kind();
        if kind == CommandKind::Ask {
            let question: Question = lgwks_std::json::from_slice(payload)?;
            return self.check_ask(command.tenant(), &question, at);
        }
        if kind == CommandKind::Answer {
            let answer: Answer = lgwks_std::json::from_slice(payload)?;
            return self.check_answer(command.tenant(), &answer, at, principal);
        }
        // Every other kind carries no question; admission judges it on its own
        // terms and refuses a kind it does not support.
        Ok(())
    }

    /// An `Ask` names a respondent other than its requester, expires after it
    /// is asked, targets an open run, is its effect's only question, and fits
    /// the pending bound.
    fn check_ask(
        &mut self,
        tenant: TenantId,
        question: &Question,
        at: SystemTime,
    ) -> Result<(), RuntimeError> {
        let subject = question.subject;
        if question.requester == question.respondent {
            return Err(QuestionError::RequesterIsRespondent.into());
        }
        if question.expires_at <= at {
            return Err(QuestionError::ExpiredWhenAsked.into());
        }
        if self.run_closed(subject.run) {
            return Err(QuestionError::RunClosed { run: subject.run }.into());
        }
        if self
            .questions
            .bound(tenant, subject.run, subject.operation)
            .is_some()
        {
            return Err(QuestionError::AlreadyAsked {
                operation: subject.operation,
            }
            .into());
        }
        // The global bound holds first, then the tenant's share beside it.
        let capacity = self.settings.max_pending_questions;
        if self.pending_questions(at) >= capacity {
            return Err(QuestionError::PendingFull { capacity }.into());
        }
        let share = self.settings.tenant_share(capacity);
        if self.tenant_pending_questions(tenant, at) >= share {
            return Err(QuestionError::PendingFull { capacity: share }.into());
        }
        Ok(())
    }

    /// An `Answer` is its question's first, from the respondent the question
    /// names and no other principal, never from the question's requester,
    /// about the question's exact effect, before expiry on an open run, and
    /// approves only offered scopes.
    fn check_answer(
        &self,
        tenant: TenantId,
        answer: &Answer,
        at: SystemTime,
        principal: Option<WorkerId>,
    ) -> Result<(), RuntimeError> {
        let question = answer.question;
        let Some(principal) = principal else {
            return Err(QuestionError::UnboundPrincipal { question }.into());
        };
        let asked = self
            .questions
            .asked
            .get(&question)
            .ok_or(QuestionError::UnknownQuestion { question })?;
        if asked.tenant != tenant
            || asked.question.respondent != answer.respondent
            || answer.respondent != principal
            || asked.question.subject != answer.subject
        {
            return Err(QuestionError::BindingMismatch { question }.into());
        }
        if asked.question.requester == principal {
            return Err(QuestionError::RequesterIsRespondent.into());
        }
        if asked.decision.is_some() {
            return Err(QuestionError::AlreadyAnswered { question }.into());
        }
        if self.run_closed(answer.subject.run) {
            return Err(QuestionError::RunClosed {
                run: answer.subject.run,
            }
            .into());
        }
        if at >= asked.question.expires_at {
            return Err(QuestionError::Expired { question }.into());
        }
        if let Decision::Approve(ref scopes) = answer.decision
            && let Some(&scope) = scopes.difference(&asked.question.scopes).next()
        {
            return Err(QuestionError::ScopeOutsideQuestion { question, scope }.into());
        }
        Ok(())
    }

    /// Whether `command` is a `Submit` whose effect's question is pending.
    pub(super) fn awaits_answer(&self, run: RunId, command: &Command, at: SystemTime) -> bool {
        if command.content().kind() != CommandKind::Submit {
            return false;
        }
        let Ok(proposal) = lgwks_std::json::from_slice::<WorkProposal>(command.content().payload())
        else {
            return false;
        };
        self.questions
            .bound(command.tenant(), run, proposal.operation())
            .is_some_and(|asked| asked.status(at, self.run_closed(run)) == QuestionStatus::Pending)
    }

    /// Whether the effect may pass: no question names it, or an approval
    /// covers its exact content and every scope it requires.
    pub(super) fn approved(
        &self,
        tenant: TenantId,
        proposal: &WorkProposal,
        content: ContentDigest,
        at: SystemTime,
    ) -> Result<(), DenialReason> {
        let Some(asked) = self
            .questions
            .bound(tenant, proposal.run(), proposal.operation())
        else {
            return Ok(());
        };
        match asked.status(at, self.run_closed(proposal.run())) {
            QuestionStatus::Approved(ref scopes)
                if asked.question.subject.effect == content
                    && proposal.required_scopes().is_subset(scopes) =>
            {
                Ok(())
            }
            _ => Err(DenialReason::NotApproved),
        }
    }

    /// Reduces a `Submit`: a run its question was cancelled with is past
    /// admitting work, so the effect is denied without a transition; anything
    /// else goes to the gate.
    pub(super) fn submit_work(
        &mut self,
        command: &Command,
        display: Option<&DisplayedEffect>,
        at: SystemTime,
        report: &mut TickReport,
    ) -> Result<(), RuntimeError> {
        let tenant = command.tenant();
        let proposal: WorkProposal = lgwks_std::json::from_slice(command.content().payload())?;
        let closing = self
            .projection
            .run_state(proposal.run())
            .is_some_and(|state| state == RunState::Cancelling || state.is_terminal());
        if closing
            && self
                .questions
                .bound(tenant, proposal.run(), proposal.operation())
                .is_some()
        {
            report
                .denied
                .push((proposal.operation(), DenialReason::NotApproved));
            return Ok(());
        }
        let content = command.content().digest();
        self.admit_work(command, &proposal, content, display, at, report)
    }
}
