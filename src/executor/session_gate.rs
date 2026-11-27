use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lgwks_macros::Error;

/// Most paths one typed finish request may name.
pub(super) const MAX_FINISH_PATHS: usize = 64;
/// Largest path one typed finish request may name.
pub(super) const MAX_FINISH_PATH_BYTES: usize = 1024;
/// Most entries (files, links and directories) one check fingerprint walks: the
/// snapshot copy bound (`workspace::snapshot::MAX_FILES`), so one tree bound holds
/// the copy and the gate together.
pub(super) const MAX_FINGERPRINT_ENTRIES: usize = 20_000;
/// Deepest directory nesting one check fingerprint walks: the snapshot walk bound
/// (`workspace::snapshot` walks depth 64), so one depth holds both.
pub(super) const MAX_FINGERPRINT_DEPTH: usize = 64;
/// Most file content bytes one check fingerprint hashes (256 MiB, the snapshot copy
/// bound `workspace::snapshot::MAX_BYTES`), so one byte bound holds both.
pub(super) const MAX_FINGERPRINT_BYTES: u64 = 268_435_456;
/// Bytes hashed from one file per read while fingerprinting a check.
const FINGERPRINT_CHUNK: usize = 8_192;

/// The model's typed completion request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FinishRequest {
    /// The summary shown after the second accepted request.
    pub(super) summary: String,
    /// The exact command whose fresh receipt proves the task.
    pub(super) check: String,
    /// Relative paths that must be present after the check.
    pub(super) artifacts: Vec<String>,
    /// State the operator should expect to remain.
    pub(super) state: Vec<String>,
    /// Servers the operator must keep running.
    pub(super) servers: Vec<String>,
}

/// Why a typed finish request cannot become a receipt.
#[derive(Debug, Error)]
pub(super) enum VerificationFault {
    /// No successful fresh-shell check is the last command.
    #[error("the named check has no fresh passing receipt after the last command")]
    NoFreshCheck,
    /// An artifact name could escape the session tree.
    #[error("artifact path {path} must be relative and normal")]
    BadArtifactPath {
        /// The refused path.
        path: String,
    },
    /// A named artifact is absent or unreadable.
    #[error("artifact path {path} is missing: {source}")]
    MissingArtifact {
        /// The refused path.
        path: String,
        /// The metadata failure.
        source: io::Error,
    },
    /// A named artifact is a symlink.
    #[error("artifact path {path} is a symlink")]
    SymlinkArtifact {
        /// The refused path.
        path: String,
    },
    /// The named check changed the session tree at `path`; a check must change nothing.
    #[error("the check changed the tree at {path}: a check must change nothing")]
    CheckMutated {
        /// The first relative path that differs, in sorted order.
        path: String,
    },
    /// The session tree is past a fingerprint bound, so the check is refused, never passed.
    #[error("the check's tree is past its fingerprint bound: more than {limit} {what}")]
    FingerprintTooLarge {
        /// What was counted: entries, directory levels or bytes.
        what: &'static str,
        /// The bound it passed.
        limit: u64,
    },
    /// The session tree could not be fingerprinted at `path`.
    #[error("the check's tree could not be fingerprinted at {path}: {source}")]
    FingerprintIo {
        /// The relative path that could not be read.
        path: String,
        /// Why it could not be read.
        source: io::Error,
    },
}

/// The two stable identities a session report carries.
pub(super) struct SessionIdentity<'a> {
    /// The run id.
    pub(super) task_id: &'a str,
    /// The input document digest.
    pub(super) task_digest: &'a str,
}

/// The first accepted finish shows the operator the state the model says will remain.
pub(super) fn checklist(request: &FinishRequest) -> String {
    format!(
        "verification checklist: artifacts [{}]; state left behind [{}]; servers to keep up [{}]. Repeat the same finish to end.",
        request.artifacts.join(", "),
        request.state.join(", "),
        request.servers.join(", "),
    )
}

/// A finish may name only the last successful fresh-shell command as its check.
pub(super) fn has_fresh_check(last_command: Option<&str>, last_passed: bool, check: &str) -> bool {
    last_passed && last_command == Some(check)
}

/// One entry of a check fingerprint: its kind and, for a file, its length and BLAKE3 digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FingerprintEntry {
    /// A directory; its members are fingerprinted as their own entries.
    Dir,
    /// A symlink, never followed; its target text is the entry.
    Link {
        /// Where the link points, as written.
        target: String,
    },
    /// A regular file; its length and BLAKE3 digest are the entry.
    File {
        /// Its length in bytes.
        len: u64,
        /// The BLAKE3 digest of its bytes, as 64 hex characters.
        digest: String,
    },
    /// Anything with no bytes to hash: a fifo, a socket or a device.
    Other,
}

/// The bounded fingerprint of a session tree: relative paths to their entries, sorted.
pub(super) type Fingerprint = BTreeMap<String, FingerprintEntry>;

/// Why a check fingerprint could not be taken. Clone, so a session turn can keep
/// the refusal past the command it belongs to and the finish can report it typed.
/// An unreadable tree keeps its I/O source behind `Arc`, so the clone shares it
/// instead of dropping it.
#[derive(Debug, Clone, Error)]
pub(crate) enum FingerprintFault {
    /// The tree passed a fingerprint bound; the check is refused, never passed.
    #[error("the check's tree is past its fingerprint bound: more than {limit} {what}")]
    TooLarge {
        /// What was counted: entries, directory levels or bytes.
        what: &'static str,
        /// The bound it passed.
        limit: u64,
    },
    /// The tree could not be read at `path`.
    #[error("the check's tree could not be fingerprinted at {path}: {source}")]
    Unreadable {
        /// The relative path that could not be read.
        path: String,
        /// Why it could not be read; shared, never dropped.
        source: Arc<io::Error>,
    },
}

/// What the last command proved about the session tree for the typed finish gate.
#[derive(Debug, Clone)]
pub(super) enum CheckProof {
    /// Its before and after fingerprints match: the command changed nothing.
    Clean,
    /// Its after fingerprint differs at `path`: the command is no check.
    Mutated {
        /// The first relative path that differs, in sorted order.
        path: String,
    },
    /// No fingerprint could be taken for the command.
    Fault(FingerprintFault),
}

/// Whether an entry of this kind has no bytes to hash: a fifo, a socket or a
/// device on Unix; nothing elsewhere, where only files, links and directories
/// reach the walk. Shared with the verification sim's reference walk, whose
/// traversal, whole-byte comparison and lack of bounds still differ.
#[cfg(unix)]
pub(super) fn is_special(kind: &std::fs::FileType) -> bool {
    use std::os::unix::fs::FileTypeExt as _;
    kind.is_fifo() || kind.is_socket() || kind.is_block_device() || kind.is_char_device()
}

/// Whether an entry of this kind has no bytes to hash: nothing elsewhere, where
/// only files, links and directories reach the walk.
#[cfg(not(unix))]
pub(super) fn is_special(_kind: &std::fs::FileType) -> bool {
    false
}

/// The refusal for a tree past a fingerprint bound of `what`.
fn too_large(what: &'static str, limit: usize) -> FingerprintFault {
    match u64::try_from(limit) {
        Ok(limit) => FingerprintFault::TooLarge { what, limit },
        Err(source) => FingerprintFault::Unreadable {
            path: String::from("."),
            source: Arc::new(io::Error::new(io::ErrorKind::InvalidData, source)),
        },
    }
}

/// Whether `entries` walked with `pending` still queued is past the check
/// fingerprint's entry bound: at most [`MAX_FINGERPRINT_ENTRIES`] entries may
/// be recorded or queued, past which the check is refused typed, never passed.
fn past_entries_bound(entries: usize, pending: usize) -> bool {
    entries.saturating_add(pending) > MAX_FINGERPRINT_ENTRIES
}

/// The refusal for a tree that could not be read at `relative`; the I/O source
/// moves into the fault, so nothing about why is dropped.
fn unreadable(relative: &Path, source: io::Error) -> FingerprintFault {
    FingerprintFault::Unreadable {
        path: relative.to_string_lossy().into_owned(),
        source: Arc::new(source),
    }
}

/// Hashes the file at `full` into the running `hashed` byte count, refusing past
/// [`MAX_FINGERPRINT_BYTES`]. Reads through [`FINGERPRINT_CHUNK`] pieces, so memory
/// holds no more.
fn hash_file(full: &Path, hashed: &mut u64) -> Result<String, FingerprintFault> {
    use std::io::Read as _;
    let file = std::fs::File::open(full).map_err(|source| FingerprintFault::Unreadable {
        path: full.to_string_lossy().into_owned(),
        source: Arc::new(source),
    })?;
    let mut hasher = lgwks_std::hash::Hasher::new();
    let mut limited = file.take(MAX_FINGERPRINT_BYTES.saturating_add(1));
    let mut chunk = [0_u8; FINGERPRINT_CHUNK];
    loop {
        match limited.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                let Some(piece) = chunk.get(..read) else {
                    return Err(FingerprintFault::Unreadable {
                        path: full.to_string_lossy().into_owned(),
                        source: Arc::new(io::Error::other(
                            "a read reported more bytes than its buffer holds",
                        )),
                    });
                };
                hasher.update(piece);
                let bytes = u64::try_from(read).map_err(|source| FingerprintFault::Unreadable {
                    path: full.to_string_lossy().into_owned(),
                    source: Arc::new(io::Error::new(io::ErrorKind::InvalidData, source)),
                })?;
                *hashed = hashed.saturating_add(bytes);
            }
            Err(source) if source.kind() == io::ErrorKind::Interrupted => {}
            Err(source) => {
                return Err(FingerprintFault::Unreadable {
                    path: full.to_string_lossy().into_owned(),
                    source: Arc::new(source),
                });
            }
        }
    }
    if *hashed > MAX_FINGERPRINT_BYTES {
        return Err(FingerprintFault::TooLarge {
            what: "bytes",
            limit: MAX_FINGERPRINT_BYTES,
        });
    }
    Ok(hasher.finalize().to_hex())
}

/// The bounded fingerprint of the session tree under `root`: every entry's
/// relative path, kind, length and content digest (BLAKE3 through `lgwks_std`),
/// walked without following symlinks. Past a bound the check is refused typed,
/// The walk reuses the snapshot's shape and bounds (codegraph query `snapshot`,
/// `report_members`, `workspace` before writing it): sorted entries from an
/// explicit stack, never following symlinks, under the snapshot's own 20,000
/// entries, 64 levels and 256 MiB. The digest is the same BLAKE3
/// (`lgwks_std::hash`) the workspace diff seal uses. Neither existing walk is
/// called directly: `workspace::snapshot::walk` lists copy entries only (files
/// and links, no directory entries, no digests, `.git` skipped) and
/// `sim_tree::tree_digest` is test-only, while the gate needs per-entry kind,
/// length and digest, directories included (larry#271).
pub(super) fn fingerprint_tree(root: &Path) -> Result<Fingerprint, FingerprintFault> {
    let mut entries = Fingerprint::new();
    let mut hashed: u64 = 0;
    let mut pending: Vec<(PathBuf, usize)> = vec![(PathBuf::new(), 0)];
    while let Some((relative, depth)) = pending.pop() {
        let mut names: Vec<OsString> = Vec::new();
        let dir = root.join(&relative);
        let read = std::fs::read_dir(&dir).map_err(|source| unreadable(&relative, source))?;
        for entry in read {
            let entry = entry.map_err(|source| unreadable(&relative, source))?;
            names.push(entry.file_name());
            if past_entries_bound(entries.len().saturating_add(names.len()), pending.len()) {
                return Err(too_large("entries", MAX_FINGERPRINT_ENTRIES));
            }
        }
        names.sort();
        for name in names {
            let path = relative.join(&name);
            let key = path.to_string_lossy().into_owned();
            let full = root.join(&path);
            let meta =
                std::fs::symlink_metadata(&full).map_err(|source| unreadable(&path, source))?;
            let kind = meta.file_type();
            if kind.is_dir() {
                entries.insert(key, FingerprintEntry::Dir);
                let next = depth.saturating_add(1);
                if next > MAX_FINGERPRINT_DEPTH {
                    return Err(too_large("directory levels", MAX_FINGERPRINT_DEPTH));
                }
                pending.push((path, next));
            } else if kind.is_symlink() {
                let target =
                    std::fs::read_link(&full).map_err(|source| unreadable(&path, source))?;
                entries.insert(
                    key,
                    FingerprintEntry::Link {
                        target: target.to_string_lossy().into_owned(),
                    },
                );
            } else if is_special(&kind) {
                entries.insert(key, FingerprintEntry::Other);
            } else {
                let digest = hash_file(&full, &mut hashed)?;
                entries.insert(
                    key,
                    FingerprintEntry::File {
                        len: meta.len(),
                        digest,
                    },
                );
            }
            if past_entries_bound(entries.len(), pending.len()) {
                return Err(too_large("entries", MAX_FINGERPRINT_ENTRIES));
            }
        }
    }
    Ok(entries)
}

/// The first relative path where `before` and `after` differ, in sorted order:
/// a name present on one side only, or an entry whose kind, length or digest
/// changed. `None` when the two fingerprints match.
pub(super) fn first_change(before: &Fingerprint, after: &Fingerprint) -> Option<String> {
    let mut keys = BTreeSet::new();
    keys.extend(before.keys());
    keys.extend(after.keys());
    for key in keys {
        if before.get(key) != after.get(key) {
            return Some(key.clone());
        }
    }
    None
}

/// Invalidates a checklist after a command reaches the run path.
pub(super) fn invalidate_finish(first_finish: &mut Option<FinishRequest>) {
    *first_finish = None;
}

/// Stops a session after the refusal budget is exhausted.
pub(super) const fn reached_refusal_limit(refused: usize) -> bool {
    refused >= 3
}

/// Keeps a bounded model reply in the compacted conversation.
pub(super) fn kept_reply(reply: &str, max_reply_bytes: usize) -> String {
    if reply.len() > max_reply_bytes {
        format!(
            "(a reply of {} bytes, over the bound, was dropped)",
            reply.len()
        )
    } else {
        reply.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{FinishRequest, invalidate_finish, reached_refusal_limit};

    #[test]
    fn command_invalidates_checklist_handshake() {
        let request = FinishRequest {
            summary: String::from("done"),
            check: String::from("test -f result"),
            artifacts: vec![String::from("result")],
            state: vec![String::from("ready")],
            servers: vec![String::from("none")],
        };
        let mut first = Some(request);
        invalidate_finish(&mut first);
        assert!(first.is_none(), "a later command needs a new checklist");
    }

    #[test]
    fn fresh_check_requires_the_named_passing_command() {
        assert!(!super::has_fresh_check(Some("true"), false, "true"));
        assert!(!super::has_fresh_check(None, true, "true"));
        assert!(!super::has_fresh_check(
            Some("cargo fmt"),
            true,
            "cargo test"
        ));
        assert!(super::has_fresh_check(
            Some("cargo test"),
            true,
            "cargo test"
        ));
    }

    #[test]
    fn kept_reply_is_bounded() {
        assert_eq!(super::kept_reply("ok", 2), "ok");
        assert_eq!(
            super::kept_reply("okay", 2),
            "(a reply of 4 bytes, over the bound, was dropped)"
        );
    }

    #[test]
    fn refusal_budget_stops_at_the_limit() {
        assert!(!reached_refusal_limit(2));
        assert!(reached_refusal_limit(3));
        assert!(reached_refusal_limit(4));
    }

    #[test]
    fn an_empty_tree_fingerprints_empty_and_matches_itself()
    -> Result<(), crate::sim_error::SimError> {
        let root = super::super::sim_tree::scratch("gate-fingerprint-empty")?;
        let before = super::fingerprint_tree(&root)?;
        assert!(before.is_empty(), "nothing walked, nothing recorded");
        let after = super::fingerprint_tree(&root)?;
        assert_eq!(
            super::first_change(&before, &after),
            None,
            "the same tree has no first change"
        );
        Ok(())
    }

    #[test]
    fn a_written_file_is_the_first_change_with_its_kind_length_and_digest()
    -> Result<(), crate::sim_error::SimError> {
        use super::FingerprintEntry;
        let root = super::super::sim_tree::scratch("gate-fingerprint-write")?;
        std::fs::write(root.join("a.txt"), "hello")?;
        let before = super::fingerprint_tree(&root)?;
        match before.get("a.txt") {
            Some(&FingerprintEntry::File { len, ref digest }) => {
                assert_eq!(len, 5, "the entry carries the length");
                assert_eq!(digest.len(), 64, "the entry carries a BLAKE3 hex digest");
            }
            Some(_) => {
                return Err(crate::sim_error::SimError::Missing("a file entry"));
            }
            None => {
                return Err(crate::sim_error::SimError::Missing("a.txt"));
            }
        }
        std::fs::write(root.join("a.txt"), "hello!")?;
        let after = super::fingerprint_tree(&root)?;
        assert_eq!(
            super::first_change(&before, &after),
            Some(String::from("a.txt")),
            "the changed path is named"
        );
        std::fs::remove_file(root.join("a.txt"))?;
        let gone = super::fingerprint_tree(&root)?;
        assert_eq!(
            super::first_change(&after, &gone),
            Some(String::from("a.txt")),
            "a deletion names its path too"
        );
        Ok(())
    }

    #[test]
    fn a_symlink_is_an_entry_and_never_followed() -> Result<(), crate::sim_error::SimError> {
        let root = super::super::sim_tree::scratch("gate-fingerprint-link")?;
        std::fs::write(root.join("target.txt"), "bytes")?;
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        std::os::unix::fs::symlink("target.txt", root.join("link.txt"))?;
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        std::fs::write(root.join("link.txt"), "bytes")?;
        let fingerprint = super::fingerprint_tree(&root)?;
        assert!(
            fingerprint.contains_key("link.txt"),
            "the link is fingerprinted as itself"
        );
        Ok(())
    }

    #[test]
    fn a_tree_past_the_depth_bound_is_refused_typed() -> Result<(), crate::sim_error::SimError> {
        let root = super::super::sim_tree::scratch("gate-fingerprint-deep")?;
        let mut deep = std::path::PathBuf::new();
        for level in 0..super::MAX_FINGERPRINT_DEPTH.saturating_add(4) {
            deep.push(format!("d{level}"));
        }
        std::fs::create_dir_all(root.join(&deep))?;
        match super::fingerprint_tree(&root) {
            Err(super::FingerprintFault::TooLarge { what, limit }) => {
                assert_eq!(what, "directory levels", "the bound is named");
                match u64::try_from(super::MAX_FINGERPRINT_DEPTH) {
                    Ok(bound) => assert_eq!(limit, bound, "the refusal names the bound"),
                    Err(_) => {
                        return Err(crate::sim_error::SimError::Missing(
                            "a depth bound fits in a u64",
                        ));
                    }
                }
            }
            Err(super::FingerprintFault::Unreadable { .. }) => {
                return Err(crate::sim_error::SimError::Missing("a depth refusal"));
            }
            Ok(_) => {
                return Err(crate::sim_error::SimError::Missing(
                    "a tree past the depth bound is refused",
                ));
            }
        }
        Ok(())
    }

    #[test]
    fn a_missing_tree_is_unreadable_typed() -> Result<(), crate::sim_error::SimError> {
        match super::fingerprint_tree(std::path::Path::new("gate-fingerprint-absent")) {
            Err(super::FingerprintFault::Unreadable { .. }) => Ok(()),
            Err(super::FingerprintFault::TooLarge { .. }) | Ok(_) => Err(
                crate::sim_error::SimError::Missing("a missing tree is unreadable, never passed"),
            ),
        }
    }

    #[test]
    fn the_entry_bound_refuses_past_itself_and_accepts_at_it() {
        assert!(
            !super::past_entries_bound(0, 0),
            "an empty walk is within bound"
        );
        assert!(
            !super::past_entries_bound(super::MAX_FINGERPRINT_ENTRIES.saturating_sub(1), 0),
            "one below the bound is within bound"
        );
        assert!(
            !super::past_entries_bound(super::MAX_FINGERPRINT_ENTRIES, 0),
            "exactly the bound is within bound"
        );
        assert!(
            super::past_entries_bound(super::MAX_FINGERPRINT_ENTRIES, 1),
            "one past the bound is refused"
        );
        assert!(
            super::past_entries_bound(usize::MAX, 1),
            "a saturating count past the bound is refused"
        );
    }

    #[test]
    fn a_tree_with_exactly_the_bound_entries_fingerprints_and_one_more_is_refused()
    -> Result<(), crate::sim_error::SimError> {
        let root = super::super::sim_tree::scratch("gate-fingerprint-bound")?;
        for index in 0..super::MAX_FINGERPRINT_ENTRIES {
            std::fs::write(root.join(format!("f{index}.txt")), "entry")?;
        }
        let exact = super::fingerprint_tree(&root)?;
        assert_eq!(
            exact.len(),
            super::MAX_FINGERPRINT_ENTRIES,
            "the bound itself still fingerprints"
        );
        std::fs::write(root.join("overflow.txt"), "entry")?;
        match super::fingerprint_tree(&root) {
            Err(super::FingerprintFault::TooLarge { what, limit }) => {
                assert_eq!(what, "entries", "the bound is named");
                match u64::try_from(super::MAX_FINGERPRINT_ENTRIES) {
                    Ok(bound) => assert_eq!(limit, bound, "the refusal names the bound"),
                    Err(_) => {
                        return Err(crate::sim_error::SimError::Missing(
                            "an entries bound fits in a u64",
                        ));
                    }
                }
            }
            Err(super::FingerprintFault::Unreadable { .. }) => {
                return Err(crate::sim_error::SimError::Missing("an entries refusal"));
            }
            Ok(_) => {
                return Err(crate::sim_error::SimError::Missing(
                    "a tree past the entries bound is refused",
                ));
            }
        }
        Ok(())
    }
}
