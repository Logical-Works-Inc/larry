//! One fixed read-only environment probe at task start (larry#274): the task
//! folder's own facts (its path, a bounded tree listing, the toolchain versions
//! found on PATH, the package manifests at its root, free memory and disk, and
//! git status), sealed as the report's `probe` member so the model and the
//! person see the same facts. Every section is a typed `{"ok": ...}` value: a
//! probe that fails is recorded, never dropped. No path is hardcoded: the probe
//! starts at the task folder it is given. No secret is ever recorded: the
//! probe runs fixed commands with fixed arguments and records names, versions
//! and sizes, never environment values or file contents. The probe a run
//! starts with is an injected input ([`Probe`]): live in production, one fixed
//! value wherever a run must replay, and its rendering rides after the prompt
//! prefix a provider caches (ADR 0043), never inside it.

use std::io;
use std::num::ParseIntError;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lgwks_macros::Error;
use lgwks_std::json::{self, Value, json};

/// Longest the probe waits for one tool's `--version` or one git or disk call.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Largest fixed probe value accepted, in bytes: the same bound the sealed
/// record promises the report.
pub(crate) const MAX_PROBE_BYTES: usize = 65_536;

/// Most tree entries the probe records: the root and one level beneath it.
pub(crate) const MAX_ENTRIES: usize = 200;

/// Most characters kept of one tool's version line.
pub(crate) const MAX_VERSION_CHARS: usize = 256;

/// Most status lines kept of `git status`.
pub(crate) const MAX_STATUS_LINES: usize = 50;

/// Most characters kept of the `git status` head.
const MAX_STATUS_CHARS: usize = 4096;

/// Most characters kept of one typed error field.
const MAX_ERROR_CHARS: usize = 256;

/// Most characters kept of the rendered probe the model reads.
pub(crate) const MAX_RENDER_CHARS: usize = 2048;

mod tools;

pub(crate) use tools::{ProbeCall, toolchains};

/// The fixed manifest names the probe stats at the tree root, in order.
const MANIFESTS: [&str; 10] = [
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "pom.xml",
    "Makefile",
    "CMakeLists.txt",
    "requirements.txt",
    "package-lock.json",
    "Cargo.lock",
];

/// Runs `argv` in `cwd` and returns what the call did: its exit status (or the
/// bound kill) with both streams kept. The one seam the simulations script so
/// no test spawns a host tool (larry#269). A `ProbeError` is a spawn refusal;
/// a failed exit is an `Ok` call the toolchain section records as refused.
pub(crate) type Runner<'run> = &'run dyn Fn(&[String], &Path) -> Result<ProbeCall, ProbeError>;

/// Reads one file as text: the seam behind `/proc/meminfo`, so the simulations
/// never touch the host.
pub(crate) type Reader<'read> = &'read dyn Fn(&Path) -> Result<String, ProbeError>;

/// Why one probe section failed. Every variant renders as that section's typed
/// `error` field, so a failure is recorded, never dropped and never a refused
/// task.
#[derive(Debug, Error)]
#[non_exhaustive]
pub(crate) enum ProbeError {
    /// A directory of the task tree could not be listed, or a file could not
    /// be read.
    #[error("the task folder could not be read: {0}")]
    Unreadable(#[source] io::Error),
    /// A fixed probe command failed: the runner's own reason, already bounded.
    #[error("the probe command failed: {0}")]
    Command(String),
    /// A number the probe read did not parse.
    #[error("a probe number did not parse: {0}")]
    Number(#[source] ParseIntError),
    /// A probe input holds nothing the section needs: the reason names what.
    #[error("{0}")]
    Refused(&'static str),
    /// A fixed probe value was not valid JSON.
    #[error("the probe value is not valid JSON: {0}")]
    NotJson(#[source] json::Error),
}

/// The probe a task starts with (larry#274): the live folder probe in
/// production, or one fixed value wherever a run must replay byte for byte.
/// The choice is an injected input, never a global: the same seed with the
/// same probe value drives the same prompts, so the same gateway trace and
/// the same card replay (SD-4), and the live probe never runs in a test that
/// fixed it.
#[derive(Debug, Clone)]
pub(crate) enum Probe {
    /// Run the fixed read-only probe of the task folder.
    Live,
    /// Exactly this value: it seals as it stands and renders after the prompt
    /// prefix, so the person and the model still see the same facts.
    Fixed(Value),
}

impl Probe {
    /// The probe the `LARRY_PROBE` value names: one JSON object of at most
    /// [`MAX_PROBE_BYTES`] bytes. Anything else is a typed refusal, never a
    /// silent fallback to the live probe.
    pub(crate) fn fixed(text: &str) -> Result<Probe, ProbeError> {
        if text.len() > MAX_PROBE_BYTES {
            return Err(ProbeError::Refused("the probe value is over 65536 bytes"));
        }
        let value: Value = json::from_str(text).map_err(ProbeError::NotJson)?;
        if value.as_object().is_none() {
            return Err(ProbeError::Refused("the probe value is not a JSON object"));
        }
        Ok(Probe::Fixed(value))
    }
}

/// Probes `root`: the tree listing, the toolchain versions, the manifests, the
/// memory, the disk and the git status. Every section records its own typed
/// `ok` outcome, so a failure is a field, never a dropped fact and never a
/// refused task.
#[must_use]
pub(crate) fn probe(root: &Path, run: Runner<'_>, read: Reader<'_>) -> Value {
    json!({
        "cwd": root.display().to_string(),
        "tree": list_tree(root),
        "toolchains": toolchains(run, root),
        "manifests": manifests(root),
        "memory": memory(root, run, read),
        "disk": disk(root, run),
        "git": git_status(run, root),
    })
}

/// The tree section: the root's entries and one level beneath them, at most
/// [`MAX_ENTRIES`] in sorted order, with `truncated` naming a cut.
fn list_tree(root: &Path) -> Value {
    match walk(root) {
        Ok((entries, truncated)) => json!({
            "ok": true,
            "truncated": truncated,
            "entries": entries,
        }),
        Err(cause) => json!({
            "ok": false,
            "error": sanitize(&cause.to_string(), MAX_ERROR_CHARS),
        }),
    }
}

/// The entries of `dir`, sorted by name bytes, or why they could not be read.
fn read_sorted(dir: &Path) -> Result<Vec<PathBuf>, ProbeError> {
    let mut names: Vec<PathBuf> = Vec::new();
    let listed = std::fs::read_dir(dir).map_err(ProbeError::Unreadable)?;
    for entry in listed {
        let entry = entry.map_err(ProbeError::Unreadable)?;
        names.push(PathBuf::from(entry.file_name()));
    }
    names.sort();
    Ok(names)
}

/// One tree entry: its path relative to the root, its kind
/// (`file`, `dir`, `symlink`, `other` or `error`) and a file's length in bytes.
fn entry_json(relative: &Path, kind: &str, len: Option<u64>) -> Value {
    json!({
        "path": relative.to_string_lossy(),
        "kind": kind,
        "len": len,
    })
}

/// The kind and length of `full`: a symlink reads as one even when its target
/// is gone, because the metadata never follows it.
fn describe(full: &Path) -> (&'static str, Option<u64>) {
    let meta = match std::fs::symlink_metadata(full) {
        Ok(meta) => meta,
        Err(_) => return ("error", None),
    };
    if meta.is_dir() {
        ("dir", None)
    } else if meta.is_file() {
        ("file", Some(meta.len()))
    } else if meta.file_type().is_symlink() {
        ("symlink", None)
    } else {
        ("other", None)
    }
}

/// The bounded walk of `root`: its entries, then one level into each
/// subdirectory, in sorted order, stopping at [`MAX_ENTRIES`]. `truncated`
/// says a cap cut the walk; a name that vanishes between the listing and its
/// stat reads as `error`, never as absent.
fn walk(root: &Path) -> Result<(Vec<Value>, bool), ProbeError> {
    let mut entries: Vec<Value> = Vec::new();
    let mut truncated = false;
    let mut dirs: Vec<PathBuf> = Vec::new();
    for name in read_sorted(root)? {
        if entries.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let (kind, len) = describe(&root.join(&name));
        if kind == "dir" {
            dirs.push(name.clone());
        }
        entries.push(entry_json(&name, kind, len));
    }
    if !truncated {
        for dir in &dirs {
            if entries.len() >= MAX_ENTRIES {
                truncated = true;
                break;
            }
            let nested = match read_sorted(&root.join(dir)) {
                Ok(names) => names,
                Err(_) => {
                    entries.push(entry_json(dir, "error", None));
                    continue;
                }
            };
            for name in nested {
                if entries.len() >= MAX_ENTRIES {
                    truncated = true;
                    break;
                }
                let relative = dir.join(&name);
                let (kind, len) = describe(&root.join(&relative));
                entries.push(entry_json(&relative, kind, len));
            }
        }
    }
    Ok((entries, truncated))
}

/// The manifest section: every fixed manifest name present at the root with
/// its size in bytes. Contents are never read, so a credential in one can
/// never reach the record.
fn manifests(root: &Path) -> Value {
    let mut found: Vec<Value> = Vec::new();
    for name in MANIFESTS {
        match std::fs::metadata(root.join(name)) {
            Ok(meta) if meta.is_file() => found.push(json!({
                "path": name,
                "len": meta.len(),
            })),
            Ok(_) => {}
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => {}
            Err(cause) => found.push(json!({
                "path": name,
                "len": Value::Null,
                "error": sanitize(&cause.to_string(), MAX_ERROR_CHARS),
            })),
        }
    }
    Value::Array(found)
}

/// The memory section: total and available KiB, or the typed reason neither
/// reader could say.
fn memory(root: &Path, run: Runner<'_>, read: Reader<'_>) -> Value {
    let outcome = meminfo_kb(read).or_else(|first| {
        sysctl_kb(root, run).map_err(|second| ProbeError::Command(format!("{first}; {second}")))
    });
    match outcome {
        Ok((total, available)) => json!({
            "ok": true,
            "total_kb": total,
            "avail_kb": available,
        }),
        Err(cause) => json!({
            "ok": false,
            "error": sanitize(&cause.to_string(), MAX_ERROR_CHARS),
        }),
    }
}

/// Total and available KiB from `/proc/meminfo`.
fn meminfo_kb(read: Reader<'_>) -> Result<(u64, u64), ProbeError> {
    let text = read(Path::new("/proc/meminfo"))?;
    parse_meminfo(&text)
}

/// Total and available KiB from `sysctl` and `vm_stat` (hosts without procfs).
fn sysctl_kb(root: &Path, run: Runner<'_>) -> Result<(u64, u64), ProbeError> {
    let total_out = run(
        &[
            String::from("sysctl"),
            String::from("-n"),
            String::from("hw.memsize"),
        ],
        root,
    )?;
    let total_bytes = parse_sysctl(total_out.text())?;
    let page_out = run(
        &[
            String::from("sysctl"),
            String::from("-n"),
            String::from("hw.pagesize"),
        ],
        root,
    )?;
    let page_bytes = parse_sysctl(page_out.text())?;
    let vmstat = run(&[String::from("vm_stat")], root)?;
    let free_bytes = parse_vmstat(vmstat.text(), page_bytes)?;
    Ok((total_bytes.div_ceil(1024), free_bytes.div_ceil(1024)))
}

/// The disk section: the available KiB of the file system holding `root`.
fn disk(root: &Path, run: Runner<'_>) -> Value {
    let argv = vec![
        String::from("df"),
        String::from("-k"),
        String::from("-P"),
        root.to_string_lossy().into_owned(),
    ];
    match run(&argv, root).and_then(|out| parse_df(out.text())) {
        Ok(available) => json!({
            "ok": true,
            "avail_kb": available,
        }),
        Err(cause) => json!({
            "ok": false,
            "error": sanitize(&cause.to_string(), MAX_ERROR_CHARS),
        }),
    }
}

/// The git section: how many `status` lines the tree holds and their bounded
/// head, or the typed reason git could not say (a plain folder, say).
fn git_status(run: Runner<'_>, root: &Path) -> Value {
    let argv = vec![
        String::from("git"),
        String::from("status"),
        String::from("--porcelain=v1"),
        String::from("--branch"),
        String::from("--untracked-files=normal"),
    ];
    match run(&argv, root) {
        Ok(out) => {
            let (lines, head, truncated) = parse_status(out.text());
            json!({
                "ok": true,
                "lines": lines,
                "truncated": truncated,
                "head": head,
            })
        }
        Err(cause) => json!({
            "ok": false,
            "error": sanitize(&cause.to_string(), MAX_ERROR_CHARS),
        }),
    }
}

/// The `(total, available)` KiB of `text` in `/proc/meminfo` shape: exactly
/// `MemTotal:` and `MemAvailable:` with one `kB` number each.
pub(crate) fn parse_meminfo(text: &str) -> Result<(u64, u64), ProbeError> {
    let mut total: Option<u64> = None;
    let mut available: Option<u64> = None;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(key), Some(number), Some(unit)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if parts.next().is_some() || unit != "kB" {
            continue;
        }
        let kb = number.parse::<u64>().map_err(ProbeError::Number)?;
        if key == "MemTotal:" {
            total = Some(kb);
        } else if key == "MemAvailable:" {
            available = Some(kb);
        }
    }
    match (total, available) {
        (Some(total), Some(available)) => Ok((total, available)),
        (None, _) => Err(ProbeError::Refused("meminfo holds no MemTotal")),
        (_, None) => Err(ProbeError::Refused("meminfo holds no MemAvailable")),
    }
}

/// The bytes of `text` in `sysctl -n` shape: one decimal number.
fn parse_sysctl(text: &str) -> Result<u64, ProbeError> {
    text.trim().parse::<u64>().map_err(ProbeError::Number)
}

/// The free bytes of `text` in `vm_stat` shape: the `Pages free:` count times
/// the page size.
fn parse_vmstat(text: &str, page_bytes: u64) -> Result<u64, ProbeError> {
    for line in text.lines() {
        let Some(count) = line.strip_prefix("Pages free:") else {
            continue;
        };
        let trimmed = count.trim();
        let digits = match trimmed.strip_suffix('.') {
            Some(stripped) => stripped,
            None => trimmed,
        };
        let pages = digits.parse::<u64>().map_err(ProbeError::Number)?;
        return pages
            .checked_mul(page_bytes)
            .ok_or(ProbeError::Refused("vm_stat free bytes overflow"));
    }
    Err(ProbeError::Refused("vm_stat printed no free pages"))
}

/// The available KiB of `text` in `df -k -P` shape: the `Available` column of
/// the first data row, counted from the row's end so a spaced device name
/// cannot shift it.
pub(crate) fn parse_df(text: &str) -> Result<u64, ProbeError> {
    let mut lines = text.lines();
    lines
        .next()
        .ok_or(ProbeError::Refused("df printed no header"))?;
    let row = lines
        .next()
        .ok_or(ProbeError::Refused("df printed no row"))?;
    let fields: Vec<&str> = row.split_whitespace().collect();
    if fields.len() < 6 {
        return Err(ProbeError::Refused("df printed too few fields"));
    }
    let Some(available) = fields.get(fields.len().saturating_sub(3)) else {
        return Err(ProbeError::Refused("df printed too few fields"));
    };
    available.parse::<u64>().map_err(ProbeError::Number)
}

/// The `(lines, head, truncated)` of `text` in `git status --porcelain`
/// shape: every line counted, the first [`MAX_STATUS_LINES`] kept within
/// [`MAX_STATUS_CHARS`] characters.
#[must_use]
pub(crate) fn parse_status(text: &str) -> (u64, String, bool) {
    let mut lines = 0_u64;
    let mut kept: Vec<&str> = Vec::new();
    let mut kept_lines = 0_u64;
    for line in text.lines() {
        lines = lines.saturating_add(1);
        if kept.len() < MAX_STATUS_LINES {
            kept.push(line);
            kept_lines = kept_lines.saturating_add(1);
        }
    }
    let mut head = kept.join("\n");
    let mut cut = lines > kept_lines;
    if head.chars().count() > MAX_STATUS_CHARS {
        head = head.chars().take(MAX_STATUS_CHARS).collect();
        cut = true;
    }
    (lines, head, cut)
}

/// The first line of `text`, bounded to [`MAX_VERSION_CHARS`] characters: what
/// one tool's `--version` says it is.
pub(crate) fn version_line(text: &str) -> Result<String, ProbeError> {
    let Some(first) = text.lines().next() else {
        return Err(ProbeError::Refused("the version output is empty"));
    };
    let line: String = first.chars().take(MAX_VERSION_CHARS).collect();
    let trimmed = line.trim().to_owned();
    if trimmed.is_empty() {
        return Err(ProbeError::Refused("the version output is blank"));
    }
    Ok(trimmed)
}

/// Caps `text` at `limit` characters for an error field, so a hostile command
/// cannot grow the report without bound.
#[must_use]
pub(crate) fn sanitize(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// Renders `probed` as the text the model's first prompt carries: the same
/// facts the sealed report holds, bounded to [`MAX_RENDER_CHARS`] characters.
#[must_use]
pub(crate) fn render(probed: &Value) -> String {
    let mut text = String::from("Environment probe (the sealed report holds the same facts):\n");
    if let Some(cwd) = probed.get("cwd").and_then(Value::as_str) {
        text.push_str("folder: ");
        text.push_str(cwd);
        text.push('\n');
    }
    render_tree(probed, &mut text);
    render_tools(probed, &mut text);
    render_manifests(probed, &mut text);
    render_scalar(probed, "memory", &mut text);
    render_scalar(probed, "disk", &mut text);
    render_git(probed, &mut text);
    if text.chars().count() > MAX_RENDER_CHARS {
        text = text.chars().take(MAX_RENDER_CHARS).collect();
    }
    text
}

/// Appends the tree section: `name(kind)` per entry, or the typed error.
fn render_tree(probed: &Value, text: &mut String) {
    let Some(tree) = probed.get("tree") else {
        return;
    };
    text.push_str("tree: ");
    if tree.get("ok").and_then(Value::as_bool) == Some(true) {
        let mut first = true;
        if let Some(entries) = tree.get("entries").and_then(Value::as_array) {
            for entry in entries {
                let (Some(path), Some(kind)) = (
                    entry.get("path").and_then(Value::as_str),
                    entry.get("kind").and_then(Value::as_str),
                ) else {
                    continue;
                };
                if !first {
                    text.push_str(", ");
                }
                first = false;
                text.push_str(path);
                text.push('(');
                text.push_str(kind);
                text.push(')');
            }
        }
        if tree.get("truncated").and_then(Value::as_bool) == Some(true) {
            text.push_str(" ...truncated");
        }
    } else if let Some(error) = tree.get("error").and_then(Value::as_str) {
        text.push('<');
        text.push_str(error);
        text.push('>');
    }
    text.push('\n');
}

/// Appends the toolchain section: `tool version` per tool, or the typed error.
fn render_tools(probed: &Value, text: &mut String) {
    let Some(tools) = probed.get("toolchains").and_then(Value::as_array) else {
        return;
    };
    text.push_str("toolchains: ");
    let mut first = true;
    for entry in tools {
        let Some(tool) = entry.get("tool").and_then(Value::as_str) else {
            continue;
        };
        if !first {
            text.push_str(", ");
        }
        first = false;
        text.push_str(tool);
        text.push(' ');
        if entry.get("ok").and_then(Value::as_bool) == Some(true) {
            let Some(version) = entry.get("version").and_then(Value::as_str) else {
                continue;
            };
            text.push_str(version);
        } else {
            text.push('<');
            let Some(error) = entry.get("error").and_then(Value::as_str) else {
                continue;
            };
            text.push_str(error);
            text.push('>');
        }
    }
    text.push('\n');
}

/// Appends the manifest section: `name (bytes)` per manifest present.
fn render_manifests(probed: &Value, text: &mut String) {
    let Some(found) = probed.get("manifests").and_then(Value::as_array) else {
        return;
    };
    text.push_str("manifests: ");
    if found.is_empty() {
        text.push_str("none\n");
        return;
    }
    let mut first = true;
    for entry in found {
        let Some(path) = entry.get("path").and_then(Value::as_str) else {
            continue;
        };
        if !first {
            text.push_str(", ");
        }
        first = false;
        text.push_str(path);
        if let Some(len) = entry.get("len").and_then(Value::as_u64) {
            text.push_str(" (");
            text.push_str(&len.to_string());
            text.push_str(" bytes)");
        }
    }
    text.push('\n');
}

/// Appends one scalar section (`memory`, `disk`): its members, or its error.
fn render_scalar(probed: &Value, section: &str, text: &mut String) {
    let Some(field) = probed.get(section) else {
        return;
    };
    text.push_str(section);
    text.push_str(": ");
    if field.get("ok").and_then(Value::as_bool) == Some(true) {
        text.push_str(&field.to_string());
    } else {
        text.push('<');
        let Some(error) = field.get("error").and_then(Value::as_str) else {
            return;
        };
        text.push_str(error);
        text.push('>');
    }
    text.push('\n');
}

/// Appends the git section: the line count and the bounded head, or its error.
fn render_git(probed: &Value, text: &mut String) {
    let Some(git) = probed.get("git") else {
        return;
    };
    text.push_str("git: ");
    if git.get("ok").and_then(Value::as_bool) == Some(true) {
        let Some(lines) = git.get("lines").and_then(Value::as_u64) else {
            return;
        };
        text.push_str(&lines.to_string());
        text.push_str(" status lines\n");
        if let Some(head) = git.get("head").and_then(Value::as_str) {
            text.push_str(head);
            text.push('\n');
        }
    } else {
        text.push('<');
        let Some(error) = git.get("error").and_then(Value::as_str) else {
            return;
        };
        text.push_str(error);
        text.push_str(">\n");
    }
}

#[cfg(test)]
#[path = "sim_probe.rs"]
mod sim;
