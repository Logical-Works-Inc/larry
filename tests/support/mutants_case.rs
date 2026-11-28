//! The mutants verdict's shared case builders (larry#565): one generated or hostile run,
//! its artifacts, job list and settings, and the shipped binary that judges them. Both
//! `tests/mutants_verdict_journey.rs` (the generated-fault suite) and
//! `tests/mutants_hostile_journey.rs` (the hostile-shape and boundary suite) build every
//! case here, so the builders have one home. Source: GitHub's `workflow_run` jobs API and
//! cargo-mutants 27.1.0's `outcomes.json` and `--list --json` shapes; the lane's rule is
//! ADR 0098 (larry#387).

#[path = "coverage.rs"]
mod coverage;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Failures propagate instead of panicking.
pub(crate) type Check<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The text of a file under the repository root.
pub(crate) fn read(relative: &str) -> Check<String> {
    Ok(fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(relative),
    )?)
}

/// The shipped binary as `child`: cleared env, only the runner's own names pass, and the
/// coverage profile rides along through the one support helper (Director 2026-11-27).
pub(crate) fn child(args: &[&std::ffi::OsStr]) -> Command {
    let runner = std::env::vars_os().filter(|pair| {
        let name = pair.0.to_string_lossy();
        name.starts_with("NEXTEST_") || name.starts_with("LGWKS_MUTANT")
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_larry"));
    child.args(args).env_clear().envs(runner);
    coverage::forward(&mut child);
    child
}

/// The shipped binary with `args`, a cleared environment and nothing on stdin.
pub(crate) fn larry(args: &[&std::ffi::OsStr]) -> Check<Output> {
    Ok(child(args).output()?)
}

/// `larry mutants verdict` over the run written under `dir`.
pub(crate) fn judge(settings: &Path, dir: &Path) -> Check<Output> {
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

/// A setting the lane publishes, read from its own file.
pub(crate) fn published(text: &str, name: &str) -> Check<usize> {
    let prefix = format!("{name}: ");
    let value = text
        .lines()
        .find_map(|line| line.trim_start().strip_prefix(prefix.as_str()))
        .ok_or(format!("mutants.yml publishes no {name}"))?;
    Ok(value.trim().parse()?)
}

/// One generated run: its mutants and each architecture's summaries, in plan order.
pub(crate) struct Run {
    /// The mutants' JSON, as cargo-mutants lists them.
    pub(crate) mutants: Vec<String>,
    /// x86_64's summary per mutant.
    pub(crate) x86: Vec<&'static str>,
    /// aarch64's summary per mutant.
    pub(crate) arm: Vec<&'static str>,
    /// Mutants per shard, as the plan job sizes it.
    pub(crate) per: usize,
}

/// The JSON cargo-mutants writes for mutant `index`.
pub(crate) fn mutant(index: usize) -> String {
    let line = index.saturating_add(10);
    let with = if index & 1 == 0 { "true" } else { "false" };
    format!(
        r#"{{"name":"src/m.rs:{line}:5: replace f -> bool with {with}","package":"larry-demo","file":"src/m.rs","function":{{"function_name":"f","return_type":"-> bool","span":{{"start":{{"line":1,"column":1}},"end":{{"line":99,"column":2}}}}}},"span":{{"start":{{"line":{line},"column":5}},"end":{{"line":{line},"column":9}}}},"replacement":"{with}","genre":"FnValue"}}"#
    )
}

/// One shard's outcomes file: the baseline, then each mutant it judged, then `extra`.
pub(crate) fn outcomes(
    run: &Run,
    side: &[&'static str],
    indices: &[usize],
    extra: Option<&str>,
) -> String {
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

/// A run whose plan lists no mutant.
pub(crate) const fn no_mutants() -> Run {
    Run {
        mutants: Vec::new(),
        x86: Vec::new(),
        arm: Vec::new(),
        per: 2,
    }
}

/// A symbolic link at `at` naming `target`, on each platform's own call.
#[cfg(unix)]
pub(crate) fn link(target: &Path, at: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, at)
}

/// A symbolic link at `at` naming `target`, on each platform's own call.
#[cfg(windows)]
pub(crate) fn link(target: &Path, at: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, at)
}

/// The honest job list: the plan, the warm skipped matrix entry, and every shard job ending
/// success on both architectures.
fn jobs(shards: usize) -> String {
    let mut entries = vec![
        r#"{"name":"plan","conclusion":"success"}"#.to_owned(),
        r#"{"name":"warm-aarch64","conclusion":"skipped"}"#.to_owned(),
    ];
    if shards == 0 {
        entries.push(
            r#"{"name":"shard ${{ matrix.arch }} ${{ matrix.k }}/${{ matrix.n }}","conclusion":"skipped"}"#
                .to_owned(),
        );
    }
    for arch in ["x86_64", "aarch64"] {
        for shard in 0..shards {
            entries.push(format!(
                r#"{{"name":"shard {arch} {shard}/{shards}","conclusion":"success"}}"#
            ));
        }
    }
    format!("[{}]", entries.join(","))
}

/// Writes an honest run's artifacts, job list and settings under `dir`, and returns the
/// settings file to judge against. A fault is a change made to this tree afterwards.
pub(crate) fn write_honest(dir: &Path, run: &Run, settings: &str) -> Check<PathBuf> {
    let artifacts = dir.join("artifacts");
    fs::create_dir_all(artifacts.join("mutants-plan"))?;
    let list = if run.mutants.is_empty() {
        String::new()
    } else {
        format!("[{}]", run.mutants.join(","))
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
            let side = if arch == "x86_64" { &run.x86 } else { &run.arm };
            let folder = artifacts.join(format!("mutants-outcomes-{arch}-{shard}"));
            fs::create_dir_all(&folder)?;
            fs::write(
                folder.join("outcomes.json"),
                outcomes(run, side, chunk, None),
            )?;
        }
    }
    fs::write(dir.join("jobs.json"), jobs(chunks.len()))?;
    let path = dir.join("settings.yml");
    fs::write(&path, settings)?;
    Ok(path)
}
