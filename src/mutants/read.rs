//! What `larry mutants verdict` reads of the judged run, each file through the governance
//! bounded reader (no link anywhere on its path, no FIFO, at most 4 MiB): the plan's mutant
//! list, each shard's outcomes, the run's job list and the artifact directories themselves.
//! Every shape it does not know is a typed [`Fault`], never a default.

use std::ffi::OsString;
use std::path::Path;

use lgwks_std::json::{self, Value};

use super::{ARCHES, Fault, MAX_DIGITS, MAX_JOBS, MAX_JSON_BYTES};
use crate::governance::read::read_bounded;

/// One mutant, named as cargo-mutants names it in the plan and in every outcome.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Mutant {
    /// The source file.
    pub(super) file: String,
    /// The line the mutation starts on.
    pub(super) line: u64,
    /// The replacement text.
    pub(super) replacement: String,
    /// cargo-mutants' full name for it.
    name: String,
}

impl Mutant {
    /// The mutant a plan entry or a scenario's `Mutant` member describes, if it has every field.
    fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            file: value.get("file")?.as_str()?.to_owned(),
            line: value.get("span")?.get("start")?.get("line")?.as_u64()?,
            replacement: value.get("replacement")?.as_str()?.to_owned(),
            name: value.get("name")?.as_str()?.to_owned(),
        })
    }
}

/// What cargo-mutants concluded about one mutant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Outcome {
    /// A test failed with the mutant in place.
    Caught,
    /// Every test passed with the mutant in place.
    Missed,
    /// The mutant did not build.
    Unviable,
    /// The tests ran past the timeout.
    Timeout,
}

impl Outcome {
    /// The outcome a summary names, or `None` for one this verdict does not know.
    fn parse(summary: &str) -> Option<Self> {
        match summary {
            "CaughtMutant" => Some(Self::Caught),
            "MissedMutant" => Some(Self::Missed),
            "Unviable" => Some(Self::Unviable),
            "Timeout" => Some(Self::Timeout),
            _ => None,
        }
    }

    /// The summary cargo-mutants writes for it.
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Caught => "CaughtMutant",
            Self::Missed => "MissedMutant",
            Self::Unviable => "Unviable",
            Self::Timeout => "Timeout",
        }
    }
}

/// One job of the judged run's latest attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Job {
    /// Its name as the run shows it.
    pub(super) name: String,
    /// `success`, `failure`, `skipped` and the rest, or `None` while it runs.
    pub(super) conclusion: Option<String>,
}

/// What one artifact of the judged run is, by its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// `mutants-plan`, holding `list.json`.
    Plan,
    /// `mutants-diff`, holding `pr.diff`.
    Diff,
    /// `mutants-outcomes-<arch>-<k>`, holding `outcomes.json`.
    Outcomes {
        /// The architecture the shard ran on.
        arch: &'static str,
        /// The shard's number.
        shard: usize,
    },
}

impl Kind {
    /// The one file the artifact holds.
    const fn file(self) -> &'static str {
        match self {
            Self::Plan => "list.json",
            Self::Diff => "pr.diff",
            Self::Outcomes { .. } => "outcomes.json",
        }
    }

    /// The kind an artifact's name names, or `None` for one this lane never uploads.
    fn of(name: &str) -> Option<Self> {
        match name {
            "mutants-plan" => Some(Self::Plan),
            "mutants-diff" => Some(Self::Diff),
            _ => {
                let rest = name.strip_prefix("mutants-outcomes-")?;
                ARCHES.iter().find_map(|&arch| {
                    let digits = rest.strip_prefix(arch)?.strip_prefix('-')?;
                    shard_number(digits).map(|shard| Self::Outcomes { arch, shard })
                })
            }
        }
    }
}

/// A shard number as the matrix writes it: one to six digits with no leading zero.
fn shard_number(digits: &str) -> Option<usize> {
    if digits.is_empty()
        || digits.len() > MAX_DIGITS
        || (digits.len() > 1 && digits.starts_with('0'))
    {
        return None;
    }
    digits.bytes().try_fold(0_usize, |value, byte| {
        let digit = byte.is_ascii_digit().then(|| byte.checked_sub(b'0'))??;
        value.checked_mul(10)?.checked_add(usize::from(digit))
    })
}

/// Reads `relative` under `root` through the governance bounded reader.
pub(super) fn read(root: &Path, relative: &Path) -> Result<Vec<u8>, Fault> {
    read_bounded(root, relative, MAX_JSON_BYTES).map_err(|cause| Fault::Read {
        path: root.join(relative),
        cause,
    })
}

/// Reads a file the command line names, from its own directory.
pub(super) fn read_path(path: &Path) -> Result<Vec<u8>, Fault> {
    let name = path.file_name().ok_or(Fault::Usage)?;
    let root = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        Some(_) | None => Path::new("."),
    };
    read(root, Path::new(name))
}

/// Parses one file as JSON.
fn parse(path: &Path, bytes: &[u8]) -> Result<Value, Fault> {
    json::from_slice(bytes).map_err(|cause| Fault::Json {
        path: path.to_path_buf(),
        cause,
    })
}

/// The judged run's jobs: one array of `{name, conclusion}`, at most [`MAX_JOBS`] of them.
pub(super) fn jobs(path: &Path) -> Result<Vec<Job>, Fault> {
    let value = parse(path, &read_path(path)?)?;
    let shape = |what| Fault::Shape {
        path: path.to_path_buf(),
        what,
    };
    let items = value
        .as_array()
        .ok_or_else(|| shape("is not an array of jobs"))?;
    if items.len() > MAX_JOBS {
        return Err(shape("lists more jobs than one run holds"));
    }
    items
        .iter()
        .map(|item| {
            let name = item.get("name").and_then(Value::as_str);
            let conclusion = match item.get("conclusion") {
                None | Some(&Value::Null) => Some(None),
                Some(other) => other.as_str().map(|text| Some(text.to_owned())),
            };
            match (name, conclusion) {
                (Some(name), Some(conclusion)) => Ok(Job {
                    name: name.to_owned(),
                    conclusion,
                }),
                (None, _) | (_, None) => {
                    Err(shape("holds a job without a name or a text conclusion"))
                }
            }
        })
        .collect()
}

/// The artifacts the run uploaded, at most `limit`: each a directory this lane uploads holding
/// only its one regular file.
pub(super) fn artifacts(root: &Path, limit: usize) -> Result<Vec<Kind>, Fault> {
    let listing = |cause| Fault::Artifacts {
        path: root.to_path_buf(),
        cause,
    };
    if !std::fs::symlink_metadata(root).map_err(listing)?.is_dir() {
        return Err(Fault::Artifact {
            name: root.display().to_string(),
            what: "is not a directory of artifacts",
        });
    }
    let mut kinds = Vec::new();
    for entry in std::fs::read_dir(root).map_err(listing)? {
        let entry = entry.map_err(listing)?;
        if kinds.len() >= limit {
            return Err(Fault::TooManyArtifacts { limit });
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let wrong = |what| Fault::Artifact {
            name: name.clone(),
            what,
        };
        let kind = Kind::of(&name).ok_or_else(|| wrong("is not one this lane uploads"))?;
        if !entry.file_type().map_err(listing)?.is_dir() {
            return Err(wrong("is not a directory"));
        }
        let mut held = Vec::new();
        for inner in std::fs::read_dir(entry.path()).map_err(listing)?.take(2) {
            held.push(inner.map_err(listing)?.file_name());
        }
        if held.as_slice() != [OsString::from(kind.file())] {
            return Err(wrong("holds more or other than its one file"));
        }
        let file = std::fs::symlink_metadata(entry.path().join(kind.file())).map_err(listing)?;
        if !file.is_file() {
            return Err(wrong("holds a file that is not a regular file"));
        }
        kinds.push(kind);
    }
    Ok(kinds)
}

/// The plan's mutants, sorted. An empty diff makes cargo-mutants print nothing, not `[]`; both
/// are zero mutants.
pub(super) fn plan(root: &Path) -> Result<Vec<Mutant>, Fault> {
    let relative = Path::new("mutants-plan").join("list.json");
    let path = root.join(&relative);
    let bytes = read(root, &relative)?;
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Vec::new());
    }
    let value = parse(&path, &bytes)?;
    let mut mutants = value
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .map(Mutant::from_json)
                .collect::<Option<Vec<_>>>()
        })
        .ok_or(Fault::Shape {
            path,
            what: "is not an array of mutants with a file, line, replacement and name",
        })?;
    mutants.sort();
    Ok(mutants)
}

/// Every mutant one shard judged, with its outcome; the baseline entry is skipped.
pub(super) fn shard_outcomes(
    root: &Path,
    arch: &'static str,
    shard: usize,
) -> Result<Vec<(Mutant, Outcome)>, Fault> {
    let relative = Path::new(&format!("mutants-outcomes-{arch}-{shard}")).join("outcomes.json");
    let path = root.join(&relative);
    let value = parse(&path, &read(root, &relative)?)?;
    let shape = |what| Fault::Shape {
        path: path.clone(),
        what,
    };
    let entries = value
        .get("outcomes")
        .and_then(Value::as_array)
        .ok_or_else(|| shape("is not a cargo-mutants outcomes file"))?;
    let mut judged = Vec::new();
    for entry in entries {
        let scenario = entry.get("scenario");
        if scenario.is_some_and(Value::is_string) {
            continue;
        }
        let mutant = scenario
            .and_then(|scenario| scenario.get("Mutant"))
            .and_then(Mutant::from_json)
            .ok_or_else(|| shape("holds a scenario that is neither the baseline nor a mutant"))?;
        let summary = entry
            .get("summary")
            .and_then(Value::as_str)
            .ok_or_else(|| shape("holds a mutant with no summary"))?;
        let outcome = Outcome::parse(summary).ok_or_else(|| Fault::UnknownOutcome {
            arch,
            summary: summary.to_owned(),
        })?;
        judged.push((mutant, outcome));
    }
    Ok(judged)
}
