//! Journeys against the shipped receipts (larry#86), as Claude or an auditor
//! lives them: run `larry exec`, keep the sealed report, and later pipe it to
//! `larry verify`, trusting either the state directory's key or the 64 hex
//! characters `larry key` printed.
//!
//! Source: RFC 8032 (ed25519), RFC 8785 (JSON canonicalization), POSIX exit
//! status conventions, and forge#158 (a denied tool call reported as success).
//! Requirement proved: every report carries a receipt that verifies; any edit
//! to output, exit code, diff, verdict, key or signature is refused with one
//! typed line and exit 1; a denied or failed run is sealed as rejected (exit 2)
//! and can never be edited into an accepted one; a receipt from another state
//! directory (tenant) is untrusted; the key survives a kill -9 of the executor;
//! and an offline verifier needs only the printed public key.

#[path = "support/exec_cli.rs"]
mod exec_cli;

use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Output;
use std::time::Instant;

use exec_cli::{
    TempDir, TestResult, assert_refused, denial_lines, fixture, flag_of, live_group_pid, make_fifo,
    millis, number_of, percentile, report_of, result_at, run_exec, spawn_exec, spawn_larry,
    task_json, text_of,
};
use lgwks_std::json::{self, Value};

/// Runs the latency case samples.
const SAMPLES: usize = 32;

/// Runs `larry verify` with only `env` set and `report` on stdin.
fn run_verify(env: &[(&str, &OsStr)], report: &[u8]) -> Result<Output, Box<dyn Error>> {
    Ok(spawn_larry("verify", env, report)?.wait_with_output()?)
}

/// Runs `larry verify` trusting the key of the state directory `state`.
fn verify_in(state: &Path, report: &[u8]) -> Result<Output, Box<dyn Error>> {
    run_verify(&[("LARRY_STATE_DIR", state.as_os_str())], report)
}

/// The verdict line `larry verify` printed.
fn verdict_of(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(json::from_str::<Value>(&String::from_utf8_lossy(
        &output.stdout,
    ))?)
}

/// The receipt public key `larry key` prints for the state directory `state`.
fn key_of(state: &Path) -> Result<String, Box<dyn Error>> {
    let output =
        spawn_larry("key", &[("LARRY_STATE_DIR", state.as_os_str())], b"")?.wait_with_output()?;
    assert!(output.status.success(), "larry key must exit 0");
    let printed = String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_owned();
    assert_eq!(printed.len(), 64, "larry key prints 64 hex characters");
    Ok(printed)
}

/// One step into a report: a member name or an array index.
#[derive(Clone, Copy)]
enum Step {
    /// A member of an object.
    Key(&'static str),
    /// An item of an array.
    At(usize),
}

/// A copy of `report` whose value at `path` is `member`; the last step may add a member.
fn edited(report: &Value, path: &[Step], member: Value) -> Result<Value, Box<dyn Error>> {
    let mut copy = report.clone();
    let (last, parents) = path.split_last().ok_or("an edit names a path")?;
    let mut cursor = &mut copy;
    for step in parents {
        cursor = match *step {
            Step::Key(name) => cursor.get_mut(name),
            Step::At(index) => cursor.get_mut(index),
        }
        .ok_or("the edited path exists")?;
    }
    match *last {
        Step::Key(name) => {
            cursor
                .as_object_mut()
                .ok_or("the edited parent is an object")?
                .insert(name, member);
        }
        Step::At(index) => *cursor.get_mut(index).ok_or("the edited item exists")? = member,
    }
    Ok(copy)
}

/// The report's bytes after one edit.
fn edited_bytes(report: &Value, path: &[Step], member: Value) -> Result<Vec<u8>, Box<dyn Error>> {
    Ok(edited(report, path, member)?.to_string().into_bytes())
}

/// The receipt's key member as text.
fn receipt_key(report: &Value) -> Result<&str, Box<dyn Error>> {
    report
        .get("receipt")
        .and_then(|receipt| receipt.get("key"))
        .and_then(Value::as_str)
        .ok_or_else(|| "the report carries a receipt key".into())
}

#[test]
fn an_accepted_task_carries_a_receipt_that_verifies() -> TestResult {
    // A task that edits the workspace and passes is accepted, and its receipt verifies.
    let (state, origin) = fixture()?;
    let task = task_json(origin.path(), "HEAD", r#"[["sh","-c","echo ok > NEW"]]"#);
    let output = run_exec(state.path(), &task, None)?;
    let report = report_of(&output)?;
    assert!(flag_of(&report, "accepted")?, "a passing run is accepted");
    assert!(
        text_of(&report, "diff")?.contains("NEW"),
        "the sealed diff shows the change"
    );
    assert_eq!(
        receipt_key(&report)?,
        key_of(state.path())?,
        "the receipt names the state directory's key"
    );
    let verified = verify_in(state.path(), &output.stdout)?;
    assert_eq!(
        verified.status.code(),
        Some(0),
        "an accepted receipt verifies with exit 0"
    );
    let verdict = verdict_of(&verified)?;
    assert!(flag_of(&verdict, "verified")?, "the verdict says verified");
    assert!(flag_of(&verdict, "accepted")?, "the verdict says accepted");
    assert_eq!(
        text_of(&verdict, "task_id")?,
        text_of(&report, "task_id")?,
        "the verdict names the sealed task"
    );

    // Matched workload: trusting a result by re-running its acceptance commands
    // (exec) against trusting it by its receipt (verify), 32 runs each.
    let mut exec_samples = Vec::with_capacity(SAMPLES);
    let mut verify_samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let rerun = run_exec(state.path(), &task, None)?;
        exec_samples.push(started.elapsed());
        report_of(&rerun)?;
        let started = Instant::now();
        let checked = verify_in(state.path(), &output.stdout)?;
        verify_samples.push(started.elapsed());
        assert_eq!(checked.status.code(), Some(0), "every timed verify passes");
    }
    exec_samples.sort();
    verify_samples.sort();
    let mut err = io::stderr().lock();
    writeln!(
        err,
        "rerun p50={:.3} p95={:.3} p99={:.3} verify p50={:.3} p95={:.3} p99={:.3} n={SAMPLES}",
        millis(percentile(&exec_samples, 50)),
        millis(percentile(&exec_samples, 95)),
        millis(percentile(&exec_samples, 99)),
        millis(percentile(&verify_samples, 50)),
        millis(percentile(&verify_samples, 95)),
        millis(percentile(&verify_samples, 99)),
    )?;
    Ok(())
}

#[test]
fn a_tampered_receipt_is_refused() -> TestResult {
    // Every edit an attacker could make to a sealed report is refused with one line.
    let (state, origin) = fixture()?;
    let task = task_json(
        origin.path(),
        "HEAD",
        r#"[["sh","-c","echo hi; echo ok > NEW"]]"#,
    );
    let report = report_of(&run_exec(state.path(), &task, None)?)?;
    assert_eq!(
        number_of(result_at(&report, 0)?, "exit_code")?,
        0,
        "the sealed command exited 0"
    );
    let first = [Step::Key("results"), Step::At(0)];
    let tampers: [(&str, Vec<u8>, &str); 7] = [
        (
            "stdout",
            edited_bytes(
                &report,
                &[first[0], first[1], Step::Key("stdout")],
                json::json!("forged\n"),
            )?,
            "altered",
        ),
        (
            "exit code",
            edited_bytes(
                &report,
                &[first[0], first[1], Step::Key("exit_code")],
                json::json!(1),
            )?,
            "altered",
        ),
        (
            "diff",
            edited_bytes(&report, &[Step::Key("diff")], json::json!(""))?,
            "altered",
        ),
        (
            "an added member",
            edited_bytes(&report, &[Step::Key("note")], json::json!("trust me"))?,
            "altered",
        ),
        (
            "signature",
            edited_bytes(
                &report,
                &[Step::Key("receipt"), Step::Key("signature")],
                json::json!("0".repeat(128)),
            )?,
            "altered",
        ),
        (
            "receipt nulled",
            edited_bytes(&report, &[Step::Key("receipt")], Value::Null)?,
            "algorithm",
        ),
        (
            "trailing bytes",
            format!("{report}}}").into_bytes(),
            "not valid JSON",
        ),
    ];
    for (what, bytes, needle) in tampers {
        let output = verify_in(state.path(), &bytes)?;
        assert_refused(&output, needle);
        assert!(
            output.stdout.is_empty(),
            "a tampered {what} prints no verdict"
        );
    }
    let mut unsigned = report;
    unsigned
        .as_object_mut()
        .ok_or("the report is an object")?
        .remove("receipt");
    assert_refused(
        &verify_in(state.path(), unsigned.to_string().as_bytes())?,
        "missing",
    );
    Ok(())
}

#[test]
fn a_denied_run_never_reports_success() -> TestResult {
    // A task asking for model commands is denied: nothing runs, the denial is sealed.
    let (state, origin) = fixture()?;
    let denied_task = format!(
        r#"{{"repo":"{}","base":"HEAD","acceptance":[["true"]],"model_commands":[]}}"#,
        origin.path().display()
    );
    let output = run_exec(state.path(), &denied_task, None)?;
    assert_eq!(output.status.code(), Some(2), "a denied task exits 2");
    let report = report_of(&output)?;
    assert!(
        !flag_of(&report, "accepted")?,
        "a denied task is never accepted"
    );
    assert!(
        denial_lines(&report)?
            .iter()
            .any(|denial| denial.starts_with("model_commands")),
        "the denial names model_commands"
    );
    assert!(
        result_at(&report, 0).is_err(),
        "nothing ran after the denial"
    );
    let verified = verify_in(state.path(), &output.stdout)?;
    assert_eq!(
        verified.status.code(),
        Some(2),
        "a genuine denial verifies as rejected"
    );
    assert!(
        !flag_of(&verdict_of(&verified)?, "accepted")?,
        "the verdict says not accepted"
    );

    // Editing the denial into a success is refused, whichever members change.
    let flipped = edited_bytes(&report, &[Step::Key("accepted")], json::json!(true))?;
    assert_refused(&verify_in(state.path(), &flipped)?, "altered");
    let accepted = edited(&report, &[Step::Key("accepted")], json::json!(true))?;
    let undenied = edited(&accepted, &[Step::Key("denied")], json::json!([]))?;
    let laundered = edited_bytes(&undenied, &[Step::Key("diff")], json::json!(""))?;
    assert_refused(&verify_in(state.path(), &laundered)?, "altered");

    // A failing command is sealed as rejected and verifies as rejected.
    let failing = task_json(origin.path(), "HEAD", r#"[["sh","-c","exit 4"]]"#);
    let output = run_exec(state.path(), &failing, None)?;
    assert_eq!(output.status.code(), Some(2), "a failing run exits 2");
    assert_eq!(
        verify_in(state.path(), &output.stdout)?.status.code(),
        Some(2),
        "a failing run's genuine receipt verifies as rejected"
    );
    Ok(())
}

#[test]
fn a_receipt_from_another_state_dir_is_untrusted() -> TestResult {
    // Two tenants run at once on one origin; each trusts only its own receipts.
    let (first_state, origin) = fixture()?;
    let second_state = TempDir::new()?;
    let task = task_json(origin.path(), "HEAD", r#"[["true"]]"#);
    let first_child = spawn_exec(Some(first_state.path()), None, &task)?;
    let second_child = spawn_exec(Some(second_state.path()), None, &task)?;
    let first = first_child.wait_with_output()?;
    let second = second_child.wait_with_output()?;
    report_of(&first)?;
    report_of(&second)?;
    assert_ne!(
        key_of(first_state.path())?,
        key_of(second_state.path())?,
        "two state directories hold two keys"
    );
    assert_eq!(
        verify_in(first_state.path(), &first.stdout)?.status.code(),
        Some(0),
        "the first tenant verifies its own receipt"
    );
    assert_eq!(
        verify_in(second_state.path(), &second.stdout)?
            .status
            .code(),
        Some(0),
        "the second tenant verifies its own receipt"
    );
    assert_refused(&verify_in(second_state.path(), &first.stdout)?, "untrusted");
    assert_refused(&verify_in(first_state.path(), &second.stdout)?, "untrusted");
    Ok(())
}

#[test]
fn the_signing_key_survives_a_kill_9_of_the_executor() -> TestResult {
    // A receipt sealed before a kill -9 still verifies, and the next run signs under the same key.
    let (state, origin) = fixture()?;
    let quick = task_json(origin.path(), "HEAD", r#"[["true"]]"#);
    let before = run_exec(state.path(), &quick, None)?;
    report_of(&before)?;
    let key_before = key_of(state.path())?;

    let scratch = TempDir::new()?;
    let ready = scratch.path().join("ready");
    let hold = scratch.path().join("hold");
    make_fifo(&ready)?;
    make_fifo(&hold)?;
    let blocked = task_json(
        origin.path(),
        "HEAD",
        &format!(
            r#"[["sh","-c","echo up > {}; cat < {}"]]"#,
            ready.display(),
            hold.display()
        ),
    );
    let mut child = spawn_exec(Some(state.path()), None, &blocked)?;
    let _ready_open = fs::File::open(&ready)?;
    live_group_pid(&state.path().join("executor.db"))?;
    child.kill()?;
    child.wait()?;

    let after = run_exec(state.path(), &quick, None)?;
    let sealed_after = report_of(&after)?;
    assert_eq!(
        key_of(state.path())?,
        key_before,
        "the key is the same after the kill -9"
    );
    assert_eq!(
        receipt_key(&sealed_after)?,
        key_before,
        "the next run signs under the surviving key"
    );
    for (which, bytes) in [("before", &before.stdout), ("after", &after.stdout)] {
        assert_eq!(
            verify_in(state.path(), bytes)?.status.code(),
            Some(0),
            "the receipt sealed {which} the kill -9 verifies"
        );
    }
    Ok(())
}

#[test]
fn an_offline_verifier_trusts_only_the_printed_key() -> TestResult {
    // `LARRY_RECEIPT_KEY` alone verifies, with no state directory; anything else is refused.
    let (state, origin) = fixture()?;
    let stranger = TempDir::new()?;
    let output = run_exec(
        state.path(),
        &task_json(origin.path(), "HEAD", r#"[["true"]]"#),
        None,
    )?;
    report_of(&output)?;
    let printed = key_of(state.path())?;
    let offline = run_verify(
        &[("LARRY_RECEIPT_KEY", OsStr::new(&printed))],
        &output.stdout,
    )?;
    assert_eq!(
        offline.status.code(),
        Some(0),
        "the printed key alone verifies the receipt"
    );
    let other = key_of(stranger.path())?;
    assert_refused(
        &run_verify(&[("LARRY_RECEIPT_KEY", OsStr::new(&other))], &output.stdout)?,
        "untrusted",
    );
    assert_refused(
        &run_verify(
            &[("LARRY_RECEIPT_KEY", OsStr::new("not-hex"))],
            &output.stdout,
        )?,
        "64-hex",
    );
    assert_refused(&run_verify(&[], &output.stdout)?, "LARRY_STATE_DIR");
    assert_refused(&verify_in(state.path(), b"")?, "not valid JSON");
    Ok(())
}

#[test]
fn a_task_over_one_mib_seals_unaccepted_and_verify_agrees() -> TestResult {
    // A change of 1.2 MB is cut at 1 MiB: the report says so, is never accepted and verifies as rejected.
    let (state, origin) = fixture()?;
    let task = task_json(
        origin.path(),
        "HEAD",
        r#"[["sh","-c","head -c 1200000 /dev/zero | tr '\\000' x > BIG"]]"#,
    );
    let output = run_exec(state.path(), &task, None)?;
    let report = report_of(&output)?;
    assert!(
        !flag_of(&report, "accepted")?,
        "a cut diff is never accepted"
    );
    assert!(
        flag_of(&report, "diff_truncated")?,
        "the report says the diff was cut"
    );
    assert!(
        number_of(&report, "diff_bytes")? > 1_048_576,
        "the full diff is larger than the kept megabyte"
    );
    assert_eq!(
        text_of(&report, "diff")?.len(),
        1_048_576,
        "the kept diff is exactly the limit"
    );
    assert_eq!(
        text_of(&report, "diff_digest")?.len(),
        64,
        "the digest is 64 hex characters"
    );
    let verified = verify_in(state.path(), &output.stdout)?;
    assert_eq!(
        verified.status.code(),
        Some(2),
        "a cut diff verifies as rejected"
    );
    assert!(
        !flag_of(&verdict_of(&verified)?, "accepted")?,
        "the verdict says not accepted"
    );
    let flipped = edited_bytes(&report, &[Step::Key("diff_truncated")], json::json!(false))?;
    assert_refused(&verify_in(state.path(), &flipped)?, "altered");
    assert_eq!(
        fs::read_dir(state.path().join("ws"))?.count(),
        0,
        "the run's workspace is removed when the run ends"
    );
    Ok(())
}

#[test]
fn a_failing_command_that_prints_two_mib_keeps_its_last_line() -> TestResult {
    // 2 MiB of 'a', a newline, LAST-LINE and a newline, then exit 1: 2097163 bytes in all.
    let (state, origin) = fixture()?;
    let task = task_json(
        origin.path(),
        "HEAD",
        r#"[["sh","-c","head -c 2097152 /dev/zero | tr '\\000' a; echo; echo LAST-LINE; exit 1"]]"#,
    );
    let output = run_exec(state.path(), &task, None)?;
    let report = report_of(&output)?;
    let result = result_at(&report, 0)?;
    let stdout = text_of(result, "stdout")?;
    assert!(
        stdout.ends_with("LAST-LINE\n"),
        "the last line of a failing command is kept"
    );
    assert_eq!(
        number_of(result, "stdout_dropped")?,
        1_048_587,
        "the dropped count is exact: 2097163 bytes less the 1048576 kept"
    );
    assert!(
        stdout.contains("bytes dropped"),
        "a marker line says what was dropped"
    );
    assert!(
        stdout.len() <= 1_048_576 + 64,
        "the kept output stays within the bound plus its marker"
    );
    assert_eq!(number_of(result, "exit_code")?, 1, "the command exited 1");

    // Matched workload: the same failing command, 32 runs, timed end to end.
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let rerun = run_exec(state.path(), &task, None)?;
        samples.push(started.elapsed());
        report_of(&rerun)?;
    }
    samples.sort();
    let mut err = io::stderr().lock();
    writeln!(
        err,
        "capture p50={:.3} p95={:.3} p99={:.3} n={SAMPLES}",
        millis(percentile(&samples, 50)),
        millis(percentile(&samples, 95)),
        millis(percentile(&samples, 99)),
    )?;
    Ok(())
}
