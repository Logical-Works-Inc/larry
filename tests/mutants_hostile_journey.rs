//! Hostile shapes and boundaries for the mutation lane's verdict (larry#565, Director
//! 2026-11-27): malformed and hostile `jobs.json`, plan and outcomes files, the job and
//! capacity bounds at one past each limit, hostile artifact trees and names, unreadable
//! settings and directories, a bare settings name, the typed cause under each refusal, and
//! an unwritable report. The generated-fault suite
//! lives in `tests/mutants_verdict_journey.rs`; both start from the honest tree
//! `tests/support/mutants_case.rs` writes. Source: CWE-20, CWE-59 and CWE-400;
//! cargo-mutants 27.1.0 shapes; GitHub's `workflow_run` jobs API. Requirement: every
//! shape the lane does not know fails closed, and each file ends at 95% line coverage or
//! more (ADR 0098, larry#387).

#[path = "support/mutants_case.rs"]
mod mutants_case;
#[path = "support/temp_dir.rs"]
mod temp_dir;

use std::error::Error as _;
use std::ffi::OsString;
use std::fs;
use std::io::{ErrorKind, Write};
use std::num::{IntErrorKind, ParseIntError};
use std::path::{Path, PathBuf};
use std::process::Output;

use larry_demo::mutants::run;
use lgwks_std::json;
use mutants_case::{
    Check, Run, child, judge, link, mutant, no_mutants, published, read, write_honest,
};
use temp_dir::TempDir;

/// A run of `count` mutants every architecture scores as `summary`, sized as the lane sizes it.
fn sized_run(count: usize, per_shard: usize, max_shards: usize, summary: &'static str) -> Run {
    Run {
        mutants: (0..count).map(mutant).collect(),
        x86: vec![summary; count],
        arm: vec![summary; count],
        per: count.div_ceil(max_shards).max(per_shard),
    }
}

/// Judges a fresh tree of `run` with the file at `relative` replaced by `text`.
fn judge_with(run: &Run, settings: &str, relative: &str, text: &str) -> Check<Output> {
    let dir = TempDir::new()?;
    let path = write_honest(&dir, run, settings)?;
    fs::write(dir.join(relative), text)?;
    judge(&path, &dir)
}

/// The refusal `output` prints names `words` and fails the lane closed.
fn refused(output: &Output, words: &str, what: &str) -> Check {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.code() == Some(1) && stderr.contains(words) && stderr.contains("(fail closed)")
    {
        Ok(())
    } else {
        Err(format!(
            "{what} names {words:?}: exit {:?}: {stderr}",
            output.status.code()
        )
        .into())
    }
}

/// Hostile data files: every `jobs.json`, plan and outcomes shape the lane does not know is
/// refused by name and fails closed (CWE-20; cargo-mutants 27.1.0 shapes; GitHub's
/// `workflow_run` jobs API; ADR 0098, larry#387).
#[test]
fn hostile_data_files_are_refused_by_shape() -> Check {
    let settings = read(".github/workflows/mutants.yml")?;
    let per_shard = published(&settings, "MUTANTS_PER_SHARD")?;
    let max_shards = published(&settings, "MUTANTS_MAX_SHARDS")?;
    let empty = no_mutants();
    let one = sized_run(1, per_shard, max_shards, "CaughtMutant");
    let missing = format!("[{}]", mutant(0).replace("\"replacement\":\"true\",", ""));
    let no_summary = format!(
        "{{\"outcomes\": [{{\"scenario\": {{\"Mutant\": {}}}, \"log_path\": \"m.log\"}}]}}",
        mutant(0)
    );
    let weird = format!(
        "{{\"outcomes\": [{{\"scenario\": {{\"Mutant\": {}}}, \"summary\": \"Weird\", \"log_path\": \"m.log\"}}]}}",
        mutant(0)
    );
    let job_shape = "holds a job without a name or a text conclusion";
    let plan_shape = "is not an array of mutants with a file, line, replacement and name";
    let file_shape = "is not a cargo-mutants outcomes file";
    let scenario_shape = "holds a scenario that is neither the baseline nor a mutant";
    let shard = "artifacts/mutants-outcomes-x86_64-0/outcomes.json";
    let cases: Vec<(&Run, &str, String, &str)> = vec![
        (&empty, "jobs.json", "{\"plan\": []}".to_owned(), "is not an array of jobs"),
        (&empty, "jobs.json", "\"plan\"".to_owned(), "is not an array of jobs"),
        (&empty, "jobs.json", "[{\"conclusion\":\"success\"}]".to_owned(), job_shape),
        (&empty, "jobs.json", "[{\"name\":\"plan\"}]".to_owned(), "the plan job did not end success"),
        (&empty, "jobs.json", "[{\"name\":\"plan\",\"conclusion\":7}]".to_owned(), job_shape),
        (&empty, "jobs.json", "[{\"name\":7,\"conclusion\":\"success\"}]".to_owned(), job_shape),
        (&empty, "jobs.json", "[{\"name\":\"plan\",\"conclusion\":null}]".to_owned(), "the plan job did not end success"),
        (&empty, "jobs.json", "[{\"name\":\"plan\",\"conclusion\":\"success\"},{\"name\":\"plan\",\"conclusion\":\"success\"}]".to_owned(), "the plan job did not end success"),
        (&empty, "jobs.json", "[{\"name\":\"warm-aarch64\",\"conclusion\":\"skipped\"}]".to_owned(), "the plan job did not end success"),
        (&one, "artifacts/mutants-plan/list.json", "{}".to_owned(), plan_shape),
        (&one, "artifacts/mutants-plan/list.json", "\"[]\"".to_owned(), plan_shape),
        (&one, "artifacts/mutants-plan/list.json", missing, plan_shape),
        (&one, "artifacts/mutants-plan/list.json", "[7]".to_owned(), plan_shape),
        (&one, shard, "{}".to_owned(), file_shape),
        (&one, shard, "[]".to_owned(), file_shape),
        (&one, shard, "{\"outcomes\": {}}".to_owned(), file_shape),
        (&one, shard, "{\"outcomes\": [".to_owned(), "is not JSON"),
        (&one, shard, "{\"outcomes\": [{\"summary\": \"CaughtMutant\"}]}".to_owned(), scenario_shape),
        (&one, shard, "{\"outcomes\": [{\"scenario\": 7, \"summary\": \"CaughtMutant\"}]}".to_owned(), scenario_shape),
        (&one, shard, "{\"outcomes\": [{\"scenario\": null, \"summary\": \"CaughtMutant\"}]}".to_owned(), scenario_shape),
        (&one, shard, no_summary, "holds a mutant with no summary"),
        (&one, shard, weird, "is not one this verdict knows"),
    ];
    for (run, relative, text, words) in cases {
        refused(
            &judge_with(run, &settings, relative, &text)?,
            words,
            &format!("{relative} {text:?}"),
        )?;
    }
    Ok(())
}

/// Bounds bind the job list at one past the run's job limit: 512 jobs are admitted and 513
/// are refused (CWE-400; ADR 0066, larry#310).
#[test]
fn hostile_job_bounds_bind_at_one_past_each_limit() -> Check {
    let settings = read(".github/workflows/mutants.yml")?;
    let mut entries = vec![
        "{\"name\":\"plan\",\"conclusion\":\"success\"}".to_owned(),
        "{\"name\":\"warm-aarch64\",\"conclusion\":\"skipped\"}".to_owned(),
    ];
    for index in 0..510 {
        entries.push(format!(
            "{{\"name\":\"noise-{index}\",\"conclusion\":\"success\"}}"
        ));
    }
    let dir = TempDir::new()?;
    let path = write_honest(&dir, &no_mutants(), &settings)?;
    fs::write(dir.join("jobs.json"), format!("[{}]", entries.join(",")))?;
    let output = judge(&path, &dir)?;
    assert!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout)
                .contains("verdict: pass: no mutants on the changed lines"),
        "512 jobs are admitted"
    );
    entries.push("{\"name\":\"noise-past\",\"conclusion\":\"success\"}".to_owned());
    fs::write(dir.join("jobs.json"), format!("[{}]", entries.join(",")))?;
    refused(
        &judge(&path, &dir)?,
        "lists more jobs than one run holds",
        "513 jobs",
    )?;
    Ok(())
}

/// Bounds bind the plan at one past each cap: 288 mutants in 96 shards pass with their exact
/// tally and 289 are refused (CWE-400; ADR 0066, larry#310).
#[test]
fn hostile_plan_capacity_binds_at_one_past_each_limit() -> Check {
    let settings = read(".github/workflows/mutants.yml")?;
    let per_shard = published(&settings, "MUTANTS_PER_SHARD")?;
    let max_shards = published(&settings, "MUTANTS_MAX_SHARDS")?;
    let dir = TempDir::new()?;
    let full = sized_run(288, per_shard, max_shards, "CaughtMutant");
    let path = write_honest(&dir, &full, &settings)?;
    let output = judge(&path, &dir)?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success()
            && stdout.contains("planned 288 mutants in 96 shard(s) on each of 2 architectures")
            && stdout.contains("x86_64: viable 288 = caught 288 + missed 0; left out of the denominator: unviable 0, timeout 0")
            && stdout.lines().last().is_some_and(|line| line == "verdict: pass: x86_64 caught 288 of 288 viable, aarch64 caught 288 of 288 viable, floor 60%"),
        "288 mutants pass with their exact tally: {stdout}"
    );
    let dir = TempDir::new()?;
    let past = sized_run(289, per_shard, max_shards, "CaughtMutant");
    let path = write_honest(&dir, &past, &settings)?;
    refused(
        &judge(&path, &dir)?,
        "289 mutants exceed the default branch's capacity of 288: split the pull request",
        "289 mutants",
    )?;
    Ok(())
}

/// Scores pass or fail with their exact reports: an empty denominator passes with its zero
/// tally, a cut below the floor fails, and a run the architectures judge apart fails naming
/// each mutant (CWE-20; ADR 0098, larry#387).
#[test]
fn hostile_scores_pass_or_fail_with_their_exact_reports() -> Check {
    let settings = read(".github/workflows/mutants.yml")?;
    let per_shard = published(&settings, "MUTANTS_PER_SHARD")?;
    let max_shards = published(&settings, "MUTANTS_MAX_SHARDS")?;
    let dir = TempDir::new()?;
    let none_viable = sized_run(2, per_shard, max_shards, "Unviable");
    let path = write_honest(&dir, &none_viable, &settings)?;
    let output = judge(&path, &dir)?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success()
            && stdout.contains("x86_64: viable 0 = caught 0 + missed 0; left out of the denominator: unviable 2, timeout 0")
            && stdout.lines().last().is_some_and(|line| line == "verdict: pass: x86_64 caught 0 of 0 viable, aarch64 caught 0 of 0 viable, floor 60%"),
        "an unviable run passes with its zero tally: {stdout}"
    );
    // One missed mutant of one viable misses the floor and exits 2 with its exact report.
    let dir = TempDir::new()?;
    let weak = Run {
        mutants: vec![mutant(0), mutant(1)],
        x86: vec!["MissedMutant", "Timeout"],
        arm: vec!["MissedMutant", "Timeout"],
        per: 2_usize.div_ceil(max_shards).max(per_shard),
    };
    let path = write_honest(&dir, &weak, &settings)?;
    let output = judge(&path, &dir)?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.code() == Some(2)
            && stdout.contains("x86_64: viable 1 = caught 0 + missed 1; left out of the denominator: unviable 0, timeout 1")
            && stdout.contains("mutation score under the 60% floor")
            && stdout.lines().last().is_some_and(|line| line
                == "verdict: fail: x86_64 caught 0 of 1 viable, aarch64 caught 0 of 1 viable, floor 60%"),
        "a run under the floor fails with its exact report: {stdout}"
    );
    // Mutants the architectures judge apart fail with the apart report naming each one.
    let dir = TempDir::new()?;
    let split = Run {
        mutants: (0..4).map(mutant).collect(),
        x86: vec!["Unviable", "CaughtMutant", "Timeout", "CaughtMutant"],
        arm: vec![
            "MissedMutant",
            "CaughtMutant",
            "CaughtMutant",
            "CaughtMutant",
        ],
        per: 4_usize.div_ceil(max_shards).max(per_shard),
    };
    let path = write_honest(&dir, &split, &settings)?;
    let output = judge(&path, &dir)?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.code() == Some(2)
            && stdout.contains("x86_64 Unviable, aarch64 MissedMutant")
            && stdout.contains("x86_64 Timeout, aarch64 CaughtMutant")
            && stdout.contains("1 mutant(s) are judged apart"),
        "a run judged apart fails naming each mutant: {stdout}"
    );
    Ok(())
}

/// Hostile artifact trees and names: a missing root, a file where a directory belongs, an
/// artifact that holds no file or two, an outcomes file that is a directory, and hostile
/// artifact names (CWE-59, CWE-20; ADR 0098, larry#387).
#[test]
fn hostile_artifact_trees_are_refused() -> Check {
    let settings = read(".github/workflows/mutants.yml")?;
    let fresh = || -> Check<(TempDir, PathBuf)> {
        let dir = TempDir::new()?;
        let path = write_honest(&dir, &no_mutants(), &settings)?;
        Ok((dir, path))
    };
    let (dir, path) = fresh()?;
    fs::remove_dir_all(dir.join("artifacts"))?;
    refused(
        &judge(&path, &dir)?,
        "cannot be listed",
        "a missing artifacts root",
    )?;
    let (dir, path) = fresh()?;
    fs::remove_dir_all(dir.join("artifacts"))?;
    fs::write(dir.join("artifacts"), "not a directory")?;
    refused(
        &judge(&path, &dir)?,
        "is not a directory of artifacts",
        "a file artifacts root",
    )?;
    let (dir, path) = fresh()?;
    fs::remove_dir_all(dir.join("artifacts/mutants-diff"))?;
    fs::write(dir.join("artifacts/mutants-diff"), "not a directory")?;
    refused(
        &judge(&path, &dir)?,
        "the artifact mutants-diff is not a directory",
        "a file artifact",
    )?;
    for held in [Vec::new(), vec!["outcomes.json", "notes.txt"]] {
        let (dir, path) = fresh()?;
        let folder = dir.join("artifacts/mutants-outcomes-x86_64-0");
        fs::create_dir_all(&folder)?;
        for name in &held {
            fs::write(folder.join(name), "{}")?;
        }
        refused(
            &judge(&path, &dir)?,
            "holds more or other than its one file",
            &format!("an artifact holding {held:?}"),
        )?;
    }
    let per_shard = published(&settings, "MUTANTS_PER_SHARD")?;
    let max_shards = published(&settings, "MUTANTS_MAX_SHARDS")?;
    let dir = TempDir::new()?;
    let one = sized_run(1, per_shard, max_shards, "CaughtMutant");
    let path = write_honest(&dir, &one, &settings)?;
    let held = dir.join("artifacts/mutants-outcomes-x86_64-0/outcomes.json");
    fs::remove_file(&held)?;
    fs::create_dir(&held)?;
    refused(
        &judge(&path, &dir)?,
        "holds a file that is not a regular file",
        "a directory outcomes file",
    )?;
    for name in [
        "mutants-outcomes-x86-",
        "mutants-outcomes-ppc64-0",
        "mutants-outcomes-x86_64-1a",
        "mutants-outcomes-x86_64-00",
        "mutants-outcomes-X86_64-0",
    ] {
        let (dir, path) = fresh()?;
        fs::create_dir_all(dir.join("artifacts").join(name))?;
        refused(
            &judge(&path, &dir)?,
            "is not one this lane uploads",
            &format!("artifact {name:?}"),
        )?;
    }
    Ok(())
}

/// Hostile settings files: a settings file that is missing, a directory or a link, under
/// a file, oversized, or publishing a setting on no line (CWE-59, CWE-20 and CWE-400;
/// ADR 0098, larry#387).
#[test]
fn hostile_settings_files_are_refused() -> Check {
    let settings = read(".github/workflows/mutants.yml")?;
    let fresh = || -> Check<(TempDir, PathBuf)> {
        let dir = TempDir::new()?;
        let path = write_honest(&dir, &no_mutants(), &settings)?;
        Ok((dir, path))
    };
    let (dir, _) = fresh()?;
    refused(
        &judge(&dir.join("absent.yml"), &dir)?,
        "is missing",
        "a missing settings file",
    )?;
    refused(
        &judge(&dir, &dir)?,
        "is not a regular file",
        "a directory settings file",
    )?;
    let (dir, path) = fresh()?;
    let real = dir.join("real-settings.yml");
    fs::rename(&path, &real)?;
    link(&real, &path)?;
    refused(
        &judge(&path, &dir)?,
        "is a symbolic link",
        "a linked settings file",
    )?;
    refused(
        &judge(&dir.join("jobs.json").join("settings.yml"), &dir)?,
        "cannot be read",
        "a settings file under a file",
    )?;
    let (dir, path) = fresh()?;
    fs::write(
        &path,
        format!("MUTANTS_FLOOR: 60\n{}", " ".repeat(4_194_305)),
    )?;
    refused(
        &judge(&path, &dir)?,
        "past the bound of 4194304",
        "an oversize settings file",
    )?;
    let broken = settings.replacen("MUTANTS_FLOOR: 60", "MUTANTS_FLOOR 60", 1);
    let dir = TempDir::new()?;
    let path = write_honest(&dir, &no_mutants(), &broken)?;
    refused(
        &judge(&path, &dir)?,
        "the default branch publishes MUTANTS_FLOOR on 0 lines, not one",
        "a setting with no colon",
    )?;
    Ok(())
}

/// Hostile shard jobs and outcome counts: one shard job failing and one missing are not
/// each planned shard on both architectures ending success, and one planned mutant with no
/// outcome on one architecture is not exactly one per plan (CWE-20; ADR 0098, larry#387).
#[test]
fn hostile_shard_jobs_and_outcome_counts_are_refused() -> Check {
    let settings = read(".github/workflows/mutants.yml")?;
    let per_shard = published(&settings, "MUTANTS_PER_SHARD")?;
    let max_shards = published(&settings, "MUTANTS_MAX_SHARDS")?;
    let dir = TempDir::new()?;
    let path = write_honest(&dir, &no_mutants(), &settings)?;
    fs::remove_file(dir.join("jobs.json"))?;
    refused(&judge(&path, &dir)?, "is missing", "a missing jobs file")?;
    // One shard job failing and one shard job missing are not each planned shard on both
    // architectures ending success.
    let dir = TempDir::new()?;
    let one = sized_run(1, per_shard, max_shards, "CaughtMutant");
    let path = write_honest(&dir, &one, &settings)?;
    let honest = fs::read_to_string(dir.join("jobs.json"))?;
    let failed = honest.replacen(
        "shard x86_64 0/1\",\"conclusion\":\"success",
        "shard x86_64 0/1\",\"conclusion\":\"failure",
        1,
    );
    fs::write(dir.join("jobs.json"), failed)?;
    refused(
        &judge(&path, &dir)?,
        "the shard jobs are not",
        "a failed shard job",
    )?;
    let dropped = honest.replacen(
        ",{\"name\":\"shard aarch64 0/1\",\"conclusion\":\"success\"}",
        "",
        1,
    );
    fs::write(dir.join("jobs.json"), dropped)?;
    refused(
        &judge(&path, &dir)?,
        "the shard jobs are not",
        "a missing shard job",
    )?;
    // One planned mutant with no outcome on one architecture is not exactly one per plan.
    let dir = TempDir::new()?;
    let two = sized_run(2, per_shard, max_shards, "CaughtMutant");
    let path = write_honest(&dir, &two, &settings)?;
    let single = format!(
        "{{\"cargo_mutants_version\":\"27.1.0\",\"outcomes\":[{{\"scenario\":\"Baseline\",\"summary\":\"Success\"}},{{\"scenario\":{{\"Mutant\":{}}},\"summary\":\"CaughtMutant\",\"log_path\":\"m.log\"}}],\"total_mutants\":1}}",
        mutant(0)
    );
    fs::write(
        dir.join("artifacts/mutants-outcomes-x86_64-0/outcomes.json"),
        single,
    )?;
    refused(
        &judge(&path, &dir)?,
        "not exactly one per planned mutant",
        "a short outcomes file",
    )?;
    Ok(())
}

/// A settings name with no parent reads beside the working tree; a path that names no
/// file is usage, never a read (CWE-20; ADR 0098, larry#387).
#[test]
fn a_bare_settings_name_reads_beside_the_working_tree_and_a_path_with_no_file_is_usage() -> Check {
    let settings = read(".github/workflows/mutants.yml")?;
    let dir = TempDir::new()?;
    write_honest(&dir, &no_mutants(), &settings)?;
    let run = |name: &str| -> Check<Output> {
        Ok(child(&[
            "mutants".as_ref(),
            "verdict".as_ref(),
            name.as_ref(),
            "artifacts".as_ref(),
            "jobs.json".as_ref(),
        ])
        .current_dir(dir.path())
        .output()?)
    };
    let output = run("settings.yml")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success()
            && stdout.contains("verdict: pass: no mutants on the changed lines"),
        "a bare settings name reads beside the working tree: {stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = run("..")?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.code() == Some(1) && stderr.contains("usage: larry mutants verdict"),
        ".. as the settings is refused as usage: {stderr}"
    );
    Ok(())
}

/// What sits under a refusal on its error chain.
#[derive(Debug, PartialEq, Eq)]
enum Cause {
    /// The operating system or the writer refused.
    Io(ErrorKind),
    /// The JSON parser refused.
    Json,
    /// A setting's digits did not parse.
    Number(IntErrorKind),
    /// The verdict's own rule refused: nothing sits under it.
    Own,
}

/// A report writer that refuses its writes or its flush, as a closed pipe or a full disk does.
struct Failing {
    /// What every write is refused with.
    write: Option<ErrorKind>,
    /// What the flush is refused with.
    flush: Option<ErrorKind>,
}

impl Write for Failing {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        match self.write {
            Some(kind) => Err(kind.into()),
            None => Ok(bytes.len()),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self.flush {
            Some(kind) => Err(kind.into()),
            None => Ok(()),
        }
    }
}

/// The library's verdict command over the tree under `dir`.
fn command(dir: &Path, settings: &Path) -> [OsString; 4] {
    [
        OsString::from("verdict"),
        settings.into(),
        dir.join("artifacts").into(),
        dir.join("jobs.json").into(),
    ]
}

/// Requires the library's verdict on the tree under `dir` to refuse naming `words`, with
/// `cause` under the refusal.
fn refuses_with<W: Write>(
    dir: &Path,
    settings: &Path,
    out: &mut W,
    words: &str,
    cause: &Cause,
) -> Check {
    let Err(refusal) = run(&command(dir, settings), out) else {
        return Err(format!("{words:?}: the run was judged").into());
    };
    let fault = refusal.source().ok_or("a refusal with no fault under it")?;
    let found = match fault.source() {
        None => Cause::Own,
        Some(under) => {
            if let Some(io) = under.downcast_ref::<std::io::Error>() {
                Cause::Io(io.kind())
            } else if let Some(number) = under.downcast_ref::<ParseIntError>() {
                Cause::Number(*number.kind())
            } else if under.is::<json::Error>() {
                Cause::Json
            } else {
                return Err(format!("{words:?}: an unknown cause {under}").into());
            }
        }
    };
    assert_eq!(&found, cause, "{words:?}: {refusal}");
    assert!(
        fault.to_string().contains(words),
        "the fault names {words:?}: {fault}"
    );
    assert_eq!(
        refusal.to_string(),
        format!("larry mutants verdict: {fault} (fail closed)"),
        "the refusal wraps its fault and says it fails closed"
    );
    Ok(())
}

/// Every refusal keeps its typed cause on the error chain, so a caller tells a missing file
/// from a broken one and from a full disk (CWE-391; ADR 0098, larry#387). A report that
/// cannot be written or flushed refuses a run that would otherwise pass.
#[test]
fn each_refusal_keeps_its_typed_cause_on_the_chain() -> Check {
    let settings = read(".github/workflows/mutants.yml")?;
    let dir = TempDir::new()?;
    let path = write_honest(&dir, &no_mutants(), &settings)?;
    let root = dir.path();

    let mut report = Vec::new();
    assert!(
        run(&command(root, &path), &mut report)?,
        "the honest tree passes"
    );
    assert_eq!(
        String::from_utf8(report)?,
        "planned 0 mutants in 0 shard(s) on each of 2 architectures\n\
         verdict: pass: no mutants on the changed lines\n"
    );
    let words = "the report cannot be written";
    for (write, flush, kind) in [
        (Some(ErrorKind::BrokenPipe), None, ErrorKind::BrokenPipe),
        (None, Some(ErrorKind::StorageFull), ErrorKind::StorageFull),
    ] {
        let mut full = Failing { write, flush };
        refuses_with(root, &path, &mut full, words, &Cause::Io(kind))?;
    }

    let mut out = Vec::new();
    for (named, words, cause) in [
        (
            root.join("absent.yml"),
            "is missing",
            Cause::Io(ErrorKind::NotFound),
        ),
        (
            root.join("jobs.json").join("settings.yml"),
            "cannot be read",
            Cause::Io(ErrorKind::NotADirectory),
        ),
        (root.to_path_buf(), "is not a regular file", Cause::Own),
    ] {
        refuses_with(root, &named, &mut out, words, &cause)?;
    }
    let sixty = settings.replacen("MUTANTS_FLOOR: 60", "MUTANTS_FLOOR: sixty", 1);
    fs::write(&path, sixty)?;
    refuses_with(
        root,
        &path,
        &mut out,
        "MUTANTS_FLOOR is not a number",
        &Cause::Number(IntErrorKind::InvalidDigit),
    )?;
    fs::write(&path, &settings)?;
    fs::write(root.join("jobs.json"), "[")?;
    refuses_with(root, &path, &mut out, "is not JSON", &Cause::Json)?;
    assert!(out.is_empty(), "a refused run writes no report line");
    Ok(())
}

/// A directory the verdict may not list or search is refused naming the operating system's
/// reason, never read as an empty run (CWE-280; ADR 0098, larry#387). Needs a user the mode
/// bits bind, which every CI runner and build box user is.
#[cfg(unix)]
#[test]
fn an_unlistable_artifact_tree_is_refused_with_the_permission_cause() -> Check {
    use std::os::unix::fs::PermissionsExt as _;

    let settings = read(".github/workflows/mutants.yml")?;
    // The artifacts root unlistable, one artifact unlistable, one artifact unsearchable.
    for (relative, mode) in [
        ("artifacts", 0o000),
        ("artifacts/mutants-diff", 0o000),
        ("artifacts/mutants-diff", 0o600),
    ] {
        let dir = TempDir::new()?;
        let path = write_honest(&dir, &no_mutants(), &settings)?;
        let locked = dir.join(relative);
        fs::set_permissions(&locked, fs::Permissions::from_mode(mode))?;
        let mut out = Vec::new();
        let judged = refuses_with(
            dir.path(),
            &path,
            &mut out,
            "cannot be listed",
            &Cause::Io(ErrorKind::PermissionDenied),
        );
        // Restored before the verdict is read, so the directory can be removed either way.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700))?;
        judged.map_err(|cause| format!("{relative} at {mode:o}: {cause}"))?;
    }
    Ok(())
}

/// A report nobody can read is a typed refusal, never an unwritten pass (CWE-391; ADR 0098,
/// larry#387): the shipped binary writes into a pipe whose reader is already gone, so a run
/// that would pass exits 1 naming the report it could not write.
#[test]
fn an_unwritable_report_is_a_typed_refusal() -> Check {
    let settings = read(".github/workflows/mutants.yml")?;
    let per_shard = published(&settings, "MUTANTS_PER_SHARD")?;
    let max_shards = published(&settings, "MUTANTS_MAX_SHARDS")?;
    let dir = TempDir::new()?;
    let one = sized_run(1, per_shard, max_shards, "CaughtMutant");
    let path = write_honest(&dir, &one, &settings)?;
    let (reader, writer) = std::io::pipe()?;
    drop(reader);
    let artifacts = dir.join("artifacts");
    let jobs = dir.join("jobs.json");
    let output = child(&[
        "mutants".as_ref(),
        "verdict".as_ref(),
        path.as_os_str(),
        artifacts.as_os_str(),
        jobs.as_os_str(),
    ])
    .stdout(writer)
    .stderr(std::process::Stdio::piped())
    .spawn()?
    .wait_with_output()?;
    refused(
        &output,
        "the report cannot be written",
        "a closed report pipe",
    )
}
