//! Shared support for the governance simulations: the typed error their helpers return, the
//! real tree loaded once per test, in-memory mutation, and scratch copies of the governed files
//! that remove themselves when dropped.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lgwks_std::json;
use lgwks_std::seeded::{DrawError, Seeded};

use super::documents::{self, DOCUMENTS, Documents};
use super::error::GovernanceError;

/// Why a simulation helper could not build its case.
#[derive(Debug)]
pub(crate) enum SimError {
    /// The seeded stream refused a draw.
    Draw(DrawError),
    /// A scratch file could not be read or written.
    Io(io::Error),
    /// The checker refused to load or compile.
    Refused(GovernanceError),
    /// A JSON text the case reads did not parse.
    Json(json::Error),
    /// A text the case mutates is not in the tree.
    Absent(String),
}

impl fmt::Display for SimError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Draw(ref cause) => write!(out, "seeded draw refused: {cause:?}"),
            Self::Io(ref cause) => write!(out, "scratch io: {cause}"),
            Self::Refused(ref cause) => write!(out, "checker refused: {cause}"),
            Self::Json(ref cause) => write!(out, "json refused: {cause}"),
            Self::Absent(ref what) => write!(out, "the tree lacks {what}"),
        }
    }
}

impl std::error::Error for SimError {}

impl From<DrawError> for SimError {
    fn from(cause: DrawError) -> Self {
        Self::Draw(cause)
    }
}

impl From<io::Error> for SimError {
    fn from(cause: io::Error) -> Self {
        Self::Io(cause)
    }
}

impl From<GovernanceError> for SimError {
    fn from(cause: GovernanceError) -> Self {
        Self::Refused(cause)
    }
}

/// What every simulation returns.
pub(crate) type SimResult = Result<(), Box<dyn std::error::Error>>;

/// The repository root, the directory this crate is built from.
pub(crate) fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The governed documents of the real tree.
pub(crate) fn loaded() -> Result<Documents, SimError> {
    Ok(documents::load(root())?)
}

/// The errors `validate` reports for in-memory documents.
pub(crate) fn errors_of(documents: &Documents) -> Result<Vec<String>, SimError> {
    let (_body, errors) = super::validated(documents)?;
    Ok(errors)
}

/// Asserts one error contains `expected`.
pub(crate) fn assert_refused(errors: &[String], expected: &str, seed: u64) {
    assert!(
        errors.iter().any(|error| error.contains(expected)),
        "seed {seed}: expected an error containing {expected:?}, got {errors:?}"
    );
}

/// The count `validate` reports for `label`.
pub(crate) fn count_of(documents: &Documents, label: &str) -> Result<usize, SimError> {
    let (body, _errors) = super::validated(documents)?;
    let count = body
        .get("counts")
        .and_then(|counts| counts.get(label))
        .and_then(lgwks_std::json::Value::as_u64);
    match count.map(usize::try_from) {
        Some(Ok(count)) => Ok(count),
        Some(Err(_)) | None => Err(SimError::Absent(format!("count {label}"))),
    }
}

/// The names of every loaded document, in load order.
pub(crate) fn names_of(documents: &Documents) -> Vec<String> {
    documents
        .iter()
        .map(|(name, _)| String::from(name))
        .collect()
}

/// The text of document `name`, owned.
pub(crate) fn text_of(documents: &Documents, name: &str) -> Result<String, SimError> {
    match documents.get(name) {
        Some(text) => Ok(String::from(text)),
        None => Err(SimError::Absent(String::from(name))),
    }
}

/// `docs[name] = docs[name].replace(old, new, 1)`, refusing when `old` is absent.
pub(crate) fn replace_once(
    documents: &mut Documents,
    name: &str,
    old: &str,
    new: &str,
) -> Result<(), SimError> {
    let text = text_of(documents, name)?;
    if !text.contains(old) {
        return Err(SimError::Absent(format!("{old:?} in {name}")));
    }
    documents.set(name, text.replacen(old, new, 1));
    Ok(())
}

/// One seeded element of `items`.
pub(crate) fn pick<'item, T>(rng: &mut Seeded, items: &'item [T]) -> Result<&'item T, SimError> {
    let index = rng.index(items.len())?;
    match items.get(index) {
        Some(item) => Ok(item),
        None => Err(SimError::Absent(String::from("an item to pick"))),
    }
}

/// Every document name of the real tree under `docs/`.
pub(crate) fn docs_tree(documents: &Documents) -> Vec<String> {
    documents
        .iter()
        .map(|(name, _)| String::from(name))
        .filter(|name| name.starts_with("docs/"))
        .collect()
}

/// Distinguishes the scratch trees of one test process.
static SCRATCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A scratch directory under the system temporary directory, removed when dropped.
pub(crate) struct Scratch {
    /// The directory.
    path: PathBuf,
}

impl Scratch {
    /// A fresh, empty scratch directory tagged `tag`.
    pub(crate) fn new(tag: &str) -> Result<Self, SimError> {
        let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "larry-governance-{}-{tag}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    /// A scratch copy of every file `larry governance` reads: the documents, the `docs/` tree,
    /// the receipts and the tracker.
    pub(crate) fn governed(tag: &str) -> Result<Self, SimError> {
        let scratch = Self::new(tag)?;
        let documents = loaded()?;
        for (name, text) in documents.iter() {
            scratch.write(name, text.as_bytes())?;
        }
        for directory in ["tools/receipts", "tools/tracker"] {
            for entry in fs::read_dir(root().join(directory))? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.file_type()?.is_file()
                    && (name.ends_with(".json") || name.ends_with(".html"))
                {
                    scratch.write(&format!("{directory}/{name}"), &fs::read(entry.path())?)?;
                }
            }
        }
        Ok(scratch)
    }

    /// The scratch root.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Writes `bytes` at `relative`, creating its parents.
    pub(crate) fn write(&self, relative: &str, bytes: &[u8]) -> Result<(), SimError> {
        let target = self.path.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, bytes)?;
        Ok(())
    }

    /// Reads the text at `relative`.
    pub(crate) fn read(&self, relative: &str) -> Result<String, SimError> {
        Ok(fs::read_to_string(self.path.join(relative))?)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Best effort: a leftover scratch tree never fails a simulation.
        drop(fs::remove_dir_all(&self.path));
    }
}

/// The top-level document names, owned, for a seeded pick.
pub(crate) fn top_level() -> Vec<String> {
    DOCUMENTS.iter().map(|name| String::from(*name)).collect()
}
