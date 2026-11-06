//! Strongly-typed identifiers and content digests (spec C02, Cell 04).
//!
//! Each identifier is a distinct newtype over [`lgwks_std::id::Uuid`]
//! (lgwks_std feature `random`): two identifier kinds are never
//! interchangeable at the type level, so a `RunId` can never be passed
//! where a `TenantId` is required (rust-skills `type-newtype-ids`,
//! `api-newtype-safety`). Generation uses OS entropy through lgwks_std;
//! rehydration accepts canonical RFC 4122 text or raw bytes.
//!
//! [`ContentDigest`] is a BLAKE3 digest (lgwks_std feature `hash`,
//! INV-HASH-DETERMINISTIC: the same bytes always produce the same digest)
//! and is the canonical content-identity primitive used for command
//! dedup and terminal receipts.
//!
//! Wire note: serialized forms are transport data. C02 is a stateless
//! contract layer and makes no claim about authenticated wire integrity;
//! digest re-validation belongs to the reducer boundary, not to serde.

use core::fmt;
use core::str::FromStr;

use lgwks_std::hash;
use lgwks_std::id::Uuid;

#[cfg(feature = "serde")]
use lgwks_std::json::serde as lgwks_serde;
#[cfg(feature = "serde")]
use lgwks_std::json::serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::IdError;

/// Generates the identifier newtypes shared across the contract layer.
macro_rules! define_id {
    (
        $(#[$doc:meta])*
        $name:ident
    ) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Uuid);

        impl $name {
            /// Generates a fresh identifier from OS entropy.
            ///
            /// # Errors
            ///
            /// Returns [`IdError::Entropy`] when the OS entropy source fails.
            pub fn generate() -> Result<Self, IdError> {
                Uuid::new_v4().map(Self).map_err(IdError::Entropy)
            }

            /// Rehydrates an identifier from its raw 16 network-order bytes.
            ///
            /// This is the deserialization path for recorded values; it
            /// imposes no version bits, mirroring `Uuid`'s `From<[u8; 16]>`.
            #[must_use]
            pub fn from_bytes(raw: [u8; 16]) -> Self {
                Self(Uuid::from(raw))
            }

            /// The identifier's raw 16 network-order bytes.
            #[must_use]
            pub fn as_bytes(&self) -> &[u8; 16] {
                self.0.as_bytes()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                Uuid::parse(text).map(Self).map_err(IdError::Parse)
            }
        }

        #[cfg(feature = "serde")]
        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.collect_str(&self.0)
            }
        }

        #[cfg(feature = "serde")]
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let text = String::deserialize(deserializer)?;
                Uuid::parse(&text)
                    .map(Self)
                    .map_err(lgwks_serde::de::Error::custom)
            }
        }
    };
}

define_id!(
    /// Identifies a tenant: the isolation boundary for every record the
    /// runtime admits (Cell 04 ordering; E35 cursor isolation).
    TenantId
);
define_id!(
    /// Identifies one run of the finite state algebra (Cell 04).
    RunId
);
define_id!(
    /// Identifies a Moo-owned workspace, distinct from a run or tenant.
    WorkspaceId
);
define_id!(
    /// Identifies a worker process holding delegated, attenuated authority
    /// (Cell 04 authority equation; D07 standing authority).
    WorkerId
);
define_id!(
    /// Identifies a command submission; stable across retries so duplicate
    /// submission is suppressed by identity (Cell 04 ordering, E28).
    CommandId
);
define_id!(
    /// Identifies the local effect record inside one run; distinct from the
    /// stable operation ID presented to the external endpoint.
    EffectId
);
define_id!(
    /// Identifies an effect operation at the external endpoint; assigned
    /// once at dispatch and reused verbatim on every reconciliation
    /// attempt, so endpoints can deduplicate (Cell 04 effect protocol).
    OperationId
);

/// A BLAKE3 content digest (32 bytes) over canonical contract content.
///
/// Determinism is inherited from lgwks_std hashing (INV-HASH-DETERMINISTIC):
/// identical parts always produce an identical digest, which is what makes
/// dedup decisions and receipt identity replayable (E28, E55).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentDigest(hash::Digest);

impl ContentDigest {
    /// Wraps an already-computed lgwks BLAKE3 digest (crate-internal
    /// rehydration for constructors that stream their parts).
    #[must_use]
    pub(crate) const fn from_blake3(digest: hash::Digest) -> Self {
        Self(digest)
    }

    /// Hashes the given byte parts, in order, with BLAKE3.
    ///
    /// Callers pass fixed-width parts (or a single trailing variable-width
    /// part) so concatenation stays unambiguous.
    #[must_use]
    pub fn over(parts: &[&[u8]]) -> Self {
        let mut hasher = hash::Hasher::new();
        for part in parts {
            hasher.update(part);
        }
        Self(hasher.finalize())
    }

    /// The raw 32 digest bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        self.0.as_bytes()
    }

    /// Lowercase hex form (64 characters).
    #[must_use]
    pub fn to_hex(&self) -> String {
        self.0.to_hex()
    }
}

impl fmt::Display for ContentDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.to_hex())
    }
}

#[cfg(feature = "serde")]
impl Serialize for ContentDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(&self.0.to_hex())
    }
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for ContentDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        hash::Digest::from_hex(&text)
            .map(Self)
            .map_err(lgwks_serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: [u8; 16] = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32,
        0x10,
    ];

    #[test]
    fn same_bytes_rehydrate_to_equal_identifiers() {
        let run_a = RunId::from_bytes(RAW);
        let run_b = RunId::from_bytes(RAW);
        assert_eq!(run_a, run_b);
    }

    #[test]
    fn identifier_kinds_are_distinct_newtypes() {
        // Distinct kinds over the same bytes are equal to themselves but
        // cannot be assigned to each other; the type system enforces it.
        let run = RunId::from_bytes(RAW);
        let tenant = TenantId::from_bytes(RAW);
        assert_eq!(run, RunId::from_bytes(run.as_bytes().to_owned()));
        assert_eq!(tenant, TenantId::from_bytes(tenant.as_bytes().to_owned()));
    }

    #[test]
    fn display_roundtrips_through_parse() {
        let id = TenantId::from_bytes(RAW);
        let text = id.to_string();
        assert_eq!(text.len(), 36);
        let parsed = text.parse::<TenantId>();
        assert!(
            matches!(parsed, Ok(ref value) if *value == id),
            "canonical display must parse: {parsed:?}"
        );
    }

    #[test]
    fn malformed_identifier_text_is_a_typed_error() {
        let parsed = TenantId::from_str("not-a-uuid");
        assert!(matches!(parsed, Err(IdError::Parse(_))));
    }

    #[test]
    fn generate_yields_a_usable_identifier() {
        let generated = WorkerId::generate();
        assert!(
            generated.is_ok(),
            "entropy must be available: {generated:?}"
        );
    }

    #[test]
    fn digest_is_deterministic_over_identical_parts() {
        let first = ContentDigest::over(&[b"kind", b"payload"]);
        let repeated = ContentDigest::over(&[b"kind", b"payload"]);
        assert_eq!(first, repeated);
        assert_eq!(first.to_hex().len(), 64);
    }

    #[test]
    fn digest_differs_over_different_parts() {
        let alpha = ContentDigest::over(&[b"alpha"]);
        let beta = ContentDigest::over(&[b"beta"]);
        assert_ne!(alpha, beta);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn identifiers_and_digests_roundtrip_through_json() -> Result<(), Box<dyn std::error::Error>> {
        let id = RunId::from_bytes(RAW);
        let decoded: RunId = lgwks_std::json::from_str(&lgwks_std::json::to_string(&id)?)?;
        assert_eq!(decoded, id, "identifier must roundtrip through JSON");

        let digest = ContentDigest::over(&[b"x"]);
        let decoded_digest: ContentDigest =
            lgwks_std::json::from_str(&lgwks_std::json::to_string(&digest)?)?;
        assert_eq!(decoded_digest, digest, "digest must roundtrip through JSON");
        Ok(())
    }
}
