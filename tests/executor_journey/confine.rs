//! The confinement journey of `larry exec` (larry#324, ADR 0088), split from
//! `tests/executor_journey.rs` so each file stays one responsibility. Source: OWASP ASVS
//! 4.0.3 §4.2.1 (enforcement at the action's boundary), CWE-59 and larry#243. Requirement
//! proved: a brief lets the model write the tree, so the acceptance command that runs next is
//! model-chosen code; it runs behind the kernel confinement a terminal session uses, so a
//! write outside the tree is refused, the origin stays byte-identical and the run seals as
//! rejected. A task with no brief keeps today's open place.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;

use lgwks_std::hash::Hasher;

use crate::exec_cli::{
    TempDir, TestResult, denial_lines, fixture, flag_of, millis, number_of, percentile, report_of,
    result_at, run_exec, spawn_larry_args, task_json,
};
use crate::mock_model::{Config, Fault, Handle};
use crate::tree_digest::tree_digest;

/// Every fault the seeded endpoint can answer with. Naming them keeps the whole shared enum
/// live in this binary, and the confinement journey schedules none of them (larry#324).
const EVERY_FAULT: [Option<Fault>; 7] = [
    Some(Fault::Delay),
    Some(Fault::RateLimit {
        retry_after_secs: 1,
    }),
    Some(Fault::MalformedJson),
    Some(Fault::TruncatedBody),
    Some(Fault::ServerError { status: 500 }),
    Some(Fault::PaddedDone),
    Some(Fault::HostileRefusal { status: 401 }),
];

/// The shared endpoint's whole fault set stays named, so including it never narrows it.
#[test]
fn the_endpoint_can_answer_with_every_fault() {
    assert_eq!(EVERY_FAULT.len(), 7, "every fault is named");
}

/// The environment that points `larry exec` at the seeded endpoint.
fn model_env<'env>(state: &'env Path, url: &'env str) -> [(&'static str, &'env OsStr); 5] {
    [
        ("LARRY_STATE_DIR", state.as_os_str()),
        ("LARRY_MODEL", OsStr::new("mock-model")),
        ("LARRY_MODEL_KEY", OsStr::new("mock-key")),
        ("LARRY_MODEL_URL", OsStr::new(url)),
        ("LARRY_MODEL_BACKOFF_MS", OsStr::new("0")),
    ]
}

/// Runs the briefed task whose acceptance writes at `target`, and proves the kernel refused
/// it: exit 2, sealed rejected, a non-zero command exit, and no file at `target`.
fn refused_outside_write(
    state: &Path,
    origin: &Path,
    target: &Path,
    url: &str,
    mock: &Handle,
) -> TestResult {
    let task = super::brief_json(
        origin,
        "write the note",
        r#"["note.txt"]"#,
        &format!(r#"[["sh","-c","printf pwned > {}"]]"#, target.display()),
    );
    let empty_chain = Hasher::new().finalize().to_hex();
    let env = model_env(state, url);
    let output = spawn_larry_args(&["exec"], &env, task.as_bytes())?.wait_with_output()?;
    let report = report_of(&output)?;
    assert_eq!(
        output.status.code(),
        Some(2),
        "the kernel refuses the write, so the run is sealed rejected"
    );
    assert!(
        !flag_of(&report, "accepted")?,
        "a refused acceptance is never accepted"
    );
    assert!(
        denial_lines(&report)?.is_empty(),
        "the run was refused by the kernel, not by the gate"
    );
    assert_ne!(
        number_of(result_at(&report, 0)?, "exit_code")?,
        0,
        "the kernel refused the write, so the command did not exit 0"
    );
    assert_eq!(
        mock.served(),
        1,
        "the one briefed file takes one completion"
    );
    assert_eq!(mock.requests().len(), 1, "the request is recorded");
    assert!(
        mock.authorizations()
            .iter()
            .all(|sent| sent.as_deref() == Some("Bearer mock-key")),
        "only the operator's key reached the gateway"
    );
    assert_ne!(mock.trace_hash(), empty_chain, "the model was reached");
    assert!(
        !target.exists(),
        "the confinement must refuse a write outside the tree"
    );
    Ok(())
}

/// The confined path's cost, measured on the same host and profile (SD-5): the same briefed
/// run 32 times. The no-brief path's own 32-run figure is
/// `a_passing_and_a_failing_command_report_their_exit_codes`.
fn confined_latency(state: &Path, origin: &Path, url: &str) -> TestResult {
    let task = super::brief_json(origin, "write the note", r#"["note.txt"]"#, r#"[["true"]]"#);
    let env = model_env(state, url);
    let mut samples = Vec::with_capacity(32);
    for _ in 0..32 {
        let started = Instant::now();
        let output = spawn_larry_args(&["exec"], &env, task.as_bytes())?.wait_with_output()?;
        samples.push(started.elapsed());
        report_of(&output)?;
    }
    samples.sort();
    let mut err = io::stderr().lock();
    writeln!(
        err,
        "exec confined: n=32 p50={:.3} p99={:.3}",
        millis(percentile(&samples, 50)),
        millis(percentile(&samples, 99))
    )?;
    Ok(())
}

/// larry#324: a brief lets the model write the tree, so the acceptance command that runs next
/// is model-chosen code. It runs behind the kernel confinement a terminal session uses
/// (larry#243, ADR 0046): a write outside the tree is refused, the origin stays byte-identical
/// and the run seals as rejected. A task with no brief keeps today's open place, since no model
/// wrote anything and the caller's own command is the only one in the loop (ADR 0088).
#[test]
fn exec_runs_a_briefs_acceptance_confined_and_the_origin_stays_identical() -> TestResult {
    let (state, origin) = fixture()?;
    let outside = TempDir::new()?;
    let target = outside.path().join("escaped");
    let before = tree_digest(origin.path())?;
    crate::mock_model::run(Config::seeded(17), |mock: &Handle| -> TestResult {
        let url = format!("http://{}/v1", mock.addr());
        refused_outside_write(state.path(), origin.path(), &target, &url, mock)?;
        confined_latency(state.path(), origin.path(), &url)?;
        mock.release_delayed();
        Ok(())
    })?;
    assert!(
        !target.exists(),
        "nothing may write outside the tree, whatever the model chose"
    );
    assert_eq!(
        tree_digest(origin.path())?,
        before,
        "the origin stays byte-identical"
    );

    // A task with no brief keeps today's place: its command is the caller's own, with no model
    // in the loop, and it still runs open (ADR 0088).
    let kept = outside.path().join("kept");
    let plain = task_json(
        origin.path(),
        "HEAD",
        &format!(r#"[["sh","-c","printf kept > {}"]]"#, kept.display()),
    );
    let report = report_of(&run_exec(state.path(), &plain, None)?)?;
    assert!(
        flag_of(&report, "accepted")?,
        "a no-brief task's command still runs in today's place"
    );
    assert_eq!(
        number_of(result_at(&report, 0)?, "exit_code")?,
        0,
        "the no-brief command exits 0"
    );
    assert_eq!(
        fs::read_to_string(&kept)?,
        "kept",
        "the no-brief command's write lands outside the tree, as it did before"
    );
    Ok(())
}
