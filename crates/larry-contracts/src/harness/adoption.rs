//! Per-tenant adoption records: the pure state machine behind larry#61 part 2.
//!
//! An adoption is carried by an ordinary submitted run. The tenant's record holds the trees it has
//! adopted, the adoptions in order, and at most one adoption in flight. [`TenantHarness::vet`] is the
//! one rule set; the runtime asks it before it writes a command and again when it folds the intent,
//! so check and fold cannot drift. A hash becomes active only in [`TenantHarness::settle`], which the
//! runtime calls when the run's terminal receipt says the effect succeeded.
//!
//! Rules, in order: the tree decodes canonically and hashes to the version; no adoption is in flight;
//! the version's parent is the active hash (a rollback is written against the harness it replaces:
//! its parent is the active hash and its hash an earlier one); a recorded tree must fit the version
//! bound; a rollback must name a hash this tenant adopted.

use std::collections::{BTreeMap, VecDeque};

use crate::ids::{OperationId, RunId};

use super::error::HarnessError;
use super::tree::{HarnessHash, HarnessTree};
use super::version::HarnessVersion;

#[cfg(feature = "serde")]
use lgwks_std::json::serde::{Deserialize, Serialize};

/// The most distinct trees one tenant may have adopted.
pub const MAX_ADOPTED_VERSIONS: usize = 64;
/// The most canonical tree bytes one adoption may carry. The journal record that holds the proposal
/// is capped by lgwks_bot at 64 KiB and JSON-encodes bytes as numbers, so the cap is measured, not
/// chosen from the value's own bounds (ADR 0029).
pub const MAX_ADOPTED_TREE_BYTES: usize = 2048;
/// The most adoptions one tenant's history keeps; older ones are dropped, the active one never is.
pub const MAX_ADOPTION_HISTORY: usize = 256;

/// What a run proposes: a version and, for a new harness, the tree's canonical bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub struct HarnessAdoption {
    /// The version to adopt.
    version: HarnessVersion,
    /// Canonical bytes of the tree for a new version; `None` for a rollback to an adopted hash.
    tree: Option<Vec<u8>>,
}

impl HarnessAdoption {
    /// Proposes a new harness: the version and the tree it names.
    #[must_use]
    pub fn recording(version: HarnessVersion, tree: &HarnessTree) -> Self {
        Self {
            version,
            tree: Some(tree.encode()),
        }
    }

    /// Proposes a rollback: the version names a hash this tenant already adopted.
    #[must_use]
    pub const fn rolling_back(version: HarnessVersion) -> Self {
        Self {
            version,
            tree: None,
        }
    }

    /// The version proposed.
    #[must_use]
    pub const fn version(&self) -> &HarnessVersion {
        &self.version
    }

    /// The canonical tree bytes, absent for a rollback.
    #[must_use]
    pub fn tree_bytes(&self) -> Option<&[u8]> {
        self.tree.as_deref()
    }
}

/// An adoption every rule has passed, with its tree decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vetted {
    /// The version to adopt.
    version: HarnessVersion,
    /// The decoded tree for a new version.
    tree: Option<HarnessTree>,
}

/// The adoption a tenant has recorded and not yet settled.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pending {
    /// The effect that adopts it.
    operation: OperationId,
    /// The run that carries the effect.
    run: RunId,
    /// What was vetted.
    vetted: Vetted,
}

/// One tenant's harness record. Two tenants with identical trees hold separate records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TenantHarness {
    /// Every tree this tenant has adopted, by hash.
    trees: BTreeMap<HarnessHash, HarnessTree>,
    /// Adopted versions in adoption order; the last is active.
    history: VecDeque<HarnessVersion>,
    /// The adoption in flight, if any.
    pending: Option<Pending>,
}

impl TenantHarness {
    /// The active version and its tree, if the tenant has adopted anything.
    #[must_use]
    pub fn active(&self) -> Option<(&HarnessVersion, &HarnessTree)> {
        let version = self.history.back()?;
        self.trees.get(&version.hash()).map(|tree| (version, tree))
    }

    /// The adopted versions in order, oldest kept first (at most [`MAX_ADOPTION_HISTORY`]).
    pub fn adoptions(&self) -> impl Iterator<Item = &HarnessVersion> {
        self.history.iter()
    }

    /// Whether an adoption is in flight.
    #[must_use]
    pub const fn in_flight(&self) -> bool {
        self.pending.is_some()
    }

    /// Runs every adoption rule against this record without changing it.
    ///
    /// # Errors
    ///
    /// The first refusing rule: a tree decode fault, [`HarnessError::HashMismatch`],
    /// [`HarnessError::AdoptionInFlight`], [`HarnessError::StaleParent`],
    /// [`HarnessError::TooManyVersions`] or [`HarnessError::UnknownVersion`].
    pub fn vet(&self, adoption: &HarnessAdoption) -> Result<Vetted, HarnessError> {
        if let Some(ref bytes) = adoption.tree
            && bytes.len() > MAX_ADOPTED_TREE_BYTES
        {
            return Err(HarnessError::AdoptedTreeTooLarge {
                limit: MAX_ADOPTED_TREE_BYTES,
                actual: bytes.len(),
            });
        }
        let version = adoption.version.clone();
        let tree = match adoption.tree {
            Some(ref bytes) => {
                let decoded = HarnessTree::decode(bytes)?;
                if decoded.hash() != version.hash() {
                    return Err(HarnessError::HashMismatch);
                }
                Some(decoded)
            }
            None => None,
        };
        if self.pending.is_some() {
            return Err(HarnessError::AdoptionInFlight);
        }
        if version.parent() != self.history.back().map(HarnessVersion::hash) {
            return Err(HarnessError::StaleParent);
        }
        let known = self.trees.contains_key(&version.hash());
        if tree.is_some() && !known && self.trees.len() >= MAX_ADOPTED_VERSIONS {
            return Err(HarnessError::TooManyVersions {
                limit: MAX_ADOPTED_VERSIONS,
            });
        }
        if tree.is_none() && !known {
            return Err(HarnessError::UnknownVersion);
        }
        Ok(Vetted { version, tree })
    }

    /// Records a vetted adoption as in flight. It is not active until [`Self::settle`].
    pub fn reserve(&mut self, operation: OperationId, run: RunId, vetted: Vetted) {
        self.pending = Some(Pending {
            operation,
            run,
            vetted,
        });
    }

    /// Ends the in-flight adoption of `operation`; with `activate` it becomes the active harness.
    pub fn settle(&mut self, operation: OperationId, activate: bool) {
        if self
            .pending
            .as_ref()
            .is_none_or(|pending| pending.operation != operation)
        {
            return;
        }
        let Some(pending) = self.pending.take() else {
            return;
        };
        if !activate {
            return;
        }
        let Vetted { version, tree } = pending.vetted;
        if let Some(adopted) = tree {
            self.trees.insert(version.hash(), adopted);
        }
        if self.trees.contains_key(&version.hash()) {
            self.history.push_back(version);
            while self.history.len() > MAX_ADOPTION_HISTORY {
                self.history.pop_front();
            }
        }
    }

    /// Drops the in-flight adoption carried by `run` without activating it (the run ended without a
    /// succeeded receipt).
    pub fn abandon(&mut self, run: RunId) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.run == run)
        {
            self.pending = None;
        }
    }
}
