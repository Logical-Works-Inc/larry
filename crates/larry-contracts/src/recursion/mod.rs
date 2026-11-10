//! The recursion budget identity as a reducer (larry#62 part 1, ADR 0031).
//!
//! Harness² checks its budget (R = 4k rollouts, P = 3k sessions) once, at the end, with an
//! exception. Here the identity is a property of a pure ledger that refuses the step that would
//! break it, before it is counted.
//!
//! Larry's reading. One **round** is two proposer sessions (two candidate harnesses), four rollouts
//! (each candidate probed twice on the pinned task) and one composer session: three sessions and four
//! rollouts. **Parallel** mode runs `k` independent rounds from the plan's base harness (2k
//! candidates, k compositions). **Sequential** mode runs `k` nested rounds, and round `i + 1` starts
//! from round `i`'s composed harness. Both modes total R = 4k rollouts and P = 3k sessions, for `k`
//! in 1 to 8. A proposer retry after an invalid proposal is the same session asked again with the
//! typed error, so it is not counted twice.
//!
//! Dependencies: a probe waits for its candidate's proposal; a composer waits until all four probes
//! of its round are settled (succeeded or failed); in sequential mode a round's proposers wait for
//! the previous round's composer. A proposal or composition that fails falls back to the round's base
//! harness. An `Unknown` outcome is ambiguous: it is never counted as success, never rescheduled,
//! and blocks its dependents until a reconciliation settles it. At most `probe_width` probes are
//! open (in flight or ambiguous) at once; overflow waits and is never dropped. Cancelling stops new
//! steps and lets open ones settle.
//!
//! The ledger is the fold of its own [`RecursionFact`]s. [`RecursionLedger::replay`] over the
//! recorded facts equals the live ledger, so part 2 can journal the facts and re-derive the ledger.
//! Nothing here reads a clock, iterates a hash map or starts a thread.

mod ledger;

pub use ledger::{
    ComposeInput, Label, RecursionError, RecursionFact, RecursionLedger, Refusal, Stage, Step,
    StepOutcome, StepState,
};

use crate::harness::HarnessHash;
use crate::ids::ContentDigest;

/// The most rounds (and so the largest `k`) a plan may declare.
pub const MAX_K: u8 = 8;
/// The widest probe fan-out a tenant may declare.
pub const MAX_PROBE_WIDTH: usize = 16;
/// Rollouts per round: two candidates probed twice each.
pub const ROLLOUTS_PER_ROUND: usize = 4;
/// Sessions per round: two proposers and one composer.
pub const SESSIONS_PER_ROUND: usize = 3;

/// How the rounds relate.
#[cfg_attr(
    feature = "serde",
    derive(lgwks_std::json::serde::Serialize, lgwks_std::json::serde::Deserialize)
)]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum RecursionMode {
    /// `k` independent rounds from the base harness.
    Parallel,
    /// `k` nested rounds; each starts from the previous round's composition.
    Sequential,
}

impl RecursionMode {
    /// Stable canonical tag; independent of Rust enum layout.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Parallel => 0,
            Self::Sequential => 1,
        }
    }
}

/// Why a plan was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PlanError {
    /// `k` is outside 1 to [`MAX_K`].
    #[error("k must be between 1 and {MAX_K}, got {got}")]
    Rounds {
        /// The k offered.
        got: u8,
    },
    /// The probe width is outside 1 to [`MAX_PROBE_WIDTH`].
    #[error("probe width must be between 1 and {MAX_PROBE_WIDTH}, got {got}")]
    Width {
        /// The width offered.
        got: usize,
    },
}

/// The declared shape of one recursion over one pinned task.
#[cfg_attr(
    feature = "serde",
    derive(lgwks_std::json::serde::Serialize, lgwks_std::json::serde::Deserialize)
)]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecursionPlan {
    /// Digest of the pinned task input.
    task: ContentDigest,
    /// How the rounds relate.
    mode: RecursionMode,
    /// The number of rounds, the plan's k.
    rounds: u8,
    /// The harness the first round starts from.
    base: HarnessHash,
    /// The most probes open at once.
    probe_width: usize,
}

impl RecursionPlan {
    /// Declares a recursion.
    ///
    /// # Errors
    ///
    /// [`PlanError`] for `k` or the probe width out of bounds.
    pub const fn new(
        task: ContentDigest,
        mode: RecursionMode,
        rounds: u8,
        base: HarnessHash,
        probe_width: usize,
    ) -> Result<Self, PlanError> {
        if rounds == 0 || rounds > MAX_K {
            return Err(PlanError::Rounds { got: rounds });
        }
        if probe_width == 0 || probe_width > MAX_PROBE_WIDTH {
            return Err(PlanError::Width { got: probe_width });
        }
        Ok(Self {
            task,
            mode,
            rounds,
            base,
            probe_width,
        })
    }

    /// The pinned task input digest.
    #[must_use]
    pub const fn task(&self) -> ContentDigest {
        self.task
    }

    /// The mode.
    #[must_use]
    pub const fn mode(&self) -> RecursionMode {
        self.mode
    }

    /// The number of rounds.
    #[must_use]
    pub const fn rounds(&self) -> u8 {
        self.rounds
    }

    /// The base harness.
    #[must_use]
    pub const fn base(&self) -> HarnessHash {
        self.base
    }

    /// The probe width.
    #[must_use]
    pub const fn probe_width(&self) -> usize {
        self.probe_width
    }

    /// The rollouts the identity allows: 4k.
    #[must_use]
    pub fn rollout_limit(&self) -> usize {
        usize::from(self.rounds).saturating_mul(ROLLOUTS_PER_ROUND)
    }

    /// The sessions the identity allows: 3k.
    #[must_use]
    pub fn session_limit(&self) -> usize {
        usize::from(self.rounds).saturating_mul(SESSIONS_PER_ROUND)
    }

    /// The idempotency key of a step: task, mode, k and label, as canonical bytes hashed.
    #[must_use]
    pub fn step_key(&self, label: Label) -> ContentDigest {
        ContentDigest::over(&[
            self.task.as_bytes(),
            &[self.mode.tag(), self.rounds],
            &[label.stage().tag(), label.round(), label.slot()],
        ])
    }
}

impl RecursionPlan {
    /// The plan again, through the constructor's bounds. A plan decoded from a journal is checked
    /// with this before it is trusted.
    ///
    /// # Errors
    ///
    /// [`PlanError`] for `k` or the probe width out of bounds.
    pub const fn validated(self) -> Result<Self, PlanError> {
        Self::new(
            self.task,
            self.mode,
            self.rounds,
            self.base,
            self.probe_width,
        )
    }

    /// The plan's identity: what a tenant's ledger is keyed by.
    #[must_use]
    pub fn identity(&self) -> ContentDigest {
        ContentDigest::over(&[
            b"larry.recursion.plan.v1\0",
            self.task.as_bytes(),
            &[self.mode.tag(), self.rounds],
            self.base.as_bytes(),
            &self.probe_width.to_be_bytes(),
        ])
    }
}

/// A runtime step's reference into its plan: which step, which attempt, and the harness hashes it
/// reads or declares. Carried by a work proposal and recorded with its intent.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(lgwks_std::json::serde::Serialize, lgwks_std::json::serde::Deserialize)
)]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub struct RecursionStep {
    /// The plan.
    pub plan: RecursionPlan,
    /// Which step.
    pub label: Label,
    /// 0, or 1 for the retry of an invalid proposal.
    pub attempt: u8,
    /// For a probe, the candidate harness it runs; it must be the one its proposer settled on.
    pub candidate: Option<HarnessHash>,
    /// For a proposer or composer, the harness the step declares it produces.
    pub produces: Option<HarnessHash>,
}

impl RecursionStep {
    /// A reference to `label` of `plan`.
    #[must_use]
    pub const fn new(plan: RecursionPlan, label: Label, attempt: u8) -> Self {
        Self {
            plan,
            label,
            attempt,
            candidate: None,
            produces: None,
        }
    }

    /// Names the candidate a probe runs.
    #[must_use]
    pub const fn with_candidate(mut self, candidate: HarnessHash) -> Self {
        self.candidate = Some(candidate);
        self
    }

    /// Names the harness a proposer or composer produces.
    #[must_use]
    pub const fn producing(mut self, produced: HarnessHash) -> Self {
        self.produces = Some(produced);
        self
    }
}
