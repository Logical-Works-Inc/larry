//! A temporary copy of the governed files and the one way a journey runs the shipped
//! `larry` against it (larry#267, larry#269). `tests/governance_journey.rs` includes this
//! module by `#[path]`, so the copy, the cleared environment and the tracker edit have one
//! home.

use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use lgwks_std::hash;

use super::temp_dir::TempDir;

/// The repository the journeys copy from.
pub(crate) const ROOT: &str = env!("CARGO_MANIFEST_DIR");
/// The shipped binary.
pub(crate) const LARRY: &str = env!("CARGO_BIN_EXE_larry");
/// The top-level governed documents.
pub(crate) const DOCUMENTS: [&str; 7] = [
    "README.md",
    "index.md",
    "specification.md",
    "research-ledger.md",
    "AGENTS.md",
    "GOVERNANCE.md",
    "log.md",
];
/// The directories copied whole beside the documents.
const COPIED: [&str; 3] = ["docs", "tools/receipts", "tools/tracker"];

/// A temporary copy of the governed files, removed when dropped: the guard is the one temp
/// directory of the integration tests (larry#267).
pub(crate) struct Tree {
    /// The copy's root.
    pub(crate) path: TempDir,
}

impl Tree {
    /// A fresh copy of every file `larry governance` and `larry tracker` read. The guard makes the
    /// directory empty, so a copy never inherits another run's files.
    pub(crate) fn governed() -> Result<Tree, Box<dyn Error>> {
        let tree = Tree {
            path: TempDir::new()?,
        };
        for name in DOCUMENTS {
            fs::copy(Path::new(ROOT).join(name), tree.at(name))?;
        }
        for directory in COPIED {
            copy_files(&Path::new(ROOT).join(directory), &tree.at(directory))?;
        }
        Ok(tree)
    }

    /// The path of `relative` inside the copy.
    pub(crate) fn at(&self, relative: &str) -> PathBuf {
        self.path.join(relative)
    }
}

/// Copies every Markdown, JSON and HTML file under `from` to the same place under `to`: the
/// kinds the gate reads, never a host's ignored build litter.
fn copy_files(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let read = [".md", ".json", ".html"]
            .iter()
            .any(|suffix| name.ends_with(suffix));
        if kind.is_dir() {
            copy_files(&entry.path(), &to.join(entry.file_name()))?;
        } else if !kind.is_symlink() && read {
            fs::copy(entry.path(), to.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Every file under `root`, `/`-separated and relative, sorted.
pub(crate) fn files_under(
    root: &Path,
    directory: &Path,
    found: &mut Vec<String>,
) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            files_under(root, &path, found)?;
        } else {
            let relative = path.strip_prefix(root).map_err(io::Error::other)?;
            let parts: Vec<String> = relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            found.push(parts.join("/"));
        }
    }
    found.sort();
    Ok(())
}

/// Every file of the tree with its BLAKE3 digest.
pub(crate) fn digests(tree: &Tree) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let mut names = Vec::new();
    files_under(&tree.path, &tree.path, &mut names)?;
    let mut digested = Vec::with_capacity(names.len());
    for name in names {
        let bytes = fs::read(tree.at(&name))?;
        digested.push((name, hash::blake3(&bytes).to_hex()));
    }
    Ok(digested)
}

/// Runs the shipped `larry` with `args` from `directory`, with only `PATH` set.
pub(crate) fn larry(args: &[&str], directory: &Path) -> Result<Output, Box<dyn Error>> {
    Ok(Command::new(LARRY)
        .args(args)
        .current_dir(directory)
        .env_clear()
        .env("PATH", env::var("PATH")?)
        .stdin(Stdio::null())
        .output()?)
}

/// A stream as text.
pub(crate) fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Sets one cell's percentage in the tree's tracker data, by text, so the file stays as written.
pub(crate) fn edit_tracker(tree: &Tree, pct: u32) -> Result<(), Box<dyn Error>> {
    let path = tree.at("tools/tracker/tracker.json");
    let data = fs::read_to_string(&path)?;
    let edited = data.replacen("\"pct\": 0,", &format!("\"pct\": {pct},"), 1);
    assert_ne!(edited, data, "the tracker data has a zero cell to edit");
    fs::write(path, edited)?;
    Ok(())
}
