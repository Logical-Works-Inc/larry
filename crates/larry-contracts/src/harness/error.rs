//! Typed refusals of the harness value: validation, bounds and decoding.

use super::component::HarnessComponent;

/// Why a harness edit, tree or encoding was refused. Every refusal leaves the current tree as it was.
#[cfg_attr(
    feature = "serde",
    derive(lgwks_std::json::serde::Serialize, lgwks_std::json::serde::Deserialize)
)]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum HarnessError {
    /// The path is not `<component>/<name>` on the harness surface (extra segment, `..`, unknown
    /// component, separator tricks).
    #[error("path is off the harness surface")]
    OffSurface,
    /// The name breaks the charset or length rule of its component.
    #[error("invalid {} name", component.dir())]
    InvalidName {
        /// The component the name was meant for.
        component: HarnessComponent,
    },
    /// An entry is larger than its component allows.
    #[error("{} entry of {actual} bytes exceeds the limit of {limit}", component.dir())]
    Oversize {
        /// The component that refused.
        component: HarnessComponent,
        /// The per-entry byte limit.
        limit: usize,
        /// The bytes offered.
        actual: usize,
    },
    /// The component already holds as many entries as it may.
    #[error("{} already holds its limit of {limit} entries", component.dir())]
    TooManyEntries {
        /// The component that refused.
        component: HarnessComponent,
        /// The per-component entry limit.
        limit: usize,
    },
    /// The name is reserved by the host and the tenant policy refuses reserved names.
    #[error("{} name {name} is reserved", component.dir())]
    Reserved {
        /// The component holding the reserved name.
        component: HarnessComponent,
        /// The reserved name.
        name: String,
    },
    /// The tenant policy forbids this whole component.
    #[error("{} is forbidden by the tenant policy", component.dir())]
    Forbidden {
        /// The forbidden component.
        component: HarnessComponent,
    },
    /// An edit carries more operations than allowed.
    #[error("edit of {actual} operations exceeds the limit of {limit}")]
    EditTooLarge {
        /// The operation limit.
        limit: usize,
        /// The operations offered.
        actual: usize,
    },
    /// A version needs a reason.
    #[error("harness version reason is empty")]
    ReasonEmpty,
    /// A version reason is longer than allowed.
    #[error("harness version reason of {actual} bytes exceeds the limit of {limit}")]
    ReasonTooLong {
        /// The byte limit.
        limit: usize,
        /// The bytes offered.
        actual: usize,
    },
    /// The encoding ended before its declared content.
    #[error("harness encoding is truncated")]
    Truncated,
    /// Bytes remain after the declared content.
    #[error("harness encoding has trailing bytes")]
    TrailingBytes,
    /// The encoding names a format version this build does not read.
    #[error("unsupported harness encoding version {got}")]
    UnsupportedVersion {
        /// The version byte found.
        got: u8,
    },
    /// An entry names a component tag outside the closed set.
    #[error("unknown harness component tag {tag}")]
    UnknownComponent {
        /// The tag found.
        tag: u8,
    },
    /// Entries are not in canonical order.
    #[error("harness entries are not sorted")]
    Unsorted,
    /// Two entries share one component and name.
    #[error("harness entries repeat a component and name")]
    Duplicate,
    /// The adoption's tree is longer than a journal record can carry.
    #[error("adoption tree of {actual} bytes exceeds the record limit of {limit}")]
    AdoptedTreeTooLarge {
        /// The byte limit.
        limit: usize,
        /// The bytes offered.
        actual: usize,
    },
    /// The tree the adoption carries does not hash to the version it names.
    #[error("adoption tree does not match the version hash")]
    HashMismatch,
    /// This tenant already has an adoption in flight; one settles before the next is recorded.
    #[error("an adoption is already in flight for this tenant")]
    AdoptionInFlight,
    /// The version's parent is not the hash this tenant has active.
    #[error("adoption parent is not the active harness")]
    StaleParent,
    /// A rollback names a hash this tenant never adopted.
    #[error("rollback names a harness this tenant never adopted")]
    UnknownVersion,
    /// The tenant already holds as many distinct adopted trees as it may.
    #[error("tenant already holds its limit of {limit} adopted harnesses")]
    TooManyVersions {
        /// The per-tenant limit.
        limit: usize,
    },
    /// An adoption must declare the write scope so the gate decides it.
    #[error("adoption does not declare the write scope")]
    ScopeNotDeclared,
    /// The encoding is longer than any tree can encode to.
    #[error("harness encoding of {actual} bytes exceeds the limit of {limit}")]
    EncodingTooLarge {
        /// The byte limit.
        limit: usize,
        /// The bytes offered.
        actual: usize,
    },
    /// The declared entry count exceeds what any tree may hold.
    #[error("harness declares {actual} entries; no tree holds more than {limit}")]
    TreeTooLarge {
        /// The whole-tree entry limit.
        limit: usize,
        /// The count declared.
        actual: usize,
    },
}
