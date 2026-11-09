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
use std::fmt;
use std::time::SystemTime;

use larry_contracts::authority::Scope;
use larry_contracts::command::{Command, CommandKind};
use larry_contracts::ids::{CommandId, ContentDigest, OperationId, RunId, TenantId, WorkerId};
use larry_contracts::replay::DenialReason;
use larry_contracts::state::RunState;
use lgwks_std::json::serde::{Deserialize, Serialize};

use super::{DisplayedEffect, Runtime, TickReport};
use crate::error::RuntimeError;
use crate::journal::{Journal, JournalEntry};
use crate::owner::EffectOwner;
use crate::payload::WorkProposal;
use crate::projection::ProjectionBackend;

/// Most questions one runtime holds pending at once (Cell 05 bounds). A scope
/// set needs no bound of its own: `Scope` is closed, so a set holds at most
/// its five variants.
const MAX_PENDING_QUESTIONS: usize = 256;

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
    /// Who asks.
    requester: WorkerId,
    /// The only identity whose answer counts.
    respondent: WorkerId,
    /// The instant past which no answer is taken and nothing is approved.
    expires_at: SystemTime,
    /// The scopes an approval may grant.
    scopes: BTreeSet<Scope>,
}

impl Question {
    /// Asks `respondent`, on behalf of `requester`, about `subject` until
    /// `expires_at`, offering `scopes`.
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
    /// Who answers; it must be the question's respondent.
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
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum QuestionError {
    /// The runtime's bound of pending questions (256) is reached.
    PendingFull {
        /// The bound.
        capacity: usize,
    },
    /// The effect already has a question.
    AlreadyAsked {
        /// The effect's operation.
        operation: OperationId,
    },
    /// The question expires no later than it is asked.
    ExpiredWhenAsked,
    /// No question has this identity.
    UnknownQuestion {
        /// The identity the answer named.
        question: CommandId,
    },
    /// The answer's tenant, respondent or effect is not the question's.
    BindingMismatch {
        /// The question.
        question: CommandId,
    },
    /// The question already has its answer.
    AlreadyAnswered {
        /// The question.
        question: CommandId,
    },
    /// An approved scope the question did not offer.
    ScopeOutsideQuestion {
        /// The question.
        question: CommandId,
        /// The first such scope in order.
        scope: Scope,
    },
    /// The answer arrived at or past the question's expiry.
    Expired {
        /// The question.
        question: CommandId,
    },
    /// The run is cancelling, closed, or has a cancellation pending.
    RunClosed {
        /// The run.
        run: RunId,
    },
}

impl fmt::Display for QuestionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::PendingFull { capacity } => {
                write!(f, "{capacity} questions are pending; answer one first")
            }
            Self::AlreadyAsked { operation } => {
                write!(f, "operation {operation} already has a question")
            }
            Self::ExpiredWhenAsked => f.write_str("the question expires before it is asked"),
            Self::UnknownQuestion { question } => write!(f, "no question {question}"),
            Self::BindingMismatch { question } => write!(
                f,
                "the answer's tenant, respondent or effect is not question {question}'s"
            ),
            Self::AlreadyAnswered { question } => {
                write!(f, "question {question} already has its answer")
            }
            Self::ScopeOutsideQuestion { question, scope } => {
                write!(f, "question {question} did not offer scope {scope:?}")
            }
            Self::Expired { question } => write!(
                f,
                "question {question} expired; expiry is not consent, ask again"
            ),
            Self::RunClosed { run } => write!(f, "run {run} is cancelled or closed"),
        }
    }
}

impl std::error::Error for QuestionError {}

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

impl Asked {
    /// Where the question stands at `at`, its run `closed` or not.
    fn status(&self, at: SystemTime, closed: bool) -> QuestionStatus {
        match self.decision {
            Some(Decision::Approve(ref scopes)) => QuestionStatus::Approved(scopes.clone()),
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
    /// Questions in identity order.
    asked: BTreeMap<CommandId, Asked>,
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
                    self.asked.insert(
                        command.id(),
                        Asked {
                            tenant: command.tenant(),
                            question,
                            decision: None,
                        },
                    );
                }
            }
            CommandKind::Answer => {
                if let Ok(answer) = lgwks_std::json::from_slice::<Answer>(payload)
                    && let Some(asked) = self.asked.get_mut(&answer.question)
                    && asked.decision.is_none()
                {
                    asked.decision = Some(answer.decision);
                }
            }
            _ => {}
        }
    }

    /// The question bound to `tenant`'s effect `operation` on `run`.
    fn bound(&self, tenant: TenantId, run: RunId, operation: OperationId) -> Option<&Asked> {
        self.asked.values().find(|asked| {
            asked.tenant == tenant
                && asked.question.subject.run == run
                && asked.question.subject.operation == operation
        })
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

    /// How many questions are pending at `at`.
    fn pending_questions(&self, at: SystemTime) -> usize {
        self.questions
            .asked
            .values()
            .filter(|asked| {
                asked.status(at, self.run_closed(asked.question.subject.run))
                    == QuestionStatus::Pending
            })
            .count()
    }

    /// Checks an `Ask` or `Answer` before the journal takes it.
    pub(super) fn check_question(
        &self,
        command: &Command,
        at: SystemTime,
    ) -> Result<(), RuntimeError> {
        let payload = command.content().payload();
        match command.content().kind() {
            CommandKind::Ask => {
                let question: Question = lgwks_std::json::from_slice(payload)?;
                self.check_ask(command.tenant(), &question, at)
            }
            CommandKind::Answer => {
                let answer: Answer = lgwks_std::json::from_slice(payload)?;
                self.check_answer(command.tenant(), &answer, at)
            }
            _ => Ok(()),
        }
    }

    /// An `Ask` expires after it is asked, targets an open run, is its
    /// effect's only question, and fits the pending bound.
    fn check_ask(
        &self,
        tenant: TenantId,
        question: &Question,
        at: SystemTime,
    ) -> Result<(), RuntimeError> {
        let subject = question.subject;
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
        if self.pending_questions(at) >= MAX_PENDING_QUESTIONS {
            return Err(QuestionError::PendingFull {
                capacity: MAX_PENDING_QUESTIONS,
            }
            .into());
        }
        Ok(())
    }

    /// An `Answer` matches its question's tenant, respondent and effect, is
    /// its first, arrives before expiry on an open run, and approves only
    /// offered scopes.
    fn check_answer(
        &self,
        tenant: TenantId,
        answer: &Answer,
        at: SystemTime,
    ) -> Result<(), RuntimeError> {
        let question = answer.question;
        let asked = self
            .questions
            .asked
            .get(&question)
            .ok_or(QuestionError::UnknownQuestion { question })?;
        if asked.tenant != tenant
            || asked.question.respondent != answer.respondent
            || asked.question.subject != answer.subject
        {
            return Err(QuestionError::BindingMismatch { question }.into());
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
            .bound(command.tenant(), run, proposal.operation)
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
            .bound(tenant, proposal.run, proposal.operation)
        else {
            return Ok(());
        };
        match asked.status(at, self.run_closed(proposal.run)) {
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
            .run_state(proposal.run)
            .is_some_and(|state| state == RunState::Cancelling || state.is_terminal());
        if closing
            && self
                .questions
                .bound(tenant, proposal.run, proposal.operation)
                .is_some()
        {
            report
                .denied
                .push((proposal.operation, DenialReason::NotApproved));
            return Ok(());
        }
        let content = command.content().digest();
        self.admit_work(tenant, &proposal, content, display, at, report)
    }
}
