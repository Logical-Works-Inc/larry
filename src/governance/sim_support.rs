//! Shared support for the governance simulations: the typed error their helpers return, the
//! real tree loaded once per test, in-memory mutation, and scratch copies of the governed files
//! that remove themselves when dropped.

use std::cell::OnceCell;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use lgwks_macros::Error;
use lgwks_std::json;
use lgwks_std::seeded::{DrawError, Seeded};

use crate::sim_lanes::{LaneError, sweep_on};

use super::documents::{self, DOCUMENTS, Documents};
use super::error::GovernanceError;
use super::patterns::Patterns;
use super::tracker::TrackerError;

/// Why a simulation helper could not build its case.
#[derive(Debug, Error)]
pub(crate) enum SimError {
    /// The seeded stream refused a draw.
    #[error("seeded draw refused: {0:?}")]
    Draw(#[from] DrawError),
    /// A scratch file could not be read or written.
    #[error("scratch io: {0}")]
    Io(#[from] io::Error),
    /// The checker refused to load or compile.
    #[error("checker refused: {0}")]
    Refused(#[from] GovernanceError),
    /// A JSON text the case reads did not parse.
    #[error("json refused: {0}")]
    Json(json::Error),
    /// A text the case mutates is not in the tree.
    #[error("the tree lacks {0}")]
    Absent(String),
    /// A case text could not be formatted.
    #[error("case text refused: {0}")]
    Format(#[from] fmt::Error),
    /// A lane of the seed sweep failed outright.
    #[error("seed sweep: {0}")]
    Lane(#[from] LaneError),
    /// The tracker could not be rendered or read.
    #[error("tracker: {0}")]
    Tracker(#[from] TrackerError),
    /// The shared scratch guard could not make its directory.
    #[error("scratch tree: {0}")]
    Tree(#[from] crate::sim_error::SimError),
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
    items
        .get(index)
        .ok_or_else(|| SimError::Absent(String::from("an item to pick")))
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

/// A scratch directory under the system temporary directory, removed when dropped. It is the one
/// guard of the crate's tests, `executor::sim_tree::Scratch`: empty when it is made, so a process
/// id the system reuses never lands a test in a stale tree (larry#267).
pub(crate) struct Scratch {
    /// The directory and the guard that removes it.
    tree: crate::executor::sim_tree::Scratch,
}

impl Scratch {
    /// A fresh, empty scratch directory tagged `tag`.
    pub(crate) fn new(tag: &str) -> Result<Self, SimError> {
        let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let label = format!("governance-{tag}-{sequence}");
        Ok(Self {
            tree: crate::executor::sim_tree::scratch(&label)?,
        })
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
                let kind = entry.file_type()?;
                // A directory or a link is never governed data; anything else must copy or fail.
                if !kind.is_dir()
                    && !kind.is_symlink()
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
        &self.tree
    }

    /// Writes `bytes` at `relative`, creating its parents.
    pub(crate) fn write(&self, relative: &str, bytes: &[u8]) -> Result<(), SimError> {
        let target = self.tree.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, bytes)?;
        Ok(())
    }

    /// Reads the text at `relative`.
    pub(crate) fn read(&self, relative: &str) -> Result<String, SimError> {
        Ok(fs::read_to_string(self.tree.join(relative))?)
    }
}

/// The top-level document names, owned, for a seeded pick.
pub(crate) fn top_level() -> Vec<String> {
    DOCUMENTS.iter().map(|name| String::from(*name)).collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        DrawError, Scratch, SimError, SimResult, count_of, docs_tree, each_seed, loaded, names_of,
        pick, replace_all, replace_once, text_of, top_level, whole_of,
    };

    /// A leftover a dead process with this process id left at a fixture's old name is present, a
    /// fresh scratch is empty anyway, and the guard removes the tree when it drops (larry#267).
    #[test]
    fn a_scratch_starts_empty_and_removes_itself() -> Result<(), Box<dyn std::error::Error>> {
        let stale = crate::executor::sim_tree::leftover("larry-governance-stale", "file")?;
        assert!(
            stale.join("file").exists(),
            "the stale file of a dead run is present"
        );
        let fresh = Scratch::new("stale")?;
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

    /// Every helper that cannot build its case says so by its own variant, so a simulation never
    /// passes on a case it did not make (CWE-754).
    #[test]
    fn sim_every_simulation_helper_refusal_is_typed() -> SimResult {
        use lgwks_std::seeded::Seeded;
        let base = loaded()?;
        let absent = |refused: Result<(), SimError>| match refused {
            Err(SimError::Absent(what)) => Ok(what),
            other => Err(SimError::Absent(format!("an absence, got {other:?}"))),
        };
        let mut documents = base.clone();
        for refused in [
            text_of(&base, "no-such.md").map(drop),
            whole_of(&base, "no-such.md").map(drop),
            count_of(&base, "no-such-count").map(drop),
            replace_once(&mut documents, "README.md", "definitely absent", "x").map(drop),
            replace_all(&mut documents, "README.md", "x", "xx"),
        ] {
            let what = absent(refused)?;
            assert!(!what.is_empty(), "the refusal names what is missing");
        }
        let mut rng = Seeded::from_seed(1);
        let empty: &[String] = &[];
        let drawn = pick(&mut rng, empty);
        assert!(
            matches!(drawn, Err(SimError::Draw(DrawError::EmptyRange))),
            "{drawn:?}"
        );
        assert!(names_of(&base).contains(&String::from("README.md")));
        assert!(
            docs_tree(&base)
                .iter()
                .all(|name| name.starts_with("docs/"))
        );
        assert_eq!(top_level().len(), 7);

        let scratch = Scratch::new("helper-roundtrip")?;
        scratch.write("nested/file.txt", b"hello")?;
        assert_eq!(scratch.read("nested/file.txt")?, "hello");
        let missing = scratch.read("missing.txt");
        assert!(
            matches!(&missing, Err(SimError::Io(cause))
                if cause.kind() == std::io::ErrorKind::NotFound),
            "{missing:?}"
        );
        let first = each_seed(4, |seed| {
            if seed >= 2 {
                return Err(SimError::Absent(format!("seed {seed}")));
            }
            Ok(())
        });
        assert_eq!(absent(first)?, "seed 2", "the first failure in seed order");
        Ok(())
    }
}
