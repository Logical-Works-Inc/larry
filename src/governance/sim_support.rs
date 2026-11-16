//! Shared support for the governance simulations: the typed error their helpers return, the
//! real tree loaded once per test, in-memory mutation, and scratch copies of the governed files
//! that remove themselves when dropped.

use std::cell::OnceCell;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lgwks_std::json;
use lgwks_std::seeded::{DrawError, Seeded};

use crate::sim_lanes::{LaneError, sweep_on};

use super::documents::{self, DOCUMENTS, Documents};
use super::error::GovernanceError;
use super::patterns::Patterns;
use super::tracker::TrackerError;

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
    /// A case text could not be formatted.
    Format(fmt::Error),
    /// A lane of the seed sweep failed outright.
    Lane(LaneError),
    /// The tracker could not be rendered or read.
    Tracker(TrackerError),
}

impl fmt::Display for SimError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Draw(ref cause) => write!(out, "seeded draw refused: {cause:?}"),
            Self::Io(ref cause) => write!(out, "scratch io: {cause}"),
            Self::Refused(ref cause) => write!(out, "checker refused: {cause}"),
            Self::Json(ref cause) => write!(out, "json refused: {cause}"),
            Self::Absent(ref what) => write!(out, "the tree lacks {what}"),
            Self::Format(ref cause) => write!(out, "case text refused: {cause}"),
            Self::Lane(ref cause) => write!(out, "seed sweep: {cause}"),
            Self::Tracker(ref cause) => write!(out, "tracker: {cause}"),
        }
    }
}

impl std::error::Error for SimError {}

impl From<TrackerError> for SimError {
    fn from(cause: TrackerError) -> Self {
        Self::Tracker(cause)
    }
}

impl From<fmt::Error> for SimError {
    fn from(cause: fmt::Error) -> Self {
        Self::Format(cause)
    }
}

impl From<LaneError> for SimError {
    fn from(cause: LaneError) -> Self {
        Self::Lane(cause)
    }
}

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

thread_local! {
    /// This thread's compiled patterns, built on first use and kept for every check it runs, so
    /// a seed sweep compiles them once per lane and not once per seed.
    static PATTERNS: OnceCell<Result<Patterns, String>> = const { OnceCell::new() };
}

/// Runs `body` with this thread's compiled patterns.
pub(crate) fn with_patterns<T>(
    body: impl FnOnce(&Patterns) -> Result<T, SimError>,
) -> Result<T, SimError> {
    PATTERNS.with(|cell| {
        match *cell.get_or_init(|| Patterns::new().map_err(|cause| cause.to_string())) {
            Ok(ref patterns) => body(patterns),
            Err(ref cause) => Err(SimError::Absent(format!("compiled patterns: {cause}"))),
        }
    })
}

/// Runs `check` for every seed in `0..count` on one lane (a governance check is CPU-bound, see
/// [`sweep_on`]); the first failing seed's error is returned, in seed order.
pub(crate) fn each_seed(
    count: u64,
    check: impl Fn(u64) -> Result<(), SimError> + Sync,
) -> Result<(), SimError> {
    for outcome in sweep_on(1, count, |_lane, seed| check(seed))? {
        outcome?;
    }
    Ok(())
}

/// The errors `validate` reports for in-memory documents.
pub(crate) fn errors_of(documents: &Documents) -> Result<Vec<String>, SimError> {
    with_patterns(|patterns| {
        let (_body, errors) = super::validated(documents, patterns)?;
        Ok(errors)
    })
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
    let (body, _errors) = with_patterns(|patterns| Ok(super::validated(documents, patterns)?))?;
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

/// The whole text of `name`: a split document's index and parts (larry#155), else its text.
pub(crate) fn whole_of(documents: &Documents, name: &str) -> Result<String, SimError> {
    if documents.get(name).is_none() {
        return Err(SimError::Absent(String::from(name)));
    }
    Ok(documents.whole(name))
}

/// `docs[name] = docs[name].replace(old, new, 1)` over the whole document: the edit lands in
/// the file (index or part) that holds `old` first, whose name is returned. Refused when no
/// file of `name` holds `old`.
pub(crate) fn replace_once(
    documents: &mut Documents,
    name: &str,
    old: &str,
    new: &str,
) -> Result<String, SimError> {
    let holder = documents
        .holder(name, old)
        .ok_or_else(|| SimError::Absent(format!("{old:?} in {name}")))?;
    let text = text_of(documents, &holder)?;
    documents.set(&holder, text.replacen(old, new, 1));
    Ok(holder)
}

/// `docs[name] = docs[name].replace(old, new)` in every file of the whole document.
pub(crate) fn replace_all(
    documents: &mut Documents,
    name: &str,
    old: &str,
    new: &str,
) -> Result<(), SimError> {
    if new.contains(old) {
        return Err(SimError::Absent(format!(
            "a replacement of {old:?} that removes it"
        )));
    }
    while let Some(holder) = documents.holder(name, old) {
        let text = text_of(documents, &holder)?;
        documents.set(&holder, text.replace(old, new));
    }
    Ok(())
}

/// Appends `tail` to the whole document `name`, in its last file; returns that file's name.
pub(crate) fn append(
    documents: &mut Documents,
    name: &str,
    tail: &str,
) -> Result<String, SimError> {
    let last = documents.last_of(name);
    let text = text_of(documents, &last)?;
    documents.set(&last, format!("{text}{tail}"));
    Ok(last)
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
        Self::at(std::env::temp_dir().join(format!(
            "larry-governance-{}-{tag}-{sequence}",
            std::process::id()
        )))
    }

    /// A scratch at `path`: one already there is gone before it is made, and dropping the guard
    /// removes the tree, so a process id the system reuses never lands a test in a stale tree.
    fn at(path: PathBuf) -> Result<Self, SimError> {
        match fs::remove_dir_all(&path) {
            Ok(()) => {}
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => {}
            Err(cause) => return Err(cause.into()),
        }
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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::Scratch;

    /// A scratch left at its name by a dead process is gone before a fresh one is made, and the
    /// guard removes the tree when it drops (larry#270).
    #[test]
    fn a_scratch_starts_empty_and_removes_itself() -> Result<(), Box<dyn std::error::Error>> {
        let path =
            std::env::temp_dir().join(format!("larry-governance-stale-{}", std::process::id()));
        drop(fs::remove_dir_all(&path));
        fs::create_dir_all(path.join("old"))?;
        fs::write(path.join("old").join("file"), "left by a dead run")?;
        let fresh = Scratch::at(path)?;
        assert!(
            fs::read_dir(fresh.path())?.next().is_none(),
            "a stale tree leaked into a fresh scratch"
        );
        fs::write(fresh.path().join("mine"), "x")?;
        let kept = fresh.path().to_path_buf();
        drop(fresh);
        assert!(!kept.exists(), "the scratch outlived its guard");
        Ok(())
    }
}
