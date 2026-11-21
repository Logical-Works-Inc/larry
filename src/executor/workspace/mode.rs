//! The mode reset a workspace sweep runs before it removes a tree (larry#334, larry#366).
//!
//! A command that leaves a directory mode 000 makes its tree undeletable, so every directory is
//! reset to owner rwx first. The check that a name is a directory and the reset are two calls a
//! concurrent writer can sit between, so the walk holds a [`Dir`] on the workspace's parent and
//! resolves every name from the directory it was listed under: a mode is never changed through a
//! path (CWE-367, CWE-59). A link found where a directory was is reported once and never followed.
//! Source: POSIX `fchmodat(2)`, and Darwin's `AT_SYMLINK_NOFOLLOW`, which changes a link's own mode.

use std::ffi::OsStr;
use std::io::{self, Write};
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use lgwks_std::fs::capability::{Dir, FileKind};

/// The mode every directory is reset to: owner rwx, in the type `Permissions::from_mode` takes.
#[cfg(target_os = "linux")]
const OWNER_RWX: u32 = 0o700;

/// The mode every directory is reset to: owner rwx, in the type `Mode::from_raw_mode` takes.
#[cfg(not(target_os = "linux"))]
const OWNER_RWX: rustix::fs::RawMode = 0o700;

/// One entry the reset left alone because it is a symbolic link: a link where a directory was is
/// never followed and never reset, so a name swapped in after its check cannot send a mode change
/// outside the tree, and the walk goes on (larry#366, CWE-59).
#[derive(Debug)]
pub(super) struct LinkKept {
    /// The entry, as the held directory listed it.
    entry: PathBuf,
}

impl LinkKept {
    /// The entry, as the held directory listed it.
    pub(super) fn entry(&self) -> &Path {
        self.entry.as_path()
    }

    /// Reports the kept link as one typed JSON line on standard error, the operator's channel.
    fn report(&self) {
        let event = lgwks_std::json::json!({
            "workspace": "link",
            "entry": format!("{}", self.entry().display()),
        });
        let mut err = io::stderr().lock();
        let written = writeln!(err, "{event}");
        drop(written);
    }
}

/// Resets `root` and every directory beneath it to owner rwx through held descriptors, following
/// no symlink: every name is resolved from the directory it was listed under, so a link swapped in
/// after its check is reported and never followed (larry#334, larry#366, CWE-367, CWE-59). An
/// explicit stack, never recursion: a command can nest directories past any stack.
pub(super) fn reset(root: &Path) -> io::Result<()> {
    let links = reset_with(root, &mut |_| Ok(()))?;
    for link in &links {
        link.report();
    }
    Ok(())
}

/// [`reset`] with one seam after an entry is judged a directory and before its mode is reset, so a
/// simulation can swap or remove the name in the window the hardened reset must survive.
/// Production passes a no-op and resets exactly as [`reset`] does.
///
/// The directory names come from a path listing, which is only a hint: every mode change and every
/// descent resolves the name through the held [`Dir`] it was listed under, so a swap can make the
/// walk miss an entry or report it, never act outside the tree.
///
/// # Errors
///
/// The operating system's refusal while listing, opening or resetting an entry that is not a link.
pub(super) fn reset_with(
    root: &Path,
    after_check: &mut impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<Vec<LinkKept>> {
    let parent = root.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} has no parent directory", root.display()),
        )
    })?;
    let name = root.file_name().and_then(OsStr::to_str).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} does not end in a UTF-8 name", root.display()),
        )
    })?;
    let parent_dir = Dir::open(parent)?;
    let mut links = Vec::new();
    let Some(root_dir) = reset_entry(&parent_dir, name, root, after_check, &mut links)? else {
        return Ok(links);
    };
    let mut pending = vec![(root_dir, root.to_path_buf())];
    while let Some((directory, path)) = pending.pop() {
        walk(&directory, &path, after_check, &mut links, &mut pending)?;
    }
    Ok(links)
}

/// Resets and opens every directory listed under `directory`, whose path is only the seam's and the
/// event's name for an entry: a name that is not UTF-8 cannot be passed to the held [`Dir`] calls,
/// so it is left where it is and the walk goes on.
fn walk(
    directory: &Dir,
    path: &Path,
    after_check: &mut impl FnMut(&Path) -> io::Result<()>,
    links: &mut Vec<LinkKept>,
    pending: &mut Vec<(Dir, PathBuf)>,
) -> io::Result<()> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str() {
            let entry_path = entry.path();
            if let Some(child) = reset_entry(directory, name, &entry_path, after_check, links)? {
                pending.push((child, entry_path));
            }
        }
    }
    Ok(())
}

/// Judges the entry `name` under the held `parent`, resets a directory's mode and opens it, and
/// records the link or the vanished name a hostile writer left there instead (larry#366). Every
/// operation resolves through `parent`; `path` is only the seam's and the event's name.
fn reset_entry(
    parent: &Dir,
    name: &str,
    path: &Path,
    after_check: &mut impl FnMut(&Path) -> io::Result<()>,
    links: &mut Vec<LinkKept>,
) -> io::Result<Option<Dir>> {
    match parent.kind(name) {
        Ok(FileKind::Directory) => {}
        Ok(FileKind::Symlink) => {
            links.push(LinkKept {
                entry: path.to_path_buf(),
            });
            return Ok(None);
        }
        Ok(_) => return Ok(None),
        // The name may have changed since the listing; the refusal is read the same way the
        // reset's own refusal is, so a vanished name is skipped and every other is typed.
        Err(cause) => return link_or_error(parent, name, path, cause, links),
    }
    after_check(path)?;
    if let Err(cause) = reset_mode(parent, name) {
        return link_or_error(parent, name, path, cause, links);
    }
    match parent.open_subdir(name) {
        Ok(child) => Ok(Some(child)),
        Err(cause) => link_or_error(parent, name, path, cause, links),
    }
}

/// How a reset or an open that refused is read: a link is kept and reported as one event, a name
/// that went away is gone, and every other refusal is the walk's error.
fn link_or_error(
    parent: &Dir,
    name: &str,
    path: &Path,
    cause: io::Error,
    links: &mut Vec<LinkKept>,
) -> io::Result<Option<Dir>> {
    match parent.kind(name) {
        Ok(FileKind::Symlink) => {
            links.push(LinkKept {
                entry: path.to_path_buf(),
            });
            Ok(None)
        }
        Ok(_) => Err(cause),
        Err(stat) if stat.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(stat) => Err(stat),
    }
}

/// Resets the mode of `name` under the held `parent` to owner rwx, never following a final link.
/// Linux's `fchmodat` implements no no-follow flag, so the entry is opened
/// `O_PATH | O_NOFOLLOW | O_DIRECTORY` and the reset goes through that descriptor's own
/// `/proc/self/fd` name, which pins the object that was opened; a link or a file swapped in is
/// refused by the open itself (CWE-367, CWE-59).
#[cfg(target_os = "linux")]
fn reset_mode(parent: &Dir, name: &str) -> io::Result<()> {
    let entry = rustix::fs::openat(
        parent.as_fd(),
        name,
        rustix::fs::OFlags::PATH
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(io::Error::from)?;
    std::fs::set_permissions(
        format!("/proc/self/fd/{}", entry.as_raw_fd()),
        std::fs::Permissions::from_mode(OWNER_RWX),
    )
}

/// Resets the mode of `name` under the held `parent` to owner rwx, never following a final link:
/// `fchmodat` with `AT_SYMLINK_NOFOLLOW` changes a link's own mode instead of its target, as
/// Darwin and the BSDs implement it (CWE-367, CWE-59).
#[cfg(not(target_os = "linux"))]
fn reset_mode(parent: &Dir, name: &str) -> io::Result<()> {
    rustix::fs::chmodat(
        parent.as_fd(),
        name,
        rustix::fs::Mode::from_raw_mode(OWNER_RWX),
        rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
    )
    .map_err(io::Error::from)
}
