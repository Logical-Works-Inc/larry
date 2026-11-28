//! Admission and loading of the governed Markdown documents. A document refused here stops the
//! whole load, as the Python loader raised for the same cases.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;

use super::error::GovernanceError;
use super::pystr;
use super::read::{self, ReadFault};

/// The top-level documents, in the Python `DOCUMENTS` order.
pub(crate) const DOCUMENTS: [&str; 7] = [
    "README.md",
    "index.md",
    "specification.md",
    "research-ledger.md",
    "AGENTS.md",
    "GOVERNANCE.md",
    "log.md",
];
/// The top-level documents split into parts to meet the 120-line law (larry#155): each keeps its
/// name as an index, and every governed document under its directory is one of its parts, in
/// load order. The rules that read a whole document read the index and then each part's body.
pub(crate) const SPLIT: [(&str, &str); 3] = [
    ("specification.md", "docs/specification/"),
    ("research-ledger.md", "docs/research-ledger/"),
    ("log.md", "docs/log/"),
];
/// The directory whose `*.md` entries join the admitted set, at any depth.
const DOCS_DIRECTORY: &str = "docs";
/// Largest admitted document, in bytes.
pub(crate) const MAX_DOCUMENT_BYTES: usize = 2 * 1024 * 1024;
/// The same limit as a `u64`, the width the reader's byte budget is counted in.
const MAX_DOCUMENT_BYTES_WIDE: u64 = 2 * 1024 * 1024;
/// Largest admitted number of documents, counted before any is read.
pub(crate) const MAX_DOCUMENTS: usize = 4096;

/// The loaded documents, keyed by their `/`-separated path, in load order.
#[derive(Clone)]
pub(crate) struct Documents {
    /// Load order, the order the Python `dict` kept.
    order: Vec<String>,
    /// Text by name.
    texts: BTreeMap<String, String>,
}

impl Documents {
    /// The text of one document, or `None` when it is absent.
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.texts.get(name).map(String::as_str)
    }

    /// `documents.get(name, "")`: the text, or the empty text when the document is absent.
    pub(crate) fn text(&self, name: &str) -> &str {
        match self.texts.get(name) {
            Some(text) => text.as_str(),
            None => "",
        }
    }

    /// The documents in load order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.order.iter().filter_map(|name| {
            self.texts
                .get(name)
                .map(|text| (name.as_str(), text.as_str()))
        })
    }

    /// The whole text of `name`: its own text, then, for a split document, each part's body after
    /// its frontmatter, in load order. A document that is not split is its own text.
    pub(crate) fn whole(&self, name: &str) -> String {
        let mut whole = String::from(self.text(name));
        if let Some(directory) = parts_directory(name) {
            for (part, text) in self.iter() {
                if part.starts_with(directory) {
                    whole.push_str(body(text));
                }
            }
        }
        whole
    }

    /// The physical document of `name` (its index or a part) whose text holds `needle`, first in
    /// load order: where a control's edit of the whole document lands.
    #[cfg(test)]
    pub(crate) fn holder(&self, name: &str, needle: &str) -> Option<String> {
        let directory = parts_directory(name);
        self.iter()
            .filter(|&(physical, _)| {
                physical == name || directory.is_some_and(|parts| physical.starts_with(parts))
            })
            .find(|&(_, text)| text.contains(needle))
            .map(|(physical, _)| String::from(physical))
    }

    /// The last physical document of `name`: where text appended to the whole document lands.
    #[cfg(test)]
    pub(crate) fn last_of(&self, name: &str) -> String {
        let directory = parts_directory(name);
        let last = self
            .iter()
            .filter(|&(physical, _)| directory.is_some_and(|parts| physical.starts_with(parts)))
            .last()
            .map(|(physical, _)| String::from(physical));
        match last {
            Some(part) => part,
            None => String::from(name),
        }
    }

    /// The documents sorted by name, the order the report's digests are listed in.
    pub(crate) fn sorted(&self) -> impl Iterator<Item = (&str, &str)> {
        self.texts
            .iter()
            .map(|(name, text)| (name.as_str(), text.as_str()))
    }

    /// Replaces a document's text, or appends it in load order when new: the in-memory mutation
    /// the controls make, never a write to the tree.
    #[cfg(test)]
    pub(crate) fn set(&mut self, name: &str, text: String) {
        if !self.texts.contains_key(name) {
            self.order.push(String::from(name));
        }
        self.texts.insert(String::from(name), text);
    }

    /// Drops a document, as `docs.pop(name)` did.
    #[cfg(test)]
    pub(crate) fn remove(&mut self, name: &str) {
        self.order.retain(|kept| kept != name);
        self.texts.remove(name);
    }
}

/// The parts directory of a split document, `None` for one that is not split.
fn parts_directory(name: &str) -> Option<&'static str> {
    SPLIT
        .iter()
        .find(|&&(index, _)| index == name)
        .map(|&(_, directory)| directory)
}

/// The text after the frontmatter envelope (`---\n ... \n---\n`), or all of it without one.
fn body(text: &str) -> &str {
    let after = text.strip_prefix("---\n").and_then(|rest| {
        rest.find("\n---\n")
            .and_then(|end| rest.get(end.saturating_add("\n---\n".len())..))
    });
    match after {
        Some(rest) => rest,
        None => text,
    }
}

/// Loads every governed document under `root`, refusing the whole set on the first violation.
pub(crate) fn load(root: &Path) -> Result<Documents, GovernanceError> {
    let mut paths: Vec<String> = DOCUMENTS.iter().map(|name| String::from(*name)).collect();
    let mut nested = Vec::new();
    let docs_root = root.join(DOCS_DIRECTORY);
    if docs_root.is_dir() {
        collect_markdown(root, &docs_root, &mut nested)?;
    }
    nested.sort_by(|left, right| left.split('/').cmp(right.split('/')));
    paths.extend(nested);
    let mut documents = Documents {
        order: Vec::new(),
        texts: BTreeMap::new(),
    };
    for relative in paths {
        if let Some(text) = read_one(root, &relative)? {
            documents.order.push(relative.clone());
            documents.texts.insert(relative, text);
        }
    }
    Ok(documents)
}

/// Whether `path` is a symbolic link; a missing path is not one, as `Path.is_symlink` says.
fn is_symlink(path: &Path, relative: &str) -> Result<bool, GovernanceError> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(meta.file_type().is_symlink()),
        Err(cause) if cause.kind() == ErrorKind::NotFound => Ok(false),
        Err(cause) => Err(unreadable(relative, cause)),
    }
}

/// The refusal for a path the operating system would not read.
fn unreadable(relative: &str, cause: io::Error) -> GovernanceError {
    GovernanceError::Unreadable {
        relative: String::from(relative),
        cause,
    }
}

/// `rglob("*.md")`: every entry named `*.md` at any depth, a directory or a symlink included,
/// never descending into a symlinked directory. More than `MAX_DOCUMENTS` entries is refused
/// before any is read.
fn collect_markdown(
    root: &Path,
    directory: &Path,
    found: &mut Vec<String>,
) -> Result<(), GovernanceError> {
    let shown = relative_of(root, directory);
    let entries = fs::read_dir(directory).map_err(|cause| unreadable(&shown, cause))?;
    for entry in entries {
        let entry = entry.map_err(|cause| unreadable(&shown, cause))?;
        let path = entry.path();
        let relative = relative_of(root, &path);
        let named_md = entry.file_name().to_string_lossy().ends_with(".md");
        if named_md {
            found.push(relative.clone());
            if found.len().saturating_add(DOCUMENTS.len()) > MAX_DOCUMENTS {
                return Err(GovernanceError::TooMany {
                    limit: MAX_DOCUMENTS,
                });
            }
        }
        if !is_symlink(&path, &relative)? && path.is_dir() {
            collect_markdown(root, &path, found)?;
        }
    }
    Ok(())
}

/// The `/`-separated path of `path` relative to `root`.
fn relative_of(root: &Path, path: &Path) -> String {
    let inside = match path.strip_prefix(root) {
        Ok(rest) => rest,
        Err(_) => path,
    };
    inside
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<String>>()
        .join("/")
}

/// Reads one admitted document through the bounded, link-refusing reader. `Ok(None)` skips a
/// missing top-level file; every refusal names the document, as the Python loader's messages do,
/// and a decode error quotes CPython's codec.
fn read_one(root: &Path, relative: &str) -> Result<Option<String>, GovernanceError> {
    let named = || String::from(relative);
    let bytes = match read::read_bounded(root, Path::new(relative), MAX_DOCUMENT_BYTES_WIDE) {
        Ok(bytes) => bytes,
        Err(ReadFault::Missing(_)) => return Ok(None),
        Err(ReadFault::Symlink) => {
            return Err(GovernanceError::Symlink { relative: named() });
        }
        Err(ReadFault::NotRegular { directory: true }) => {
            return Err(GovernanceError::IsDirectory { relative: named() });
        }
        Err(ReadFault::NotRegular { directory: false }) => {
            return Err(GovernanceError::NotRegular { relative: named() });
        }
        Err(ReadFault::TooLarge { .. }) => {
            return Err(GovernanceError::TooLarge { relative: named() });
        }
        Err(ReadFault::Unreadable(cause)) => return Err(unreadable(relative, cause)),
    };
    match String::from_utf8(bytes) {
        Ok(text) => Ok(Some(text)),
        Err(error) => Err(GovernanceError::NotUtf8 {
            codec: pystr::utf8_error(error.as_bytes(), &error.utf8_error()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::super::sim_support::{Scratch, SimResult};
    use super::*;

    /// Requirement: the in-memory document set behaves as the Python `dict` did.
    /// Source: `tools/check_governance.py` loader. Adversarial: split parts,
    /// missing frontmatter, non-UTF-8 names and every reader refusal stay typed.
    #[test]
    fn sim_every_document_set_operation_is_typed() -> SimResult {
        // Pure helpers: split routing, bodies and relative names.
        assert_eq!(
            parts_directory("specification.md"),
            Some("docs/specification/")
        );
        assert_eq!(parts_directory("README.md"), None);
        assert_eq!(body("---\na: b\n---\nrest"), "rest");
        assert_eq!(body("no envelope"), "no envelope");
        assert_eq!(
            relative_of(Path::new("/r"), Path::new("/r/docs/a.md")),
            "docs/a.md"
        );
        assert_eq!(
            relative_of(Path::new("/r"), Path::new("/elsewhere")),
            "//elsewhere"
        );
        // In-memory set/get/text/iter/sorted/whole/holder/last/set/remove.
        let mut documents = Documents {
            order: Vec::new(),
            texts: BTreeMap::new(),
        };
        assert_eq!(documents.get("missing"), None);
        assert_eq!(documents.text("missing"), "");
        assert!(documents.whole("specification.md").is_empty());
        documents.set("README.md", String::from("hello"));
        documents.set(
            "docs/specification/part-01.md",
            String::from("---\nf: v\n---\nbody"),
        );
        assert_eq!(documents.get("README.md"), Some("hello"));
        assert!(documents.whole("specification.md").contains("body"));
        assert_eq!(
            documents.holder("specification.md", "body"),
            Some(String::from("docs/specification/part-01.md"))
        );
        assert_eq!(
            documents.last_of("specification.md"),
            "docs/specification/part-01.md"
        );
        assert_eq!(documents.last_of("README.md"), "README.md");
        assert_eq!(documents.sorted().count(), 2);
        assert_eq!(documents.iter().count(), 2);
        assert_eq!(documents.whole("README.md"), "hello");
        documents.remove("README.md");
        assert_eq!(documents.get("README.md"), None);
        // Load faults: a symlink, a directory, an over-bound file and bad UTF-8.
        let scratch = Scratch::new("documents-faults")?;
        scratch.write("README.md", b"plain")?;
        assert_eq!(load(scratch.path())?.get("README.md"), Some("plain"));
        std::fs::create_dir_all(scratch.path().join("index.md"))?;
        assert!(matches!(
            load(scratch.path()),
            Err(GovernanceError::IsDirectory { .. })
        ));
        std::fs::remove_dir_all(scratch.path().join("index.md"))?;
        #[cfg(unix)]
        {
            let target = scratch.path().join("README.md");
            std::os::unix::fs::symlink(&target, scratch.path().join("index.md"))?;
            assert!(matches!(
                load(scratch.path()),
                Err(GovernanceError::Symlink { .. })
            ));
            std::fs::remove_file(scratch.path().join("index.md"))?;
        }
        scratch.write("index.md", &[0xff, 0xfe])?;
        assert!(matches!(
            load(scratch.path()),
            Err(GovernanceError::NotUtf8 { .. })
        ));
        std::fs::remove_file(scratch.path().join("index.md"))?;
        // A FIFO where a document belongs is not a regular file.
        #[cfg(unix)]
        {
            let status = std::process::Command::new("mkfifo")
                .arg(scratch.path().join("index.md"))
                .status()?;
            assert!(status.success(), "mkfifo refused: {status}");
            assert!(matches!(
                load(scratch.path()),
                Err(GovernanceError::NotRegular { .. })
            ));
            std::fs::remove_file(scratch.path().join("index.md"))?;
        }
        // A missing tree loads with no documents; an unreadable root refuses typed.
        let absent = scratch.path().join("no-such-root");
        assert_eq!(load(&absent)?.iter().count(), 0);
        assert!(matches!(is_symlink(&absent.join("x"), "x"), Ok(false)));
        // A root that is a file refuses before any document is read.
        assert!(matches!(
            load(&scratch.path().join("README.md")),
            Err(GovernanceError::Unreadable { .. })
        ));
        // An entry the reader cannot stat refuses typed, never silently.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let blocked = scratch.path().join("blocked");
            std::fs::create_dir_all(&blocked)?;
            std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000))?;
            let refused = is_symlink(&blocked.join("child"), "blocked/child");
            std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o755))?;
            assert!(matches!(refused, Err(GovernanceError::Unreadable { .. })));
        }
        Ok(())
    }
}
