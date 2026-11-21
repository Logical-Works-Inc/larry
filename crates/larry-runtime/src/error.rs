//! Typed failures of the runtime.
//!
//! Every variant is a refused operation with structured context; there is no
//! `String` error and no panic path. `Display` and `std::error::Error` are
//! written by hand rather than derived: `thiserror`'s register row in
//! `contract/APPROVED.toml` names `larry-contracts` as its only
//! `allowed_consumers` entry, and widening that is a Director sign-off
//! rather than a local edit (see the crate manifest).

use core::fmt;

use larry_contracts::error::{
    AuthorityError, CommandError, OrderingError, SequenceError, TransitionError,
};
use larry_contracts::ids::{OperationId, RunId, TenantId};
use larry_contracts::replay::DenialReason;
use larry_contracts::state::RunState;

/// Every way the runtime refuses an operation.
#[derive(Debug)]
#[non_exhaustive]
pub enum RuntimeError {
    /// Display or resolved effect binding refused a reach.
    Binding(larry_contracts::binding::BindingError),
    /// An authorization decision is not understood; dispatch is refused.
    UnsupportedDecision(larry_contracts::replay::Decision),
    /// A command kind is not understood; execution is refused.
    UnsupportedCommand(larry_contracts::command::CommandKind),
    /// An event kind is not understood; replay is refused.
    UnsupportedEvent(larry_contracts::event::EventKind),
    /// A disposition is not understood; no terminal receipt is published.
    UnsupportedDisposition(larry_contracts::effect::TerminalDisposition),
    /// A reconciliation action is not understood; no owner interaction is permitted.
    UnsupportedReconciliationAction(larry_contracts::effect::ReconciliationAction),
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
    /// A configuration bound is zero or outside the range its meaning allows
    /// (larry#212). The runtime is not built: a bad setting is refused, never
    /// clamped into something the host did not ask for.
    BadSetting {
        /// The setting's name.
        name: &'static str,
        /// The value supplied and the range it must fall in.
        range: Box<SettingRange>,
    },
    /// A setting's value cannot be read in the unit its range is declared in,
    /// so no bound can judge it (larry#212).
    SettingUnreadable {
        /// The setting's name.
        name: &'static str,
        /// Why the reading failed.
        cause: Box<dyn std::error::Error + Send + Sync>,
    },
    /// An admitted command's payload is past the configured byte ceiling
    /// (larry#212). The check runs before the payload is decoded or
    /// journalled, so nothing hostile is held or recorded.
    PayloadTooLarge {
        /// The payload's length in bytes.
        bytes: usize,
        /// The configured ceiling.
        ceiling: usize,
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
    /// The clock's reading, placed after the Unix epoch, is past what the platform's
    /// `SystemTime` represents, so the tick has no instant to pin and runs no stage (E30).
    ClockOutOfRange {
        /// The reading since the epoch that could not be placed.
        elapsed: std::time::Duration,
    },
    /// The system clock reads before the Unix epoch, so no live reading can be placed after it
    /// and the runtime has no instant to pin (E30, larry#182).
    ClockBeforeEpoch(std::time::SystemTimeError),
    /// A Larry field cannot name an lgwks_bot effect key (larry#76, larry#78).
    EffectIdentity {
        /// The operation whose key could not be formed.
        operation: OperationId,
        /// `run`, `tenant` or `operation` when all-zero, or `admission_epoch`
        /// when it is past the last broker generation.
        field: &'static str,
    },
    /// The dispatch fence refused a change or a read (larry#76).
    Fence(crate::fence::FenceError),
    /// Protocol negotiation or protocol-bound command ingress was refused.
    Protocol(crate::protocol::ProtocolError),
    /// A question or its answer was refused before anything was written (E30).
    Question(crate::runtime::QuestionError),
    /// The journal's file refused (larry#75): another writer holds it, its
    /// committed bytes break the chain, a ceiling was reached, the device
    /// refused, or an append's outcome is unknown until a reopen.
    Durable(lgwks_bot::journal::JournalError),
    /// The journal's file holds a command identity twice, which no append
    /// this runtime made can produce (larry#75).
    DurableRepeat {
        /// The zero-based record that repeats an earlier one.
        index: usize,
    },
    /// The journal's file acknowledged an append under a promise weaker than
    /// surviving a process crash (larry#75).
    DurabilityShort {
        /// What the acknowledgment promised.
        promised: lgwks_bot::journal::DurabilityPromise,
    },
}

/// A setting's value and the range its meaning allows.
///
/// Boxed inside [`RuntimeError::BadSetting`] so one refused setting does not
/// widen every result that can carry a runtime error (larry#212).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SettingRange {
    /// The value supplied.
    pub value: u128,
    /// The smallest value the setting accepts.
    pub min: u128,
    /// The largest value the setting accepts.
    pub max: u128,
}

impl fmt::Display for RuntimeError {
    /// Variants that wrap a typed cause print here; every other variant
    /// continues to [`RuntimeError::fmt_limit`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Binding(ref err) => write!(f, "{err}"),
            Self::Sequence(ref err) => write!(f, "event sequencing refused: {err}"),
            Self::Ordering(ref err) => write!(f, "record ordering refused: {err}"),
            Self::Command(ref err) => write!(f, "command refused: {err}"),
            Self::Transition(ref err) => write!(f, "run-state transition refused: {err}"),
            Self::Authority(ref err) => write!(f, "effect-boundary authority refused: {err}"),
            Self::Codec(ref err) => write!(f, "journal payload codec failed: {err}"),
            Self::Fence(ref err) => write!(f, "dispatch fence refused: {err}"),
            Self::Protocol(ref err) => write!(f, "protocol ingress refused: {err}"),
            Self::Question(ref err) => write!(f, "question refused: {err}"),
            Self::Durable(ref err) => write!(f, "journal file refused: {err}"),
            Self::SettingUnreadable { name, ref cause } => {
                write!(f, "runtime setting {name} cannot be read: {cause}")
            }
            Self::ClockBeforeEpoch(ref cause) => {
                write!(f, "the system clock reads before the Unix epoch: {cause}")
            }
            ref other => other.fmt_limit(f),
        }
    }
}

impl RuntimeError {
    /// The text of the unsupported-kind and capacity variants; every other
    /// variant continues to [`RuntimeError::fmt_detail`]. `source` below is
    /// the one exhaustive match, so a new variant cannot compile unvisited.
    fn fmt_limit(&self, sink: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::UnsupportedDecision(value) => {
                write!(sink, "unsupported authorization decision: {value:?}")
            }
            Self::UnsupportedCommand(value) => {
                write!(sink, "unsupported command kind: {value:?}")
            }
            Self::UnsupportedEvent(value) => write!(sink, "unsupported event kind: {value:?}"),
            Self::UnsupportedDisposition(value) => {
                write!(sink, "unsupported terminal disposition: {value:?}")
            }
            Self::UnsupportedReconciliationAction(value) => {
                write!(sink, "unsupported reconciliation action: {value:?}")
            }
            Self::JournalFull { capacity } => write!(
                sink,
                "journal is at capacity {capacity}; admission is backpressured"
            ),
            Self::RunLimitReached { capacity } => {
                write!(sink, "run table is at capacity {capacity}")
            }
            Self::ReadyWorkFull { capacity } => write!(
                sink,
                "ready-work queue is at capacity {capacity}; drain a tick first"
            ),
            Self::EffectLimitReached { run, capacity } => write!(
                sink,
                "run {run} already holds the maximum of {capacity} recorded effects"
            ),
            Self::BadSetting { name, ref range } => {
                let SettingRange { value, min, max } = **range;
                write!(
                    sink,
                    "runtime setting {name}={value} is outside {min}..={max}"
                )
            }
            Self::PayloadTooLarge { bytes, ceiling } => write!(
                sink,
                "command payload of {bytes} bytes exceeds the {ceiling}-byte ceiling"
            ),
            _ => self.fmt_detail(sink),
        }
    }

    /// The text of the identity, outcome and journal-file variants. A variant
    /// no arm describes prints its structured `Debug` form: never a wrong
    /// message, and never a `fmt::Error` that makes `to_string` panic.
    fn fmt_detail(&self, sink: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Denied { operation, reason } => {
                write!(sink, "effect {operation} denied: {reason:?}")
            }
            Self::RunNotTracked { run } => sink.write_fmt(format_args!("run {run} is not tracked")),
            Self::RunTenantMismatch { run, expected, got } => write!(
                sink,
                "run {run} belongs to tenant {expected}; command carried tenant {got}"
            ),
            Self::IntentNotDurable { operation } => write!(
                sink,
                "operation {operation} has no durable intent; dispatch refused"
            ),
            Self::DuplicateOperation { operation } => {
                write!(sink, "operation {operation} was already reserved")
            }
            Self::UnresolvableOutcome { operation } => write!(
                sink,
                "operation {operation}'s outcome stays open: not a success, not a failure"
            ),
            Self::ReceiptAlreadyPublished { operation } => write!(
                sink,
                "operation {operation} already has an immutable terminal receipt"
            ),
            Self::FoldDivergence {
                run,
                recorded,
                rederived,
            } => write!(
                sink,
                "run {run} fold diverged: journal recorded {recorded}, algebra re-derived {rederived}"
            ),
            Self::ClockOutOfRange { elapsed } => write!(
                sink,
                "clock reading {elapsed:?} after the Unix epoch is outside the platform's time range"
            ),
            Self::EffectIdentity { operation, field } => write!(
                sink,
                "operation {operation}'s {field} names no effect key: all-zero or past the last generation"
            ),
            Self::DurableRepeat { index } => write!(
                sink,
                "journal file record {index} repeats an accepted command; the file is not this runtime's"
            ),
            Self::DurabilityShort { promised } => write!(
                sink,
                "journal file promised {promised}, short of surviving a process crash"
            ),
            ref unrouted => write!(sink, "{unrouted:?}"),
        }
    }
}

impl RuntimeError {
    /// Whether this refusal ends the tick for every tenant (`true`) or belongs
    /// to the one command, effect or run being processed (`false`).
    ///
    /// Only the journal's own refusals and the instant the tick pins before any
    /// stage end a tick: a file that refused, a ceiling it reached, an outcome
    /// it cannot prove, a bad setting, and a clock that cannot be placed. Every
    /// other variant is one item's refusal: the tick records it in the
    /// [`crate::runtime::TickReport`] and goes on, so one tenant's refused
    /// command never skips reconciliation or cancellation for the others
    /// (larry#329). The match has no wildcard arm, so a new variant must be
    /// classified here before it compiles.
    pub(crate) const fn ends_the_tick(&self) -> bool {
        match *self {
            Self::JournalFull { .. }
            | Self::Durable(_)
            | Self::DurableRepeat { .. }
            | Self::DurabilityShort { .. }
            | Self::ClockOutOfRange { .. }
            | Self::ClockBeforeEpoch(_)
            | Self::BadSetting { .. }
            | Self::SettingUnreadable { .. } => true,
            Self::Binding(_)
            | Self::UnsupportedDecision(_)
            | Self::UnsupportedCommand(_)
            | Self::UnsupportedEvent(_)
            | Self::UnsupportedDisposition(_)
            | Self::UnsupportedReconciliationAction(_)
            | Self::RunLimitReached { .. }
            | Self::ReadyWorkFull { .. }
            | Self::EffectLimitReached { .. }
            | Self::PayloadTooLarge { .. }
            | Self::Sequence(_)
            | Self::Ordering(_)
            | Self::Command(_)
            | Self::Transition(_)
            | Self::Authority(_)
            | Self::Codec(_)
            | Self::RunTenantMismatch { .. }
            | Self::Denied { .. }
            | Self::RunNotTracked { .. }
            | Self::IntentNotDurable { .. }
            | Self::DuplicateOperation { .. }
            | Self::UnresolvableOutcome { .. }
            | Self::ReceiptAlreadyPublished { .. }
            | Self::FoldDivergence { .. }
            | Self::EffectIdentity { .. }
            | Self::Protocol(_)
            | Self::Question(_)
            | Self::Fence(_) => false,
        }
    }
}

impl std::error::Error for RuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Binding(ref err) => Some(err),
            Self::Sequence(ref err) => Some(err),
            Self::Ordering(ref err) => Some(err),
            Self::Command(ref err) => Some(err),
            Self::Transition(ref err) => Some(err),
            Self::Authority(ref err) => Some(err),
            Self::Codec(ref err) => Some(err),
            Self::Protocol(ref err) => Some(err),
            Self::Question(ref err) => Some(err),
            Self::Durable(ref err) => Some(err),
            Self::Fence(ref err) => Some(err),
            Self::ClockBeforeEpoch(ref cause) => Some(cause),
            Self::SettingUnreadable { ref cause, .. } => Some(cause.as_ref()),
            Self::DurableRepeat { .. } | Self::DurabilityShort { .. } => None,
            Self::UnsupportedDecision(_)
            | Self::UnsupportedCommand(_)
            | Self::UnsupportedEvent(_)
            | Self::UnsupportedDisposition(_)
            | Self::UnsupportedReconciliationAction(_)
            | Self::JournalFull { .. }
            | Self::RunLimitReached { .. }
            | Self::ReadyWorkFull { .. }
            | Self::EffectLimitReached { .. }
            | Self::BadSetting { .. }
            | Self::PayloadTooLarge { .. }
            | Self::Denied { .. }
            | Self::RunNotTracked { .. }
            | Self::RunTenantMismatch { .. }
            | Self::IntentNotDurable { .. }
            | Self::DuplicateOperation { .. }
            | Self::UnresolvableOutcome { .. }
            | Self::ReceiptAlreadyPublished { .. }
            | Self::FoldDivergence { .. }
            | Self::ClockOutOfRange { .. }
            | Self::EffectIdentity { .. } => None,
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

impl From<larry_contracts::binding::BindingError> for RuntimeError {
    fn from(error: larry_contracts::binding::BindingError) -> Self {
        Self::Binding(error)
    }
}

impl From<lgwks_bot::journal::JournalError> for RuntimeError {
    fn from(err: lgwks_bot::journal::JournalError) -> Self {
        Self::Durable(err)
    }
}

impl From<crate::fence::FenceError> for RuntimeError {
    fn from(err: crate::fence::FenceError) -> Self {
        Self::Fence(err)
    }
}

impl From<crate::protocol::ProtocolError> for RuntimeError {
    fn from(err: crate::protocol::ProtocolError) -> Self {
        Self::Protocol(err)
    }
}

impl From<crate::runtime::QuestionError> for RuntimeError {
    fn from(err: crate::runtime::QuestionError) -> Self {
        Self::Question(err)
    }
}
