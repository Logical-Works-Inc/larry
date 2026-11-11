//! Grader-blind learning as a type boundary (larry#63, ADR 0030).
//!
//! Nothing a grader, verifier, rubric or score produced may reach the model that proposes, composes,
//! selects or reflects. Evidence carries its provenance in its type: [`Evidence<P>`] is generic over
//! [`Execution`], [`Deliverable`], [`Source`] or [`Grader`]. [`ImproverSafe`] is sealed and is
//! implemented for the first three only, so the only way into an [`ImproverItem`], and from there an
//! [`ImproverContext`], is [`ImproverItem::new`], which a `Evidence<Grader>` cannot call: the compiler
//! refuses it, no runtime check is involved. Bytes of unknown provenance come as [`AnyEvidence`] and
//! convert only through [`AnyEvidence::into_improver`], which refuses a grader.
//!
//! [`ImproverContext::assemble`] then scans every item (see `scan.rs`) before the context exists and
//! fails closed with a [`LeakError`] naming the path. The scan is defence in depth: it cannot prove a
//! text clean, and a type cannot see a rubric a person pasted into a deliverable.
//!
//! Bounds: a path is 1 to 3 segments of 1 to 64 bytes of `a-z`, `0-9`, `-`, `_`, `.` (not starting
//! with `-` or `.`), at most 192 bytes; an item is at most 16 KiB; a context is at most 128 items and
//! 256 KiB.
//!
//! The type boundary, with its positive control:
//!
//! ```
//! use larry_contracts::evidence::{Evidence, Execution, ImproverItem};
//! let log = Evidence::<Execution>::new("rollout/run.log", b"exit 0").unwrap();
//! let item = ImproverItem::new(log);
//! assert_eq!(item.path(), "rollout/run.log");
//! ```
//!
//! A grader's evidence cannot enter an improver item (the compiler refuses):
//!
//! ```compile_fail
//! use larry_contracts::evidence::{Evidence, Grader, ImproverItem};
//! let verdict = Evidence::<Grader>::new("rollout/verdict", b"pass").unwrap();
//! let _ = ImproverItem::new(verdict);
//! ```
//!
//! And no one outside the crate can declare a new provenance safe (the trait is sealed):
//!
//! ```compile_fail
//! use larry_contracts::evidence::{Grader, ImproverSafe};
//! impl ImproverSafe for Grader {}
//! ```

mod scan;

use core::marker::PhantomData;

/// The longest evidence path in bytes.
pub const MAX_PATH_BYTES: usize = 192;
/// The most bytes one evidence item may hold.
pub const MAX_EVIDENCE_BYTES: usize = 16 * 1024;
/// The most items one improver context may hold.
pub const MAX_CONTEXT_ITEMS: usize = 128;
/// The most bytes one improver context may hold in all.
pub const MAX_CONTEXT_BYTES: usize = 256 * 1024;

/// Seals [`Provenance`] and [`ImproverSafe`] so only this module decides what is safe.
mod sealed {
    /// Implemented for the provenance markers of this module only.
    pub trait Sealed {}
}

/// Where a piece of evidence came from, as a value (for bytes read back from a journal or a disk).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum EvidenceKind {
    /// What the agent did: commands, outputs, traces.
    Execution,
    /// What the agent produced for the person.
    Deliverable,
    /// Material the task supplied.
    Source,
    /// Anything a grader, verifier, rubric or score produced.
    Grader,
}

impl EvidenceKind {
    /// Stable canonical tag; independent of Rust enum layout.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Execution => 0,
            Self::Deliverable => 1,
            Self::Source => 2,
            Self::Grader => 3,
        }
    }
}

/// A provenance marker type.
pub trait Provenance: sealed::Sealed {
    /// The runtime value of this marker.
    const KIND: EvidenceKind;
}

/// A provenance an improver may see. Sealed, and implemented for every marker but [`Grader`].
pub trait ImproverSafe: Provenance {}

/// Provenance: what the agent did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Execution;
/// Provenance: what the agent produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Deliverable;
/// Provenance: material the task supplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Source;
/// Provenance: a grader, verifier, rubric or score. Never improver-safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Grader;

impl sealed::Sealed for Execution {}
impl sealed::Sealed for Deliverable {}
impl sealed::Sealed for Source {}
impl sealed::Sealed for Grader {}
impl Provenance for Execution {
    const KIND: EvidenceKind = EvidenceKind::Execution;
}
impl Provenance for Deliverable {
    const KIND: EvidenceKind = EvidenceKind::Deliverable;
}
impl Provenance for Source {
    const KIND: EvidenceKind = EvidenceKind::Source;
}
impl Provenance for Grader {
    const KIND: EvidenceKind = EvidenceKind::Grader;
}
impl ImproverSafe for Execution {}
impl ImproverSafe for Deliverable {}
impl ImproverSafe for Source {}

/// Why evidence could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EvidenceError {
    /// The path breaks the path rule.
    #[error("evidence path is invalid")]
    InvalidPath,
    /// The item is larger than allowed.
    #[error("evidence of {actual} bytes exceeds the limit of {limit}")]
    TooLarge {
        /// The byte limit.
        limit: usize,
        /// The bytes offered.
        actual: usize,
    },
}

/// What the scan or the type refusal found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LeakKind {
    /// A grader's verdict.
    GraderVerdict,
    /// A rubric line.
    RubricLine,
    /// A score.
    Score,
    /// The item's provenance is the grader's.
    GraderProvenance,
}

/// An item the improver must not see, and where it is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{kind:?} in {path}")]
#[non_exhaustive]
pub struct LeakError {
    /// The logical path of the refused item.
    pub path: String,
    /// What was found.
    pub kind: LeakKind,
}

impl LeakError {
    /// A refusal of the item at `path`.
    #[must_use]
    pub fn new(path: &str, kind: LeakKind) -> Self {
        Self {
            path: path.to_owned(),
            kind,
        }
    }
}

/// Why a context was not assembled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ContextError {
    /// An item leaks grader material.
    #[error(transparent)]
    Leak(#[from] LeakError),
    /// More items than a context holds.
    #[error("context of {actual} items exceeds the limit of {limit}")]
    TooManyItems {
        /// The item limit.
        limit: usize,
        /// The items offered.
        actual: usize,
    },
    /// More bytes than a context holds.
    #[error("context of {actual} bytes exceeds the limit of {limit}")]
    TooLarge {
        /// The byte limit.
        limit: usize,
        /// The bytes offered.
        actual: usize,
    },
}

/// Whether `path` obeys the path rule.
fn path_is_valid(path: &str) -> bool {
    path.len() <= MAX_PATH_BYTES
        && (1..=3).contains(&path.split('/').count())
        && path.split('/').all(|segment| {
            segment.len() <= 64
                && segment
                    .bytes()
                    .next()
                    .is_some_and(|first| first != b'-' && first != b'.')
                && segment
                    .bytes()
                    .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.'))
        })
}

/// Checks a path and a byte count together.
fn checked(path: &str, len: usize) -> Result<(), EvidenceError> {
    if !path_is_valid(path) {
        return Err(EvidenceError::InvalidPath);
    }
    if len > MAX_EVIDENCE_BYTES {
        return Err(EvidenceError::TooLarge {
            limit: MAX_EVIDENCE_BYTES,
            actual: len,
        });
    }
    Ok(())
}

/// One bounded piece of evidence whose provenance is its type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence<P: Provenance> {
    /// The logical path.
    path: String,
    /// The content.
    bytes: Vec<u8>,
    /// The provenance, carried in the type only.
    provenance: PhantomData<P>,
}

impl<P: Provenance> Evidence<P> {
    /// Builds evidence of provenance `P`.
    ///
    /// # Errors
    ///
    /// [`EvidenceError`] for a bad path or an oversize item.
    pub fn new(path: &str, bytes: &[u8]) -> Result<Self, EvidenceError> {
        checked(path, bytes.len())?;
        Ok(Self {
            path: path.to_owned(),
            bytes: bytes.to_vec(),
            provenance: PhantomData,
        })
    }
}

/// Evidence whose provenance is only known at run time, such as bytes read back from a journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnyEvidence {
    /// The claimed provenance.
    kind: EvidenceKind,
    /// The logical path.
    path: String,
    /// The content.
    bytes: Vec<u8>,
}

impl AnyEvidence {
    /// Builds evidence of a runtime-known provenance.
    ///
    /// # Errors
    ///
    /// [`EvidenceError`] for a bad path or an oversize item.
    pub fn new(kind: EvidenceKind, path: &str, bytes: &[u8]) -> Result<Self, EvidenceError> {
        checked(path, bytes.len())?;
        Ok(Self {
            kind,
            path: path.to_owned(),
            bytes: bytes.to_vec(),
        })
    }

    /// The only way from runtime-tagged evidence to something an improver may hold.
    ///
    /// # Errors
    ///
    /// [`LeakError`] with [`LeakKind::GraderProvenance`] for grader evidence.
    pub fn into_improver(self) -> Result<ImproverItem, LeakError> {
        match self.kind {
            EvidenceKind::Grader => Err(LeakError::new(&self.path, LeakKind::GraderProvenance)),
            safe => Ok(ImproverItem {
                kind: safe,
                path: self.path,
                bytes: self.bytes,
            }),
        }
    }
}

/// An evidence item an improver may hold: its provenance is not the grader's, by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImproverItem {
    /// One of the three safe kinds.
    kind: EvidenceKind,
    /// The logical path.
    path: String,
    /// The content.
    bytes: Vec<u8>,
}

impl ImproverItem {
    /// Wraps evidence whose provenance is improver-safe. `Evidence<Grader>` does not compile here.
    #[must_use]
    pub fn new<P: ImproverSafe>(evidence: Evidence<P>) -> Self {
        Self {
            kind: P::KIND,
            path: evidence.path,
            bytes: evidence.bytes,
        }
    }

    /// The provenance, never [`EvidenceKind::Grader`].
    #[must_use]
    pub const fn kind(&self) -> EvidenceKind {
        self.kind
    }

    /// The logical path.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The content.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// What a proposer, composer, selector or reflector may receive. It exists only if every item passed
/// the scan, and it cannot hold grader provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImproverContext {
    /// The scanned items in the order given.
    items: Vec<ImproverItem>,
}

impl ImproverContext {
    /// Scans every item and assembles the context, or refuses the whole of it.
    ///
    /// # Errors
    ///
    /// [`ContextError::Leak`] naming the first leaking item, or a bound error. Nothing is
    /// assembled on refusal.
    pub fn assemble(items: Vec<ImproverItem>) -> Result<Self, ContextError> {
        if items.len() > MAX_CONTEXT_ITEMS {
            return Err(ContextError::TooManyItems {
                limit: MAX_CONTEXT_ITEMS,
                actual: items.len(),
            });
        }
        let total = items
            .iter()
            .fold(0_usize, |sum, item| sum.saturating_add(item.bytes.len()));
        if total > MAX_CONTEXT_BYTES {
            return Err(ContextError::TooLarge {
                limit: MAX_CONTEXT_BYTES,
                actual: total,
            });
        }
        for item in &items {
            if let Some(kind) = scan::scan(&item.bytes) {
                return Err(LeakError::new(&item.path, kind).into());
            }
        }
        Ok(Self { items })
    }

    /// The items, in order.
    #[must_use]
    pub fn items(&self) -> &[ImproverItem] {
        &self.items
    }
}

impl<P: Provenance> From<Evidence<P>> for AnyEvidence {
    fn from(evidence: Evidence<P>) -> Self {
        Self {
            kind: P::KIND,
            path: evidence.path,
            bytes: evidence.bytes,
        }
    }
}
