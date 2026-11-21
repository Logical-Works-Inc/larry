//! Kernel confinement of one command (larry#243, ADR 0046): a command that writes
//! outside its workspace copy is refused by the kernel, so the person's folder is
//! never touched by a command, whatever the model chose and the person approved.
//!
//! The mechanism is the platform's own wrapper in front of the command's argv:
//! `/usr/bin/sandbox-exec -p <profile>` on macOS (reads are open except beneath the
//! state directory, which holds the receipt key, and the session's own copy beneath
//! it stays readable; writes are denied everywhere but the workspace's tree, its
//! scratch directory and the devices a shell writes),
//! and `/usr/bin/setpriv` with a Landlock allowlist on Linux (reads beneath every
//! path but the state directory and `/proc`, read and write beneath the tree and
//! scratch directory). Every descendant of the command inherits it. Source:
//! Apple's Seatbelt profile language (`sandbox-exec`), the Linux Landlock
//! documentation, and `lgwks_bot::rt::process`, whose `SandboxProfile::validate`
//! and `LandlockRules::validate` refuse what could not confine anything before a
//! spawn. The estate has no public wrapper for a std spawn and no write-only
//! profile (logicalworks-crates#1075), so the two argv spellings are Larry's
//! until it does.
//!
//! The state directory is not a read root (larry#322, ADR 0080): `executor.db`
//! holds the receipt signing seed, so a command that read it could seal receipts
//! `larry verify` accepts. The one path beneath it a command reads is the session's
//! own copy (`<state>/ws/<task>`), whose git directory sits beside the work tree
//! because the clone names it separately; without it every `git` command in the copy
//! failed to resolve its repository. The executor also holds `LARRY_MODEL_KEY` and
//! every `LARRY_KEY_*` in its own environment, and Rust 2024 makes
//! `std::env::remove_var` unsafe, which this workspace forbids, so the kernel keeps
//! the command from the environment instead: Linux reads no `/proc` entry but the
//! command's own process directory and the host's tunables, and macOS denies the
//! kernel interface that returns a process's arguments and environment.
//!
//! Every other read and the network are not confined: the person approves each
//! command, and Nova owns the fuller sandbox (logicalworks-crates#846). A
//! platform with no wrapper, a wrapper that is missing, and a wrapper that does
//! not confine on this host each refuse the session with a typed [`ConfineError`];
//! nothing runs bare.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use lgwks_bot::rt::process::{LandlockRules, PathAccess, SandboxProfile};

use super::run::clip;

/// The macOS wrapper: it confines itself, then execs the command.
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// The util-linux wrapper that applies Landlock rules, then execs the command.
const SETPRIV: &str = "/usr/bin/setpriv";

/// The devices a shell writes, which the profile and the rules leave open.
const DEVICES: [&str; 2] = ["/dev/null", "/dev/tty"];

/// The `/proc` entries a command reads instead of all of `/proc`, whose
/// `/proc/<pid>/environ` holds the executor's own environment and its model keys
/// (larry#322). A command still sees itself and the host's tunables; Landlock grants
/// only directory rules, so a single summary file cannot be a rule of its own.
const PROC_READS: [&str; 2] = ["/proc/self", "/proc/sys"];

/// `setpriv`'s access list for read and execute.
const READ_LIST: &str = "execute,read-file,read-dir";

/// `setpriv`'s access list for read, execute, create, change and remove.
const WRITE_LIST: &str = "execute,write-file,read-file,read-dir,remove-dir,remove-file,\
make-char,make-dir,make-reg,make-sock,make-fifo,make-block,make-sym,refer,truncate";

/// What a device file may be given (Landlock refuses a directory right on a file).
const DEVICE_LIST: &str = "write-file,read-file,truncate";

/// The one device Landlock opens, as a file rule beside the directory rules.
const LANDLOCK_DEVICE: &str = "/dev/null";

/// Bytes of a refused probe's stderr kept for its message.
const PROBE_STDERR: usize = 256;

/// The probe: write the allowed path (`$1`), then the forbidden one (`$2`). Exit 2
/// when the allowed write fails, 3 when the forbidden one lands, 0 when the
/// confinement lets exactly the first through.
const PROBE_SCRIPT: &str = "printf '' > \"$1\" || exit 2; printf '' > \"$2\" && exit 3; exit 0";

/// Where a command runs, as the confinement knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Platform {
    /// macOS: Seatbelt through `sandbox-exec`.
    Macos,
    /// Linux: Landlock through `setpriv`.
    Linux,
    /// Any other target: there is no wrapper, so no command runs.
    Other,
}

impl Platform {
    /// The platform this binary was built for.
    pub(crate) const fn host() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }

    /// The platform's name in a refusal.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Other => "this platform",
        }
    }

    /// The platform's wrapper program, when it has one.
    const fn wrapper(self) -> Option<&'static str> {
        match self {
            Self::Macos => Some(SANDBOX_EXEC),
            Self::Linux => Some(SETPRIV),
            Self::Other => None,
        }
    }
}

/// Why a command could not be confined; the session starts nothing.
#[derive(Debug)]
#[non_exhaustive]
pub enum ConfineError {
    /// The platform has no wrapper (Windows, the BSDs): commands are refused.
    Unsupported {
        /// The platform's name.
        platform: &'static str,
    },
    /// The platform's wrapper program is not installed.
    MissingWrapper {
        /// The wrapper's path.
        tool: &'static str,
    },
    /// A path the confinement names could not be resolved.
    Path {
        /// The path.
        path: PathBuf,
        /// Why it could not be resolved.
        source: io::Error,
    },
    /// The workspace copy is not a directory beneath the state directory, so a read
    /// exception for it would open the state directory and the key store in it.
    NotBeneath {
        /// The workspace copy.
        workspace: PathBuf,
        /// The state directory.
        state: PathBuf,
    },
    /// The estate refused the profile or the rules: not UTF-8, a control byte, over a bound.
    Invalid(io::Error),
    /// The wrapper ran and did not confine, or did not run at all.
    Probe {
        /// What the probe saw.
        detail: String,
    },
}

impl fmt::Display for ConfineError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Unsupported { platform } => write!(
                out,
                "{platform} has no kernel confinement for commands, so Larry runs none (macOS and Linux have one)"
            ),
            Self::MissingWrapper { tool } => {
                write!(out, "the confinement tool {tool} is not installed")
            }
            Self::Path {
                ref path,
                ref source,
            } => {
                write!(
                    out,
                    "the workspace path {} cannot be confined: {source}",
                    path.display()
                )
            }
            Self::Invalid(ref cause) => write!(out, "the confinement was refused: {cause}"),
            Self::NotBeneath {
                ref workspace,
                ref state,
            } => write!(
                out,
                "the workspace copy {} is not beneath the state directory {}, so confining the state would open it",
                workspace.display(),
                state.display()
            ),
            Self::Probe { ref detail } => {
                write!(out, "the confinement does not hold on this host: {detail}")
            }
        }
    }
}

impl std::error::Error for ConfineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Path { ref source, .. } => Some(source),
            Self::Invalid(ref cause) => Some(cause),
            Self::Unsupported { .. }
            | Self::MissingWrapper { .. }
            | Self::NotBeneath { .. }
            | Self::Probe { .. } => None,
        }
    }
}

/// Whether `platform` can confine a command, `present` saying whether a tool is installed.
fn supported_with(platform: Platform, present: &dyn Fn(&str) -> bool) -> Result<(), ConfineError> {
    match platform.wrapper() {
        None => Err(ConfineError::Unsupported {
            platform: platform.name(),
        }),
        Some(tool) if present(tool) => Ok(()),
        Some(tool) => Err(ConfineError::MissingWrapper { tool }),
    }
}

/// Whether `platform` can confine a command on this host: it has a wrapper and the wrapper is installed.
pub(crate) fn supported(platform: Platform) -> Result<(), ConfineError> {
    supported_with(platform, &|tool| Path::new(tool).is_file())
}

/// `path` as the UTF-8 text a profile and a rule carry.
fn text(path: &Path) -> Result<&str, ConfineError> {
    path.to_str().ok_or_else(|| {
        ConfineError::Invalid(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a confined path is UTF-8 text",
        ))
    })
}

/// `path` resolved the way the kernel resolves it, which is the path a profile must name.
fn resolved(path: &Path) -> Result<PathBuf, ConfineError> {
    std::fs::canonicalize(path).map_err(|source| ConfineError::Path {
        path: path.to_path_buf(),
        source,
    })
}

/// `value` as one SBPL string literal's content.
fn quoted(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(character),
        }
    }
    out
}

/// The macOS argv prefix: deny every read beneath `state` but the session's own copy
/// (`workspace`, which holds the tree and the scratch directory), and every write but
/// beneath the tree and scratch directory and to the devices.
fn seatbelt(
    state: &str,
    workspace: &str,
    tree: &str,
    scratch: &str,
) -> Result<Vec<String>, ConfineError> {
    let mut source = String::from("(version 1)(allow default)(deny file-read* (require-all");
    source.push_str(" (subpath \"");
    source.push_str(&quoted(state));
    source.push_str("\") (require-not (require-any");
    for path in [workspace, tree, scratch] {
        source.push_str(" (subpath \"");
        source.push_str(&quoted(path));
        source.push_str("\")");
    }
    source.push_str("))))");
    source.push_str("(deny file-write* (require-not (require-any");
    for path in [tree, scratch] {
        source.push_str(" (subpath \"");
        source.push_str(&quoted(path));
        source.push_str("\")");
    }
    for device in DEVICES {
        source.push_str(" (literal \"");
        source.push_str(device);
        source.push_str("\")");
    }
    source.push_str(")))");
    // The one kernel interface that returns another process's arguments and
    // environment (`KERN_PROCARGS2`), which `ps e` reads: the executor's holds its
    // model keys (larry#322).
    source.push_str("(deny sysctl-read (sysctl-name \"kern.procargs2\"))");
    let profile = SandboxProfile::new(source);
    profile.validate().map_err(ConfineError::Invalid)?;
    Ok(vec![
        String::from(SANDBOX_EXEC),
        String::from("-p"),
        String::from(profile.source()),
    ])
}

/// The top-level paths Landlock grants read: every child of `/` but the one that
/// holds the state directory, and the `/proc` entries in [`PROC_READS`] instead of
/// all of `/proc`. An allowlist cannot subtract a child from a granted path, so the
/// state directory's top component is simply never granted (larry#322, ADR 0080).
///
/// Each child is resolved, because a symlink (`/bin` and `/lib` on most hosts) is
/// no rule path, and a child already beneath a granted child is dropped. The
/// `/proc` entries keep their spelling: `/proc/self` names the wrapper's own
/// process, which is the command's once it execs.
fn read_roots(state: &Path) -> Result<Vec<PathBuf>, ConfineError> {
    let root = Path::new("/");
    let mut roots = Vec::new();
    let entries = std::fs::read_dir(root).map_err(|source| ConfineError::Path {
        path: root.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let path = entry
            .map_err(|source| ConfineError::Path {
                path: root.to_path_buf(),
                source,
            })?
            .path();
        if let Some(root) = grantable(&path, state)? {
            roots.push(root);
        }
    }
    roots.sort();
    roots.dedup();
    // A directory already beneath a granted one needs no rule of its own.
    let mut kept: Vec<PathBuf> = Vec::new();
    for path in roots {
        if !kept.iter().any(|granted| path.starts_with(granted)) {
            kept.push(path);
        }
    }
    kept.extend(PROC_READS.iter().map(PathBuf::from));
    Ok(kept)
}

/// A top-level path as one read root, or `None` when it cannot be one: `/proc`, which
/// is granted entry by entry; a file, a device or a dangling link, which a rule on a
/// file cannot name; and the state directory's own ancestor, which would grant it.
fn grantable(path: &Path, state: &Path) -> Result<Option<PathBuf>, ConfineError> {
    if path == Path::new("/proc") || !path.is_dir() {
        return Ok(None);
    }
    let resolved = resolved(path)?;
    Ok((!state.starts_with(&resolved)).then_some(resolved))
}

/// The Linux argv prefix: read the roots above and the session's own copy, read and write
/// beneath `tree` and `scratch`.
fn landlock(
    state: &Path,
    workspace: &Path,
    tree: &str,
    scratch: &str,
) -> Result<Vec<String>, ConfineError> {
    let mut rules = LandlockRules::new();
    for root in read_roots(state)? {
        rules.allow(root, PathAccess::Read);
    }
    rules.allow(workspace, PathAccess::Read);
    rules.allow(tree, PathAccess::ReadWrite);
    rules.allow(scratch, PathAccess::ReadWrite);
    rules.validate().map_err(ConfineError::Invalid)?;
    let mut prefix = vec![
        String::from(SETPRIV),
        String::from("--landlock-access"),
        String::from("fs"),
    ];
    for (path, access) in rules.rules() {
        let list = match access {
            PathAccess::Read => READ_LIST,
            PathAccess::ReadWrite => WRITE_LIST,
            _ => {
                return Err(ConfineError::Invalid(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "an access the confinement does not know",
                )));
            }
        };
        prefix.push(String::from("--landlock-rule"));
        prefix.push(format!("path-beneath:{list}:{}", text(path)?));
    }
    prefix.push(String::from("--landlock-rule"));
    prefix.push(format!("path-beneath:{DEVICE_LIST}:{LANDLOCK_DEVICE}"));
    prefix.push(String::from("--"));
    Ok(prefix)
}

/// One workspace's confinement: writes land beneath its tree and scratch directory only,
/// and nothing reads beneath `state` but the session's own copy, its tree and its scratch
/// directory.
#[derive(Debug, Clone)]
pub(crate) struct Confine {
    /// The workspace's work tree, as the kernel resolves it.
    tree: PathBuf,
    /// The workspace's own temporary directory, `TMPDIR` for every command.
    scratch: PathBuf,
    /// The wrapper's argv, which the command's argv follows.
    prefix: Vec<String>,
}

impl Confine {
    /// The confinement of `platform` over `state`, `tree` and `scratch`, all existing
    /// directories. Nothing is spawned: the estate's validation refuses a path or a
    /// profile that could not confine anything, a tree whose copy is not beneath the
    /// state directory is refused, and a platform with no wrapper is refused.
    pub(crate) fn new(
        platform: Platform,
        state: &Path,
        tree: &Path,
        scratch: &Path,
    ) -> Result<Self, ConfineError> {
        let state = resolved(state)?;
        let tree = resolved(tree)?;
        // The session's own copy is the one path beneath the state directory a command
        // reads: the workspace lays the tree down beside the clone's git directory and
        // the scratch directory (larry#322, ADR 0080).
        let workspace = tree
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| ConfineError::Path {
                path: tree.clone(),
                source: io::Error::new(
                    io::ErrorKind::NotFound,
                    "the workspace copy is the tree's parent directory",
                ),
            })?;
        if state.starts_with(&workspace) {
            return Err(ConfineError::NotBeneath { workspace, state });
        }
        let scratch = resolved(scratch)?;
        let prefix = match platform {
            Platform::Macos => seatbelt(
                text(&state)?,
                text(&workspace)?,
                text(&tree)?,
                text(&scratch)?,
            )?,
            Platform::Linux => landlock(&state, &workspace, text(&tree)?, text(&scratch)?)?,
            Platform::Other => {
                return Err(ConfineError::Unsupported {
                    platform: platform.name(),
                });
            }
        };
        Ok(Self {
            tree,
            scratch,
            prefix,
        })
    }

    /// The directory every command runs in.
    pub(crate) fn tree(&self) -> &Path {
        self.tree.as_path()
    }

    /// The directory `TMPDIR` names for every command.
    pub(crate) fn scratch(&self) -> &Path {
        self.scratch.as_path()
    }

    /// `argv` behind the wrapper, so the kernel confines it and everything it forks.
    pub(crate) fn wrap(&self, argv: &[String]) -> Vec<String> {
        let mut wrapped = self.prefix.clone();
        wrapped.extend_from_slice(argv);
        wrapped
    }

    /// Proves this host enforces it before the model is called: a write beneath the
    /// scratch directory lands, a write beside the workspace is refused and leaves
    /// nothing. A wrapper that is missing, refuses to run or lets the second write
    /// through is [`ConfineError::Probe`], never a command run bare.
    pub(crate) fn probe(&self, path_env: &str) -> Result<(), ConfineError> {
        let outside = match self.scratch.parent() {
            Some(parent) => parent.join("probe-outside"),
            None => {
                return Err(ConfineError::Probe {
                    detail: String::from("the workspace has no parent directory"),
                });
            }
        };
        let allowed = self.scratch.join("probe-inside");
        let argv = self.wrap(&[
            String::from("/bin/sh"),
            String::from("-c"),
            String::from(PROBE_SCRIPT),
            String::from("sh"),
            text(&allowed)?.to_owned(),
            text(&outside)?.to_owned(),
        ]);
        let Some((program, arguments)) = argv.split_first() else {
            return Err(ConfineError::Probe {
                detail: String::from("the probe has no argv"),
            });
        };
        let ran = Command::new(program)
            .args(arguments)
            .current_dir(&self.tree)
            .env_clear()
            .env("PATH", path_env)
            .stdin(Stdio::null())
            .output();
        let landed = outside.exists();
        let cleaned = std::fs::remove_file(&outside).or_else(ignore_missing);
        let cleaned_inside = std::fs::remove_file(&allowed).or_else(ignore_missing);
        let output = ran.map_err(|source| ConfineError::Probe {
            detail: format!("{program} did not start: {source}"),
        })?;
        let probe_io = |source: io::Error| ConfineError::Probe {
            detail: format!("the probe's files could not be removed: {source}"),
        };
        cleaned.map_err(probe_io)?;
        cleaned_inside.map_err(probe_io)?;
        match (output.status.code(), landed) {
            (Some(0), false) => Ok(()),
            (Some(3), _) | (Some(0), true) => Err(ConfineError::Probe {
                detail: String::from("a write beside the workspace was not refused"),
            }),
            (Some(2), _) => Err(ConfineError::Probe {
                detail: String::from("a write inside the workspace was refused"),
            }),
            (_, _) => {
                let mut stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                clip(&mut stderr, PROBE_STDERR);
                Err(ConfineError::Probe {
                    detail: format!("{program} exited {:?}: {stderr}", output.status.code()),
                })
            }
        }
    }
}

/// A file that is already gone is a clean probe.
fn ignore_missing(error: io::Error) -> io::Result<()> {
    if error.kind() == io::ErrorKind::NotFound {
        Ok(())
    } else {
        Err(error)
    }
}

/// Seeded simulations of the confinement's argv, refusals and enforcement.
#[cfg(test)]
#[path = "sim_confine.rs"]
mod sim;
