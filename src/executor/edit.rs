//! A session's read of one file (larry#586, part one of the anchored edit,
//! larry#277): the file's text and the BLAKE3 digest an edit of it will name as
//! its precondition. The path is a writable repo-relative path beneath the
//! session tree and through no symlink the tree carries, and a refusal reads
//! nothing outside it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use lgwks_macros::Error;

use super::author::{checked_target, is_writable_path};
use super::{AuthorFault, ExecError};

/// Longest file a read takes, in bytes: the brief's per-file bound, so a read
/// never carries more than a prompt would.
const MAX_FILE_BYTES: u64 = 262_144;

/// Why a read was refused. Nothing in the tree changes on any of them.
#[derive(Debug, Error)]
#[non_exhaustive]
pub(crate) enum EditError {
    /// The path is not a writable repo-relative path, or escapes the snapshot.
    #[error("edit path {path} escapes the snapshot")]
    PathOutside {
        /// The refused path.
        path: String,
    },
    /// The path names nothing regular in the snapshot.
    #[error("edit path {path} is not a regular file")]
    NotAFile {
        /// The refused path.
        path: String,
    },
    /// The file is past the bound a read takes.
    #[error(
        "edit file {path} is {len} bytes, over the {}-byte bound",
        MAX_FILE_BYTES
    )]
    TooLarge {
        /// The refused path.
        path: String,
        /// The file's length in bytes.
        len: u64,
    },
    /// The file is not UTF-8 text.
    #[error("edit file {path} is not UTF-8 text")]
    NotText {
        /// The refused path.
        path: String,
        /// The decoding failure.
        #[source]
        cause: std::str::Utf8Error,
    },
    /// The file could not be read.
    #[error("edit file {path} could not be read: {cause}")]
    Io {
        /// The refused path.
        path: String,
        /// The filesystem failure.
        #[source]
        cause: io::Error,
    },
}

/// One file as a session read shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileView {
    /// The BLAKE3 hex of `text`.
    pub(crate) digest: String,
    /// The whole file.
    pub(crate) text: String,
}

/// The file `path` of the snapshot `root`, with its digest.
pub(crate) fn read_file(root: &Path, path: &str) -> Result<FileView, EditError> {
    let target = resolve_target(root, path)?;
    let text = read_text(&target, path)?;
    Ok(FileView {
        digest: lgwks_std::hash::blake3(text.as_bytes()).to_hex(),
        text,
    })
}

/// The file `path` names inside the snapshot `root`: writable, beneath the
/// root and through no symlink the tree carries (CWE-59).
fn resolve_target(root: &Path, path: &str) -> Result<PathBuf, EditError> {
    let outside = || EditError::PathOutside {
        path: path.to_owned(),
    };
    if !is_writable_path(path) {
        return Err(outside());
    }
    checked_target(root, path).map_err(|refused| match refused {
        ExecError::Author {
            fault: AuthorFault::Symlink,
            ..
        } => outside(),
        other => EditError::Io {
            path: path.to_owned(),
            cause: io::Error::other(other.to_string()),
        },
    })
}

/// The current text of the snapshot file `target`, called `path`.
fn read_text(target: &Path, path: &str) -> Result<String, EditError> {
    let fault = |cause| EditError::Io {
        path: path.to_owned(),
        cause,
    };
    let meta = fs::metadata(target).map_err(fault)?;
    if !meta.is_file() {
        return Err(EditError::NotAFile {
            path: path.to_owned(),
        });
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(EditError::TooLarge {
            path: path.to_owned(),
            len: meta.len(),
        });
    }
    let bytes = fs::read(target).map_err(fault)?;
    String::from_utf8(bytes).map_err(|failure| EditError::NotText {
        path: path.to_owned(),
        cause: failure.utf8_error(),
    })
}

#[cfg(test)]
#[path = "sim_edit.rs"]
mod sim;
