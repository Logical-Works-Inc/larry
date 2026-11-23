//! The mutation lane's verdict runs from the default branch (larry#387, ADR 0098). Source: GitHub
//! Security Lab, "Keeping your GitHub Actions and workflows secure Part 1: Preventing pwn
//! requests" (2021); GitHub's `workflow_run` event, which runs the default branch's copy of the
//! workflow; CWE-829; cargo-mutants 27.1.0's `outcomes.json` and `--list --json` shapes.
//! Requirement: a pull request that edits `mutants.yml`'s verdict to `exit 0` is still judged by
//! main's `larry mutants verdict`; the judged run's artifacts and job list are data read with
//! fixed bounds, and a missing, malformed, oversized, linked or unexpected one fails closed.

#[path = "support/rng.rs"]
mod rng;
#[path = "support/temp_dir.rs"]
mod temp_dir;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rng::Rng;
use temp_dir::TempDir;

/// Failures propagate instead of panicking.
type Check<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The text of a file under the repository root.
fn read(relative: &str) -> Check<String> {
    Ok(fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(relative),
    )?)
}

/// The shipped binary with `args`, a cleared environment and nothing on stdin. Only the test
/// runner's own names pass: nextest's `NEXTEST_*` and the mutation gate's `LGWKS_MUTANT` and
/// `LGWKS_MUTANT_REACH`, so the child runs the mutant under test and traces it to this test.
fn larry(args: &[&std::ffi::OsStr]) -> Check<Output> {
    let runner = std::env::vars_os().filter(|pair| {
        let name = pair.0.to_string_lossy();
        name.starts_with("NEXTEST_") || name.starts_with("LGWKS_MUTANT")
    });
    Ok(Command::new(env!("CARGO_BIN_EXE_larry"))
        .args(args)
        .env_clear()
        .envs(runner)
        .output()?)
}

/// A symbolic link at `at` naming `target`, on each platform's own call.
#[cfg(unix)]
fn link(target: &Path, at: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, at)
}

/// A symbolic link at `at` naming `target`, on each platform's own call.
#[cfg(windows)]
fn link(target: &Path, at: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, at)
}

/// `larry mutants verdict` over the run written under `dir`.
fn judge(settings: &Path, dir: &Path) -> Check<Output> {
    let artifacts = dir.join("artifacts");
    let jobs = dir.join("jobs.json");
    larry(&[
        "mutants".as_ref(),
        "verdict".as_ref(),
        settings.as_os_str(),
        artifacts.as_os_str(),
        jobs.as_os_str(),
    ])
}

/// The verdict workflow runs on `workflow_run`, from the default branch's checkout, builds and
/// runs main's `larry`, reads no value a pull request names, and is the lane's one verdict.
#[test]
fn the_verdict_runs_from_the_default_branch_and_runs_nothing_of_the_pull_request() -> Check {
    let verdict = read(".github/workflows/mutants-verdict.yml")?;
    let lane = read(".github/workflows/mutants.yml")?;
    assert!(
        verdict.contains(
            "on:\n  workflow_run:\n    workflows: [mutants]\n    types: [requested, completed]\n"
        ),
        "the verdict follows every run of the lane, from the default branch's file"
    );
    assert!(
        verdict.contains("permissions:\n  actions: read\n  contents: read\n  statuses: write\n"),
        "the verdict reads the run and writes one commit status, nothing else"
    );
    for (name, text) in [("mutants.yml", &lane), ("mutants-verdict.yml", &verdict)] {
        assert!(
            !text.contains("pull_request_target"),
            "{name} never runs with the target repository's token over a pull request"
        );
    }
    assert!(
        !lane.contains("\n  verdict:\n"),
        "the lane holds no verdict job a pull request could edit to `exit 0`"
    );
    assert!(
        !verdict.contains("ref:") && verdict.contains("persist-credentials: false"),
        "the checkout is the default branch's, with no token left in it"
    );
    assert!(
        verdict.contains("run-id: ${{ github.event.workflow_run.id }}")
            && verdict.contains("cargo build --locked --bin larry")
            && verdict.contains(
                "target/debug/larry mutants verdict .github/workflows/mutants.yml artifacts jobs.json"
            ),
        "main's larry judges the run's artifacts and job list against main's settings"
    );
    assert!(
        verdict.contains("context=\"mutants / verdict\""),
        "the verdict is one commit status the pull request shows"
    );
    assert!(
        !verdict.contains("head_branch") && !verdict.contains("pull_requests"),
        "no name a pull request chooses is read"
    );
    for line in verdict.lines().filter(|line| line.contains("${{")) {
        let trimmed = line.trim_start().trim_start_matches("- ");
        let key = trimmed.split_once(": ${{").map(|(key, _)| key);
        assert!(
            key.is_some_and(|key| key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')),
            "an expression is only ever a key's value, never spliced into a script: {line}"
        );
    }
    assert!(
        lane.contains("name: mutants-plan") && lane.contains("path: list.json"),
        "the plan publishes its mutant list for the verdict"
    );
    Ok(())
}

/// One way a judged run can be wrong, or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// An honest run.
    None,
    /// One mutant caught on x86_64 and missed on aarch64.
    Apart,
    /// One mutant timed out on aarch64 alone: reported, never a failure by itself.
    TimeoutOneSide,
    /// aarch64 has no outcome for one planned mutant.
    DropOutcome,
    /// x86_64 judged a mutant the plan never listed.
    ForeignOutcome,
    /// One shard uploaded no outcomes.
    MissingArtifact,
    /// An artifact this lane never uploads.
    UnknownArtifact,
    /// An outcomes artifact holds a second file.
    ExtraFile,
    /// An outcomes file is cut short.
    Malformed,
    /// An outcomes file is past the 4 MiB bound.
    Oversize,
    /// An outcomes file is a link.
    Symlink,
    /// A scenario that is neither the baseline nor a mutant.
    ScenarioShape,
    /// One shard job failed.
    ShardFailed,
    /// One shard job never ran.
    ShardMissing,
    /// The plan job failed.
    PlanFailed,
    /// The plan's mutant list is cut short.
    PlanMalformed,
    /// A summary cargo-mutants never writes.
    UnknownSummary,
    /// The default branch publishes its floor twice.
    Settings,
}

/// Every fault, each taken by three seeds.
const FAULTS: [Fault; 18] = [
    Fault::None,
    Fault::Apart,
    Fault::TimeoutOneSide,
    Fault::DropOutcome,
    Fault::ForeignOutcome,
    Fault::MissingArtifact,
    Fault::UnknownArtifact,
    Fault::ExtraFile,
    Fault::Malformed,
    Fault::Oversize,
    Fault::Symlink,
    Fault::ScenarioShape,
    Fault::ShardFailed,
    Fault::ShardMissing,
    Fault::PlanFailed,
    Fault::PlanMalformed,
    Fault::UnknownSummary,
    Fault::Settings,
];

impl Fault {
    /// Whether the fault needs at least one shard to land on.
    const fn needs_outcomes(self) -> bool {
        !matches!(
            self,
            Self::None
                | Self::UnknownArtifact
                | Self::PlanFailed
                | Self::PlanMalformed
                | Self::Settings
        )
    }

    /// The words the refusal names, or `None` when the run is judged on its scores.
    const fn refusal(self) -> Option<&'static str> {
        match self {
            Self::None | Self::Apart | Self::TimeoutOneSide => None,
            Self::DropOutcome | Self::ForeignOutcome => Some("not exactly one per planned mutant"),
            Self::MissingArtifact => Some("outcomes artifacts for"),
            Self::UnknownArtifact => Some("is not one this lane uploads"),
            Self::ExtraFile => Some("holds more or other than its one file"),
            Self::Malformed | Self::PlanMalformed => Some("is not JSON"),
            Self::Oversize => Some("past the bound of 4194304"),
            Self::Symlink => Some("holds a file that is not a regular file"),
            Self::ScenarioShape => Some("neither the baseline nor a mutant"),
            Self::ShardFailed | Self::ShardMissing => Some("the shard jobs are not"),
            Self::PlanFailed => Some("the plan job did not end success"),
            Self::UnknownSummary => Some("is not one this verdict knows"),
            Self::Settings => Some("MUTANTS_FLOOR on 2 lines"),
        }
    }
}

/// A setting the lane publishes, read from its own file.
fn published(text: &str, name: &str) -> Check<usize> {
    let prefix = format!("{name}: ");
    let value = text
        .lines()
        .find_map(|line| line.trim_start().strip_prefix(prefix.as_str()))
        .ok_or(format!("mutants.yml publishes no {name}"))?;
    Ok(value.trim().parse()?)
}

/// One generated run: its mutants and each architecture's summaries, in plan order.
struct Run {
    /// The mutants' JSON, as cargo-mutants lists them.
    mutants: Vec<String>,
    /// x86_64's summary per mutant.
    x86: Vec<&'static str>,
    /// aarch64's summary per mutant.
    arm: Vec<&'static str>,
    /// Mutants per shard, as the plan job sizes it.
    per: usize,
}

/// The JSON cargo-mutants writes for mutant `index`.
fn mutant(index: usize) -> String {
    let line = index.saturating_add(10);
    let with = if index & 1 == 0 { "true" } else { "false" };
    format!(
        r#"{{"name":"src/m.rs:{line}:5: replace f -> bool with {with}","package":"larry-demo","file":"src/m.rs","function":{{"function_name":"f","return_type":"-> bool","span":{{"start":{{"line":1,"column":1}},"end":{{"line":99,"column":2}}}}}},"span":{{"start":{{"line":{line},"column":5}},"end":{{"line":{line},"column":9}}}},"replacement":"{with}","genre":"FnValue"}}"#
    )
}

/// A summary drawn six caught, two missed, one unviable and one timeout in ten.
fn summary(rng: &mut Rng) -> &'static str {
    match rng.below(10) {
        0..=5 => "CaughtMutant",
        6 | 7 => "MissedMutant",
        8 => "Unviable",
        _ => "Timeout",
    }
}

/// Sets the first summary of `side`, when it has one.
fn first(side: &mut [&'static str], summary: &'static str) {
    if let Some(slot) = side.first_mut() {
        *slot = summary;
    }
}

/// Draws a run of 0 to 12 mutants and applies the fault's change to its summaries.
fn generate(rng: &mut Rng, fault: Fault, per_shard: usize, max_shards: usize) -> Check<Run> {
    let mut count = rng.index(13)?;
    if fault.needs_outcomes() && count == 0 {
        count = 1;
    }
    let mut run = Run {
        mutants: (0..count).map(mutant).collect(),
        x86: Vec::new(),
        arm: Vec::new(),
        per: count.div_ceil(max_shards).max(per_shard),
    };
    for _ in 0..count {
        let drawn = summary(rng);
        run.x86.push(drawn);
        run.arm.push(drawn);
    }
    match fault {
        Fault::Apart => {
            first(&mut run.x86, "CaughtMutant");
            first(&mut run.arm, "MissedMutant");
        }
        Fault::TimeoutOneSide => first(&mut run.arm, "Timeout"),
        Fault::UnknownSummary => first(&mut run.x86, "Weird"),
        _ => {}
    }
    Ok(run)
}

/// The verdict the lane's rule gives an honest run: each architecture at the floor, and no
/// mutant the two judge apart unless one side timed out.
fn reference(run: &Run, floor: usize) -> bool {
    for side in [&run.x86, &run.arm] {
        let caught = side
            .iter()
            .filter(|&&drawn| drawn == "CaughtMutant")
            .count();
        let missed = side
            .iter()
            .filter(|&&drawn| drawn == "MissedMutant")
            .count();
        let viable = caught.saturating_add(missed);
        if viable > 0 && caught.saturating_mul(100) < floor.saturating_mul(viable) {
            return false;
        }
    }
    !run.x86
        .iter()
        .zip(&run.arm)
        .any(|(&left, &right)| left != right && left != "Timeout" && right != "Timeout")
}

/// One shard's outcomes file: the baseline, then each mutant it judged, then `extra`.
fn outcomes(run: &Run, side: &[&'static str], indices: &[usize], extra: Option<&str>) -> String {
    let mut entries = vec![r#"{"scenario":"Baseline","summary":"Success"}"#.to_owned()];
    for &index in indices {
        if let (Some(json), Some(drawn)) = (run.mutants.get(index), side.get(index)) {
            entries.push(format!(
                r#"{{"scenario":{{"Mutant":{json}}},"summary":"{drawn}","log_path":"log/m.log"}}"#
            ));
        }
    }
    entries.extend(extra.map(str::to_owned));
    format!(
        r#"{{"cargo_mutants_version":"27.1.0","outcomes":[{}],"total_mutants":{}}}"#,
        entries.join(","),
        indices.len()
    )
}

/// The judged run's job list, with the fault's job changes.
fn job_list(fault: Fault, shards: usize) -> String {
    let plan = if fault == Fault::PlanFailed {
        "failure"
    } else {
        "success"
    };
    let mut jobs = vec![
        format!(r#"{{"name":"plan","conclusion":"{plan}"}}"#),
        r#"{"name":"warm-aarch64","conclusion":"skipped"}"#.to_owned(),
    ];
    if shards == 0 {
        jobs.push(
            r#"{"name":"shard ${{ matrix.arch }} ${{ matrix.k }}/${{ matrix.n }}","conclusion":"skipped"}"#
                .to_owned(),
        );
    }
    for arch in ["x86_64", "aarch64"] {
        for shard in 0..shards {
            let last = shard.saturating_add(1) == shards;
            if fault == Fault::ShardMissing && arch == "x86_64" && last {
                continue;
            }
            let failed = fault == Fault::ShardFailed && arch == "aarch64" && shard == 0;
            let conclusion = if failed { "failure" } else { "success" };
            jobs.push(format!(
                r#"{{"name":"shard {arch} {shard}/{shards}","conclusion":"{conclusion}"}}"#
            ));
        }
    }
    format!("[{}]", jobs.join(","))
}

/// Writes one shard's outcomes artifact for `arch`, with the fault applied to x86_64's first.
fn write_shard(
    dir: &Path,
    run: &Run,
    fault: Fault,
    arch: &str,
    shard: usize,
    chunk: &[usize],
) -> Check {
    let x86 = arch == "x86_64";
    let side = if x86 { &run.x86 } else { &run.arm };
    let last_mutant = run.mutants.len().saturating_sub(1);
    let held: Vec<usize> = chunk
        .iter()
        .copied()
        .filter(|&index| !(fault == Fault::DropOutcome && !x86 && index == last_mutant))
        .collect();
    let here = x86 && shard == 0;
    let foreign = format!(
        r#"{{"scenario":{{"Mutant":{}}},"summary":"CaughtMutant"}}"#,
        mutant(9_000)
    );
    let extra = match fault {
        Fault::ForeignOutcome if here => Some(foreign.as_str()),
        Fault::ScenarioShape if here => {
            Some(r#"{"scenario":{"Other":{}},"summary":"CaughtMutant"}"#)
        }
        _ => None,
    };
    let mut text = outcomes(run, side, &held, extra);
    if fault == Fault::Malformed && here {
        text.truncate(text.len().saturating_sub(5));
    }
    if fault == Fault::Oversize && here {
        text.push_str(&" ".repeat(4_194_305));
    }
    let folder = dir.join(format!("artifacts/mutants-outcomes-{arch}-{shard}"));
    fs::create_dir_all(&folder)?;
    if fault == Fault::Symlink && here {
        fs::write(dir.join("elsewhere.json"), &text)?;
        link(&dir.join("elsewhere.json"), &folder.join("outcomes.json"))?;
    } else {
        fs::write(folder.join("outcomes.json"), &text)?;
    }
    if fault == Fault::ExtraFile && here {
        fs::write(folder.join("notes.txt"), "not an outcome")?;
    }
    Ok(())
}

/// Writes the run's artifacts, job list and settings under `dir`, with the fault applied, and
/// returns the settings file to judge against.
fn write(dir: &Path, run: &Run, fault: Fault, settings: &str) -> Check<PathBuf> {
    let artifacts = dir.join("artifacts");
    fs::create_dir_all(artifacts.join("mutants-plan"))?;
    let list = match fault {
        Fault::PlanMalformed => r#"[{"name":"#.to_owned(),
        _ if run.mutants.is_empty() => String::new(),
        _ => format!("[{}]", run.mutants.join(",")),
    };
    fs::write(artifacts.join("mutants-plan/list.json"), list)?;
    fs::create_dir_all(artifacts.join("mutants-diff"))?;
    fs::write(
        artifacts.join("mutants-diff/pr.diff"),
        "diff --git a/src/m.rs b/src/m.rs\n",
    )?;
    let indices: Vec<usize> = (0..run.mutants.len()).collect();
    let chunks: Vec<&[usize]> = indices.chunks(run.per.max(1)).collect();
    for arch in ["x86_64", "aarch64"] {
        for (shard, chunk) in chunks.iter().enumerate() {
            let last = shard.saturating_add(1) == chunks.len();
            if !(fault == Fault::MissingArtifact && arch == "aarch64" && last) {
                write_shard(dir, run, fault, arch, shard, chunk)?;
            }
        }
    }
    if fault == Fault::UnknownArtifact {
        fs::create_dir_all(artifacts.join("mutants-extra"))?;
        fs::write(artifacts.join("mutants-extra/list.json"), "[]")?;
    }
    fs::write(dir.join("jobs.json"), job_list(fault, chunks.len()))?;
    let path = dir.join("settings.yml");
    let text = if fault == Fault::Settings {
        format!("{settings}\n      MUTANTS_FLOOR: 0\n")
    } else {
        settings.to_owned()
    };
    fs::write(&path, text)?;
    Ok(path)
}

/// Every generated run, honest or carrying one fault, gets the verdict the lane's rule gives
/// it from the shipped binary: an honest run exits 0 at the floor and 2 under it or judged
/// apart, and every fault exits 1 naming what it refused (fail closed). Two passes over the
/// same seeds print the same trace.
#[test]
fn sim_every_generated_run_gets_the_reference_verdict() -> Check {
    let settings = read(".github/workflows/mutants.yml")?;
    let floor = published(&settings, "MUTANTS_FLOOR")?;
    let per_shard = published(&settings, "MUTANTS_PER_SHARD")?;
    let max_shards = published(&settings, "MUTANTS_MAX_SHARDS")?;
    let mut traces = Vec::new();
    for _pass in 0..2 {
        let mut trace = Vec::new();
        let faults = FAULTS.iter().cycle().take(FAULTS.len().saturating_mul(3));
        for (seed, &fault) in (0_u64..).zip(faults) {
            let mut rng = Rng(seed);
            let run = generate(&mut rng, fault, per_shard, max_shards)?;
            let dir = TempDir::new()?;
            let output = judge(&write(&dir, &run, fault, &settings)?, &dir)?;
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            let code = output.status.code();
            match fault.refusal() {
                Some(words) => assert!(
                    code == Some(1) && stderr.contains(words) && stderr.contains("(fail closed)"),
                    "seed {seed} {fault:?}: exit {code:?}, wanted 1 naming {words:?}: {stderr}{stdout}"
                ),
                None => {
                    let want = if reference(&run, floor) { 0 } else { 2 };
                    let last = stdout.lines().last();
                    assert!(
                        code == Some(want)
                            && last.is_some_and(|line| line.starts_with("verdict: ")),
                        "seed {seed} {fault:?}: exit {code:?}, wanted {want}: {stdout}{stderr}"
                    );
                }
            }
            let report = format!("{stdout}{stderr}").replace(&dir.display().to_string(), "<run>");
            trace.push(format!("{seed} {fault:?} {code:?} {report}"));
        }
        traces.push(trace);
    }
    assert!(
        traces.first() == traces.last(),
        "the same seeds give the same verdicts and reports"
    );
    Ok(())
}

/// A run whose plan lists no mutant.
const fn no_mutants() -> Run {
    Run {
        mutants: Vec::new(),
        x86: Vec::new(),
        arm: Vec::new(),
        per: 2,
    }
}

/// A run with no mutant on the changed lines passes and says so; the verdict refuses a command
/// line that is not `verdict <settings> <artifacts> <jobs.json>` before reading anything.
#[test]
fn no_mutants_pass_and_a_wrong_command_line_is_refused() -> Check {
    let settings = read(".github/workflows/mutants.yml")?;
    let dir = TempDir::new()?;
    let run = no_mutants();
    let output = judge(&write(&dir, &run, Fault::None, &settings)?, &dir)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success()
            && stdout.contains("verdict: pass: no mutants on the changed lines"),
        "zero mutants is a pass that says so: {stdout}"
    );
    let wrong: [&[&str]; 3] = [
        &["mutants"],
        &["mutants", "judge", "a", "b", "c"],
        &["mutants", "verdict", "a", "b"],
    ];
    for args in wrong {
        let words: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
        let output = larry(&words)?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.code() == Some(1) && stderr.contains("usage: larry mutants verdict"),
            "{args:?} is refused with the usage: {stderr}"
        );
    }
    Ok(())
}

/// The edges of a name and a setting: a shard number has one to six digits and no leading
/// zero, a `NAME:` line that is not a number still counts, so it is refused and never skipped,
/// and the floor is at most 100 with every cap at least 1.
#[test]
fn the_mutants_verdict_refuses_the_edges_of_a_name_and_a_setting() -> Check {
    let settings = read(".github/workflows/mutants.yml")?;
    let run = no_mutants();
    let edges = [
        (
            "mutants-outcomes-x86_64-999999",
            "",
            "",
            "1 outcomes artifacts for 0 shard(s)",
        ),
        (
            "mutants-outcomes-x86_64-1000000",
            "",
            "",
            "is not one this lane uploads",
        ),
        (
            "mutants-outcomes-aarch64-07",
            "",
            "",
            "is not one this lane uploads",
        ),
        (
            "",
            "MUTANTS_FLOOR: 60",
            "MUTANTS_FLOOR: 60\n      MUTANTS_FLOOR: sixty",
            "MUTANTS_FLOOR on 2 lines",
        ),
        (
            "",
            "MUTANTS_FLOOR: 60",
            "MUTANTS_FLOOR: sixty",
            "MUTANTS_FLOOR is not a number",
        ),
        (
            "",
            "MUTANTS_FLOOR: 60",
            "MUTANTS_FLOOR: 101",
            "floor is past 100 or a cap is zero",
        ),
        (
            "",
            "MUTANTS_PER_SHARD: 2",
            "MUTANTS_PER_SHARD: 0",
            "floor is past 100 or a cap is zero",
        ),
        (
            "",
            "MUTANTS_MAX_SHARDS: 96",
            "MUTANTS_MAX_SHARDS: 0",
            "floor is past 100 or a cap is zero",
        ),
        (
            "",
            "MUTANTS_SHARD_CAPACITY: 3",
            "MUTANTS_SHARD_CAPACITY: 0",
            "floor is past 100 or a cap is zero",
        ),
        // One shard at most: the plan, the diff and two outcomes fill the bound of four,
        // and a fifth artifact is one past it.
        (
            "mutants-outcomes-x86_64-0 mutants-outcomes-x86_64-1 mutants-outcomes-x86_64-2",
            "MUTANTS_MAX_SHARDS: 96",
            "MUTANTS_MAX_SHARDS: 1",
            "uploaded more than 4 artifacts",
        ),
        (
            "mutants-outcomes-x86_64-0 mutants-outcomes-x86_64-1",
            "MUTANTS_MAX_SHARDS: 96",
            "MUTANTS_MAX_SHARDS: 1",
            "2 outcomes artifacts for 0 shard(s)",
        ),
    ];
    for (artifacts, from, to, words) in edges {
        let dir = TempDir::new()?;
        let path = write(&dir, &run, Fault::None, &settings.replacen(from, to, 1))?;
        for artifact in artifacts.split_whitespace() {
            let shard = dir.join("artifacts").join(artifact);
            fs::create_dir_all(&shard)?;
            fs::write(shard.join("outcomes.json"), r#"{"outcomes":[]}"#)?;
        }
        let output = judge(&path, &dir)?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.code() == Some(1) && stderr.contains(words),
            "{artifacts}{to} is refused naming {words:?}: {stderr}"
        );
    }
    // The floor's own edge: 100 is a floor, not a refusal.
    let dir = TempDir::new()?;
    let full = settings.replacen("MUTANTS_FLOOR: 60", "MUTANTS_FLOOR: 100", 1);
    let output = judge(&write(&dir, &run, Fault::None, &full)?, &dir)?;
    assert!(
        output.status.success(),
        "a floor of 100% is allowed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
