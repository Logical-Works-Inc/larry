//! Typed errors for the C02 contract layer.
//!
//! Every failure mode of the contract surface is a typed variant of one of
//! the enums in this module: callers match exhaustively instead of parsing
//! strings (rust-skills `err-thiserror-lib`, `type-no-stringly`). Error
//! messages start lowercase and carry no trailing punctuation.

use std::time::SystemTime;

use lgwks_std::id::ParseError as UuidParseError;
use lgwks_std::random::EntropyError;

use crate::authority::{BudgetUnit, Scope};
use crate::ids::{CommandId, ContentDigest, TenantId};
use crate::state::{RunState, TransitionEvent};

/// Why an identifier could not be produced or rehydrated.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IdError {
    /// The OS entropy source refused to serve UUID generation.
    #[error("OS entropy source failed: {0}")]
    Entropy(#[from] EntropyError),
    /// The text was not a canonical RFC 4122 UUID.
    #[error("malformed identifier text: {0}")]
    Parse(#[from] UuidParseError),
}

/// Why a run-state transition was refused.
///
/// The transition function is total: every `(RunState, TransitionEvent)`
/// pair maps to exactly one outcome, and every refusal is one of these
/// variants (Cell 04).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TransitionError {
    /// A terminal disposition is immutable; no event may leave it. Later
    /// evidence is recorded as a superseding resolution, never as an
    /// in-place state rewrite (Cell 04: durable terminal receipt).
    #[error("run state {state} is terminal and immutable; no transition may leave it")]
    Terminal {
        /// The terminal state that refused the event.
        state: RunState,
    },
    /// The event is not part of the algebra for this state.
    #[error("event {event:?} is not allowed in state {state}")]
    NotAllowed {
        /// The state that received the event.
        state: RunState,
        /// The refused event.
        event: TransitionEvent,
    },
    /// Rollback was claimed with partial external effects outstanding.
    /// Partial external effects must never be labelled `RolledBack`
    /// (Cell 04); compensation is a new authorized effect.
    #[error(
        "rollback refused in state {state}: partial external effects must never be labelled rolled back"
    )]
    PartialRollbackRefused {
        /// The state that received the rollback claim.
        state: RunState,
    },
}

/// Why per-run event sequencing failed.
///
/// Sequence assignment is total and gap-free for accepted events
/// (Cell 04 ordering); every violation is typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SequenceError {
    /// The `u64` sequence space is exhausted.
    #[error("event sequence space exhausted at u64::MAX")]
    Exhausted,
    /// A candidate sequence skipped over unassigned numbers.
    #[error("sequence gap: expected {expected}, got {got}")]
    Gap {
        /// The next gap-free sequence number.
        expected: u64,
        /// The candidate that would have introduced a gap.
        got: u64,
    },
    /// The first accepted event of a run must carry sequence 0.
    #[error("first event sequence must be 0, got {got}")]
    NotFirst {
        /// The non-zero first candidate.
        got: u64,
    },
}

/// Why an ordering-constrained record (command or event) was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum OrderingError {
    /// Too many causal parents were supplied; the bound keeps records
    /// bounded (Cell 05 resource bounds).
    #[error("{len} causal parents exceed the bound of {max}")]
    CausalParentsExceeded {
        /// The supplied number of causal parents.
        len: usize,
        /// The maximum accepted number.
        max: usize,
    },
    /// A sequence constraint was violated.
    #[error(transparent)]
    Sequence(#[from] SequenceError),
}

/// Why a command submission or duplicate-suppression check failed.
///
/// Same [`CommandId`](crate::ids::CommandId) with identical content is
/// idempotent replay; the same ID with any different content is a typed
/// error and is never silently accepted (Cell 04 ordering).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CommandError {
    /// The CommandId was seen before with different canonical content.
    #[error(
        "command {id} was already accepted with different content: existing digest {existing}, incoming digest {incoming}"
    )]
    ConflictingRetry {
        /// The contested command identity.
        id: CommandId,
        /// Canonical content digest of the accepted command.
        existing: ContentDigest,
        /// Canonical content digest of the conflicting retry.
        incoming: ContentDigest,
    },
    /// The retry was paired with a different command identity.
    #[error("identity mismatch: existing command {existing}, incoming {incoming}")]
    IdentityMismatch {
        /// The identity of the already-accepted command.
        existing: CommandId,
        /// The identity of the offered command.
        incoming: CommandId,
    },
    /// A retry under one command identity crossed tenants.
    #[error("command {command} belongs to tenant {expected}; retry carried tenant {got}")]
    TenantMismatch {
        /// The command identity.
        command: CommandId,
        /// The tenant of the accepted command.
        expected: TenantId,
        /// The tenant carried by the retry.
        got: TenantId,
    },
}

/// Why authority attenuation or an effect-boundary validation failed.
///
/// The authority equation (Cell 04) requires child grants to be a subset of
/// parent grants intersected with current resource policy, budget and
/// expiry, re-validated at the effect boundary. Expiry never consents
/// (E30): an expired grant yields a typed refusal, not a fallback.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AuthorityError {
    /// Delegation would exceed the maximum depth (Cell 05: depth <= 3).
    #[error("delegation depth {depth} exceeds the maximum of {max}")]
    DelegationDepthExceeded {
        /// The depth the child grant would have reached.
        depth: u8,
        /// The maximum allowed depth.
        max: u8,
    },
    /// Budgets are only comparable within one unit; units are never coerced.
    #[error("budget unit mismatch: parent budget in {parent:?}, requested in {requested:?}")]
    BudgetUnitMismatch {
        /// The parent grant's unit.
        parent: BudgetUnit,
        /// The requested child unit.
        requested: BudgetUnit,
    },
    /// No requested scope survived the parent-and-policy intersection, so
    /// the child grant would carry no authority at all.
    #[error(
        "attenuation refused: no requested scope survives the parent grant and resource policy intersection"
    )]
    EmptyAttenuation,
    /// A required scope is granted but the current resource policy no
    /// longer allows it at the effect boundary.
    #[error("scope {scope:?} is granted but outside the current resource policy")]
    ScopeOutsidePolicy {
        /// The scope the policy refuses.
        scope: Scope,
    },
    /// A required scope was never granted.
    #[error("scope {scope:?} is required but not granted")]
    ScopeNotGranted {
        /// The missing scope.
        scope: Scope,
    },
    /// The grant's validity ended before the observation instant. Time is
    /// an authority-supplied pinned input; a worker's own clock can never
    /// extend a lease (Cell 04).
    #[error("grant expired at {expiry:?}; observed at {now:?}: expiry never consents")]
    GrantExpired {
        /// The instant validity ended.
        expiry: SystemTime,
        /// The observation instant.
        now: SystemTime,
    },
    /// The effect cost exceeds the granted budget.
    #[error("budget exhausted: required {required} {unit:?}, available {available}")]
    BudgetExhausted {
        /// The required amount.
        required: u64,
        /// The available amount.
        available: u64,
        /// The budget unit both amounts are denominated in.
        unit: BudgetUnit,
    },
}
