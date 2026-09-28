//! Bounded, process-scoped human approval question routing (E30).
//!
//! This is a routing seam only. It never authorizes or dispatches an effect;
//! durable question state remains the responsibility of Logical-DB.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::time::SystemTime;

use rocco_contracts::authority::Scope;
use rocco_contracts::effect::EffectIntent;
use rocco_contracts::ids::{ContentDigest, OperationId, RunId, TenantId, WorkerId};
use rocco_contracts::state::RunState;

/// Maximum approval questions retained by one runtime instance.
pub const MAX_APPROVAL_QUESTIONS: usize = 256;

/// Maximum authority scopes retained in one question or response.
pub const MAX_APPROVAL_ANSWER_SCOPES: usize = 16;

/// Stable identity of a question within one runtime lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ApprovalQuestionId(u64);

impl ApprovalQuestionId {
    /// Returns the question's runtime-local sequence number.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Requester, respondent and tenant bound to one approval question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalQuestionParties {
    /// Tenant owning the question.
    tenant: TenantId,
    /// Run whose effect is in question.
    run: RunId,
    /// Worker that requested the decision.
    requester: WorkerId,
    /// Sole worker permitted to answer.
    respondent: WorkerId,
}

impl ApprovalQuestionParties {
    /// Binds a request and its only authorized human respondent.
    #[must_use]
    pub const fn new(
        tenant: TenantId,
        run: RunId,
        requester: WorkerId,
        respondent: WorkerId,
    ) -> Self {
        Self {
            tenant,
            run,
            requester,
            respondent,
        }
    }
}

/// Effect identity and answer-schema version bound to a question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalEffectBinding {
    /// Stable effect identity.
    operation: OperationId,
    /// Hash of the complete effect payload supplied by its owner.
    effect_digest: ContentDigest,
    /// Hash of the answer schema revision.
    schema_digest: ContentDigest,
}

impl ApprovalEffectBinding {
    /// Binds an answer to one operation payload and schema revision.
    #[must_use]
    pub const fn new(
        operation: OperationId,
        effect_digest: ContentDigest,
        schema_digest: ContentDigest,
    ) -> Self {
        Self {
            operation,
            effect_digest,
            schema_digest,
        }
    }
}

/// Immutable binding required to open one approval question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalQuestionSpec {
    /// Tenant, run and human identities.
    parties: ApprovalQuestionParties,
    /// Exact operation and schema binding.
    effect: ApprovalEffectBinding,
    /// Pinned question expiry.
    expires_at: SystemTime,
    /// Maximum scopes an approval answer may contain.
    allowed_scopes: Vec<Scope>,
}

impl ApprovalQuestionSpec {
    /// Constructs an approval question binding its parties, effect and schema.
    ///
    /// # Errors
    ///
    /// Returns [`ApprovalQuestionError::ScopeListTooLarge`] when the scope
    /// list exceeds [`MAX_APPROVAL_ANSWER_SCOPES`].
    pub fn new(
        parties: ApprovalQuestionParties,
        effect: ApprovalEffectBinding,
        expires_at: SystemTime,
        mut allowed_scopes: Vec<Scope>,
    ) -> Result<Self, ApprovalQuestionError> {
        if allowed_scopes.len() > MAX_APPROVAL_ANSWER_SCOPES {
            return Err(ApprovalQuestionError::ScopeListTooLarge {
                len: allowed_scopes.len(),
                capacity: MAX_APPROVAL_ANSWER_SCOPES,
            });
        }
        allowed_scopes.sort_unstable();
        allowed_scopes.dedup();
        Ok(Self {
            parties,
            effect,
            expires_at,
            allowed_scopes,
        })
    }

    /// The tenant that owns this question.
    #[must_use]
    pub const fn tenant(&self) -> TenantId {
        self.parties.tenant
    }

    /// The run whose effect is being considered.
    #[must_use]
    pub const fn run(&self) -> RunId {
        self.parties.run
    }

    /// The worker that requested the decision.
    #[must_use]
    pub const fn requester(&self) -> WorkerId {
        self.parties.requester
    }

    /// The only worker allowed to answer this question.
    #[must_use]
    pub const fn respondent(&self) -> WorkerId {
        self.parties.respondent
    }

    /// The exact effect operation bound by the question.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.effect.operation
    }

    /// Digest of the effect payload supplied by the authority owner.
    #[must_use]
    pub const fn effect_digest(&self) -> ContentDigest {
        self.effect.effect_digest
    }

    /// Digest identifying the answer schema.
    #[must_use]
    pub const fn schema_digest(&self) -> ContentDigest {
        self.effect.schema_digest
    }

    /// Expiry instant, compared only with a tick's pinned observation.
    #[must_use]
    pub const fn expires_at(&self) -> SystemTime {
        self.expires_at
    }

    /// The scope values an approval answer may contain.
    #[must_use]
    pub fn allowed_scopes(&self) -> &[Scope] {
        &self.allowed_scopes
    }
}

/// A decision submitted by the question's authorized respondent.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ApprovalDecision {
    /// Approve only the listed subset of the question's allowed scopes.
    Approve(Vec<Scope>),
    /// Refuse the requested effect.
    Reject,
}

/// One response bound to the exact question, tenant, respondent and effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalAnswer {
    /// Runtime-local question identity.
    question: ApprovalQuestionId,
    /// Tenant scope of the question.
    tenant: TenantId,
    /// Only respondent allowed to answer.
    respondent: WorkerId,
    /// Operation and answer-schema binding from the original request.
    effect: ApprovalEffectBinding,
    /// Proposed decision, applied only at the next pinned tick.
    decision: ApprovalDecision,
}

impl ApprovalAnswer {
    /// Binds one answer to its original question and immutable effect scope.
    #[must_use]
    pub fn new(
        question: ApprovalQuestionId,
        tenant: TenantId,
        respondent: WorkerId,
        effect: ApprovalEffectBinding,
        decision: ApprovalDecision,
    ) -> Self {
        Self {
            question,
            tenant,
            respondent,
            effect,
            decision,
        }
    }
}

/// Current state of one approval question.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ApprovalQuestionStatus {
    /// Waiting for the identified respondent.
    Pending,
    /// The respondent approved the listed scopes; this is not an effect grant.
    Approved(Vec<Scope>),
    /// The respondent refused the request.
    Rejected,
    /// The owning run was cancelled before the answer was applied.
    Cancelled,
    /// The pinned tick instant reached the expiry before the answer was applied.
    TimedOut,
}

/// A refused question operation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ApprovalQuestionError {
    /// No interactive client can present or receive this human question.
    NoInteractiveClient {
        /// The effect that cannot be asked about.
        operation: OperationId,
    },
    /// The connected client is not the respondent named by the question.
    ClientIsNotRespondent {
        /// The respondent named by the question.
        expected: WorkerId,
        /// The connected client identity.
        got: WorkerId,
    },
    /// The retained question bound has been reached.
    CapacityReached {
        /// The maximum retained questions.
        capacity: usize,
    },
    /// The runtime-local question sequence cannot advance without wrapping.
    IdentityExhausted,
    /// The requested question identity is not retained by this runtime.
    UnknownQuestion {
        /// The missing runtime-local identity.
        question: ApprovalQuestionId,
    },
    /// The answer does not match the question's tenant/effect/respondent binding.
    BindingMismatch {
        /// The contested question.
        question: ApprovalQuestionId,
    },
    /// The question already has a queued or terminal answer.
    AlreadyResolved {
        /// The terminal or already answered question.
        question: ApprovalQuestionId,
    },
    /// At least one approved scope was not permitted by the question schema.
    ScopeOutsideQuestion {
        /// The question whose schema refused the scope.
        question: ApprovalQuestionId,
        /// The refused scope.
        scope: Scope,
    },
    /// The question or answer exceeds the finite authority-scope bound.
    ScopeListTooLarge {
        /// Number of submitted scope entries.
        len: usize,
        /// Maximum accepted entry count.
        capacity: usize,
    },
    /// The run or effect does not exist in this runtime's current lifecycle.
    EffectNotTracked {
        /// The operation that is not active in this runtime.
        operation: OperationId,
    },
    /// The run has been cancelled or already reached a terminal state.
    RunNotAwaitingApproval {
        /// The run that cannot accept a new question.
        run: RunId,
    },
}

impl fmt::Display for ApprovalQuestionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NoInteractiveClient { operation } => write!(
                f,
                "approval for operation {operation} awaits authority; no interactive client is connected"
            ),
            Self::ClientIsNotRespondent { expected, got } => write!(
                f,
                "interactive client {got} is not authorized respondent {expected}"
            ),
            Self::CapacityReached { capacity } => {
                write!(f, "approval question capacity {capacity} is reached")
            }
            Self::IdentityExhausted => f.write_str("approval question identity space is exhausted"),
            Self::UnknownQuestion { question } => {
                write!(f, "approval question {} is not retained", question.value())
            }
            Self::BindingMismatch { question } => write!(
                f,
                "answer does not match approval question {} tenant, respondent or effect",
                question.value()
            ),
            Self::AlreadyResolved { question } => write!(
                f,
                "approval question {} is no longer pending",
                question.value()
            ),
            Self::ScopeOutsideQuestion { question, scope } => write!(
                f,
                "approval question {} does not allow scope {scope:?}",
                question.value()
            ),
            Self::ScopeListTooLarge { len, capacity } => write!(
                f,
                "approval scope list has {len} entries; maximum is {capacity}"
            ),
            Self::EffectNotTracked { operation } => {
                write!(f, "approval operation {operation} is not a tracked effect")
            }
            Self::RunNotAwaitingApproval { run } => write!(
                f,
                "run {run} is cancelled or terminal and cannot open an approval question"
            ),
        }
    }
}

impl Error for ApprovalQuestionError {}

/// One retained approval question and its queued response state.
#[derive(Debug)]
struct Question {
    /// Immutable parties, effect, schema and expiry.
    spec: ApprovalQuestionSpec,
    /// Last settled public status.
    status: ApprovalQuestionStatus,
    /// At most one response waiting for the next pinned tick.
    answer: Option<ApprovalDecision>,
}

/// The bounded map owned by `Runtime`; mutations are synchronous and per-key.
#[derive(Debug, Default)]
pub(crate) struct ApprovalBook {
    /// Next checked runtime-local question number.
    next_id: u64,
    /// Bounded retained questions in stable identity order.
    questions: BTreeMap<ApprovalQuestionId, Question>,
}

impl ApprovalBook {
    /// Adds one question after verifying its owner effect and interactive client.
    pub(crate) fn open(
        &mut self,
        spec: ApprovalQuestionSpec,
        client: Option<WorkerId>,
        effect: Option<EffectIntent>,
        run_state: Option<RunState>,
    ) -> Result<ApprovalQuestionId, ApprovalQuestionError> {
        if run_state.is_some_and(|state| state == RunState::Cancelling || state.is_terminal()) {
            return Err(ApprovalQuestionError::RunNotAwaitingApproval {
                run: spec.parties.run,
            });
        }
        if self.questions.len() >= MAX_APPROVAL_QUESTIONS {
            return Err(ApprovalQuestionError::CapacityReached {
                capacity: MAX_APPROVAL_QUESTIONS,
            });
        }
        let Some(effect) = effect else {
            return Err(ApprovalQuestionError::EffectNotTracked {
                operation: spec.effect.operation,
            });
        };
        if effect.run != spec.parties.run
            || effect.tenant != spec.parties.tenant
            || effect.operation != spec.effect.operation
        {
            return Err(ApprovalQuestionError::EffectNotTracked {
                operation: spec.effect.operation,
            });
        }
        let Some(client) = client else {
            return Err(ApprovalQuestionError::NoInteractiveClient {
                operation: spec.effect.operation,
            });
        };
        if client != spec.parties.respondent {
            return Err(ApprovalQuestionError::ClientIsNotRespondent {
                expected: spec.parties.respondent,
                got: client,
            });
        }
        let next_id = self
            .next_id
            .checked_add(1)
            .ok_or(ApprovalQuestionError::IdentityExhausted)?;
        let id = ApprovalQuestionId(self.next_id);
        self.next_id = next_id;
        self.questions.insert(
            id,
            Question {
                spec,
                status: ApprovalQuestionStatus::Pending,
                answer: None,
            },
        );
        Ok(id)
    }

    /// Queues an answer only when every respondent and effect binding matches.
    pub(crate) fn answer(&mut self, answer: ApprovalAnswer) -> Result<(), ApprovalQuestionError> {
        let ApprovalAnswer {
            question: id,
            tenant,
            respondent,
            effect,
            decision,
        } = answer;
        let question = self
            .questions
            .get_mut(&id)
            .ok_or(ApprovalQuestionError::UnknownQuestion { question: id })?;
        if question.status != ApprovalQuestionStatus::Pending {
            return Err(ApprovalQuestionError::AlreadyResolved { question: id });
        }
        if question.spec.parties.tenant != tenant
            || question.spec.parties.respondent != respondent
            || question.spec.effect != effect
        {
            return Err(ApprovalQuestionError::BindingMismatch { question: id });
        }
        let decision = match decision {
            ApprovalDecision::Approve(scopes) => {
                Self::validate_scopes(id, &question.spec.allowed_scopes, &scopes)?;
                ApprovalDecision::Approve(scopes)
            }
            ApprovalDecision::Reject => ApprovalDecision::Reject,
        };
        if question.answer.is_some() {
            return Err(ApprovalQuestionError::AlreadyResolved { question: id });
        }
        question.answer = Some(decision);
        Ok(())
    }

    /// Refuses oversized or out-of-schema approved scopes before queueing.
    fn validate_scopes(
        question: ApprovalQuestionId,
        allowed: &[Scope],
        scopes: &[Scope],
    ) -> Result<(), ApprovalQuestionError> {
        if scopes.len() > MAX_APPROVAL_ANSWER_SCOPES {
            return Err(ApprovalQuestionError::ScopeListTooLarge {
                len: scopes.len(),
                capacity: MAX_APPROVAL_ANSWER_SCOPES,
            });
        }
        if let Some(scope) = scopes.iter().find(|scope| !allowed.contains(scope)) {
            return Err(ApprovalQuestionError::ScopeOutsideQuestion {
                question,
                scope: *scope,
            });
        }
        Ok(())
    }

    /// Returns one question state without consuming it.
    pub(crate) fn status(
        &self,
        id: ApprovalQuestionId,
    ) -> Result<&ApprovalQuestionStatus, ApprovalQuestionError> {
        self.questions
            .get(&id)
            .map(|question| &question.status)
            .ok_or(ApprovalQuestionError::UnknownQuestion { question: id })
    }

    /// Settles independent questions against one pinned tick instant.
    pub(crate) fn settle(&mut self, at: SystemTime, cancelled: &[RunId]) {
        for question in self.questions.values_mut() {
            if question.status != ApprovalQuestionStatus::Pending {
                continue;
            }
            if cancelled.contains(&question.spec.parties.run) {
                question.status = ApprovalQuestionStatus::Cancelled;
                question.answer = None;
                continue;
            }
            if at >= question.spec.expires_at {
                question.status = ApprovalQuestionStatus::TimedOut;
                question.answer = None;
                continue;
            }
            match question.answer.take() {
                Some(ApprovalDecision::Approve(mut scopes)) => {
                    scopes.sort_unstable();
                    scopes.dedup();
                    question.status = ApprovalQuestionStatus::Approved(scopes);
                }
                Some(ApprovalDecision::Reject) => {
                    question.status = ApprovalQuestionStatus::Rejected
                }
                None => {}
            }
        }
    }
}
