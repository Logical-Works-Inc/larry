//! A harness version: the hash, its parent, who wrote it and why.

use super::MAX_REASON_BYTES;
use super::error::HarnessError;
use super::tree::{HarnessHash, HarnessTree};

#[cfg(feature = "serde")]
use lgwks_std::json::serde::{Deserialize, Serialize};

/// Who authored a harness version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub enum HarnessAuthor {
    /// A person wrote it.
    Human,
    /// The proposer (a model suggesting a change) wrote it.
    Proposer,
    /// The composer (the part that merges changes) wrote it.
    Composer,
}

impl HarnessAuthor {
    /// Stable canonical tag; independent of Rust enum layout.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Human => 0,
            Self::Proposer => 1,
            Self::Composer => 2,
        }
    }
}

/// One recorded harness version. The hash is derived from the tree at construction, so a version can
/// never name a hash its tree does not have.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub struct HarnessVersion {
    /// Hash of the tree.
    hash: HarnessHash,
    /// The version this one was derived from; `None` for the first.
    parent: Option<HarnessHash>,
    /// Who wrote it.
    author: HarnessAuthor,
    /// Why, at most [`MAX_REASON_BYTES`] bytes.
    reason: String,
}

impl HarnessVersion {
    /// Records `tree` as a version.
    ///
    /// # Errors
    ///
    /// [`HarnessError::ReasonEmpty`] or [`HarnessError::ReasonTooLong`].
    pub fn new(
        tree: &HarnessTree,
        parent: Option<HarnessHash>,
        author: HarnessAuthor,
        reason: &str,
    ) -> Result<Self, HarnessError> {
        Self::naming(tree.hash(), parent, author, reason)
    }

    /// Records a version by hash alone, for a rollback to a tree the tenant already adopted. Nothing
    /// here proves the hash names a tree; the tenant's record refuses one it never adopted.
    ///
    /// # Errors
    ///
    /// [`HarnessError::ReasonEmpty`] or [`HarnessError::ReasonTooLong`].
    pub fn naming(
        hash: HarnessHash,
        parent: Option<HarnessHash>,
        author: HarnessAuthor,
        reason: &str,
    ) -> Result<Self, HarnessError> {
        if reason.is_empty() {
            return Err(HarnessError::ReasonEmpty);
        }
        if reason.len() > MAX_REASON_BYTES {
            return Err(HarnessError::ReasonTooLong {
                limit: MAX_REASON_BYTES,
                actual: reason.len(),
            });
        }
        Ok(Self {
            hash,
            parent,
            author,
            reason: reason.to_owned(),
        })
    }

    /// The tree's hash.
    #[must_use]
    pub const fn hash(&self) -> HarnessHash {
        self.hash
    }

    /// The parent version's hash.
    #[must_use]
    pub const fn parent(&self) -> Option<HarnessHash> {
        self.parent
    }

    /// Who wrote it.
    #[must_use]
    pub const fn author(&self) -> HarnessAuthor {
        self.author
    }

    /// Why it was written.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Whether `tree` is the tree this version names.
    #[must_use]
    pub fn describes(&self, tree: &HarnessTree) -> bool {
        self.hash == tree.hash()
    }
}
