//! One bounded reader for every file `larry governance` reads (larry#336, larry#359): a link
//! anywhere on a governed path, a FIFO and a device are refused, and every read stops one byte
//! past its bound. The root and every directory component are held open with `openat`, and the
//! final component is opened no-follow and non-blocking through the held descriptor, so a link
//! or FIFO swapped in between the check and the open is refused too. Source: CWE-59 (link
//! following), CWE-367 (TOCTOU) and CWE-400 (uncontrolled resource consumption). The four
//! readers of the governor, a session's file read (`executor::edit`, larry#606) and the read
//! of a brief's files (`executor::author`, larry#617) call this one, so the rule has one home.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path};

use lgwks_std::fs::capability::{Dir, FileKind, OpenFlags};

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
/// The governed root is opened once with [`Dir::open`] and every directory component beneath it
/// is resolved from that held descriptor, so a name swapped under the walk cannot redirect it.
/// The final component is judged by name, then opened with [`OpenFlags::read`] (no final link,
/// non-blocking); the opened descriptor's own `fstat` refuses every kind but a regular file, and
/// its length read from that descriptor must be at most `limit` bytes. The open file is read
/// through `take(limit + 1)`, so one byte past the bound is refused and memory holds no more.
pub(crate) fn read_bounded(root: &Path, relative: &Path, limit: u64) -> Result<Vec<u8>, ReadFault> {
    read_bounded_with(root, relative, limit, |_| Ok(()))
}

/// [`read_bounded`] with one seam after the final component is judged and before it is opened, so
/// a simulation can swap the name in the window the hardened open must survive. Production passes
/// a no-op and reads exactly as `read_bounded` does.
///
/// # Errors
///
/// The same faults as `read_bounded`, and `Unreadable` when the seam itself fails.
pub(crate) fn read_bounded_with(
    root: &Path,
    relative: &Path,
    limit: u64,
    after_check: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<Vec<u8>, ReadFault> {
    let (_, file) = open_regular(root, relative, after_check)?;
    read_open(&file, limit, || Ok(()))
}

/// The regular file at `relative` under `root`, opened by the walk [`read_bounded`] describes,
/// with the directory that holds it, held open.
///
/// # Errors
///
/// As [`read_bounded_with`], less the length bound, which [`read_open`] holds.
pub(crate) fn open_regular(
    root: &Path,
    relative: &Path,
    after_check: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(Dir, File), ReadFault> {
    let mut directory = Dir::open(root).map_err(missing_or_unreadable)?;
    let mut current = root.to_path_buf();
    let mut parts = relative.components().peekable();
    while let Some(part) = parts.next() {
        let Component::Normal(name) = part else {
            return Err(ReadFault::Unreadable(io::Error::other(format!(
                "{} is not a relative path",
                relative.display()
            ))));
        };
        let Some(name) = name.to_str() else {
            return Err(ReadFault::Unreadable(io::Error::other(format!(
                "{} is not a UTF-8 path",
                relative.display()
            ))));
        };
        current.push(name);
        if parts.peek().is_none() {
            check_final(&directory, name)?;
            after_check(&current).map_err(ReadFault::Unreadable)?;
            let file = open_final(&directory, name)?;
            return Ok((directory, file));
        }
        match directory.kind(name).map_err(missing_or_unreadable)? {
            FileKind::Directory => {}
            FileKind::Symlink => return Err(ReadFault::Symlink),
            _ => return Err(not_a_directory(&current)),
        }
        directory = directory
            .open_subdir(name)
            .map_err(|cause| opened_dir_fault(&directory, name, &current, cause))?;
    }
    Err(ReadFault::Missing(io::Error::from(io::ErrorKind::NotFound)))
}

/// Refuses the final component by name before it is opened: a link, a directory, a FIFO or a
/// device. The open that follows judges it again, because this check and that open are two
/// calls a concurrent writer can sit between (CWE-367).
fn check_final(directory: &Dir, name: &str) -> Result<(), ReadFault> {
    match directory.kind(name).map_err(missing_or_unreadable)? {
        FileKind::File => Ok(()),
        FileKind::Symlink => Err(ReadFault::Symlink),
        FileKind::Directory => Err(ReadFault::NotRegular { directory: true }),
        _ => Err(ReadFault::NotRegular { directory: false }),
    }
}

/// Opens the final component through the held directory: no final link, non-blocking, so a FIFO
/// swapped in after the check cannot block the reader.
fn open_final(directory: &Dir, name: &str) -> Result<File, ReadFault> {
    match directory.open_entry(name, OpenFlags::read()) {
        Ok(file) => Ok(file),
        Err(cause) => Err(final_fault(directory, name, cause)),
    }
}

/// The fault of a final open, read from the name's kind at the moment the open refused it: a
/// link is a link, a directory keeps its own refusal, and a FIFO or a device is a `NotRegular`.
/// The kind is read without following, and nothing is read from the refused name.
fn final_fault(directory: &Dir, name: &str, cause: io::Error) -> ReadFault {
    match directory.kind(name) {
        Ok(FileKind::Symlink) => ReadFault::Symlink,
        Ok(FileKind::Directory) => ReadFault::NotRegular { directory: true },
        Ok(FileKind::File) => ReadFault::Unreadable(cause),
        Ok(_) => ReadFault::NotRegular { directory: false },
        Err(stat) => missing_or_unreadable(stat),
    }
}

/// Judges the file the reader opened by its descriptor's metadata. `open_entry` has already
/// refused every kind but a regular file from that descriptor's own `fstat`; this reads the
/// length from the same descriptor and then reads at most `limit` bytes plus the one that proves
/// the bound was passed, so a file that grows after the length check is still refused.
/// `after_stat` is the window between the two, for a simulation; production passes a no-op.
///
/// # Errors
///
/// `TooLarge` past `limit`, `Unreadable` when the metadata call, the window or the read fails.
pub(crate) fn read_open(
    file: &File,
    limit: u64,
    after_stat: impl FnOnce() -> io::Result<()>,
) -> Result<Vec<u8>, ReadFault> {
    let metadata = file.metadata().map_err(ReadFault::Unreadable)?;
    if metadata.len() > limit {
        return Err(ReadFault::TooLarge {
            bytes: metadata.len(),
        });
    }
    after_stat().map_err(ReadFault::Unreadable)?;
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

/// The refusal for a path component that is not a directory.
fn not_a_directory(path: &Path) -> ReadFault {
    ReadFault::Unreadable(io::Error::other(format!(
        "{} is not a directory",
        path.display()
    )))
}

/// The fault of opening a directory, read from the name's kind at the moment the open refused
/// it: a link swapped in after its check is a link, anything that is not a directory keeps the
/// reader's own message, and a name that went away is missing.
fn opened_dir_fault(directory: &Dir, name: &str, path: &Path, cause: io::Error) -> ReadFault {
    match directory.kind(name) {
        Ok(FileKind::Symlink) => ReadFault::Symlink,
        Ok(FileKind::Directory) => ReadFault::Unreadable(cause),
        Ok(_) => not_a_directory(path),
        Err(stat) => missing_or_unreadable(stat),
    }
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
