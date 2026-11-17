//! The temp-path rule (larry#267, larry#293, larry#311): in test code a temp directory may be named
//! with the process id only inside the one guard that empties the directory before use and removes
//! it on drop. A fixture named `<label>-<pid>` that is never removed lands a later run in a dead
//! run's tree when the system reuses that id: larry#243 measured 3,566 such directories in the
//! system temporary directory, and larry#267 names six fixtures whose stale contents failed the
//! tests that followed them.
//!
//! The rule reads a syntax tree, not text and not a lexer of its own (larry#293, ADR 0061): every
//! Rust source is parsed by the estate reader `lgwks_ast`, and the rule judges one function at a
//! time. A function that reaches a temp source and names `std::process::id`, as a call or as a
//! value, through any `use` path, alias, `self` import, glob or `::std` prefix, is refused at its
//! first use. Patterns, closure arguments, method calls and a shadowing local are not the process
//! id.
//!
//! The roots are the workspace's members, read from the root `Cargo.toml` (larry#311, ADR 0063):
//! the `[workspace] members` array, each entry a literal path or a trailing `*` glob, plus the root
//! package when the manifest declares `[package]`. Every member's `src` and `tests` are read, a
//! missing one is a typed refusal naming it, and `#[path]` and `include!` modules are followed and
//! inherit the test-code status of the file that names them. The sources are parsed once each on a
//! scoped pool of at most the host's available cores, and each answer is stored at its path's
//! index, so the count of workers can never change the report (larry#316). The directory walk never
//! follows a symlink (CWE-59).
//!
//! Source: The Rust Reference on path expressions, use declarations, modules and macros-by-example;
//! the Cargo Book on workspaces; POSIX `mkstemp(3)` (a temporary name must be unique and owned by
//! its maker); CWE-377 (insecure temporary file) and CWE-59 (link following).

mod syntax;
mod uses;

use std::collections::{BTreeSet, VecDeque};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use lgwks_ast::{Language, MAX_SOURCE_BYTES, Parsed, try_parse};

use super::error::GovernanceError;
use syntax::{crate_root, follow_names, is_test_code, verdicts};

/// The one guard of the crate's tests and the one guard of the integration tests. Each empties its
/// directory before use and removes it on drop (`executor::sim_tree::scratch`, `TempDir` behind
/// `support/temp_dir.rs`), so the process id there names a directory its owner replaces and then
/// removes, never a leftover. Every other temp path in test code reaches its directory through one
/// of them.
const GUARDS: [&str; 2] = ["src/executor/sim_tree.rs", "tests/support/temp_dir.rs"];

/// The directories the rule reads under every workspace member, in order.
const ROOTS: [&str; 2] = ["src", "tests"];

/// The deepest nesting the walk follows, so a pathological tree ends in a refusal, never a stack
/// overflow.
const MAX_DEPTH: usize = 64;

/// A source the rule reads: its path on disk, its root-relative name, and whether it inherits the
/// test code of the file that followed it.
type Source = (PathBuf, String, bool);

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
}

impl Workspace {
    /// Reads the manifest at `root`. A tree with no manifest (a fixture of documents alone) names
    /// no member; a manifest that is a symlink, is not UTF-8, or names an unsupported member form
    /// is a typed refusal.
    pub(crate) fn read(root: &Path) -> Result<Self, GovernanceError> {
        let manifest = root.join("Cargo.toml");
        let bytes = match std::fs::symlink_metadata(&manifest) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(GovernanceError::Workspace {
                    cause: String::from("Cargo.toml is a symbolic link"),
                });
            }
            Ok(metadata) if !metadata.is_file() => {
                return Err(GovernanceError::Workspace {
                    cause: String::from("Cargo.toml is not a file"),
                });
            }
            Ok(_) => std::fs::read(&manifest).map_err(|cause| GovernanceError::Unreadable {
                relative: String::from("Cargo.toml"),
                cause,
            })?,
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => {
                return Ok(Self {
                    members: Vec::new(),
                });
            }
            Err(cause) => {
                return Err(GovernanceError::Unreadable {
                    relative: String::from("Cargo.toml"),
                    cause,
                });
            }
        };
        let text = match std::str::from_utf8(&bytes) {
            Ok(text) => text,
            Err(cause) => {
                return Err(GovernanceError::Workspace {
                    cause: format!("Cargo.toml is not UTF-8: {cause}"),
                });
            }
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
        Ok(Self { members })
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
    rule_errors_with(root, &workspace, Some(lanes))
}

/// The lanes one whole-tree check may use: the host's available parallelism and never more than
/// its pending files, and never fewer than one, so a host that cannot report its cores still
/// checks every file in path order.
fn lane_count(files: usize) -> usize {
    let cores = match std::thread::available_parallelism() {
        Ok(count) => count.get(),
        Err(_) => 1,
    };
    cores.clamp(1, files.max(1))
}

/// Every refusal the rule raises over the sources of `workspace`: the member roots, the walk, the
/// parse, the verdicts and the followed modules.
pub(crate) fn rule_errors(
    root: &Path,
    workspace: &Workspace,
) -> Result<Vec<String>, GovernanceError> {
    rule_errors_with(root, workspace, None)
}

/// The refusals over `workspace`'s sources, every round of followed modules scanned on `lanes`
/// threads when one names them, else on the host's own lane count.
fn rule_errors_with(
    root: &Path,
    workspace: &Workspace,
    lanes: Option<usize>,
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
        for scanned in scan_all(&canonical_root, &pending, lanes) {
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
fn scan_all(canonical_root: &Path, pending: &[Source], lanes: usize) -> Vec<Scanned> {
    let lanes = lanes.clamp(1, pending.len().max(1));
    let mut scanned: Vec<Option<Scanned>> = pending.iter().map(|_| None).collect();
    if lanes == 1 {
        for (slot, source) in scanned.iter_mut().zip(pending) {
            *slot = Some(scan(canonical_root, &source.0, &source.1, source.2));
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
                            mine.push((at, scan(canonical_root, &source.0, &source.1, source.2)));
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

/// The order the rule reports in: by the file, then by the line inside it, so a report reads in
/// the order a person opens the files.
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
/// a pass.
fn scan(canonical_root: &Path, path: &Path, relative: &str, inherited_test: bool) -> Scanned {
    let mut scanned = Scanned {
        errors: Vec::new(),
        follows: Vec::new(),
    };
    let contents = match read_source(path) {
        Ok(SourceRead::Contents(contents)) => contents,
        Ok(SourceRead::TooLarge(bytes)) => {
            scanned.errors.push(
                GovernanceError::SourceTooLarge {
                    relative: String::from(relative),
                    bytes,
                }
                .to_string(),
            );
            return scanned;
        }
        Err(cause) => {
            scanned.errors.push(
                GovernanceError::Unreadable {
                    relative: String::from(relative),
                    cause,
                }
                .to_string(),
            );
            return scanned;
        }
    };
    let text = match std::str::from_utf8(&contents) {
        Ok(text) => text,
        Err(_) => {
            scanned.errors.push(
                GovernanceError::SourceNotUtf8 {
                    relative: String::from(relative),
                }
                .to_string(),
            );
            return scanned;
        }
    };
    let tree: Parsed = match try_parse(text, Language::Rust) {
        Ok(tree) => tree,
        Err(cause) => {
            scanned.errors.push(
                GovernanceError::Source {
                    relative: String::from(relative),
                    cause,
                }
                .to_string(),
            );
            return scanned;
        }
    };
    let test_code = inherited_test || is_test_code(relative, &tree);
    scanned.follows = follows(
        canonical_root,
        path,
        relative,
        &tree,
        test_code,
        &mut scanned.errors,
    );
    if test_code {
        scanned.errors.extend(verdicts(relative, text, &tree));
    }
    scanned
}

/// The `#[path = "..."]` and `include!("...")` sources `tree` names, resolved beside its file and
/// kept inside the checked tree; an absolute path, a link, a missing file or a path outside the
/// tree is a refusal naming the file that names it.
fn follows(
    canonical_root: &Path,
    path: &Path,
    relative: &str,
    tree: &Parsed,
    inherited_test: bool,
    errors: &mut Vec<String>,
) -> Vec<Source> {
    let Some(base) = path.parent() else {
        return Vec::new();
    };
    let mut follows = Vec::new();
    for named in follow_names(tree) {
        let name = named.name;
        if name.starts_with('/') {
            errors.push(format!(
                "a followed module names an absolute path: {relative}: {name}"
            ));
            continue;
        }
        let mut rooted = base.to_path_buf();
        if !named.inline.is_empty()
            && !crate_root(relative)
            && let Some(stem) = Path::new(relative).file_stem()
            && let Some(stem) = stem.to_str()
        {
            rooted.push(stem);
        }
        for inline in &named.inline {
            rooted.push(inline);
        }
        let target = settle(&rooted.join(&name));
        let canonical = match resolve_follow(&target) {
            Ok(canonical) => canonical,
            Err(cause) => {
                let what = if target.strip_prefix(canonical_root).is_ok() {
                    "a followed module names no readable source"
                } else {
                    "a followed module leaves the checked tree"
                };
                errors.push(format!("{what}: {relative}: {name}: {cause}"));
                continue;
            }
        };
        let inner = match canonical.strip_prefix(canonical_root) {
            Ok(inner) => inner,
            Err(_) => {
                errors.push(format!(
                    "a followed module leaves the checked tree: {relative}: {name}"
                ));
                continue;
            }
        };
        let inner = inner.to_string_lossy().into_owned();
        follows.push((canonical, inner, inherited_test));
    }
    follows
}

/// `path` with `.` and `..` applied as text, as rustc roots a followed module (larry#311).
fn settle(path: &Path) -> PathBuf {
    let mut settled = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(settled.components().next_back(), Some(Component::Normal(_))) =>
            {
                settled.pop();
            }
            _ => settled.push(part),
        }
    }
    settled
}

/// Resolves a followed module's path: a symlink is never followed and a missing file is refused.
fn resolve_follow(target: &Path) -> io::Result<PathBuf> {
    let metadata = std::fs::symlink_metadata(target)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::other("a symbolic link is never followed"));
    }
    std::fs::canonicalize(target)
}

/// Reads one source file within the reader's byte bound.
fn read_source(path: &Path) -> io::Result<SourceRead> {
    let metadata = std::fs::metadata(path)?;
    let bytes = match usize::try_from(metadata.len()) {
        Ok(bytes) => bytes,
        Err(_) => usize::MAX,
    };
    if bytes > MAX_SOURCE_BYTES {
        return Ok(SourceRead::TooLarge(bytes));
    }
    std::fs::read(path).map(SourceRead::Contents)
}
