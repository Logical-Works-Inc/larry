//! Typed failures of the runtime.
//!
//! Every variant is a refused operation with structured context; there is no
//! `String` error and no panic path. `Display` and `std::error::Error` are
//! written by hand rather than derived: `thiserror`'s register row in
//! `contract/APPROVED.toml` names `rocco-contracts` as its only
//! `allowed_consumers` entry, and widening that is a Director sign-off
//! rather than a local edit (see the crate manifest).

use core::fmt;

use rocco_contracts::error::{
    AuthorityError, CommandError, OrderingError, SequenceError, TransitionError,
};
use rocco_contracts::ids::{OperationId, RunId};
use rocco_contracts::replay::DenialReason;
use rocco_contracts::state::RunState;

/// Every way the runtime refuses an operation.
#[derive(Debug)]
#[non_exhaustive]
pub enum RuntimeError {
    /// The bounded journal is at capacity. Admission is backpressured; no
    /// recorded fact is ever silently dropped (Cell 05).
    JournalFull {
        /// The configured entry ceiling.
        capacity: usize,
    },
    /// The run table is at capacity.
    RunLimitReached {
        /// The configured run ceiling.
        capacity: usize,
    },
    /// The ready-work queue is at capacity; the caller must drain a tick
    /// before submitting more (Cell 05: 256 descriptors).
    ReadyWorkFull {
        /// The configured ceiling.
        capacity: usize,
    },
    /// A run already holds the maximum number of recorded effects.
    EffectLimitReached {
        /// The run that is full.
        run: RunId,
        /// The configured ceiling.
        capacity: usize,
    },
    /// Per-run event sequencing was violated.
    Sequence(SequenceError),
    /// A command or event violated an ordering constraint.
    Ordering(OrderingError),
    /// Duplicate-suppression or identity validation failed.
    Command(CommandError),
    /// The finite run-state algebra refused the transition.
    Transition(TransitionError),
    /// The effect-boundary authority re-validation failed.
    Authority(AuthorityError),
    /// A journal payload could not be encoded or decoded.
    Codec(lgwks_std::json::Error),
    /// The pure authorization reducer denied the effect (D36).
    Denied {
        /// The stable operation ID that was refused.
        operation: OperationId,
        /// The deterministic first-failing reason.
        reason: DenialReason,
    },
    /// The run is not present in the projection.
    RunNotTracked {
        /// The untracked run.
        run: RunId,
    },
    /// Dispatch was attempted for an effect with no durable intent on
    /// record. Reservation must be persisted *before* dispatch, so that a
    /// crash in the dispatch window leaves evidence the effect may have
    /// happened (Cell 04).
    IntentNotDurable {
        /// The operation whose intent is missing.
        operation: OperationId,
    },
    /// The operation identity was already reserved in this runtime.
    DuplicateOperation {
        /// The repeated operation.
        operation: OperationId,
    },
    /// The observed outcome cannot terminate the effect under its declared
    /// strategy: it stays open for status query or bounded retry, and is
    /// never declared a victory or a failure (R72, E56).
    UnresolvableOutcome {
        /// The still-open operation.
        operation: OperationId,
    },
    /// A terminal receipt already exists for this operation; the receipt is
    /// immutable and later evidence is a superseding resolution, never an
    /// in-place rewrite (Cell 04).
    ReceiptAlreadyPublished {
        /// The already-closed operation.
        operation: OperationId,
    },
    /// Folding the journal re-derived a state that differs from the state
    /// the journal recorded. The journal is authoritative and self-checking:
    /// a mismatch is surfaced, never silently accepted (E55).
    FoldDivergence {
        /// The run whose fold diverged.
        run: RunId,
        /// The state the recorded event claimed.
        recorded: RunState,
        /// The state the pure algebra re-derived.
        rederived: RunState,
    },
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::JournalFull { capacity } => {
                write!(
                    f,
                    "journal is at capacity {capacity}; admission is backpressured"
                )
            }
            Self::RunLimitReached { capacity } => {
                write!(f, "run table is at capacity {capacity}")
            }
            Self::ReadyWorkFull { capacity } => {
                write!(
                    f,
                    "ready-work queue is at capacity {capacity}; drain a tick first"
                )
            }
            Self::EffectLimitReached { run, capacity } => {
                write!(
                    f,
                    "run {run} already holds the maximum of {capacity} recorded effects"
                )
            }
            Self::Sequence(err) => write!(f, "event sequencing refused: {err}"),
            Self::Ordering(err) => write!(f, "record ordering refused: {err}"),
            Self::Command(err) => write!(f, "command refused: {err}"),
            Self::Transition(err) => write!(f, "run-state transition refused: {err}"),
            Self::Authority(err) => write!(f, "effect-boundary authority refused: {err}"),
            Self::Codec(err) => write!(f, "journal payload codec failed: {err}"),
            Self::Denied { operation, reason } => {
                write!(f, "effect {operation} denied: {reason:?}")
            }
            Self::RunNotTracked { run } => write!(f, "run {run} is not tracked"),
            Self::IntentNotDurable { operation } => {
                write!(
                    f,
                    "operation {operation} has no durable intent; dispatch refused"
                )
            }
            Self::DuplicateOperation { operation } => {
                write!(f, "operation {operation} was already reserved")
            }
            Self::UnresolvableOutcome { operation } => write!(
                f,
                "operation {operation}'s outcome stays open: not a success, not a failure"
            ),
            Self::ReceiptAlreadyPublished { operation } => write!(
                f,
                "operation {operation} already has an immutable terminal receipt"
            ),
            Self::FoldDivergence {
                run,
                recorded,
                rederived,
            } => write!(
                f,
                "run {run} fold diverged: journal recorded {recorded}, algebra re-derived {rederived}"
            ),
        }
    }
}

impl std::error::Error for RuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sequence(err) => Some(err),
            Self::Ordering(err) => Some(err),
            Self::Command(err) => Some(err),
            Self::Transition(err) => Some(err),
            Self::Authority(err) => Some(err),
            Self::Codec(err) => Some(err),
            Self::JournalFull { .. }
            | Self::RunLimitReached { .. }
            | Self::ReadyWorkFull { .. }
            | Self::EffectLimitReached { .. }
            | Self::Denied { .. }
            | Self::RunNotTracked { .. }
            | Self::IntentNotDurable { .. }
            | Self::DuplicateOperation { .. }
            | Self::UnresolvableOutcome { .. }
            | Self::ReceiptAlreadyPublished { .. }
            | Self::FoldDivergence { .. } => None,
        }
    }
}

impl From<SequenceError> for RuntimeError {
    fn from(err: SequenceError) -> Self {
        Self::Sequence(err)
    }
}

impl From<OrderingError> for RuntimeError {
    fn from(err: OrderingError) -> Self {
        Self::Ordering(err)
    }
}

impl From<CommandError> for RuntimeError {
    fn from(err: CommandError) -> Self {
        Self::Command(err)
    }
}

impl From<TransitionError> for RuntimeError {
    fn from(err: TransitionError) -> Self {
        Self::Transition(err)
    }
}

impl From<AuthorityError> for RuntimeError {
    fn from(err: AuthorityError) -> Self {
        Self::Authority(err)
    }
}

impl From<lgwks_std::json::Error> for RuntimeError {
    fn from(err: lgwks_std::json::Error) -> Self {
        Self::Codec(err)
    }
}
