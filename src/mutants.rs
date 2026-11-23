//! `larry mutants verdict`: the mutation lane's one verdict (larry#387, ADR 0098).
//!
//! `.github/workflows/mutants-verdict.yml` runs it on `workflow_run` from a build of the default
//! branch, so a pull request that edits `mutants.yml`, its jobs or its floor is still judged by
//! main's code and main's settings. What the judged run produced is data ([`read`]), and
//! anything missing, malformed, unexpected or not matching the plan is a typed refusal, never a
//! pass. Source: GitHub Security Lab, "Keeping your GitHub Actions and workflows secure Part 1:
//! Preventing pwn requests" (2021); CWE-829.

mod read;

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Write};
use std::num::ParseIntError;
use std::path::{Path, PathBuf};

use lgwks_std::json;

use crate::governance::read::ReadFault;
use read::{Job, Kind, Mutant, Outcome};

/// Bytes one JSON file of the judged run may hold: a plan or a shard lists a few mutants.
const MAX_JSON_BYTES: u64 = 4_194_304;
/// Jobs one run may list: GitHub runs at most 256 jobs a matrix.
const MAX_JOBS: usize = 512;
/// The architectures every mutant is judged on (ADR 0095), in report order.
const ARCHES: [&str; 2] = ["x86_64", "aarch64"];
/// The most digits a shard number holds: a matrix runs at most 256 jobs.
const MAX_DIGITS: usize = 6;

/// The floor and the caps the default branch's `mutants.yml` publishes, each once as
/// `NAME: <number>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Settings {
    /// The lowest caught share of the viable mutants, in percent.
    floor: usize,
    /// The fewest mutants a shard holds.
    per_shard: usize,
    /// The most shards a plan runs.
    max_shards: usize,
    /// The most mutants a shard may hold.
    shard_capacity: usize,
}

impl Settings {
    /// Reads the four settings; each must appear exactly once as one number.
    fn parse(text: &str) -> Result<Self, Fault> {
        let settings = Self {
            floor: setting(text, "MUTANTS_FLOOR")?,
            per_shard: setting(text, "MUTANTS_PER_SHARD")?,
            max_shards: setting(text, "MUTANTS_MAX_SHARDS")?,
            shard_capacity: setting(text, "MUTANTS_SHARD_CAPACITY")?,
        };
        if settings.floor > 100
            || settings.per_shard == 0
            || settings.max_shards == 0
            || settings.shard_capacity == 0
        {
            return Err(Fault::Range);
        }
        Ok(settings)
    }

    /// The most mutants a plan may list.
    const fn capacity(self) -> usize {
        self.max_shards.saturating_mul(self.shard_capacity)
    }

    /// The plan job's sizing: at least `per_shard` mutants a shard, never more shards than
    /// `max_shards`, and no shard left empty (run 44638202506).
    fn shards(self, count: usize) -> usize {
        let per = count.div_ceil(self.max_shards).max(self.per_shard);
        count.div_ceil(per)
    }
}

/// The value of `NAME: <number>`. Every `NAME:` line counts, a number or not, so a second or
/// malformed line is a refusal, never a line skipped.
fn setting(text: &str, name: &'static str) -> Result<usize, Fault> {
    let found: Vec<&str> = text
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix(name)?.strip_prefix(':'))
        .map(str::trim)
        .collect();
    match *found.as_slice() {
        [digits] => digits
            .parse()
            .map_err(|cause| Fault::Number { name, cause }),
        _ => Err(Fault::Setting {
            name,
            seen: found.len(),
        }),
    }
}

/// Why no verdict was given: each refusal fails the lane closed.
#[derive(Debug)]
pub(crate) enum Fault {
    /// The command line is not `verdict <settings> <artifacts> <jobs.json>`.
    Usage,
    /// A file was refused before or while it was read.
    Read {
        /// The file.
        path: PathBuf,
        /// The bounded reader's refusal.
        cause: ReadFault,
    },
    /// A file is not JSON.
    Json {
        /// The file.
        path: PathBuf,
        /// The parser's refusal.
        cause: json::Error,
    },
    /// A file is JSON of another shape.
    Shape {
        /// The file.
        path: PathBuf,
        /// What it lacks.
        what: &'static str,
    },
    /// A setting is missing or published more than once.
    Setting {
        /// The setting.
        name: &'static str,
        /// The lines that publish it as a number.
        seen: usize,
    },
    /// A setting's digits are not a number.
    Number {
        /// The setting.
        name: &'static str,
        /// The parser's refusal.
        cause: ParseIntError,
    },
    /// The floor is past 100 or a cap is zero.
    Range,
    /// The artifacts directory cannot be listed.
    Artifacts {
        /// The directory.
        path: PathBuf,
        /// The operating system's refusal.
        cause: io::Error,
    },
    /// An artifact this lane never uploads, or one that holds the wrong files.
    Artifact {
        /// The artifact's name.
        name: String,
        /// What is wrong with it.
        what: &'static str,
    },
    /// More artifacts than a plan at the caps uploads.
    TooManyArtifacts {
        /// The bound.
        limit: usize,
    },
    /// The plan job did not end `success`, or the run lists it twice.
    PlanJob,
    /// The shard jobs are not every planned shard on both architectures, each ending success.
    ShardJobs {
        /// The shards the plan sizes.
        shards: usize,
    },
    /// The outcomes artifacts are not one per shard and architecture.
    OutcomeArtifacts {
        /// The artifacts found.
        found: usize,
        /// The shards the plan sizes.
        shards: usize,
    },
    /// The plan lists more mutants than the default branch's caps hold.
    Capacity {
        /// The mutants listed.
        count: usize,
        /// The caps' product.
        capacity: usize,
    },
    /// An architecture's outcomes are not exactly one per planned mutant.
    Mismatch {
        /// The architecture.
        arch: &'static str,
    },
    /// A mutant's summary is one this verdict does not know.
    UnknownOutcome {
        /// The architecture.
        arch: &'static str,
        /// The summary, as written.
        summary: String,
    },
    /// The report could not be written.
    Output(io::Error),
}

/// The bounded reader's refusal in words.
fn refused(cause: &ReadFault) -> String {
    match *cause {
        ReadFault::Missing(ref io) => format!("is missing: {io}"),
        ReadFault::Symlink => "is a symbolic link or lies under one".to_owned(),
        ReadFault::NotRegular { .. } => "is not a regular file".to_owned(),
        ReadFault::TooLarge { bytes } => {
            format!("holds {bytes} bytes, past the bound of {MAX_JSON_BYTES}")
        }
        ReadFault::Unreadable(ref io) => format!("cannot be read: {io}"),
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Usage => {
                out.write_str("usage: larry mutants verdict <settings> <artifacts> <jobs.json>")
            }
            Self::Read {
                ref path,
                ref cause,
            } => write!(out, "{} {}", path.display(), refused(cause)),
            Self::Json {
                ref path,
                ref cause,
            } => write!(out, "{} is not JSON: {cause}", path.display()),
            Self::Shape { ref path, what } => write!(out, "{} {what}", path.display()),
            Self::Setting { name, seen } => write!(
                out,
                "the default branch publishes {name} on {seen} lines, not one"
            ),
            Self::Number { name, ref cause } => write!(out, "{name} is not a number: {cause}"),
            Self::Range => out.write_str("the default branch's floor is past 100 or a cap is zero"),
            Self::Artifacts {
                ref path,
                ref cause,
            } => write!(
                out,
                "the artifacts at {} cannot be listed: {cause}",
                path.display()
            ),
            Self::Artifact { ref name, what } => write!(out, "the artifact {name} {what}"),
            Self::TooManyArtifacts { limit } => {
                write!(out, "the judged run uploaded more than {limit} artifacts")
            }
            Self::PlanJob => {
                out.write_str("the plan job did not end success: there is no plan to judge")
            }
            Self::ShardJobs { shards } => write!(
                out,
                "the shard jobs are not each of {shards} planned shard(s) on both architectures, each ending success"
            ),
            Self::OutcomeArtifacts { found, shards } => write!(
                out,
                "{found} outcomes artifacts for {shards} shard(s) on 2 architectures"
            ),
            Self::Capacity { count, capacity } => write!(
                out,
                "{count} mutants exceed the default branch's capacity of {capacity}: split the pull request"
            ),
            Self::Mismatch { arch } => {
                write!(
                    out,
                    "{arch}: the outcomes are not exactly one per planned mutant"
                )
            }
            Self::UnknownOutcome { arch, ref summary } => {
                write!(
                    out,
                    "{arch}: the outcome {summary:?} is not one this verdict knows"
                )
            }
            Self::Output(ref cause) => write!(out, "the report cannot be written: {cause}"),
        }
    }
}

impl std::error::Error for Fault {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Read {
                cause: ReadFault::Missing(ref io) | ReadFault::Unreadable(ref io),
                ..
            }
            | Self::Artifacts { cause: ref io, .. }
            | Self::Output(ref io) => Some(io),
            Self::Json { ref cause, .. } => Some(cause),
            Self::Number { ref cause, .. } => Some(cause),
            Self::Read { .. }
            | Self::Usage
            | Self::Shape { .. }
            | Self::Setting { .. }
            | Self::Range
            | Self::Artifact { .. }
            | Self::TooManyArtifacts { .. }
            | Self::PlanJob
            | Self::ShardJobs { .. }
            | Self::OutcomeArtifacts { .. }
            | Self::Capacity { .. }
            | Self::Mismatch { .. }
            | Self::UnknownOutcome { .. } => None,
        }
    }
}

/// Why `larry mutants` gave no verdict. The lane reads it as a red verdict (fail closed).
#[derive(Debug)]
pub struct VerdictError(Fault);

impl fmt::Display for VerdictError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "larry mutants verdict: {} (fail closed)", self.0)
    }
}

impl std::error::Error for VerdictError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// The verdict on one judged run: the report lines, the verdict last, and whether every
/// architecture met the floor and agreed with the other.
#[derive(Debug)]
struct Judgement {
    /// The report, one line each.
    lines: Vec<String>,
    /// Whether the lane passes.
    passed: bool,
}

/// `larry mutants verdict <settings> <artifacts> <jobs.json>`: judges one mutation run from its
/// data and prints the report on `out`, the verdict last. Returns whether the lane passes.
///
/// # Errors
/// A [`VerdictError`] when the command line is wrong or the run's data is missing, malformed,
/// oversized, linked, unexpected or does not match its plan.
pub fn run<W: Write>(args: &[OsString], out: &mut W) -> Result<bool, VerdictError> {
    let [ref word, ref settings, ref artifacts, ref jobs] = *args else {
        return Err(VerdictError(Fault::Usage));
    };
    if word != "verdict" {
        return Err(VerdictError(Fault::Usage));
    }
    let judgement =
        judge(Path::new(settings), Path::new(artifacts), Path::new(jobs)).map_err(VerdictError)?;
    for line in &judgement.lines {
        writeln!(out, "{line}").map_err(|cause| VerdictError(Fault::Output(cause)))?;
    }
    out.flush()
        .map_err(|cause| VerdictError(Fault::Output(cause)))?;
    Ok(judgement.passed)
}

/// Requires every planned shard job on both architectures, each ending success, and no other
/// shard job; with no shard, every shard job is skipped.
fn check_shard_jobs(jobs: &[Job], shards: usize) -> Result<(), Fault> {
    let want = if shards == 0 { "skipped" } else { "success" };
    let mut names: Vec<&str> = Vec::new();
    for job in jobs.iter().filter(|job| job.name.starts_with("shard ")) {
        if job.conclusion.as_deref() != Some(want) {
            return Err(Fault::ShardJobs { shards });
        }
        names.push(&job.name);
    }
    if shards == 0 {
        return Ok(());
    }
    names.sort_unstable();
    let mut expected: Vec<String> = ARCHES
        .iter()
        .flat_map(|arch| (0..shards).map(move |shard| format!("shard {arch} {shard}/{shards}")))
        .collect();
    expected.sort_unstable();
    if names != expected {
        return Err(Fault::ShardJobs { shards });
    }
    Ok(())
}

/// Judges the run whose settings, artifacts and jobs the command line names.
fn judge(settings: &Path, root: &Path, jobs_path: &Path) -> Result<Judgement, Fault> {
    let text = String::from_utf8_lossy(&read::read_path(settings)?).into_owned();
    let settings = Settings::parse(&text)?;
    let jobs = read::jobs(jobs_path)?;
    let mut plans = jobs.iter().filter(|job| job.name == "plan");
    match (plans.next(), plans.next()) {
        (Some(job), None) if job.conclusion.as_deref() == Some("success") => {}
        (Some(_) | None, _) => return Err(Fault::PlanJob),
    }
    let limit = settings.max_shards.saturating_mul(2).saturating_add(2);
    let kinds = read::artifacts(root, limit)?;
    let planned = read::plan(root)?;
    let count = planned.len();
    if count > settings.capacity() {
        return Err(Fault::Capacity {
            count,
            capacity: settings.capacity(),
        });
    }
    let shards = settings.shards(count);
    check_shard_jobs(&jobs, shards)?;
    let present: BTreeSet<(&str, usize)> = kinds
        .iter()
        .filter_map(|kind| match *kind {
            Kind::Outcomes { arch, shard } => Some((arch, shard)),
            Kind::Plan | Kind::Diff => None,
        })
        .collect();
    let every = ARCHES
        .iter()
        .all(|&arch| (0..shards).all(|shard| present.contains(&(arch, shard))));
    if present.len() != shards.saturating_mul(2) || !every {
        return Err(Fault::OutcomeArtifacts {
            found: present.len(),
            shards,
        });
    }
    let mut lines = vec![format!(
        "planned {count} mutants in {shards} shard(s) on each of 2 architectures"
    )];
    if shards == 0 {
        lines.push("verdict: pass: no mutants on the changed lines".to_owned());
        return Ok(Judgement {
            lines,
            passed: true,
        });
    }
    let mut judged: [Vec<(Mutant, Outcome)>; 2] = [Vec::new(), Vec::new()];
    for (all, arch) in judged.iter_mut().zip(ARCHES) {
        for shard in 0..shards {
            all.extend(read::shard_outcomes(root, arch, shard)?);
        }
        all.sort();
        let same = all.len() == planned.len()
            && all
                .iter()
                .zip(&planned)
                .all(|(judged, plan)| judged.0 == *plan);
        if !same {
            return Err(Fault::Mismatch { arch });
        }
    }
    Ok(score(settings, &judged, lines))
}

/// Holds each architecture to the floor on its own and the two to each other.
fn score(
    settings: Settings,
    judged: &[Vec<(Mutant, Outcome)>; 2],
    mut lines: Vec<String>,
) -> Judgement {
    let mut passed = true;
    let mut tallies = Vec::new();
    for (arch, outcomes) in ARCHES.iter().zip(judged) {
        let count = |want: Outcome| {
            outcomes
                .iter()
                .filter(|&&(_, outcome)| outcome == want)
                .count()
        };
        let (caught, missed) = (count(Outcome::Caught), count(Outcome::Missed));
        let viable = caught.saturating_add(missed);
        lines.push(format!(
            "{arch}: viable {viable} = caught {caught} + missed {missed}; left out of the denominator: unviable {}, timeout {}",
            count(Outcome::Unviable),
            count(Outcome::Timeout)
        ));
        for missed_one in outcomes
            .iter()
            .filter(|&&(_, outcome)| outcome == Outcome::Missed)
        {
            let mutant = &missed_one.0;
            lines.push(format!(
                "{arch} missed {}:{}: {}",
                mutant.file, mutant.line, mutant.replacement
            ));
        }
        let floor = settings.floor;
        if viable > 0 && caught.saturating_mul(100) < floor.saturating_mul(viable) {
            lines.push(format!(
                "::error::{arch}: mutation score under the {floor}% floor: {missed} missed mutant(s) listed above"
            ));
            passed = false;
        }
        tallies.push(format!("{arch} caught {caught} of {viable} viable"));
    }
    if !agree(judged, &mut lines) {
        passed = false;
    }
    let word = if passed { "pass" } else { "fail" };
    lines.push(format!(
        "verdict: {word}: {}, floor {}%",
        tallies.join(", "),
        settings.floor
    ));
    Judgement { lines, passed }
}

/// Parity: one plan, so each mutant has one outcome per architecture, in the same sorted
/// place. Caught on one and missed or unviable on the other is a test or a build that depends
/// on the machine; a timeout on one side is load, not code, and is only reported. Returns
/// whether the two agree.
fn agree(judged: &[Vec<(Mutant, Outcome)>; 2], lines: &mut Vec<String>) -> bool {
    let [ref left, ref right] = *judged;
    let mut apart = 0_usize;
    for (&(ref mutant, one), &(_, other)) in left.iter().zip(right) {
        if one == other {
            continue;
        }
        lines.push(format!(
            "apart {}:{}: {}: x86_64 {}, aarch64 {}",
            mutant.file,
            mutant.line,
            mutant.replacement,
            one.label(),
            other.label()
        ));
        if one != Outcome::Timeout && other != Outcome::Timeout {
            apart = apart.saturating_add(1);
        }
    }
    if apart > 0 {
        lines.push(format!(
            "::error::{apart} mutant(s) are judged apart by x86_64 and aarch64, listed above: an outcome depends on the machine"
        ));
    }
    apart == 0
}
