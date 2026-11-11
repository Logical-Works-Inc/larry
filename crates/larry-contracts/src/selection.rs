//! Blind, seeded selection of the final harness (larry#64 part 1, ADR 0032).
//!
//! After a recursion settles, the delivered harnesses are shuffled to anonymous [`CandidateId`]s by a
//! seed that depends only on (task, mode, k, tenant), so replay picks the same winner. The selector
//! sees an [`ImproverContext`] (grader-blind by #63) and the anonymous ids, never a hash. Its answer is
//! an input value: no model is called here. A missing, unknown or empty answer, or a selector failure,
//! is a recorded non-answer and falls back to the candidate the seed put first. Nothing raises.
//!
//! Candidates. Parallel mode delivers each round's composition. Sequential mode delivers each round's
//! starting harness and the last composition. Duplicate hashes count once, in first-seen order.

use std::collections::BTreeSet;

use crate::evidence::ImproverContext;
use crate::harness::HarnessHash;
use crate::ids::{ContentDigest, TenantId};
use crate::recursion::{RecursionLedger, RecursionMode};

/// The most candidates a selection holds: nine rounds' worth in sequential mode.
pub const MAX_CANDIDATES: usize = 9;
/// The longest selector reason in bytes.
pub const MAX_SELECTION_REASON_BYTES: usize = 512;

/// An anonymous candidate, numbered by the seeded shuffle; it says nothing of its origin.
#[cfg_attr(
    feature = "serde",
    derive(lgwks_std::json::serde::Serialize, lgwks_std::json::serde::Deserialize)
)]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CandidateId(u8);

impl CandidateId {
    /// An id as a selector's answer would name it.
    #[must_use]
    pub const fn new(value: u8) -> Self {
        Self(value)
    }

    /// The raw number.
    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }
}

/// Why a selection could not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SelectionError {
    /// The recursion has not settled every round, so there is nothing delivered to choose from.
    #[error("the recursion has not settled")]
    NotSettled,
    /// This plan already has its one selection (recorded or in flight).
    #[error("the plan already has a selection")]
    AlreadySelected,
}

/// What the selector said.
#[cfg_attr(
    feature = "serde",
    derive(lgwks_std::json::serde::Serialize, lgwks_std::json::serde::Deserialize)
)]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SelectorAnswer {
    /// It chose a candidate and gave a reason.
    Chose {
        /// The anonymous id it named.
        id: CandidateId,
        /// Why it won.
        reason: String,
    },
    /// It gave no answer.
    NoAnswer,
    /// The selector failed.
    Failed,
}

/// Why an answer was not used.
#[cfg_attr(
    feature = "serde",
    derive(lgwks_std::json::serde::Serialize, lgwks_std::json::serde::Deserialize)
)]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NonAnswer {
    /// No answer.
    NoAnswer,
    /// It named an id that is not a candidate.
    UnknownId,
    /// It named a candidate and gave no reason.
    EmptyReason,
    /// Its reason is longer than allowed.
    ReasonTooLong,
    /// The selector failed.
    SelectorFailed,
}

/// Why the winner won.
#[cfg_attr(
    feature = "serde",
    derive(lgwks_std::json::serde::Serialize, lgwks_std::json::serde::Deserialize)
)]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SelectionReason {
    /// The selector's own reason.
    Selector(String),
    /// The answer was dropped for this reason and the seed's first candidate won.
    Fallback(NonAnswer),
}

/// What the selector may see: evidence it may hold and the anonymous ids, never a hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorInput<'a> {
    /// The grader-blind evidence.
    context: &'a ImproverContext,
    /// The anonymous candidates.
    ids: Vec<CandidateId>,
}

impl SelectorInput<'_> {
    /// The evidence.
    #[must_use]
    pub const fn context(&self) -> &ImproverContext {
        self.context
    }

    /// The candidates to choose among.
    #[must_use]
    pub fn ids(&self) -> &[CandidateId] {
        &self.ids
    }
}

/// A selection ready for the selector's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    /// The seed the shuffle came from.
    seed: ContentDigest,
    /// Anonymous id to delivered harness, in id order.
    mapping: Vec<(CandidateId, HarnessHash)>,
    /// The evidence the selector gets.
    context: ImproverContext,
}

/// The recorded outcome of a selection; replaying the same inputs yields the same value.
#[cfg_attr(
    feature = "serde",
    derive(lgwks_std::json::serde::Serialize, lgwks_std::json::serde::Deserialize)
)]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// The seed.
    seed: ContentDigest,
    /// The chosen harness.
    winner: HarnessHash,
    /// The anonymous mapping that was shown.
    mapping: Vec<(CandidateId, HarnessHash)>,
    /// Why it won.
    reason: SelectionReason,
}

/// The seed: BLAKE3 over a domain tag, the task digest, the mode, k and the tenant. No clock, no RNG.
#[must_use]
pub fn selection_seed(
    task: ContentDigest,
    mode: RecursionMode,
    rounds: u8,
    tenant: TenantId,
) -> ContentDigest {
    ContentDigest::over(&[
        b"larry.selection.seed.v1\0",
        task.as_bytes(),
        &[mode.tag(), rounds],
        tenant.as_bytes(),
    ])
}

/// The delivered harnesses of a settled ledger, deduplicated in first-seen order.
fn delivered(ledger: &RecursionLedger) -> Option<Vec<HarnessHash>> {
    if !ledger.is_complete() {
        return None;
    }
    let rounds = ledger.plan().rounds();
    let mut found = Vec::new();
    match ledger.plan().mode() {
        RecursionMode::Parallel => {
            for round in 0..rounds {
                found.push(ledger.composed(round)?);
            }
        }
        RecursionMode::Sequential => {
            for round in 0..rounds {
                found.push(ledger.round_base(round));
            }
            found.push(ledger.composed(rounds.saturating_sub(1))?);
        }
    }
    let mut seen = BTreeSet::new();
    found.retain(|hash| seen.insert(*hash));
    Some(found)
}

/// A deterministic Fisher-Yates permutation of `0..count` driven by the seed's bytes.
fn shuffled(seed: ContentDigest, count: usize) -> Vec<usize> {
    let mut state = seed
        .as_bytes()
        .first_chunk::<8>()
        .map_or(0, |bytes| u64::from_be_bytes(*bytes));
    let mut order: Vec<usize> = (0..count).collect();
    for position in (1..count).rev() {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = state;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^= mixed >> 31;
        let Ok(bound) = u64::try_from(position.saturating_add(1)) else {
            continue;
        };
        let Ok(pick) = usize::try_from((mixed >> 32).wrapping_mul(bound) >> 32) else {
            continue;
        };
        order.swap(position, pick);
    }
    order
}

/// Prepares the blind selection among the harnesses `ledger` delivered.
///
/// # Errors
///
/// [`SelectionError::NotSettled`] until every round has composed.
pub fn prepare(
    ledger: &RecursionLedger,
    tenant: TenantId,
    context: ImproverContext,
) -> Result<Prepared, SelectionError> {
    let hashes = delivered(ledger).ok_or(SelectionError::NotSettled)?;
    let plan = ledger.plan();
    let seed = selection_seed(plan.task(), plan.mode(), plan.rounds(), tenant);
    let mut mapping = Vec::new();
    for (number, source) in shuffled(seed, hashes.len()).into_iter().enumerate() {
        if let (Ok(id), Some(hash)) = (u8::try_from(number), hashes.get(source)) {
            mapping.push((CandidateId(id), *hash));
        }
    }
    Ok(Prepared {
        seed,
        mapping,
        context,
    })
}

impl Prepared {
    /// What the selector is shown: no hash among it.
    #[must_use]
    pub fn selector_input(&self) -> SelectorInput<'_> {
        SelectorInput {
            context: &self.context,
            ids: self.mapping.iter().map(|entry| entry.0).collect(),
        }
    }

    /// Applies the selector's answer. A non-answer is recorded and the seed's first candidate wins; a
    /// failure never raises.
    #[must_use]
    pub fn decide(self, answer: &SelectorAnswer) -> Selection {
        let named = match *answer {
            SelectorAnswer::Chose { id, ref reason } => {
                if reason.trim().is_empty() {
                    Err(NonAnswer::EmptyReason)
                } else if reason.len() > MAX_SELECTION_REASON_BYTES {
                    Err(NonAnswer::ReasonTooLong)
                } else {
                    self.mapping
                        .iter()
                        .find(|entry| entry.0 == id)
                        .map(|entry| (entry.1, reason.clone()))
                        .ok_or(NonAnswer::UnknownId)
                }
            }
            SelectorAnswer::NoAnswer => Err(NonAnswer::NoAnswer),
            SelectorAnswer::Failed => Err(NonAnswer::SelectorFailed),
        };
        let first = self.mapping.first().map(|entry| entry.1);
        let (winner, reason) = match (named, first) {
            (Ok((hash, why)), _) => (Some(hash), SelectionReason::Selector(why)),
            (Err(dropped), fallback) => (fallback, SelectionReason::Fallback(dropped)),
        };
        Selection {
            seed: self.seed,
            winner: winner.unwrap_or_else(|| self.seed_hash()),
            mapping: self.mapping,
            reason,
        }
    }

    /// Only reachable with an empty mapping, which `prepare` never builds from a settled ledger.
    fn seed_hash(&self) -> HarnessHash {
        crate::harness::HarnessTree::empty().hash()
    }
}

impl Selection {
    /// The seed.
    #[must_use]
    pub const fn seed(&self) -> ContentDigest {
        self.seed
    }

    /// The chosen harness.
    #[must_use]
    pub const fn winner(&self) -> HarnessHash {
        self.winner
    }

    /// The anonymous mapping that was shown, in id order.
    #[must_use]
    pub fn mapping(&self) -> &[(CandidateId, HarnessHash)] {
        &self.mapping
    }

    /// Why it won.
    #[must_use]
    pub const fn reason(&self) -> &SelectionReason {
        &self.reason
    }

    /// Whether the selector's answer was dropped and the seed chose.
    #[must_use]
    pub const fn is_fallback(&self) -> bool {
        matches!(self.reason, SelectionReason::Fallback(_))
    }
}

/// What a selector session carries into the runtime: the plan it selects for and the selector's
/// answer. The selector's evidence context shapes the answer but is not part of the result, so it is
/// not carried; the tenant is the run table's, never the proposal's.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(lgwks_std::json::serde::Serialize, lgwks_std::json::serde::Deserialize)
)]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub struct SelectionStep {
    /// The settled plan.
    pub plan: crate::recursion::RecursionPlan,
    /// What the selector said.
    pub answer: SelectorAnswer,
}

impl SelectionStep {
    /// A selection of `plan` by `answer`.
    #[must_use]
    pub const fn new(plan: crate::recursion::RecursionPlan, answer: SelectorAnswer) -> Self {
        Self { plan, answer }
    }
}
