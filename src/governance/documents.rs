//! Admission and loading of the governed Markdown documents. A document refused here stops the
//! whole load, as the Python loader raised for the same cases.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, ErrorKind, Read};
use std::path::{Path, PathBuf};

use super::error::GovernanceError;
use super::pystr;

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
    let mut paths: Vec<(String, PathBuf)> = DOCUMENTS
        .iter()
        .map(|name| (String::from(*name), root.join(name)))
        .collect();
    let mut nested = Vec::new();
    let docs_root = root.join(DOCS_DIRECTORY);
    if docs_root.is_dir() {
        collect_markdown(root, &docs_root, &mut nested)?;
    }
    nested.sort_by(|left, right| left.0.split('/').cmp(right.0.split('/')));
    paths.extend(nested);
    let mut documents = Documents {
        order: Vec::new(),
        texts: BTreeMap::new(),
    };
    for (relative, path) in paths {
        if let Some(text) = read_one(&relative, &path)? {
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
    found: &mut Vec<(String, PathBuf)>,
) -> Result<(), GovernanceError> {
    let shown = relative_of(root, directory);
    let entries = fs::read_dir(directory).map_err(|cause| unreadable(&shown, cause))?;
    for entry in entries {
        let entry = entry.map_err(|cause| unreadable(&shown, cause))?;
        let path = entry.path();
        let relative = relative_of(root, &path);
        let named_md = entry.file_name().to_string_lossy().ends_with(".md");
        if named_md {
            found.push((relative.clone(), path.clone()));
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

/// Reads one admitted document. `Ok(None)` skips a missing top-level file; every refusal names
/// the document, as the Python loader's messages do, and a decode error quotes CPython's codec.
fn read_one(relative: &str, path: &Path) -> Result<Option<String>, GovernanceError> {
    let named = || String::from(relative);
    if is_symlink(path, relative)? {
        return Err(GovernanceError::Symlink { relative: named() });
    }
    if !path.exists() {
        return Ok(None);
    }
    if path.is_dir() {
        return Err(GovernanceError::IsDirectory { relative: named() });
    }
    let file = fs::File::open(path).map_err(|cause| unreadable(relative, cause))?;
    let mut bytes = Vec::new();
    file.take(MAX_DOCUMENT_BYTES_WIDE.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|cause| unreadable(relative, cause))?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(GovernanceError::TooLarge { relative: named() });
    }
    match String::from_utf8(bytes) {
        Ok(text) => Ok(Some(text)),
        Err(error) => Err(GovernanceError::NotUtf8 {
            codec: pystr::utf8_error(error.as_bytes(), &error.utf8_error()),
        }),
    }
}
