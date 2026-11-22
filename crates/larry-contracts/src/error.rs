//! Typed errors for the C02 contract layer.
//!
//! Every failure mode of the contract surface is a typed variant of one of
//! the enums in this module: callers match exhaustively instead of parsing
//! strings (rust-skills `err-thiserror-lib`, `type-no-stringly`). Error
//! messages start lowercase and carry no trailing punctuation.
//!
//! `Display`, `std::error::Error` (with `source`) and `From` are written here
//! by hand: the estate's `lgwks_macros::Error` derive does not forward a
//! `#[from]` field as the error's source, so a derive would quietly drop the
//! cause chain these types carry (larry#339, logicalworks-crates#842).

use std::fmt;
use std::time::SystemTime;

use lgwks_std::id::ParseError as UuidParseError;
use lgwks_std::random::EntropyError;

use crate::authority::{BudgetUnit, Scope};
use crate::ids::{CommandId, ContentDigest, TenantId};
use crate::state::{RunState, TransitionEvent};

/// Why an identifier could not be produced or rehydrated.
#[derive(Debug)]
#[non_exhaustive]
pub enum IdError {
    /// The OS entropy source refused to serve UUID generation.
    Entropy(EntropyError),
    /// The text was not a canonical RFC 4122 UUID.
    Parse(UuidParseError),
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Entropy(ref cause) => write!(f, "OS entropy source failed: {cause}"),
            Self::Parse(ref cause) => write!(f, "malformed identifier text: {cause}"),
        }
    }
}

impl std::error::Error for IdError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Entropy(ref cause) => Some(cause),
            Self::Parse(ref cause) => Some(cause),
        }
    }
}

impl From<EntropyError> for IdError {
    fn from(cause: EntropyError) -> Self {
        Self::Entropy(cause)
    }
}

impl From<UuidParseError> for IdError {
    fn from(cause: UuidParseError) -> Self {
        Self::Parse(cause)
    }
}

/// Why a run-state transition was refused.
///
/// The transition function is total: every `(RunState, TransitionEvent)`
/// pair maps to exactly one outcome, and every refusal is one of these
/// variants (Cell 04).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransitionError {
    /// A terminal disposition is immutable; no event may leave it. Later
    /// evidence is recorded as a superseding resolution, never as an
    /// in-place state rewrite (Cell 04: durable terminal receipt).
    Terminal {
        /// The terminal state that refused the event.
        state: RunState,
    },
    /// The event is not part of the algebra for this state.
    NotAllowed {
        /// The state that received the event.
        state: RunState,
        /// The refused event.
        event: TransitionEvent,
    },
    /// Rollback was claimed with partial external effects outstanding.
    /// Partial external effects must never be labelled `RolledBack`
    /// (Cell 04); compensation is a new authorized effect.
    PartialRollbackRefused {
        /// The state that received the rollback claim.
        state: RunState,
    },
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Terminal { state } => {
                write!(
                    f,
                    "run state {state} is terminal and immutable; no transition may leave it"
                )
            }
            Self::NotAllowed { state, event } => {
                write!(f, "event {event:?} is not allowed in state {state}")
            }
            Self::PartialRollbackRefused { state } => write!(
                f,
                "rollback refused in state {state}: partial external effects must never be labelled rolled back"
            ),
        }
    }
}

impl std::error::Error for TransitionError {}

/// Why per-run event sequencing failed.
///
/// Sequence assignment is total and gap-free for accepted events
/// (Cell 04 ordering); every violation is typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SequenceError {
    /// The `u64` sequence space is exhausted.
    Exhausted,
    /// A candidate sequence skipped over unassigned numbers.
    Gap {
        /// The next gap-free sequence number.
        expected: u64,
        /// The candidate that would have introduced a gap.
        got: u64,
    },
    /// The first accepted event of a run must carry sequence 0.
    NotFirst {
        /// The non-zero first candidate.
        got: u64,
    },
}

impl fmt::Display for SequenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Exhausted => write!(f, "event sequence space exhausted at u64::MAX"),
            Self::Gap { expected, got } => {
                write!(f, "sequence gap: expected {expected}, got {got}")
            }
            Self::NotFirst { got } => write!(f, "first event sequence must be 0, got {got}"),
        }
    }
}

impl std::error::Error for SequenceError {}

/// Why an ordering-constrained record (command or event) was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OrderingError {
    /// Too many causal parents were supplied; the bound keeps records
    /// bounded (Cell 05 resource bounds).
    CausalParentsExceeded {
        /// The supplied number of causal parents.
        len: usize,
        /// The maximum accepted number.
        max: usize,
    },
    /// A sequence constraint was violated.
    Sequence(SequenceError),
}

impl fmt::Display for OrderingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::CausalParentsExceeded { len, max } => {
                write!(f, "{len} causal parents exceed the bound of {max}")
            }
            Self::Sequence(ref cause) => fmt::Display::fmt(cause, f),
        }
    }
}

impl std::error::Error for OrderingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::CausalParentsExceeded { .. } => None,
            Self::Sequence(ref cause) => std::error::Error::source(cause),
        }
    }
}

impl From<SequenceError> for OrderingError {
    fn from(cause: SequenceError) -> Self {
        Self::Sequence(cause)
    }
}

/// Why a command submission or duplicate-suppression check failed.
///
/// Same [`CommandId`](crate::ids::CommandId) with identical content is
/// idempotent replay; the same ID with any different content is a typed
/// error and is never silently accepted (Cell 04 ordering).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CommandError {
    /// The CommandId was seen before with different canonical content.
    ConflictingRetry {
        /// The contested command identity.
        id: CommandId,
        /// Canonical content digest of the accepted command.
        existing: ContentDigest,
        /// Canonical content digest of the conflicting retry.
        incoming: ContentDigest,
    },
    /// The retry was paired with a different command identity.
    IdentityMismatch {
        /// The identity of the already-accepted command.
        existing: CommandId,
        /// The identity of the offered command.
        incoming: CommandId,
    },
    /// A retry under one command identity crossed tenants.
    TenantMismatch {
        /// The command identity.
        command: CommandId,
        /// The tenant of the accepted command.
        expected: TenantId,
        /// The tenant carried by the retry.
        got: TenantId,
    },
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ConflictingRetry {
                id,
                existing,
                incoming,
            } => write!(
                f,
                "command {id} was already accepted with different content: existing digest {existing}, incoming digest {incoming}"
            ),
            Self::IdentityMismatch { existing, incoming } => write!(
                f,
                "identity mismatch: existing command {existing}, incoming {incoming}"
            ),
            Self::TenantMismatch {
                command,
                expected,
                got,
            } => write!(
                f,
                "command {command} belongs to tenant {expected}; retry carried tenant {got}"
            ),
        }
    }
}

impl std::error::Error for CommandError {}

/// Why authority attenuation or an effect-boundary validation failed.
///
/// The authority equation (Cell 04) requires child grants to be a subset of
/// parent grants intersected with current resource policy, budget and
/// expiry, re-validated at the effect boundary. Expiry never consents
/// (E30): an expired grant yields a typed refusal, not a fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthorityError {
    /// Delegation would exceed the maximum depth (Cell 05: depth <= 3).
    DelegationDepthExceeded {
        /// The depth the child grant would have reached.
        depth: u8,
        /// The maximum allowed depth.
        max: u8,
    },
    /// Budgets are only comparable within one unit; units are never coerced.
    BudgetUnitMismatch {
        /// The parent grant's unit.
        parent: BudgetUnit,
        /// The requested child unit.
        requested: BudgetUnit,
    },
    /// No requested scope survived the parent-and-policy intersection, so
    /// the child grant would carry no authority at all.
    EmptyAttenuation,
    /// A required scope is granted but the current resource policy no
    /// longer allows it at the effect boundary.
    ScopeOutsidePolicy {
        /// The scope the policy refuses.
        scope: Scope,
    },
    /// A required scope was never granted.
    ScopeNotGranted {
        /// The missing scope.
        scope: Scope,
    },
    /// The grant's validity ended before the observation instant. Time is
    /// an authority-supplied pinned input; a worker's own clock can never
    /// extend a lease (Cell 04).
    GrantExpired {
        /// The instant validity ended.
        expiry: SystemTime,
        /// The observation instant.
        now: SystemTime,
    },
    /// The effect cost exceeds the granted budget.
    BudgetExhausted {
        /// The required amount.
        required: u64,
        /// The available amount.
        available: u64,
        /// The budget unit both amounts are denominated in.
        unit: BudgetUnit,
    },
}

impl fmt::Display for AuthorityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::DelegationDepthExceeded { depth, max } => {
                write!(f, "delegation depth {depth} exceeds the maximum of {max}")
            }
            Self::BudgetUnitMismatch { parent, requested } => write!(
                f,
                "budget unit mismatch: parent budget in {parent:?}, requested in {requested:?}"
            ),
            Self::EmptyAttenuation => write!(
                f,
                "attenuation refused: no requested scope survives the parent grant and resource policy intersection"
            ),
            Self::ScopeOutsidePolicy { scope } => {
                write!(
                    f,
                    "scope {scope:?} is granted but outside the current resource policy"
                )
            }
            Self::ScopeNotGranted { scope } => {
                write!(f, "scope {scope:?} is required but not granted")
            }
            Self::GrantExpired { expiry, now } => write!(
                f,
                "grant expired at {expiry:?}; observed at {now:?}: expiry never consents"
            ),
            Self::BudgetExhausted {
                required,
                available,
                unit,
            } => write!(
                f,
                "budget exhausted: required {required} {unit:?}, available {available}"
            ),
        }
    }
}

impl std::error::Error for AuthorityError {}
