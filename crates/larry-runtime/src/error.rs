//! Typed failures of the runtime.
//!
//! Every variant is a refused operation with structured context; there is no
//! `String` error and no panic path. `Display`, `std::error::Error` (with
//! `source`) and `From` come from the estate's `lgwks_macros::Error` derive:
//! a `#[from]` field is the error's source (larry#435).

use larry_contracts::error::{
    AuthorityError, CommandError, OrderingError, SequenceError, TransitionError,
};
use larry_contracts::ids::{OperationId, RunId, TenantId};
use larry_contracts::replay::DenialReason;
use larry_contracts::state::RunState;
use lgwks_macros::Error;

/// Every way the runtime refuses an operation.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RuntimeError {
    /// Display or resolved effect binding refused a reach.
    #[error("{0}")]
    Binding(#[from] larry_contracts::binding::BindingError),
    /// An authorization decision is not understood; dispatch is refused.
    #[error("unsupported authorization decision: {0:?}")]
    UnsupportedDecision(larry_contracts::replay::Decision),
    /// A command kind is not understood; execution is refused.
    #[error("unsupported command kind: {0:?}")]
    UnsupportedCommand(larry_contracts::command::CommandKind),
    /// An event kind is not understood; replay is refused.
    #[error("unsupported event kind: {0:?}")]
    UnsupportedEvent(larry_contracts::event::EventKind),
    /// A disposition is not understood; no terminal receipt is published.
    #[error("unsupported terminal disposition: {0:?}")]
    UnsupportedDisposition(larry_contracts::effect::TerminalDisposition),
    /// A reconciliation action is not understood; no owner interaction is permitted.
    #[error("unsupported reconciliation action: {0:?}")]
    UnsupportedReconciliationAction(larry_contracts::effect::ReconciliationAction),
    /// The bounded journal is at capacity. Admission is backpressured; no
    /// recorded fact is ever silently dropped (Cell 05).
    #[error("journal is at capacity {capacity}; admission is backpressured")]
    JournalFull {
        /// The configured entry ceiling.
        capacity: usize,
    },
    /// The run table is at capacity.
    #[error("run table is at capacity {capacity}")]
    RunLimitReached {
        /// The configured run ceiling.
        capacity: usize,
    },
    /// The ready-work queue is at capacity; the caller must drain a tick
    /// before submitting more (Cell 05: 256 descriptors).
    #[error("ready-work queue is at capacity {capacity}; drain a tick first")]
    ReadyWorkFull {
        /// The configured ceiling.
        capacity: usize,
    },
    /// A run already holds the maximum number of recorded effects.
    #[error("run {run} already holds the maximum of {capacity} recorded effects")]
    EffectLimitReached {
        /// The run that is full.
        run: RunId,
        /// The configured ceiling.
        capacity: usize,
    },
    /// A configuration bound is zero or outside the range its meaning allows
    /// (larry#212). The runtime is not built: a bad setting is refused, never
    /// clamped into something the host did not ask for.
    #[error("runtime setting {name}={} is outside {}..={}", range.value, range.min, range.max)]
    BadSetting {
        /// The setting's name.
        name: &'static str,
        /// The value supplied and the range it must fall in.
        range: Box<SettingRange>,
    },
    /// A setting's value cannot be read in the unit its range is declared in,
    /// so no bound can judge it (larry#212).
    #[error("runtime setting {name} cannot be read: {cause}")]
    SettingUnreadable {
        /// The setting's name.
        name: &'static str,
        /// Why the reading failed.
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },
    /// An admitted command's payload is past the configured byte ceiling
    /// (larry#212). The check runs before the payload is decoded or
    /// journalled, so nothing hostile is held or recorded.
    #[error("command payload of {bytes} bytes exceeds the {ceiling}-byte ceiling")]
    PayloadTooLarge {
        /// The payload's length in bytes.
        bytes: usize,
        /// The configured ceiling.
        ceiling: usize,
    },
    /// Per-run event sequencing was violated.
    #[error("event sequencing refused: {0}")]
    Sequence(#[from] SequenceError),
    /// A command or event violated an ordering constraint.
    #[error("record ordering refused: {0}")]
    Ordering(#[from] OrderingError),
    /// Duplicate-suppression or identity validation failed.
    #[error("command refused: {0}")]
    Command(#[from] CommandError),
    /// The finite run-state algebra refused the transition.
    #[error("run-state transition refused: {0}")]
    Transition(#[from] TransitionError),
    /// The effect-boundary authority re-validation failed.
    #[error("effect-boundary authority refused: {0}")]
    Authority(#[from] AuthorityError),
    /// A journal payload could not be encoded or decoded.
    #[error("journal payload codec failed: {0}")]
    Codec(#[from] lgwks_std::json::Error),
    /// The pure authorization reducer denied the effect (D36).
    #[error("effect {operation} denied: {reason:?}")]
    Denied {
        /// The stable operation ID that was refused.
        operation: OperationId,
        /// The deterministic first-failing reason.
        reason: DenialReason,
    },
    /// The run is not present in the projection.
    #[error("run {run} is not tracked")]
    RunNotTracked {
        /// The untracked run.
        run: RunId,
    },
    /// A command envelope targeted a run owned by another tenant.
    #[error("run {run} belongs to tenant {expected}; command carried tenant {got}")]
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
    #[error("operation {operation} has no durable intent; dispatch refused")]
    IntentNotDurable {
        /// The operation whose intent is missing.
        operation: OperationId,
    },
    /// The operation identity was already reserved in this runtime.
    #[error("operation {operation} was already reserved")]
    DuplicateOperation {
        /// The repeated operation.
        operation: OperationId,
    },
    /// The observed outcome cannot terminate the effect under its declared
    /// strategy: it stays open for status query or bounded retry, and is
    /// never declared a victory or a failure (R72, E56).
    #[error("operation {operation}'s outcome stays open: not a success, not a failure")]
    UnresolvableOutcome {
        /// The still-open operation.
        operation: OperationId,
    },
    /// A terminal receipt already exists for this operation; the receipt is
    /// immutable and later evidence is a superseding resolution, never an
    /// in-place rewrite (Cell 04).
    #[error("operation {operation} already has an immutable terminal receipt")]
    ReceiptAlreadyPublished {
        /// The already-closed operation.
        operation: OperationId,
    },
    /// Folding the journal re-derived a state that differs from the state
    /// the journal recorded. The journal is authoritative and self-checking:
    /// a mismatch is surfaced, never silently accepted (E55).
    #[error("run {run} fold diverged: journal recorded {recorded}, algebra re-derived {rederived}")]
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
    #[error("clock reading {elapsed:?} after the Unix epoch is outside the platform's time range")]
    ClockOutOfRange {
        /// The reading since the epoch that could not be placed.
        elapsed: std::time::Duration,
    },
    /// The system clock reads before the Unix epoch, so no live reading can be placed after it
    /// and the runtime has no instant to pin (E30, larry#182).
    #[error("the system clock reads before the Unix epoch: {0}")]
    ClockBeforeEpoch(#[source] std::time::SystemTimeError),
    /// A Larry field cannot name an lgwks_bot effect key (larry#76, larry#78).
    #[error(
        "operation {operation}'s {field} names no effect key: all-zero or past the last generation"
    )]
    EffectIdentity {
        /// The operation whose key could not be formed.
        operation: OperationId,
        /// `run`, `tenant` or `operation` when all-zero, or `admission_epoch`
        /// when it is past the last broker generation.
        field: &'static str,
    },
    /// The dispatch fence refused a change or a read (larry#76).
    #[error("dispatch fence refused: {0}")]
    Fence(#[from] crate::fence::FenceError),
    /// The owner was reached, then the broker refused to count the reach: a
    /// broker defect (E65). Unlike a fenced reach the effect may have
    /// happened, so its outcome is still recorded (larry#197).
    #[error(
        "operation {operation} reached its owner, then the broker refused its spend: {refusal}"
    )]
    Unspent {
        /// The operation whose reach happened.
        operation: OperationId,
        /// The broker's refusal to count it.
        refusal: crate::fence::FenceRefusal,
    },
    /// Protocol negotiation or protocol-bound command ingress was refused.
    #[error("protocol ingress refused: {0}")]
    Protocol(#[from] crate::protocol::ProtocolError),
    /// A question or its answer was refused before anything was written (E30).
    #[error("question refused: {0}")]
    Question(#[from] crate::runtime::QuestionError),
    /// The journal's file refused (larry#75): another writer holds it, its
    /// committed bytes break the chain, a ceiling was reached, the device
    /// refused, or an append's outcome is unknown until a reopen.
    #[error("journal file refused: {0}")]
    Durable(#[from] lgwks_bot::kernel::journal::JournalError),
    /// The journal's file holds a command identity twice, which no append
    /// this runtime made can produce (larry#75).
    #[error(
        "journal file record {index} repeats an accepted command; the file is not this runtime's"
    )]
    DurableRepeat {
        /// The zero-based record that repeats an earlier one.
        index: usize,
    },
    /// The journal's file acknowledged an append under a promise weaker than
    /// surviving a process crash (larry#75).
    #[error("journal file promised {promised}, short of surviving a process crash")]
    DurabilityShort {
        /// What the acknowledgment promised.
        promised: lgwks_bot::kernel::journal::DurabilityPromise,
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
            | Self::Fence(_)
            | Self::Unspent { .. } => false,
        }
    }

    /// Whether this refusal is the journal's own bytes refusing to rebuild (`true`), or a
    /// fault of the host, its settings or its clock that a later start may not meet (`false`).
    ///
    /// A journal a killed executor left is set aside as refused only on `true`: committed bytes
    /// that break the chain or past the file's ceiling, a record that does not decode, and a
    /// replay the contracts refuse. A device, lock, clock, setting or capacity fault keeps the
    /// file where it is, so the next start recovers it and a keyed retry still finds the run
    /// (larry#394). Every unlisted case of the estate's journal error keeps the file: losing the
    /// evidence is the one outcome a retry cannot undo, so only the listed cases refuse.
    #[must_use]
    pub const fn refuses_the_record(&self) -> bool {
        use lgwks_bot::kernel::journal::JournalError as Store;
        matches!(
            *self,
            Self::Durable(Store::Corrupt(_) | Store::CapacityExceeded { .. } | Store::Exhausted)
                | Self::Codec(_)
                | Self::Sequence(_)
                | Self::Ordering(_)
                | Self::Command(_)
                | Self::Transition(_)
                | Self::UnsupportedEvent(_)
                | Self::UnsupportedCommand(_)
                | Self::UnsupportedDecision(_)
                | Self::UnsupportedDisposition(_)
                | Self::UnsupportedReconciliationAction(_)
                | Self::FoldDivergence { .. }
                | Self::DurableRepeat { .. }
                | Self::RunTenantMismatch { .. }
                | Self::EffectIdentity { .. }
        )
    }
}
