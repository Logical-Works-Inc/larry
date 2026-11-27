//! The temp-path rule (larry#267, larry#293, larry#311): in test code a temp directory may be named
//! with the process id only inside the one guard that empties the directory before use and removes
//! it on drop, or a later run lands in a dead run's tree when the system reuses that id (larry#243
//! measured 3,566 such directories; larry#267 names six fixtures whose stale contents failed
//! later tests).
//!
//! The rule reads a syntax tree from the estate reader `lgwks_ast` (larry#293, ADR 0061) and
//! judges one function at a time: one that reaches a temp source and names `std::process::id`, as
//! a call or a value, through any `use` path, alias, `self` import, glob or `::std` prefix, is
//! refused at its first use. The checked parse runs on every source; the verdict and follow passes
//! run only on a text that can raise them (larry#415, ADR 0109).
//!
//! The roots are the workspace's members, read from the root `Cargo.toml` (larry#311, ADR 0063):
//! the `[workspace] members` array (a literal path or a trailing `*` glob) plus the root package.
//! Every member's `src` and `tests` are read, a missing one is a typed refusal, and `#[path]` and
//! `include!` modules are followed and inherit the test-code status of the file naming them. The
//! sources are parsed on a scoped pool of at most the host's cores, each answer stored at its
//! path's index, so the worker count never changes the report (larry#316). The walk never follows
//! a symlink (CWE-59).
//!
//! Source: The Rust Reference on paths, use declarations, modules and macros; the Cargo Book on
//! workspaces; POSIX `mkstemp(3)`; CWE-377 (insecure temporary file) and CWE-59 (link following).

mod follow;
mod snapshot;
mod syntax;
mod uses;

use std::collections::{BTreeSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use lgwks_ast::MAX_SOURCE_BYTES;

use super::error::GovernanceError;
use super::read::{self, ReadFault};
use snapshot::Snapshot;

/// The one guard of the crate's tests and the one of the integration tests: each empties its
/// directory before use and removes it on drop, so the process id there names no leftover, and
/// every other temp path in test code reaches its directory through one of them.
const GUARDS: [&str; 2] = ["src/executor/sim_tree.rs", "tests/support/temp_dir.rs"];

/// The directories the rule reads under every workspace member, in order.
const ROOTS: [&str; 2] = ["src", "tests"];

/// The deepest nesting the walk follows, so a pathological tree ends in a refusal, never a stack
/// overflow.
const MAX_DEPTH: usize = 64;

/// A source the rule reads: its path on disk, its root-relative name, and whether it inherits the
/// test code of the file that followed it.
pub(super) type Source = (PathBuf, String, bool);

/// What reading one source file produced.
enum SourceRead {
    /// The file's bytes.
    Contents(Vec<u8>),
    /// The file is past the reader's byte bound, and this many bytes.
    TooLarge(usize),
}

/// What one source file refused and which modules it names to follow.
struct Scanned {
    /// The refusals the file itself raised.
    errors: Vec<String>,
    /// The `#[path]` and `include!` sources it names.
    follows: Vec<Source>,
}

/// The workspace's members, read from the root `Cargo.toml` (larry#311, ADR 0063).
pub(crate) struct Workspace {
    /// Every member's directory, relative to the root; "" is the root package.
    members: Vec<String>,
    /// Parsed sources and their analyses, reusable while this snapshot lives.
    snapshot: Snapshot,
}

impl Workspace {
    /// Reads the manifest at `root`. A tree with no manifest (a fixture of documents alone) names
    /// no member; a manifest that is a symlink, is not UTF-8, or names an unsupported member form
    /// is a typed refusal.
    pub(crate) fn read(root: &Path) -> Result<Self, GovernanceError> {
        let manifest = root.join("Cargo.toml");
        let broken = |cause: String| GovernanceError::Workspace { cause };
        let unreadable = |cause: io::Error| GovernanceError::Unreadable {
            relative: String::from("Cargo.toml"),
            cause,
        };
        let bytes = match std::fs::symlink_metadata(&manifest) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(broken(String::from("Cargo.toml is a symbolic link")));
            }
            Ok(metadata) if !metadata.is_file() => {
                return Err(broken(String::from("Cargo.toml is not a file")));
            }
            Ok(_) => std::fs::read(&manifest).map_err(unreadable)?,
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => {
                return Ok(Self {
                    members: Vec::new(),
                    snapshot: Snapshot::default(),
                });
            }
            Err(cause) => return Err(unreadable(cause)),
        };
        let text = match std::str::from_utf8(&bytes) {
            Ok(text) => text,
            Err(cause) => return Err(broken(format!("Cargo.toml is not UTF-8: {cause}"))),
        };
        let mut members = Vec::new();
        if section(text, "package").is_some() {
            members.push(String::new());
        }
        if let Some(body) = section(text, "workspace") {
            for entry in declared_members(body)? {
                members.extend(expand(root, &entry)?);
            }
        }
        members.sort();
        members.dedup();
        Ok(Self {
            members,
            snapshot: Snapshot::default(),
        })
    }

    /// The member directories, relative to the root.
    pub(crate) fn members(&self) -> &[String] {
        &self.members
    }
}

/// The body of the `[name]` section of a manifest, up to the next section header.
fn section<'text>(text: &'text str, name: &str) -> Option<&'text str> {
    let header = format!("[{name}]");
    let mut start: Option<usize> = None;
    let mut offset = 0_usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            if start.is_some() {
                return start.and_then(|from| text.get(from..offset));
            }
            let bare = match trimmed.find('#') {
                Some(hash) => match trimmed.get(..hash) {
                    Some(bare) => bare.trim(),
                    None => trimmed.trim(),
                },
                None => trimmed.trim(),
            };
            if bare == header {
                start = Some(offset.saturating_add(line.len()));
            }
        }
        offset = offset.saturating_add(line.len());
    }
    start.and_then(|from| text.get(from..))
}

/// The entries of the `[workspace] members` array, read as TOML strings. A `members` key that
/// never opens or closes its array is a typed refusal.
fn declared_members(body: &str) -> Result<Vec<String>, GovernanceError> {
    let mut offset = 0_usize;
    for line in body.split_inclusive('\n') {
        let start = offset;
        offset = offset.saturating_add(line.len());
        let Some(tail) = body.get(start..) else {
            continue;
        };
        let tail = tail.trim_start();
        let Some(after) = tail.strip_prefix("members") else {
            continue;
        };
        if !after.starts_with(char::is_whitespace) && !after.starts_with('=') {
            continue;
        }
        let Some(after) = after.trim_start().strip_prefix('=') else {
            continue;
        };
        return array_items(after);
    }
    Ok(Vec::new())
}

/// The strings of the first TOML array in `text`, with comments and escapes read as TOML spells
/// them. An array that opens and never closes is a typed refusal.
fn array_items(text: &str) -> Result<Vec<String>, GovernanceError> {
    let mut open: Option<usize> = None;
    let mut close: Option<usize> = None;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut comment = false;
    for (at, character) in text.char_indices() {
        if comment {
            if character == '\n' {
                comment = false;
            }
            continue;
        }
        if let Some(mark) = quote {
            if escaped {
                escaped = false;
                continue;
            }
            match character {
                '\\' if mark == '"' => escaped = true,
                found if found == mark => quote = None,
                _ => {}
            }
            continue;
        }
        match character {
            '#' => comment = true,
            '"' | '\'' => quote = Some(character),
            '[' => {
                if open.is_none() {
                    open = Some(at);
                }
            }
            ']' => {
                close = Some(at);
                break;
            }
            _ => {}
        }
    }
    let (Some(open), Some(close)) = (open, close) else {
        return Err(GovernanceError::Workspace {
            cause: String::from("the members array does not close"),
        });
    };
    let Some(inside) = text.get(open.saturating_add(1)..close) else {
        return Err(GovernanceError::Workspace {
            cause: String::from("the members array is not UTF-8"),
        });
    };
    Ok(string_items(inside))
}

/// Every quoted string of a TOML array body, in order.
fn string_items(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '"' && character != '\'' {
            continue;
        }
        let closing = character;
        let mut item = String::new();
        let mut ended = false;
        while let Some(next) = characters.next() {
            if next == '\\' && closing == '"' {
                match characters.next() {
                    Some('n') => item.push('\n'),
                    Some('t') => item.push('\t'),
                    Some(found) => item.push(found),
                    None => break,
                }
                continue;
            }
            if next == closing {
                ended = true;
                break;
            }
            item.push(next);
        }
        if ended {
            items.push(item);
        }
    }
    items
}

/// A member entry expanded to directories: a literal path as it is, and a trailing `*` glob as the
/// directories beneath it that hold a `Cargo.toml`. Any other metacharacter is a typed refusal,
/// never a silent skip.
fn expand(root: &Path, entry: &str) -> Result<Vec<String>, GovernanceError> {
    let entry = entry.trim_end_matches('/');
    if !entry.contains('*') {
        return Ok(vec![String::from(entry)]);
    }
    let Some(parent) = entry.strip_suffix("/*") else {
        return Err(GovernanceError::Workspace {
            cause: format!("unsupported member glob: {entry}"),
        });
    };
    let directory = root.join(parent);
    let mut expanded = Vec::new();
    let unreadable = |cause: io::Error| GovernanceError::Unreadable {
        relative: String::from(parent),
        cause,
    };
    for item in std::fs::read_dir(&directory).map_err(unreadable)? {
        let item = item.map_err(unreadable)?;
        let file_type = item.file_type().map_err(unreadable)?;
        if !file_type.is_dir() {
            continue;
        }
        let Some(name) = item.file_name().to_str().map(String::from) else {
            continue;
        };
        if !directory.join(&name).join("Cargo.toml").is_file() {
            continue;
        }
        expanded.push(if parent.is_empty() {
            name
        } else {
            format!("{parent}/{name}")
        });
    }
    Ok(expanded)
}

/// Every refusal the temp-path rule raises over the workspace at `root`.
pub(crate) fn errors(root: &Path) -> Result<Vec<String>, GovernanceError> {
    let workspace = Workspace::read(root)?;
    rule_errors(root, &workspace)
}

/// Every refusal over `root` scanned on at most `lanes` threads: the control names the lane count
/// so it can prove one worker and many give byte-identical refusals (larry#316).
#[cfg(test)]
pub(super) fn errors_on(root: &Path, lanes: usize) -> Result<Vec<String>, GovernanceError> {
    let workspace = Workspace::read(root)?;
    rule_errors_with(root, &workspace, Some(lanes), true)
}

/// Every refusal over `root` with every pass run on every source: what the gated report equals.
#[cfg(test)]
pub(super) fn errors_ungated(root: &Path) -> Result<Vec<String>, GovernanceError> {
    rule_errors_with(root, &Workspace::read(root)?, None, false)
}

/// The lanes one check may use: the host's cores, at most one per pending file and at least one,
/// so a host that cannot report its cores still checks every file in path order.
fn lane_count(files: usize) -> usize {
    let cores = match std::thread::available_parallelism() {
        Ok(count) => count.get(),
        Err(_) => 1,
    };
    cores.clamp(1, files.max(1))
}

/// Every refusal the rule raises over the sources of `workspace`.
pub(crate) fn rule_errors(
    root: &Path,
    workspace: &Workspace,
) -> Result<Vec<String>, GovernanceError> {
    rule_errors_with(root, workspace, None, true)
}

/// The refusals over `workspace`'s sources, every round of followed modules scanned on `lanes`
/// threads when one names them, else on the host's own lane count; `gated` as for [`scan`].
fn rule_errors_with(
    root: &Path,
    workspace: &Workspace,
    lanes: Option<usize>,
    gated: bool,
) -> Result<Vec<String>, GovernanceError> {
    let mut errors = Vec::new();
    let mut queue: VecDeque<Source> = VecDeque::new();
    for member in workspace.members() {
        for directory in ROOTS {
            let relative = if member.is_empty() {
                String::from(directory)
            } else {
                format!("{member}/{directory}")
            };
            let path = root.join(&relative);
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(GovernanceError::Unreadable {
                        relative,
                        cause: io::Error::other("a symbolic link is never followed"),
                    });
                }
                Ok(metadata) if metadata.is_dir() => {
                    collect(&path, &relative, 0, &mut queue, &mut errors)?;
                }
                Ok(_) => {
                    return Err(GovernanceError::Unreadable {
                        relative,
                        cause: io::Error::other("not a directory"),
                    });
                }
                Err(cause) if cause.kind() == io::ErrorKind::NotFound => {
                    return Err(GovernanceError::MissingRoot { relative });
                }
                Err(cause) => return Err(GovernanceError::Unreadable { relative, cause }),
            }
        }
    }
    let canonical_root =
        std::fs::canonicalize(root).map_err(|cause| GovernanceError::Unreadable {
            relative: String::from("."),
            cause,
        })?;
    let mut seen = BTreeSet::new();
    while !queue.is_empty() {
        let mut pending: Vec<Source> = Vec::with_capacity(queue.len());
        for source in queue.drain(..) {
            if !seen.insert(source.1.clone()) || GUARDS.contains(&source.1.as_str()) {
                continue;
            }
            pending.push(source);
        }
        let lanes = match lanes {
            Some(lanes) => lanes.clamp(1, pending.len().max(1)),
            None => lane_count(pending.len()),
        };
        for scanned in scan_all(&canonical_root, workspace, &pending, lanes, gated) {
            errors.extend(scanned.errors);
            queue.extend(scanned.follows);
        }
    }
    sort_by_file_and_line(&mut errors);
    errors.dedup();
    Ok(errors)
}

/// Scans every pending source on at most `lanes` scoped threads: one cursor hands each thread the
/// next path, each answer lands at its path's index, and the answers are read back in path order,
/// so the report never depends on which lane finished first.
fn scan_all(
    canonical_root: &Path,
    workspace: &Workspace,
    pending: &[Source],
    lanes: usize,
    gated: bool,
) -> Vec<Scanned> {
    let lanes = lanes.clamp(1, pending.len().max(1));
    let mut scanned: Vec<Option<Scanned>> = pending.iter().map(|_| None).collect();
    if lanes == 1 {
        for (slot, source) in scanned.iter_mut().zip(pending) {
            *slot = Some(scan(
                canonical_root,
                workspace,
                &source.0,
                &source.1,
                source.2,
                gated,
            ));
        }
    } else {
        let cursor = AtomicUsize::new(0);
        let answers: Vec<(usize, Scanned)> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..lanes)
                .map(|_| {
                    scope.spawn(|| {
                        let mut mine: Vec<(usize, Scanned)> = Vec::new();
                        loop {
                            let at = cursor.fetch_add(1, Ordering::Relaxed);
                            let Some(source) = pending.get(at) else {
                                break;
                            };
                            let answer = scan(
                                canonical_root,
                                workspace,
                                &source.0,
                                &source.1,
                                source.2,
                                gated,
                            );
                            mine.push((at, answer));
                        }
                        mine
                    })
                })
                .collect();
            let mut all = Vec::with_capacity(pending.len());
            for handle in handles {
                // A lane that panicked reports nothing; every file it took keeps an empty slot
                // below and is refused by name, never passed.
                if let Ok(mine) = handle.join() {
                    all.extend(mine);
                }
            }
            all
        });
        for (at, answer) in answers {
            if let Some(slot) = scanned.get_mut(at) {
                *slot = Some(answer);
            }
        }
    }
    scanned
        .into_iter()
        .zip(pending)
        .map(|(slot, source)| match slot {
            Some(answer) => answer,
            None => Scanned {
                errors: vec![format!(
                    "a source scan stopped without answering: {}",
                    source.1
                )],
                follows: Vec::new(),
            },
        })
        .collect()
}

/// The report order: by file, then by line, the order a person opens the files.
fn sort_by_file_and_line(errors: &mut [String]) {
    errors.sort_by_key(|error| match error.rsplit_once(':') {
        Some((file, line)) => match line.parse::<usize>() {
            Ok(line) => (String::from(file), line),
            Err(_) => (error.clone(), 0),
        },
        None => (error.clone(), 0),
    });
}

/// Every `.rs` file beneath `path`, as a root-relative path with `/`. A symlink is never followed,
/// whatever it points at, and nesting past [`MAX_DEPTH`] is refused by name.
fn collect(
    path: &Path,
    relative: &str,
    depth: usize,
    found: &mut VecDeque<Source>,
    errors: &mut Vec<String>,
) -> Result<(), GovernanceError> {
    if depth > MAX_DEPTH {
        return Err(GovernanceError::Unreadable {
            relative: String::from(relative),
            cause: io::Error::other(format!("source tree deeper than {MAX_DEPTH}")),
        });
    }
    let mut entries: Vec<(PathBuf, bool, bool)> = Vec::new();
    let unreadable = |cause: io::Error| GovernanceError::Unreadable {
        relative: String::from(relative),
        cause,
    };
    for entry in std::fs::read_dir(path).map_err(unreadable)? {
        let entry = entry.map_err(unreadable)?;
        let file_type = entry.file_type().map_err(unreadable)?;
        entries.push((entry.path(), file_type.is_dir(), file_type.is_symlink()));
    }
    entries.sort();
    for (entry, is_dir, is_symlink) in entries {
        if is_symlink {
            continue;
        }
        let name = match entry_name(&entry) {
            Ok(name) => name,
            Err(EntryError::NotUtf8) => {
                errors.push(format!(
                    "source tree names an entry that is not UTF-8: {relative}"
                ));
                continue;
            }
        };
        let child = format!("{relative}/{name}");
        if is_dir {
            collect(&entry, &child, depth.saturating_add(1), found, errors)?;
        } else if name.ends_with(".rs") {
            found.push_back((entry, child, false));
        }
    }
    Ok(())
}

/// Why a directory entry's name cannot be read.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum EntryError {
    /// The name is not UTF-8, or the entry has none.
    NotUtf8,
}

/// The UTF-8 name of a directory entry: an entry whose name is not UTF-8 is never skipped and
/// never passed. A host that cannot hold such a name (macOS) never shows one to the walk, so the
/// control feeds this function the bytes directly.
pub(super) fn entry_name(entry: &Path) -> Result<&str, EntryError> {
    match entry.file_name().and_then(|name| name.to_str()) {
        Some(name) => Ok(name),
        None => Err(EntryError::NotUtf8),
    }
}

/// Reads, parses and judges one source file; a reader refusal is a message naming the file, never
/// a pass. The checked parse always runs, so a syntax fault or an over-budget tree refuses whatever
/// the text holds; `gated` skips the verdict and follow passes on a text that cannot raise them
/// (larry#415), and `false` runs both on every source, the reference the simulation compares to.
fn scan(
    canonical_root: &Path,
    workspace: &Workspace,
    path: &Path,
    relative: &str,
    inherited_test: bool,
    gated: bool,
) -> Scanned {
    let refused = |error: GovernanceError| Scanned {
        errors: vec![error.to_string()],
        follows: Vec::new(),
    };
    let named = || String::from(relative);
    let contents = match read_source(canonical_root, relative) {
        Ok(SourceRead::Contents(contents)) => contents,
        Ok(SourceRead::TooLarge(bytes)) => {
            return refused(GovernanceError::SourceTooLarge {
                relative: named(),
                bytes,
            });
        }
        Err(cause) => {
            return refused(GovernanceError::Unreadable {
                relative: named(),
                cause,
            });
        }
    };
    let text = match std::str::from_utf8(&contents) {
        Ok(text) => text,
        Err(_) => return refused(GovernanceError::SourceNotUtf8 { relative: named() }),
    };
    let (verdict_possible, follow_possible) =
        (!gated || may_refuse(text), !gated || may_follow(text));
    let mut scanned = Scanned {
        errors: Vec::new(),
        follows: Vec::new(),
    };
    // The checked parse runs inside `analyzed` even when no pass can raise anything, so a
    // syntax fault refuses whatever the text holds.
    let (_, analysis) = match workspace
        .snapshot
        .analyzed(relative, &contents, text, gated)
    {
        Ok(found) => found,
        Err(error) => return refused(error),
    };
    let test_code = inherited_test || analysis.test_code;
    if follow_possible {
        scanned.follows = follow::follows(
            canonical_root,
            path,
            relative,
            &analysis.follows,
            test_code,
            &mut scanned.errors,
        );
    }
    if test_code && verdict_possible {
        scanned.errors.extend(analysis.verdicts.iter().cloned());
    }
    scanned
}

/// Whether the verdict pass can raise anything over `text`: a verdict needs the process id and a
/// temp source in one function. Every path to the id names `process` and every temp source names
/// `temp_dir`, a literal that starts the system temp directory (so it holds `tmp`) or `TMPDIR`; a
/// syntax tree expands no macro, so each is spelled in the text, and a source without them raises
/// nothing.
pub(super) fn may_refuse(text: &str) -> bool {
    text.contains("process")
        && ["temp_dir", "tmp", "TMPDIR"]
            .iter()
            .any(|token| text.contains(token))
}

/// Whether the follow pass can name a module over `text`: a `#[path` attribute or an `include!`.
pub(super) fn may_follow(text: &str) -> bool {
    text.contains("#[path") || text.contains("include")
}

/// Reads one source file within the reader's byte bound, through the governance reader that refuses
/// a link anywhere on the path and requires a regular file.
fn read_source(root: &Path, relative: &str) -> io::Result<SourceRead> {
    let Ok(limit) = u64::try_from(MAX_SOURCE_BYTES) else {
        return Ok(SourceRead::TooLarge(MAX_SOURCE_BYTES));
    };
    match read::read_bounded(root, Path::new(relative), limit) {
        Ok(bytes) => Ok(SourceRead::Contents(bytes)),
        Err(ReadFault::TooLarge { bytes }) => {
            let Ok(bytes) = usize::try_from(bytes) else {
                return Ok(SourceRead::TooLarge(usize::MAX));
            };
            Ok(SourceRead::TooLarge(bytes))
        }
        Err(ReadFault::Missing(cause) | ReadFault::Unreadable(cause)) => Err(cause),
        Err(ReadFault::Symlink) => Err(io::Error::other("a symbolic link is never followed")),
        Err(ReadFault::NotRegular { .. }) => Err(io::Error::other("not a regular file")),
    }
}
