//! Command envelopes and duplicate-suppression semantics (spec Cell 04
//! ordering, decision D01; work package C02).
//!
//! A command is the reducer's validated input (D01: MI proposes, MD
//! enforces). Suppression of duplicate submission is identity-driven:
//!
//! - **Same [`CommandId`], identical canonical content** — an idempotent
//!   replay acceptance ([`accept`] returns
//!   [`CommandAcceptance::DuplicateReplay`]). A crash after durable
//!   admission but before acknowledgement (E28) retries safely: the same
//!   identity plus the same content is the one accepted identity.
//! - **Same [`CommandId`], any different content** — a typed
//!   [`CommandError::ConflictingRetry`], never a silent acceptance. The
//!   comparison is over the canonical envelope digest: tenant, sorted
//!   causal parents, command kind and payload. Parent order is
//!   canonicalized upstream (see [`crate::event`]), so a retry that merely
//!   reorders its causal citations still replays idempotently, while any
//!   semantic difference diverges loudly.

use crate::error::{CommandError, OrderingError};
use crate::event::{CausalRef, MAX_CAUSAL_PARENTS};
use crate::ids::{CommandId, ContentDigest, TenantId};

#[cfg(feature = "serde")]
use lgwks_std::json::serde::{Deserialize, Serialize};

/// What a command asks the reducer to do.
///
/// The three kinds are the command classes the specification's lifecycle
/// names: submit work, steer work in flight (E28 "retry and steer during a
/// tool"), and cancel. The set is closed; extending it is a specification
/// change, not a local edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub enum CommandKind {
    /// Submit new work for admission.
    Submit,
    /// Steer already-admitted work at a safe boundary.
    Steer,
    /// Request cancellation of a run (requested, not completed).
    Cancel,
}

/// The command payload plus its content identity.
///
/// The BLAKE3 digest over the kind tag and payload bytes is computed at
/// construction and is the content half of the canonical envelope digest.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub struct CommandContent {
    kind: CommandKind,
    payload: Box<[u8]>,
    digest: ContentDigest,
}

impl CommandContent {
    /// Builds content and computes its BLAKE3 digest.
    #[must_use]
    pub fn new(kind: CommandKind, payload: Box<[u8]>) -> Self {
        let digest = ContentDigest::over(&[&[kind as u8], &payload]);
        Self {
            kind,
            payload,
            digest,
        }
    }

    /// What the command asks the reducer to do.
    #[must_use]
    pub const fn kind(&self) -> CommandKind {
        self.kind
    }

    /// The payload bytes.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// The BLAKE3 digest over kind tag and payload bytes.
    #[must_use]
    pub const fn digest(&self) -> ContentDigest {
        self.digest
    }
}

/// A command envelope: identity, tenant, causal citations and content.
///
/// Construction validates the causal-parent bound; the canonical envelope
/// digest is derived at construction and drives duplicate suppression.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub struct Command {
    id: CommandId,
    tenant: TenantId,
    causal_parents: Vec<CausalRef>,
    content: CommandContent,
    envelope_digest: ContentDigest,
}

impl Command {
    /// Builds a command envelope.
    ///
    /// # Errors
    ///
    /// Returns [`OrderingError::CausalParentsExceeded`] when more than
    /// [`MAX_CAUSAL_PARENTS`] causal parents are supplied after the
    /// canonicalization performed by the causal-citation type.
    pub fn new(
        id: CommandId,
        tenant: TenantId,
        causal_parents: Vec<CausalRef>,
        content: CommandContent,
    ) -> Result<Self, OrderingError> {
        let mut parents = causal_parents;
        parents.sort_unstable();
        parents.dedup();
        if parents.len() > MAX_CAUSAL_PARENTS {
            return Err(OrderingError::CausalParentsExceeded {
                len: parents.len(),
                max: MAX_CAUSAL_PARENTS,
            });
        }
        let envelope_digest = Self::canonical_digest(tenant, &parents, content.digest());
        Ok(Self {
            id,
            tenant,
            causal_parents: parents,
            content,
            envelope_digest,
        })
    }

    fn canonical_digest(
        tenant: TenantId,
        parents: &[CausalRef],
        content_digest: ContentDigest,
    ) -> ContentDigest {
        let mut bytes = Vec::new();
        let parent_count = u32::try_from(parents.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes();
        bytes.extend_from_slice(tenant.as_bytes());
        bytes.extend_from_slice(&parent_count);
        for parent in parents {
            bytes.extend_from_slice(parent.run.as_bytes());
            bytes.extend_from_slice(&parent.sequence.value().to_be_bytes());
        }
        bytes.extend_from_slice(content_digest.as_bytes());
        ContentDigest::over(&[&bytes])
    }

    /// The command identity.
    #[must_use]
    pub const fn id(&self) -> CommandId {
        self.id
    }

    /// The tenant the command belongs to.
    #[must_use]
    pub const fn tenant(&self) -> TenantId {
        self.tenant
    }

    /// The canonicalized causal parents, sorted and duplicate-free.
    #[must_use]
    pub fn causal_parents(&self) -> &[CausalRef] {
        &self.causal_parents
    }

    /// The command content.
    #[must_use]
    pub const fn content(&self) -> &CommandContent {
        &self.content
    }

    /// The canonical envelope digest: tenant, sorted causal parents, kind
    /// and payload. Two envelopes with the same ID and the same digest are
    /// the same submission; any digest difference under one ID is a
    /// conflicting retry.
    #[must_use]
    pub const fn envelope_digest(&self) -> ContentDigest {
        self.envelope_digest
    }
}

/// The outcome of the duplicate-suppression check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub enum CommandAcceptance {
    /// First sight of this command identity; admit it for reduction.
    Initial,
    /// Same identity and identical canonical content: idempotent replay
    /// (E28). The reducer does not fold it twice.
    DuplicateReplay,
}

/// The duplicate-suppression check (Cell 04 ordering).
///
/// Pure: no state is mutated, no store is consulted — the caller supplies
/// the already-accepted command for this identity, if any. Deterministic:
/// identical inputs always produce the identical acceptance or error.
///
/// # Errors
///
/// Returns [`CommandError::IdentityMismatch`] when the offered identity
/// differs from the accepted one, [`CommandError::TenantMismatch`] when a
/// retry under one identity crosses tenants, and
/// [`CommandError::ConflictingRetry`] when the same identity carries
/// different canonical content.
pub fn accept(
    existing: Option<&Command>,
    incoming: &Command,
) -> Result<CommandAcceptance, CommandError> {
    let Some(prev) = existing else {
        return Ok(CommandAcceptance::Initial);
    };
    if prev.id != incoming.id {
        return Err(CommandError::IdentityMismatch {
            existing: prev.id,
            incoming: incoming.id,
        });
    }
    if prev.tenant != incoming.tenant {
        return Err(CommandError::TenantMismatch {
            command: prev.id,
            expected: prev.tenant,
            got: incoming.tenant,
        });
    }
    if prev.envelope_digest == incoming.envelope_digest {
        Ok(CommandAcceptance::DuplicateReplay)
    } else {
        Err(CommandError::ConflictingRetry {
            id: prev.id,
            existing: prev.envelope_digest,
            incoming: incoming.envelope_digest,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::RunId;
    use crate::test_support::expect_ok;

    fn tid(byte: u8) -> TenantId {
        TenantId::from_bytes([byte; 16])
    }

    fn rid(byte: u8) -> RunId {
        RunId::from_bytes([byte; 16])
    }

    fn cid(byte: u8) -> CommandId {
        CommandId::from_bytes([byte; 16])
    }

    fn causal(run: RunId, sequence: u64) -> CausalRef {
        CausalRef {
            run,
            sequence: crate::event::EventSequence::new(sequence),
        }
    }

    fn command(
        id: CommandId,
        tenant: TenantId,
        payload: &[u8],
        parents: Vec<CausalRef>,
    ) -> Command {
        let content = CommandContent::new(CommandKind::Submit, Box::from(payload));
        expect_ok(Command::new(id, tenant, parents, content))
    }

    #[test]
    fn e28_first_sight_of_an_identity_is_initial_acceptance() {
        let cmd = command(cid(1), tid(2), b"do", Vec::new());
        assert_eq!(accept(None, &cmd), Ok(CommandAcceptance::Initial));
    }

    #[test]
    fn e28_identical_retry_is_idempotent_replay_not_a_second_fold() {
        let first = command(cid(1), tid(2), b"do", vec![causal(rid(3), 4)]);
        let retry = command(cid(1), tid(2), b"do", vec![causal(rid(3), 4)]);
        assert_eq!(first, retry);
        assert_eq!(
            accept(Some(&first), &retry),
            Ok(CommandAcceptance::DuplicateReplay)
        );
    }

    #[test]
    fn e28_conflicting_command_retry_is_rejected_with_one_accepted_identity() {
        let first = command(cid(1), tid(2), b"do-the-thing", Vec::new());
        let conflicting = command(cid(1), tid(2), b"do-something-else", Vec::new());
        let outcome = accept(Some(&first), &conflicting);
        assert_eq!(
            outcome,
            Err(CommandError::ConflictingRetry {
                id: cid(1),
                existing: first.envelope_digest(),
                incoming: conflicting.envelope_digest()
            })
        );
        // A silent retry that changes only the causal citations is the
        // same conflict, not a fresh acceptance.
        let redirected = command(cid(1), tid(2), b"do-the-thing", vec![causal(rid(9), 1)]);
        assert!(matches!(
            accept(Some(&first), &redirected),
            Err(CommandError::ConflictingRetry { .. })
        ));
    }

    #[test]
    fn a_retry_that_merely_reorders_causal_parents_replays_idempotently() {
        let first = command(
            cid(1),
            tid(2),
            b"do",
            vec![causal(rid(3), 4), causal(rid(5), 6)],
        );
        let retry = command(
            cid(1),
            tid(2),
            b"do",
            vec![causal(rid(5), 6), causal(rid(3), 4)],
        );
        assert_eq!(
            accept(Some(&first), &retry),
            Ok(CommandAcceptance::DuplicateReplay)
        );
    }

    #[test]
    fn cross_tenant_retry_is_a_typed_refusal() {
        let first = command(cid(1), tid(2), b"do", Vec::new());
        let foreign = command(cid(1), tid(3), b"do", Vec::new());
        assert_eq!(
            accept(Some(&first), &foreign),
            Err(CommandError::TenantMismatch {
                command: cid(1),
                expected: tid(2),
                got: tid(3)
            })
        );
    }

    #[test]
    fn identity_mismatch_is_typed_never_guessed() {
        let first = command(cid(1), tid(2), b"do", Vec::new());
        let other = command(cid(4), tid(2), b"do", Vec::new());
        assert_eq!(
            accept(Some(&first), &other),
            Err(CommandError::IdentityMismatch {
                existing: cid(1),
                incoming: cid(4)
            })
        );
    }

    #[test]
    fn excess_causal_parents_are_typed_refusals() {
        let too_many: Vec<CausalRef> = (0..=MAX_CAUSAL_PARENTS)
            .map(|index| causal(rid(u8::try_from(index).unwrap_or(u8::MAX)), 0))
            .collect();
        let content = CommandContent::new(CommandKind::Submit, Box::from(b"x".as_slice()));
        let built = Command::new(cid(1), tid(2), too_many, content);
        assert_eq!(
            built,
            Err(OrderingError::CausalParentsExceeded {
                len: MAX_CAUSAL_PARENTS + 1,
                max: MAX_CAUSAL_PARENTS
            })
        );
    }
}
