//! Journeys for one request submitted twice (larry#182 defect 3), as an operator
//! lives them: a task document that names an `idempotency_key` is the client's
//! statement that a retry of it is the same request, so the shipped `larry exec`
//! answers the retry from what it recorded instead of running the task again.
//!
//! Source: draft-ietf-httpapi-idempotency-key-header-07 section 2 (a client key
//! is what lets a server recognize a retry), the AWS client-token contract (the
//! caller must reuse one token) and the Idempotent Receiver pattern (a server
//! that processed a request and then crashed receives the client's duplicate and
//! must answer from its record). Requirement proved: one keyed request runs its
//! acceptance commands once, both reports carry one run id and both verify; a
//! document with no key is a second request; a key that names nothing is refused
//! typed; and a run killed inside its command leaves evidence its keyed retry
//! answers from without running that command again. Each child runs with a
//! cleared environment.

#[path = "support/coverage.rs"]
mod coverage;
#[path = "support/json_at.rs"]
mod json_at;
use json_at::At;

#[path = "support/exec_cli.rs"]
mod exec_cli;
#[path = "support/larry_guard.rs"]
mod larry_guard;

use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::Instant;

use exec_cli::deadline::await_fifo_writer;
use exec_cli::{
    TempDir, TestResult, assert_refused, denial_lines, fixture, flag_of, live_group_pid, make_fifo,
    millis, number_of, percentile, report_of, result_at, run_exec, spawn_exec, spawn_larry_args,
    task_json, text_of,
};
use lgwks_std::json::{self, Value};

/// Latency samples the keyed journey prints, as the other journeys print theirs.
const SAMPLES: usize = 32;

/// Thirty-two executors start at once on one fresh state directory, eight times over: each
/// start recovers the journals it finds while the others run, and no run's own journal may
/// be one of them. Found live on toronto: one in 32 failed with "journal storage refused
/// the append: No such file or directory" (larry#456). Every run exits 0 and leaves no
/// recovered evidence, because no run died.
#[test]
fn many_execs_at_once_on_a_fresh_state_dir_all_finish() -> TestResult {
    for round in 0..8 {
        let (state, origin) = fixture()?;
        let task = task_json(origin.path(), "HEAD", r#"[["true"]]"#);
        let children = (0..SAMPLES)
            .map(|_| spawn_exec(Some(state.path()), None, &task))
            .collect::<Result<Vec<_>, _>>()?;
        for (n, child) in children.into_iter().enumerate() {
            let output = child.wait_with_output()?;
            assert_eq!(
                output.status.code(),
                Some(0),
                "round {round} exec {n}: {} {}",
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            );
        }
        assert_eq!(
            journal_entries(state.path())?,
            0,
            "round {round}: no live run's journal was recovered as evidence"
        );
    }
    Ok(())
}

/// A task document over `repo` that names `key`, or no key when it is empty, whose
/// acceptance command appends both a work-tree change and one line to `count`.
fn keyed_document(repo: &Path, key: &str, count: &Path) -> String {
    let mut document = lgwks_std::json::json!({
        "repo": repo.to_string_lossy(),
        "base": "HEAD",
        "acceptance": [["sh", "-c", format!("printf x >> note; echo ran >> {}", count.display())]],
    });
    if !key.is_empty()
        && let Some(object) = document.as_object_mut()
    {
        object.insert(String::from("idempotency_key"), Value::String(key.into()));
    }
    document.to_string()
}

/// How many entries `<state>/journals` holds: one per run whose journal still
/// exists, whatever extension the recovery left it under. A dead run's `.lock` is its
/// liveness lock, not a journal (larry#456), and is not counted.
fn journal_entries(state: &Path) -> Result<usize, Box<dyn Error>> {
    match fs::read_dir(state.join("journals")) {
        Ok(listing) => {
            let mut journals = 0_usize;
            for entry in listing {
                if entry?.path().extension().is_none_or(|ext| ext != "lock") {
                    journals = journals.saturating_add(1);
                }
            }
            Ok(journals)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

/// `larry verify` on `report` bytes, trusting `state`'s key: its exit status and
/// the verdict it printed.
fn verify_report(state: &Path, report: &[u8]) -> Result<(Option<i32>, Value), Box<dyn Error>> {
    let output = spawn_larry_args(
        &["verify"],
        &[("LARRY_STATE_DIR", state.as_os_str())],
        report,
    )?
    .wait_with_output()?;
    let verdict = json::from_str::<Value>(&String::from_utf8_lossy(&output.stdout))?;
    Ok((output.status.code(), verdict))
}

#[test]
fn the_same_task_with_its_idempotency_key_runs_its_command_once() -> TestResult {
    // Source: draft-ietf-httpapi-idempotency-key-header-07 section 2 and the AWS
    // client-token contract. Two submissions of one document under one key are
    // one request: the command runs once, both reports carry the same run id,
    // and both verify under the state directory's key (larry#182 defect 3).
    let (state, origin) = fixture()?;
    let marker = TempDir::new()?;
    let count = marker.path().join("runs");
    let task = keyed_document(origin.path(), "retry-1", &count);
    let started = Instant::now();
    let first = run_exec(state.path(), &task, None)?;
    let first_run = started.elapsed();
    let retried = Instant::now();
    let second = run_exec(state.path(), &task, None)?;
    let retry_run = retried.elapsed();
    let first_report = report_of(&first)?;
    let second_report = report_of(&second)?;
    assert_eq!(
        number_of(result_at(&first_report, 0)?, "exit_code")?,
        0,
        "the first run's command must exit 0"
    );
    assert_eq!(
        number_of(result_at(&second_report, 0)?, "exit_code")?,
        0,
        "the retry reports the earlier run's own result"
    );
    assert_eq!(
        fs::read_to_string(&count)?,
        "ran\n",
        "the keyed retry must not run the acceptance command a second time"
    );
    assert_eq!(
        text_of(&first_report, "task_id")?,
        text_of(&second_report, "task_id")?,
        "one keyed request keeps one run id"
    );
    assert_eq!(
        text_of(&first_report, "task_digest")?,
        text_of(&second_report, "task_digest")?,
        "one keyed request keeps one task digest"
    );
    assert_eq!(
        text_of(&first_report, "diff")?,
        text_of(&second_report, "diff")?,
        "the retry answers with the earlier run's diff"
    );
    assert!(
        flag_of(&first_report, "accepted")? && flag_of(&second_report, "accepted")?,
        "both submissions are the accepted run"
    );
    for report in [&first, &second] {
        let (code, verdict) = verify_report(state.path(), &report.stdout)?;
        assert_eq!(code, Some(0), "an accepted receipt verifies as accepted");
        assert!(
            flag_of(&verdict, "verified")?,
            "the receipt must be genuine"
        );
        assert_eq!(
            text_of(&verdict, "task_id")?,
            text_of(&first_report, "task_id")?,
            "both receipts name the one run"
        );
    }
    assert_eq!(
        journal_entries(state.path())?,
        0,
        "a returned task removes its journal, both times"
    );

    // The retry costs one ledger read where the first run cloned, spawned and
    // sealed; both are measured and printed, never used to control a run.
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let timed = Instant::now();
        let _report = report_of(&run_exec(state.path(), &task, None)?)?;
        samples.push(timed.elapsed());
    }
    samples.sort();
    let mut err = std::io::stderr().lock();
    writeln!(
        err,
        "keyed exec first={:.3} retry={:.3} ms; retry p50={:.3} p95={:.3} p99={:.3} n={SAMPLES}",
        millis(first_run),
        millis(retry_run),
        millis(percentile(&samples, 50)),
        millis(percentile(&samples, 95)),
        millis(percentile(&samples, 99)),
    )?;
    Ok(())
}

#[test]
fn a_task_without_a_key_runs_once_per_submission() -> TestResult {
    // A document with no idempotency key is two requests when it is submitted
    // twice: the key is the client's statement, so the executor must not
    // collapse two deliberate runs of one task on its own (larry#182 defect 3).
    let (state, origin) = fixture()?;
    let marker = TempDir::new()?;
    let count = marker.path().join("runs");
    let keyed = keyed_document(origin.path(), "", &count);
    let task = task_json(origin.path(), "HEAD", &acceptance_of(&keyed)?);
    let first = report_of(&run_exec(state.path(), &task, None)?)?;
    let second = report_of(&run_exec(state.path(), &task, None)?)?;
    assert_eq!(
        fs::read_to_string(&count)?,
        "ran\nran\n",
        "each unkeyed submission runs its command"
    );
    assert_ne!(
        text_of(&first, "task_id")?,
        text_of(&second, "task_id")?,
        "two submissions are two runs with two ids"
    );
    Ok(())
}

#[test]
fn executor_owner_reads_proof_tenant_before_running_task() -> TestResult {
    // Source: OWASP ASVS 4.0.3 V4.2.1. The action boundary must enforce the
    // authority checked at admission, and the owner must keep the task tenant
    // attached to the effect it runs.
    let (state, origin) = fixture()?;
    let task = task_json(origin.path(), "HEAD", r#"[["true"]]"#);
    let output = run_exec(state.path(), &task, None)?;
    let report = report_of(&output)?;
    assert_eq!(
        number_of(result_at(&report, 0)?, "exit_code")?,
        0,
        "the owner-bound executor journey runs the authorized task"
    );
    assert!(
        flag_of(&report, "accepted")?,
        "the owner-bound executor journey seals an accepted report"
    );
    Ok(())
}

/// The `acceptance` member of `document`, rendered back as text.
fn acceptance_of(document: &str) -> Result<String, Box<dyn Error>> {
    let parsed: Value = json::from_str(document)?;
    Ok(parsed.at("/acceptance")?.to_string())
}

#[test]
fn a_key_that_names_nothing_is_refused_typed() -> TestResult {
    // A key is the client's own text, so it is bounded like every other task
    // field (larry#182 defect 3): an empty one names no request and a control
    // byte cannot be reused verbatim on a retry. Both are one typed line and
    // exit 1, and nothing is sealed.
    let (state, origin) = fixture()?;
    let marker = TempDir::new()?;
    let count = marker.path().join("runs");
    let acceptance = format!(r#"[["sh","-c","echo ran >> {}"]]"#, count.display());
    for key in ["", "retry\t1", "retry\u{1}1"] {
        let document = lgwks_std::json::json!({
            "repo": origin.path().to_string_lossy(),
            "base": "HEAD",
            "idempotency_key": key,
            "acceptance": lgwks_std::json::from_str::<Value>(&acceptance)?,
        })
        .to_string();
        let output = run_exec(state.path(), &document, None)?;
        assert_refused(&output, "idempotency_key");
    }
    assert!(!count.exists(), "a refused key runs no acceptance command");
    Ok(())
}

/// Starts a keyed task under `state` whose command marks `count` and then blocks, and kills
/// the executor with SIGKILL inside that command, after the ledger recorded its group. Returns
/// the document a retry resubmits.
fn killed_keyed_run(
    state: &Path,
    origin: &Path,
    key: &str,
    scratch: &Path,
    count: &Path,
) -> Result<String, Box<dyn Error>> {
    let hold = scratch.join(format!("hold-{key}"));
    make_fifo(&hold)?;
    let document = lgwks_std::json::json!({
        "repo": origin.to_string_lossy(),
        "base": "HEAD",
        "idempotency_key": key,
        "acceptance": [["sh", "-c", format!(
            "echo ran >> {}; cat < {}",
            count.display(),
            hold.display()
        )]],
    })
    .to_string();
    let mut child = spawn_exec(Some(state), None, &document)?;
    // The command writes its marker before it opens the fifo's read end, and the write end
    // opens only once that read end is open: the marker is on disk when this returns, and a
    // document the parser refuses fails this wait inside one deadline (larry#268).
    let _hold = await_fifo_writer(&hold)?;
    // The ledger names the group the command runs in, so the kill -9 has one to reap.
    let pgid = live_group_pid(&state.join("executor.db"))?;
    assert!(pgid > 0, "the running command holds a recorded group");
    child.kill()?;
    child.wait()?;
    assert_eq!(
        fs::read_to_string(count)?,
        "ran\n",
        "the killed run's command ran once"
    );
    Ok(document)
}

/// A fresh state directory and origin whose keyed run under `key` was killed inside its
/// command: what a retry journey starts from.
struct Killed {
    /// The state directory the killed run left its journal in.
    state: TempDir,
    /// The origin repository.
    origin: TempDir,
    /// Scratch space holding the run's fifo and marker.
    scratch: TempDir,
    /// The marker each run of the command appends one line to.
    count: std::path::PathBuf,
    /// The task document a retry resubmits.
    document: String,
}

/// Builds a fixture and kills a keyed run in it (larry#394).
fn killed_in_a_fixture(key: &str) -> Result<Killed, Box<dyn Error>> {
    let (state, origin) = fixture()?;
    let scratch = TempDir::new()?;
    let count = scratch.path().join("runs");
    let document = killed_keyed_run(state.path(), origin.path(), key, scratch.path(), &count)?;
    Ok(Killed {
        state,
        origin,
        scratch,
        count,
        document,
    })
}

/// The one journal `<state>/journals` holds; a dead run's `.lock` is not one (larry#456).
fn only_journal(state: &Path) -> Result<std::path::PathBuf, Box<dyn Error>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(state.join("journals"))? {
        let path = entry?.path();
        if path.extension().is_none_or(|ext| ext != "lock") {
            found.push(path);
        }
    }
    if found.len() != 1 {
        return Err(format!("one journal is left, found {found:?}").into());
    }
    found.pop().ok_or_else(|| "one journal is left".into())
}

#[test]
fn a_keyed_retry_after_a_recovery_fault_runs_nothing_and_keeps_the_journal() -> TestResult {
    // larry#394. A fault that is not a refusal of the journal's bytes (here the device refuses
    // the open) never sets the dead run's journal aside: it stays a journal, the start reports
    // it and goes on, the keyed retry runs nothing, and once the fault clears the next start
    // recovers it. A journal whose bytes are refused is kept as refused, and that is evidence
    // a keyed retry answers from too. Source: the Idempotent Receiver pattern.
    use std::os::unix::fs::PermissionsExt;

    let Killed {
        state,
        origin,
        scratch,
        count,
        document,
    } = killed_in_a_fixture("fault-1")?;
    let journal = only_journal(state.path())?;
    fs::set_permissions(&journal, fs::Permissions::from_mode(0o000))?;
    let faulted = run_exec(state.path(), &document, None)?;
    fs::set_permissions(&journal, fs::Permissions::from_mode(0o600))?;
    assert_eq!(
        fs::read_to_string(&count)?,
        "ran\n",
        "a keyed retry during a recovery fault must not run the dead run's command"
    );
    assert_eq!(
        only_journal(state.path())?,
        journal,
        "a passing fault keeps the dead run's journal for the next start"
    );
    assert_ne!(
        faulted.status.code(),
        Some(0),
        "a retry that cannot read the evidence is never accepted"
    );
    let retry = report_of(&run_exec(state.path(), &document, None)?)?;
    assert_eq!(fs::read_to_string(&count)?, "ran\n", "nor after it clears");
    assert_eq!(
        only_journal(state.path())?,
        journal.with_extension("recovered"),
        "once the fault clears the journal is recovered"
    );
    assert!(
        !flag_of(&retry, "accepted")?,
        "a run nobody observed is never accepted"
    );

    // A journal whose bytes are refused is set aside, and still answers its keyed retry.
    let damaged_state = TempDir::new()?;
    let damaged_state_text = damaged_state.path().to_string_lossy().into_owned();
    larry_demo::executor::provision_tenant(
        move |name| (name == "LARRY_STATE_DIR").then(|| damaged_state_text.clone()),
        "file.read,file.write,process.spawn,model.invoke,model.command",
        &[origin.path()],
    )?;
    let damaged_count = scratch.path().join("damaged-runs");
    let damaged = killed_keyed_run(
        damaged_state.path(),
        origin.path(),
        "damaged-1",
        scratch.path(),
        &damaged_count,
    )?;
    let damaged_journal = only_journal(damaged_state.path())?;
    fs::write(&damaged_journal, b"not a journal")?;
    let answered = report_of(&run_exec(damaged_state.path(), &damaged, None)?)?;
    assert_eq!(
        fs::read_to_string(&damaged_count)?,
        "ran\n",
        "a keyed retry of a run whose journal was refused must not run its command"
    );
    assert_eq!(
        only_journal(damaged_state.path())?,
        damaged_journal.with_extension("refused"),
        "a journal whose bytes are refused is kept as refused"
    );
    assert!(
        denial_lines(&answered)?
            .first()
            .is_some_and(|line| line.contains("refused")),
        "the retry says the earlier run's journal was refused"
    );
    Ok(())
}

#[test]
fn a_killed_run_with_a_key_answers_from_its_recovered_journal_and_runs_nothing() -> TestResult {
    // Source: the Idempotent Receiver pattern and INVARIANTS' journals line (a
    // dead task's journal is recovered, never re-run). The first run is killed
    // inside its command, after the ledger recorded that command's group; the
    // keyed retry must not run that command again and must keep the dead run's
    // journal as evidence (larry#182 defect 3).
    // The origin and the scratch stay alive to the end, so a retry that did run would succeed.
    let Killed {
        state,
        origin: _origin,
        scratch: _scratch,
        count,
        document,
    } = killed_in_a_fixture("killed-1")?;
    assert_eq!(
        journal_entries(state.path())?,
        1,
        "the killed run leaves its journal"
    );

    let retry = report_of(&run_exec(state.path(), &document, None)?)?;
    assert_eq!(
        fs::read_to_string(&count)?,
        "ran\n",
        "the keyed retry must not run the dead run's command again"
    );
    assert_eq!(
        journal_entries(state.path())?,
        1,
        "the retry adds no journal of its own: only the recovered one remains"
    );
    assert!(
        !flag_of(&retry, "accepted")?,
        "a run whose outcome nobody observed is never accepted"
    );
    let denied = denial_lines(&retry)?;
    assert_eq!(denied.len(), 1, "the retry names one reason: {denied:?}");
    assert!(
        denied
            .first()
            .is_some_and(|line| line.contains("recovered")),
        "the refusal says the earlier run's evidence was recovered: {denied:?}"
    );
    Ok(())
}
