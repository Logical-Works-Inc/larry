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
use rocco_contracts::ids::{OperationId, RunId, TenantId};
use rocco_contracts::replay::DenialReason;
use rocco_contracts::state::RunState;

/// Every way the runtime refuses an operation.
#[derive(Debug)]
#[non_exhaustive]
pub enum RuntimeError {
    /// No durable journal owner is attached; no acknowledgement is permitted.
    DurabilityUnavailable,
    /// Persistence failed ambiguously; reload the committed journal before writing.
    JournalRecoveryRequired,
    /// The host journal owner refused or could not confirm the append.
    JournalStorage(std::io::Error),
    /// An authorization decision is not understood; dispatch is refused.
    UnsupportedDecision(rocco_contracts::replay::Decision),
    /// A command kind is not understood; execution is refused.
    UnsupportedCommand(rocco_contracts::command::CommandKind),
    /// An event kind is not understood; replay is refused.
    UnsupportedEvent(rocco_contracts::event::EventKind),
    /// A disposition is not understood; no terminal receipt is published.
    UnsupportedDisposition(rocco_contracts::effect::TerminalDisposition),
    /// A reconciliation action is not understood; no owner interaction is permitted.
    UnsupportedReconciliationAction(rocco_contracts::effect::ReconciliationAction),
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
    /// Pending payload bytes exceed the Cell 05 admission ceiling.
    ReadyBytesFull {
        /// Maximum admitted pending bytes.
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
    /// A command envelope targeted a run owned by another tenant.
    RunTenantMismatch {
        /// The targeted run.
        run: RunId,
        /// Tenant established when the run was first admitted.
        expected: TenantId,
        /// Tenant carried by the untrusted command envelope.
        got: TenantId,
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
    /// Protocol negotiation or protocol-bound command ingress was refused.
    Protocol(crate::protocol::ProtocolError),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::DurabilityUnavailable => {
                f.write_str("durable admission requires a journal storage owner")
            }
            Self::JournalRecoveryRequired => {
                f.write_str("journal append is uncertain; recover before writing")
            }
            Self::JournalStorage(ref error) => write!(f, "journal storage refused: {error}"),
            Self::UnsupportedDecision(value) => {
                write!(f, "unsupported authorization decision: {value:?}")
            }
            Self::UnsupportedCommand(value) => write!(f, "unsupported command kind: {value:?}"),
            Self::UnsupportedEvent(value) => write!(f, "unsupported event kind: {value:?}"),
            Self::UnsupportedDisposition(value) => {
                write!(f, "unsupported terminal disposition: {value:?}")
            }
            Self::UnsupportedReconciliationAction(value) => {
                write!(f, "unsupported reconciliation action: {value:?}")
            }
            Self::JournalFull { capacity } => {
                write!(
                    f,
                    "journal is at capacity {capacity}; admission is backpressured"
                )
            }
            Self::RunLimitReached { capacity } => {
                write!(f, "run table is at capacity {capacity}")
            }
            Self::ReadyBytesFull { capacity } => write!(
                f,
                "pending payloads exceed {capacity} bytes; drain a tick first"
            ),
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
            Self::Sequence(ref err) => write!(f, "event sequencing refused: {err}"),
            Self::Ordering(ref err) => write!(f, "record ordering refused: {err}"),
            Self::Command(ref err) => write!(f, "command refused: {err}"),
            Self::Transition(ref err) => write!(f, "run-state transition refused: {err}"),
            Self::Authority(ref err) => write!(f, "effect-boundary authority refused: {err}"),
            Self::Codec(ref err) => write!(f, "journal payload codec failed: {err}"),
            Self::Denied { operation, reason } => {
                write!(f, "effect {operation} denied: {reason:?}")
            }
            Self::RunNotTracked { run } => write!(f, "run {run} is not tracked"),
            Self::RunTenantMismatch { run, expected, got } => {
                write!(
                    f,
                    "run {run} belongs to tenant {expected}; command carried tenant {got}"
                )
            }
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
            Self::Protocol(ref err) => write!(f, "protocol ingress refused: {err}"),
        }
    }
}

impl std::error::Error for RuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::JournalStorage(ref err) => Some(err),
            Self::DurabilityUnavailable | Self::JournalRecoveryRequired => None,
            Self::Sequence(ref err) => Some(err),
            Self::Ordering(ref err) => Some(err),
            Self::Command(ref err) => Some(err),
            Self::Transition(ref err) => Some(err),
            Self::Authority(ref err) => Some(err),
            Self::Codec(ref err) => Some(err),
            Self::Protocol(ref err) => Some(err),
            Self::UnsupportedDecision(_)
            | Self::UnsupportedCommand(_)
            | Self::UnsupportedEvent(_)
            | Self::UnsupportedDisposition(_)
            | Self::UnsupportedReconciliationAction(_)
            | Self::JournalFull { .. }
            | Self::RunLimitReached { .. }
            | Self::ReadyWorkFull { .. }
            | Self::ReadyBytesFull { .. }
            | Self::EffectLimitReached { .. }
            | Self::Denied { .. }
            | Self::RunNotTracked { .. }
            | Self::RunTenantMismatch { .. }
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

impl From<crate::protocol::ProtocolError> for RuntimeError {
    fn from(err: crate::protocol::ProtocolError) -> Self {
        Self::Protocol(err)
    }
}
