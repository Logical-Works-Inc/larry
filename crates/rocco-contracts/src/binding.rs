//! E65 canonical owner-resolved identity, independent of display prose.

use crate::authority::SecurityEpoch;
use crate::ids::ContentDigest;

#[cfg(feature = "serde")]
use lgwks_std::json::serde::{Deserialize, Serialize};

/// Maximum bytes hashed for any one resolved field, including payload.
pub const MAX_BOUND_FIELD_BYTES: usize = 65_536;

/// A field whose identity must survive display, admission and dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BindingField {
    /// Stable resource identity and version, not merely its path spelling.
    Resource,
    /// Resolved link identity/version or an explicit empty no-link identity.
    Symlink,
    /// Exact destination identity.
    Recipient,
    /// Exact account identity.
    Account,
    /// Exact payload or artifact bytes.
    Payload,
    /// Adapter identity and version.
    Adapter,
    /// Protocol schema version.
    Schema,
    /// Owner policy generation.
    PolicyEpoch,
    /// Runtime policy revision.
    PolicyRevision,
    /// Runtime security generation.
    SecurityEpoch,
    /// Complete command envelope, including tenant and operation.
    Command,
    /// Canonical display identity failed its integrity check.
    DisplayDigest,
}

/// Typed failures of display binding; messages disclose no field contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BindingError {
    /// A resolved identity no longer equals what was displayed.
    #[error("display binding changed: {field:?}; display the current effect before submitting")]
    Changed {
        /// Deterministic first differing field.
        field: BindingField,
    },
    /// Hashing would exceed the declared work bound.
    #[error("display field {field:?} exceeds {MAX_BOUND_FIELD_BYTES} bytes")]
    TooLarge {
        /// Oversized field.
        field: BindingField,
    },
    /// No immutable display was supplied for an external effect.
    #[error("effect has no display binding; display it before submitting")]
    MissingDisplay,
    /// Owner cannot supply the required atomic binding contract.
    #[error("owner does not support resolved compare-and-dispatch")]
    UnsupportedOwner,
    /// No single grant covers the complete effect.
    #[error("no single grant covers this tenant, holder, scopes, cost, unit and expiry")]
    NoCoveringGrant,
    /// Bound limits apply before scanning the grant list.
    #[error("grant list exceeds the bound of 64")]
    TooManyGrants,
    /// A displayed grant has expired by the time of admission or dispatch.
    #[error("display authority expired; obtain current authority and display again")]
    Expired,
}

/// Six domain-separated digests in fixed specification order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub struct EffectTarget {
    /// Resource, symlink, recipient, account, payload and adapter identities.
    fields: [ContentDigest; 6],
}

impl EffectTarget {
    /// Hashes canonical owner bytes; empty values explicitly mean absent.
    /// Resource bytes must include identity AND version. Owners must not
    /// substitute mutable names for resolved identities.
    ///
    /// # Errors
    /// Refuses any field larger than [`MAX_BOUND_FIELD_BYTES`] before hashing.
    pub fn new(
        resource: &[u8],
        symlink: &[u8],
        recipient: &[u8],
        account: &[u8],
        payload: &[u8],
        adapter: &[u8],
    ) -> Result<Self, BindingError> {
        let inputs = [resource, symlink, recipient, account, payload, adapter];
        let names = Self::names();
        for (bytes, field) in inputs.iter().zip(names) {
            if bytes.len() > MAX_BOUND_FIELD_BYTES {
                return Err(BindingError::TooLarge { field });
            }
        }
        let fields = [0_u8, 1, 2, 3, 4, 5].map(|index| {
            ContentDigest::over(&[
                b"rocco/e65/field/v1\0",
                &[index],
                inputs[usize::from(index)],
            ])
        });
        Ok(Self { fields })
    }

    /// Fixed canonical field order, independent of enum layout.
    const fn names() -> [BindingField; 6] {
        [
            BindingField::Resource,
            BindingField::Symlink,
            BindingField::Recipient,
            BindingField::Account,
            BindingField::Payload,
            BindingField::Adapter,
        ]
    }

    /// Returns immutable per-field identities in specification order.
    #[must_use]
    pub const fn fields(&self) -> &[ContentDigest; 6] {
        &self.fields
    }
}

/// Owner-resolved target with protocol and policy versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub struct ResolvedEffect {
    /// Fixed-size resolved field identities.
    target: EffectTarget,
    /// Schema understood by the owner.
    schema: u64,
    /// Policy generation observed by the owner.
    policy_epoch: SecurityEpoch,
}

impl ResolvedEffect {
    /// Records owner observations, without granting authority.
    #[must_use]
    pub const fn new(target: EffectTarget, schema: u64, policy_epoch: SecurityEpoch) -> Self {
        Self {
            target,
            schema,
            policy_epoch,
        }
    }

    /// Immutable target identities suitable for machine-readable display.
    #[must_use]
    pub const fn target(&self) -> &EffectTarget {
        &self.target
    }

    /// Canonical, versioned digest with unambiguous fixed-width framing.
    #[must_use]
    pub fn digest(&self) -> ContentDigest {
        let [resource, symlink, recipient, account, payload, adapter] = self.target.fields;
        ContentDigest::over(&[
            b"rocco/e65/resolved/v1\0",
            resource.as_bytes(),
            symlink.as_bytes(),
            recipient.as_bytes(),
            account.as_bytes(),
            payload.as_bytes(),
            adapter.as_bytes(),
            &self.schema.to_be_bytes(),
            &self.policy_epoch.value().to_be_bytes(),
        ])
    }

    /// Compares every field in specification order, naming the first swap.
    ///
    /// # Errors
    /// Returns [`BindingError::Changed`] for any changed field.
    pub fn verify(&self, current: &Self) -> Result<(), BindingError> {
        for ((expected, actual), field) in self
            .target
            .fields
            .iter()
            .zip(current.target.fields.iter())
            .zip(EffectTarget::names())
        {
            if expected != actual {
                return Err(BindingError::Changed { field });
            }
        }
        if self.schema != current.schema {
            return Err(BindingError::Changed {
                field: BindingField::Schema,
            });
        }
        if self.policy_epoch != current.policy_epoch {
            return Err(BindingError::Changed {
                field: BindingField::PolicyEpoch,
            });
        }
        Ok(())
    }
}

/// Immutable display identity persisted inside the durable intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub struct EffectBinding {
    /// Target the user was shown.
    resolved: ResolvedEffect,
    /// Complete displayed command and selected authority identity.
    digest: ContentDigest,
    /// Command and grant identity needed to verify the recorded digest.
    approval: ContentDigest,
    /// Pinned runtime policy revision.
    policy_revision: u64,
    /// Exclusive expiry of the selected authority.
    valid_until: std::time::SystemTime,
}

impl EffectBinding {
    /// Combines a resolved target with the displayed command/authority digest.
    #[must_use]
    pub fn new(
        resolved: ResolvedEffect,
        approval: ContentDigest,
        policy_revision: u64,
        valid_until: std::time::SystemTime,
    ) -> Self {
        let digest = display_digest(resolved, approval, policy_revision, valid_until);
        Self {
            resolved,
            digest,
            approval,
            policy_revision,
            valid_until,
        }
    }

    /// Exact effect identities that were displayed.
    #[must_use]
    pub const fn resolved(&self) -> &ResolvedEffect {
        &self.resolved
    }

    /// Canonical display identity retained across replay and retries.
    #[must_use]
    pub const fn digest(&self) -> ContentDigest {
        self.digest
    }

    /// Checks runtime policy and expiry in addition to owner-resolved fields.
    ///
    /// # Errors
    /// Returns a policy revision swap or an expired authority refusal.
    pub fn verify_runtime(
        &self,
        revision: u64,
        at: std::time::SystemTime,
    ) -> Result<(), BindingError> {
        if self.digest
            != display_digest(
                self.resolved,
                self.approval,
                self.policy_revision,
                self.valid_until,
            )
        {
            return Err(BindingError::Changed {
                field: BindingField::DisplayDigest,
            });
        }
        if self.policy_revision != revision {
            return Err(BindingError::Changed {
                field: BindingField::PolicyRevision,
            });
        }
        if at >= self.valid_until {
            return Err(BindingError::Expired);
        }
        Ok(())
    }
}

/// Fixed-width expiry encoding includes pre-epoch instants without arithmetic.
fn display_digest(
    resolved: ResolvedEffect,
    approval: ContentDigest,
    revision: u64,
    expiry: std::time::SystemTime,
) -> ContentDigest {
    let (sign, duration) = match expiry.duration_since(std::time::SystemTime::UNIX_EPOCH) {
        Ok(duration) => (0_u8, duration),
        Err(error) => (1_u8, error.duration()),
    };
    ContentDigest::over(&[
        b"rocco/e65/display/v1\0",
        resolved.digest().as_bytes(),
        approval.as_bytes(),
        &revision.to_be_bytes(),
        &[sign],
        &duration.as_nanos().to_be_bytes(),
    ])
}
