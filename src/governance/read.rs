//! One bounded reader for every file `larry governance` reads (larry#336): a link anywhere on a
//! governed path, a FIFO and a device are refused before the file is opened, and every read stops
//! one byte past its bound. Source: CWE-59 (link following) and CWE-400 (uncontrolled resource
//! consumption). The four readers of the governor call this one, so the rule has one home.

use std::fs;
use std::io::{self, Read};
use std::path::Path;

/// Why a governed file was refused before or while it was read.
#[derive(Debug)]
pub(crate) enum ReadFault {
    /// The file, or a directory on its path, does not exist.
    Missing(io::Error),
    /// A component of its path, from the governed root down, is a symbolic link (CWE-59).
    Symlink,
    /// The file is not a regular file: a directory, FIFO, device or socket.
    NotRegular {
        /// Whether it is a directory, which the document loader reports with its own message.
        directory: bool,
    },
    /// The file is past the byte bound; `bytes` is the length the metadata or the read reported.
    TooLarge {
        /// The length, in bytes.
        bytes: u64,
    },
    /// The operating system refused the metadata call or the read.
    Unreadable(io::Error),
}

/// Reads at most `limit` bytes of the regular file at `relative` under `root`.
///
/// Every component from `root` down is checked with `symlink_metadata`, so a link anywhere on the
/// path is refused rather than followed. The final entry must be a regular file, judged before it
/// is opened, so a FIFO or a device is refused without a blocking read. The open file is read
/// through `take(limit + 1)`, so one byte past the bound is refused and memory holds no more.
pub(crate) fn read_bounded(root: &Path, relative: &Path, limit: u64) -> Result<Vec<u8>, ReadFault> {
    let mut current = root.to_path_buf();
    let mut parts = relative.components().peekable();
    let mut length: Option<u64> = None;
    while let Some(part) = parts.next() {
        current.push(part);
        let last = parts.peek().is_none();
        let metadata = fs::symlink_metadata(&current).map_err(missing_or_unreadable)?;
        if metadata.file_type().is_symlink() {
            return Err(ReadFault::Symlink);
        }
        if last {
            if metadata.is_dir() {
                return Err(ReadFault::NotRegular { directory: true });
            }
            if !metadata.is_file() {
                return Err(ReadFault::NotRegular { directory: false });
            }
            length = Some(metadata.len());
        } else if !metadata.is_dir() {
            return Err(ReadFault::Unreadable(io::Error::other(format!(
                "{} is not a directory",
                current.display()
            ))));
        }
    }
    let Some(bytes) = length else {
        return Err(ReadFault::Missing(io::Error::from(io::ErrorKind::NotFound)));
    };
    if bytes > limit {
        return Err(ReadFault::TooLarge { bytes });
    }
    let file = fs::File::open(&current).map_err(ReadFault::Unreadable)?;
    let mut read = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut read)
        .map_err(ReadFault::Unreadable)?;
    let bytes = match u64::try_from(read.len()) {
        Ok(bytes) => bytes,
        Err(_) => return Err(ReadFault::TooLarge { bytes: u64::MAX }),
    };
    if bytes > limit {
        return Err(ReadFault::TooLarge { bytes });
    }
    Ok(read)
}

/// The fault of a metadata call: a missing path is named apart, so the document loader keeps
/// skipping an absent top-level document.
fn missing_or_unreadable(cause: io::Error) -> ReadFault {
    if cause.kind() == io::ErrorKind::NotFound {
        ReadFault::Missing(cause)
    } else {
        ReadFault::Unreadable(cause)
    }
}
