//! The ledger: a pure fold of recursion facts that refuses what would break the identity.

use std::collections::BTreeMap;

use crate::evidence::ImproverContext;
use crate::harness::{HarnessError, HarnessHash};

use super::{RecursionMode, RecursionPlan};

/// What kind of step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Stage {
    /// A proposer session producing one candidate harness.
    Propose,
    /// One rollout of a candidate on the pinned task.
    Probe,
    /// The composer session producing the round's refined harness.
    Compose,
}

impl Stage {
    /// Stable canonical tag; independent of Rust enum layout.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Propose => 0,
            Self::Probe => 1,
            Self::Compose => 2,
        }
    }
}

/// A step's identity inside a plan: round, stage and slot. Propose slots are 0 and 1 (the
/// candidate), probe slots are 0 to 3 (candidate `slot >> 1`, repeat `slot & 1`), the compose slot is
/// 0. Ordered by round, then stage, then slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Label {
    /// The round, from 0.
    round: u8,
    /// The stage.
    stage: Stage,
    /// The slot inside the stage.
    slot: u8,
}

impl Label {
    /// A label; whether the plan contains it is the ledger's question.
    #[must_use]
    pub const fn new(round: u8, stage: Stage, slot: u8) -> Self {
        Self { round, stage, slot }
    }

    /// The round.
    #[must_use]
    pub const fn round(self) -> u8 {
        self.round
    }

    /// The stage.
    #[must_use]
    pub const fn stage(self) -> Stage {
        self.stage
    }

    /// The slot.
    #[must_use]
    pub const fn slot(self) -> u8 {
        self.slot
    }
}

/// What an executor or session reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StepOutcome {
    /// It succeeded. A proposer or composer produced a harness; a probe produced none.
    Succeeded {
        /// The harness hash produced.
        produced: Option<HarnessHash>,
    },
    /// It definitely failed.
    Failed,
    /// It may or may not have happened; never counted as success.
    Unknown,
}

/// Where one step stands.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StepState {
    /// An intent is recorded and no outcome is. After a crash it stays here: the step may have run.
    InFlight {
        /// 0 for the first try, 1 for the retry.
        attempt: u8,
    },
    /// The proposal was refused once; one retry carrying `error` may be scheduled.
    RetryPending {
        /// The typed refusal from the harness reducer.
        error: HarnessError,
    },
    /// The retry was refused too; the fallback is the next fact.
    AwaitingFallback {
        /// The second refusal.
        error: HarnessError,
    },
    /// It succeeded.
    Done {
        /// The hash it produced, for a proposer or composer.
        produced: Option<HarnessHash>,
    },
    /// It failed.
    Failed,
    /// The proposal fell back to the round's base harness.
    FellBack,
    /// The outcome is unknown and awaits reconciliation.
    Ambiguous,
}

impl StepState {
    /// Whether the step is finished for its dependents.
    #[must_use]
    pub const fn is_settled(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Failed | Self::FellBack)
    }
}

/// A step ready to be scheduled.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Step {
    /// Which step.
    pub label: Label,
    /// 0, or 1 for the retry of an invalid proposal.
    pub attempt: u8,
    /// On a retry, the refusal the proposer is asked to correct.
    pub carried: Option<HarnessError>,
}

/// One recorded fact; the ledger is their fold.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RecursionFact {
    /// A step was authorized and reserved before it ran.
    IntentRecorded {
        /// Which step.
        label: Label,
        /// 0, or 1 for the retry.
        attempt: u8,
    },
    /// A step reported.
    OutcomeRecorded {
        /// Which step.
        label: Label,
        /// What it reported.
        outcome: StepOutcome,
    },
    /// A proposal failed the harness reducer.
    ProposalRefused {
        /// Which proposer step.
        label: Label,
        /// The typed refusal.
        error: HarnessError,
    },
    /// After the retry was refused too, the candidate is the round's base harness.
    FellBack {
        /// Which proposer step.
        label: Label,
    },
    /// New steps stop.
    Cancelled,
}

/// What recording a refusal led to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// One retry carrying the error is now schedulable.
    Retry,
    /// The retry was refused too and the fallback was recorded.
    FellBack,
}

/// Why the ledger refused a fact.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RecursionError {
    /// The recursion was cancelled; no new step starts.
    #[error("the recursion is cancelled")]
    Cancelled,
    /// The label already has an intent; it is refused before it is counted.
    #[error("label already recorded")]
    DuplicateLabel {
        /// The label.
        label: Label,
    },
    /// Rollout 4k + 1.
    #[error("the {limit} rollouts of the identity are already scheduled")]
    RolloutBudget {
        /// The limit, 4k.
        limit: usize,
    },
    /// Session 3k + 1.
    #[error("the {limit} sessions of the identity are already scheduled")]
    SessionBudget {
        /// The limit, 3k.
        limit: usize,
    },
    /// The plan has no such step.
    #[error("the plan contains no such step")]
    NotInPlan {
        /// The label.
        label: Label,
    },
    /// A step it depends on has not settled.
    #[error("a step this one depends on has not settled")]
    DependencyUnsettled {
        /// The label.
        label: Label,
    },
    /// The probe width is full.
    #[error("{limit} probes are already open")]
    WidthExceeded {
        /// The declared width.
        limit: usize,
    },
    /// The step has no open attempt to report on.
    #[error("the step is not in flight")]
    NotInFlight {
        /// The label.
        label: Label,
    },
    /// A retry was recorded for a step that is not awaiting one.
    #[error("the step is not awaiting a retry")]
    NotAwaitingRetry {
        /// The label.
        label: Label,
    },
    /// Only a proposer step can have a proposal refused.
    #[error("only a proposer step has a proposal")]
    NotProposal {
        /// The label.
        label: Label,
    },
    /// A fallback was recorded for a step that has not been refused twice.
    #[error("the step has not been refused twice")]
    NotAwaitingFallback {
        /// The label.
        label: Label,
    },
    /// A proposer or composer must report a hash and a probe must not.
    #[error("the produced hash does not fit the stage")]
    ProducedMismatch {
        /// The label.
        label: Label,
    },
    /// The composer's input is not for a compose step whose probes have settled.
    #[error("not a composable step")]
    NotComposable {
        /// The label.
        label: Label,
    },
}

/// What a composer receives: its round's base harness and an improver context, which by #63 cannot
/// hold grader material. Composition itself is not built here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeInput {
    /// The compose step.
    label: Label,
    /// The harness the round started from.
    base: HarnessHash,
    /// The scanned evidence.
    context: ImproverContext,
}

impl ComposeInput {
    /// The compose step.
    #[must_use]
    pub const fn label(&self) -> Label {
        self.label
    }

    /// The round's base harness.
    #[must_use]
    pub const fn base(&self) -> HarnessHash {
        self.base
    }

    /// The evidence the composer may see.
    #[must_use]
    pub const fn context(&self) -> &ImproverContext {
        &self.context
    }
}

/// The recursion's state: a fold of facts under a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecursionLedger {
    /// The declared plan.
    plan: RecursionPlan,
    /// Every step with an intent, in label order.
    steps: BTreeMap<Label, StepState>,
    /// Rollouts counted (first-attempt probe intents).
    rollouts: usize,
    /// Sessions counted (first-attempt proposer and composer intents).
    sessions: usize,
    /// Whether new steps have stopped.
    cancelled: bool,
    /// The facts folded so far, in order.
    facts: Vec<RecursionFact>,
}

impl RecursionLedger {
    /// An empty ledger under `plan`.
    #[must_use]
    pub const fn new(plan: RecursionPlan) -> Self {
        Self {
            plan,
            steps: BTreeMap::new(),
            rollouts: 0,
            sessions: 0,
            cancelled: false,
            facts: Vec::new(),
        }
    }

    /// Rebuilds the ledger by folding `facts` in order.
    ///
    /// # Errors
    ///
    /// The first [`RecursionError`] a fact is refused with.
    pub fn replay(plan: RecursionPlan, facts: &[RecursionFact]) -> Result<Self, RecursionError> {
        let mut ledger = Self::new(plan);
        for fact in facts {
            ledger.record(fact.clone())?;
        }
        Ok(ledger)
    }

    /// The plan.
    #[must_use]
    pub const fn plan(&self) -> &RecursionPlan {
        &self.plan
    }

    /// The facts folded so far.
    #[must_use]
    pub fn facts(&self) -> &[RecursionFact] {
        &self.facts
    }

    /// Rollouts counted so far.
    #[must_use]
    pub const fn rollouts(&self) -> usize {
        self.rollouts
    }

    /// Sessions counted so far.
    #[must_use]
    pub const fn sessions(&self) -> usize {
        self.sessions
    }

    /// Whether the recursion was cancelled.
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// Where a step stands, if it has an intent.
    #[must_use]
    pub fn state(&self, label: Label) -> Option<&StepState> {
        self.steps.get(&label)
    }

    /// Probes in flight or ambiguous: they hold an executor.
    #[must_use]
    pub fn open_probes(&self) -> usize {
        self.steps
            .iter()
            .filter(|entry| {
                entry.0.stage == Stage::Probe
                    && matches!(entry.1, StepState::InFlight { .. } | StepState::Ambiguous)
            })
            .count()
    }

    /// Whether every round's composer has settled.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        (0..self.plan.rounds()).all(|round| {
            self.steps
                .get(&Label::new(round, Stage::Compose, 0))
                .is_some_and(StepState::is_settled)
        })
    }

    /// The harness round `round` starts from.
    #[must_use]
    pub fn round_base(&self, round: u8) -> HarnessHash {
        let mut base = self.plan.base();
        if self.plan.mode() == RecursionMode::Sequential {
            for earlier in 0..round {
                base = self.composed_from(earlier, base);
            }
        }
        base
    }

    /// The harness round `round` composed, once its composer has settled.
    #[must_use]
    pub fn composed(&self, round: u8) -> Option<HarnessHash> {
        let state = self.steps.get(&Label::new(round, Stage::Compose, 0))?;
        state
            .is_settled()
            .then(|| self.composed_from(round, self.round_base(round)))
    }

    /// The composition of `round`, or `base` when its composer failed.
    fn composed_from(&self, round: u8, base: HarnessHash) -> HarnessHash {
        match self.steps.get(&Label::new(round, Stage::Compose, 0)) {
            Some(&StepState::Done {
                produced: Some(hash),
            }) => hash,
            _ => base,
        }
    }

    /// The candidate harness a probe of candidate `candidate` in `round` runs, once settled.
    #[must_use]
    pub fn candidate(&self, round: u8, candidate: u8) -> Option<HarnessHash> {
        match self
            .steps
            .get(&Label::new(round, Stage::Propose, candidate))?
        {
            &StepState::Done {
                produced: Some(hash),
            } => Some(hash),
            &StepState::Failed | &StepState::FellBack => Some(self.round_base(round)),
            _ => None,
        }
    }

    /// Whether the plan contains `label`.
    fn contains(&self, label: Label) -> bool {
        label.round < self.plan.rounds()
            && match label.stage {
                Stage::Propose => label.slot < 2,
                Stage::Probe => label.slot < 4,
                Stage::Compose => label.slot == 0,
            }
    }

    /// Whether the label's dependencies have settled.
    fn dependencies_settled(&self, label: Label) -> bool {
        let settled = |dep: Label| self.steps.get(&dep).is_some_and(StepState::is_settled);
        match label.stage {
            Stage::Propose => {
                label.round == 0
                    || self.plan.mode() == RecursionMode::Parallel
                    || settled(Label::new(label.round.saturating_sub(1), Stage::Compose, 0))
            }
            Stage::Probe => settled(Label::new(label.round, Stage::Propose, label.slot >> 1)),
            Stage::Compose => {
                (0..4).all(|slot| settled(Label::new(label.round, Stage::Probe, slot)))
            }
        }
    }

    /// The steps ready to schedule, in label order. Probes beyond the free width wait (backpressure)
    /// and appear again once a slot frees; nothing is dropped. A retry carries its refusal. Empty
    /// once cancelled. A step with an intent is never offered again.
    #[must_use]
    pub fn next_steps(&self) -> Vec<Step> {
        if self.cancelled {
            return Vec::new();
        }
        let mut free = self.plan.probe_width().saturating_sub(self.open_probes());
        let mut ready = Vec::new();
        for round in 0..self.plan.rounds() {
            for (stage, slots) in [
                (Stage::Propose, 2_u8),
                (Stage::Probe, 4),
                (Stage::Compose, 1),
            ] {
                for slot in 0..slots {
                    let label = Label::new(round, stage, slot);
                    if let Some(state) = self.steps.get(&label) {
                        if let StepState::RetryPending { ref error } = *state {
                            ready.push(Step {
                                label,
                                attempt: 1,
                                carried: Some(error.clone()),
                            });
                        }
                        continue;
                    }
                    if !self.dependencies_settled(label) {
                        continue;
                    }
                    if stage == Stage::Probe {
                        if free == 0 {
                            continue;
                        }
                        free = free.saturating_sub(1);
                    }
                    ready.push(Step {
                        label,
                        attempt: 0,
                        carried: None,
                    });
                }
            }
        }
        ready
    }

    /// The composer's input for `label`, whose probes must have settled.
    ///
    /// # Errors
    ///
    /// [`RecursionError::NotComposable`] for any other step.
    pub fn compose_input(
        &self,
        label: Label,
        context: ImproverContext,
    ) -> Result<ComposeInput, RecursionError> {
        if label.stage != Stage::Compose
            || !self.contains(label)
            || !self.dependencies_settled(label)
        {
            return Err(RecursionError::NotComposable { label });
        }
        Ok(ComposeInput {
            label,
            base: self.round_base(label.round),
            context,
        })
    }

    /// Records the intent to run a step, before it runs.
    ///
    /// # Errors
    ///
    /// A [`RecursionError`]; nothing is counted on refusal.
    pub fn record_intent(&mut self, label: Label, attempt: u8) -> Result<(), RecursionError> {
        self.record(RecursionFact::IntentRecorded { label, attempt })
    }

    /// Records what a step reported.
    ///
    /// # Errors
    ///
    /// A [`RecursionError`]; nothing changes on refusal.
    pub fn record_outcome(
        &mut self,
        label: Label,
        outcome: StepOutcome,
    ) -> Result<(), RecursionError> {
        self.record(RecursionFact::OutcomeRecorded { label, outcome })
    }

    /// Records that a proposal failed the harness reducer. The first refusal schedules one retry
    /// carrying `error`; the second records the fallback to the round's base harness.
    ///
    /// # Errors
    ///
    /// A [`RecursionError`]; nothing changes on refusal.
    pub fn record_refusal(
        &mut self,
        label: Label,
        error: HarnessError,
    ) -> Result<Refusal, RecursionError> {
        self.record(RecursionFact::ProposalRefused { label, error })?;
        if matches!(
            self.steps.get(&label),
            Some(&StepState::AwaitingFallback { .. })
        ) {
            self.record(RecursionFact::FellBack { label })?;
            return Ok(Refusal::FellBack);
        }
        Ok(Refusal::Retry)
    }

    /// Stops new steps; open ones still settle.
    pub fn cancel(&mut self) {
        self.cancelled = true;
        if self.facts.last() != Some(&RecursionFact::Cancelled) {
            self.facts.push(RecursionFact::Cancelled);
        }
    }

    /// Folds one fact: the single rule set, used live and by replay.
    fn record(&mut self, fact: RecursionFact) -> Result<(), RecursionError> {
        match fact {
            RecursionFact::IntentRecorded { label, attempt } => self.fold_intent(label, attempt)?,
            RecursionFact::OutcomeRecorded { label, outcome } => {
                self.fold_outcome(label, outcome)?
            }
            RecursionFact::ProposalRefused { label, ref error } => {
                self.fold_refusal(label, error)?;
            }
            RecursionFact::FellBack { label } => match self.steps.get(&label) {
                Some(&StepState::AwaitingFallback { .. }) => {
                    self.steps.insert(label, StepState::FellBack);
                }
                _ => return Err(RecursionError::NotAwaitingFallback { label }),
            },
            RecursionFact::Cancelled => self.cancelled = true,
        }
        self.facts.push(fact);
        Ok(())
    }

    /// Admits an intent: cancelled, duplicate, budget, plan, dependencies, width, in that order.
    fn fold_intent(&mut self, label: Label, attempt: u8) -> Result<(), RecursionError> {
        if self.cancelled {
            return Err(RecursionError::Cancelled);
        }
        if attempt > 0 {
            return match self.steps.get(&label) {
                Some(&StepState::RetryPending { .. }) if attempt == 1 => {
                    self.steps.insert(label, StepState::InFlight { attempt });
                    Ok(())
                }
                _ => Err(RecursionError::NotAwaitingRetry { label }),
            };
        }
        if self.steps.contains_key(&label) {
            return Err(RecursionError::DuplicateLabel { label });
        }
        if label.stage == Stage::Probe && self.rollouts >= self.plan.rollout_limit() {
            return Err(RecursionError::RolloutBudget {
                limit: self.plan.rollout_limit(),
            });
        }
        if label.stage != Stage::Probe && self.sessions >= self.plan.session_limit() {
            return Err(RecursionError::SessionBudget {
                limit: self.plan.session_limit(),
            });
        }
        if !self.contains(label) {
            return Err(RecursionError::NotInPlan { label });
        }
        if !self.dependencies_settled(label) {
            return Err(RecursionError::DependencyUnsettled { label });
        }
        if label.stage == Stage::Probe && self.open_probes() >= self.plan.probe_width() {
            return Err(RecursionError::WidthExceeded {
                limit: self.plan.probe_width(),
            });
        }
        if label.stage == Stage::Probe {
            self.rollouts = self.rollouts.saturating_add(1);
        } else {
            self.sessions = self.sessions.saturating_add(1);
        }
        self.steps.insert(label, StepState::InFlight { attempt: 0 });
        Ok(())
    }

    /// Settles an open or ambiguous step.
    fn fold_outcome(&mut self, label: Label, outcome: StepOutcome) -> Result<(), RecursionError> {
        if !matches!(
            self.steps.get(&label),
            Some(&(StepState::InFlight { .. } | StepState::Ambiguous))
        ) {
            return Err(RecursionError::NotInFlight { label });
        }
        let next = match outcome {
            StepOutcome::Succeeded { produced } => {
                if produced.is_some() == (label.stage == Stage::Probe) {
                    return Err(RecursionError::ProducedMismatch { label });
                }
                StepState::Done { produced }
            }
            StepOutcome::Failed => StepState::Failed,
            StepOutcome::Unknown => StepState::Ambiguous,
        };
        self.steps.insert(label, next);
        Ok(())
    }

    /// Moves an open proposer step to its retry or its fallback.
    fn fold_refusal(&mut self, label: Label, error: &HarnessError) -> Result<(), RecursionError> {
        if label.stage != Stage::Propose {
            return Err(RecursionError::NotProposal { label });
        }
        let next = match self.steps.get(&label) {
            Some(&StepState::InFlight { attempt: 0 }) => StepState::RetryPending {
                error: error.clone(),
            },
            Some(&StepState::InFlight { .. }) => StepState::AwaitingFallback {
                error: error.clone(),
            },
            _ => return Err(RecursionError::NotInFlight { label }),
        };
        self.steps.insert(label, next);
        Ok(())
    }
}
