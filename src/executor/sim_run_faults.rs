//! Adversarial fault injection through the run's own seams (larry#575): an empty command, a
//! deadline past the timeline, a waiter that dies, a child that vanishes before it is
//! identified, a read that lies, a grace no clock can hold and a reader that fails. Each answer
//! is asserted as its exact typed error.

use std::io;
use std::os::unix::process::ExitStatusExt as _;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use lgwks_bot::kernel::clock::Clock;
use lgwks_std::seeded::Seeded;

use super::super::{
    Captured, ExecError, Place, RunHooks, TAIL_BYTES, command_for, drain, join_reader, run_command,
    track, wait_for_status,
};
use crate::executor::ledger::Ledger;
use crate::executor::sim_tree::{path_env, scratch};

/// What every simulation here returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A process group id no live process holds: the largest positive `i32` less one. A group id of
/// zero, one or below would name a real group, and a kill of it would reach the host.
const DEAD_PID: i32 = 2_147_483_646;

/// SIGKILL's number.
const KILLED: i32 = 9;

/// The spawn failure's program and message, or why the answer was something else.
fn spawn_refusal<T>(
    answer: Result<T, ExecError>,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    match answer {
        Err(ExecError::Spawn { argv0, source }) => Ok((argv0, source.to_string())),
        Err(other) => Err(format!("not a spawn failure: {other}").into()),
        Ok(_) => Err("the fault was accepted".into()),
    }
}

/// A command with no words is refused before anything is started (CWE-20, larry#85).
#[test]
fn an_empty_command_is_refused_before_it_starts() -> TestResult {
    let refused = spawn_refusal(command_for(&[], Place::Open(Path::new(".")), "/bin"))?;
    assert_eq!(
        refused,
        (String::new(), "acceptance command is empty".to_owned())
    );
    Ok(())
}

/// A timeout the timeline cannot hold is refused with nothing run, so no leader is ever left
/// without a deadline (CWE-190, larry#436).
#[test]
fn sim_a_command_deadline_past_the_timeline_runs_nothing() -> TestResult {
    let dir = scratch("run-overflow")?;
    let ledger = Ledger::open(&dir)?;
    let words: Vec<String> = ["/bin/sh", "-c", "touch ran"].map(String::from).into();
    let never = || false;
    let (_program, message) = spawn_refusal(run_command(
        &words,
        Place::Open(&dir),
        &path_env()?,
        Duration::MAX,
        &ledger,
        "task",
        RunHooks::new(&never, None),
    ))?;
    assert_eq!(message, "the command deadline overflowed");
    assert!(!dir.join("ran").exists(), "the refused command ran");
    Ok(())
}

/// A waiter that ends without a status is its own fault, neither a timeout nor a cancel, and no
/// group is signalled for it (CWE-754, larry#442).
#[test]
fn sim_a_waiter_that_ends_without_a_status_is_neither_timeout_nor_cancel() -> TestResult {
    let clock = Clock::virtual_at(Duration::ZERO);
    let (sender, receiver) = mpsc::sync_channel::<io::Result<ExitStatus>>(1);
    drop(sender);
    let cancelled = || true;
    let (status, timed_out, interrupted) = wait_for_status(
        &clock,
        &receiver,
        DEAD_PID,
        Duration::from_secs(60),
        &cancelled,
    )?;
    assert_eq!(
        status.map_err(|cause| cause.to_string()).err(),
        Some("the waiter ended without a status".to_owned())
    );
    assert_eq!((timed_out, interrupted), (false, false));
    Ok(())
}

/// A child gone before it could be identified is killed and reaped, never left running with no
/// ledger row (CWE-404, larry#590). The identified id is one no process holds.
#[test]
fn sim_a_child_that_vanished_before_identification_is_killed_and_reaped() -> TestResult {
    let dir = scratch("run-vanished")?;
    let ledger = Ledger::open(&dir)?;
    let clock = larry_runtime::live_clock()?;
    let mut child = Command::new("sleep")
        .arg("30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()?;
    let (program, message) = spawn_refusal(track(
        &clock,
        &mut child,
        DEAD_PID,
        "sleep",
        (&ledger, "task"),
    ))?;
    assert_eq!(
        (program.as_str(), message.as_str()),
        ("sleep", "the child vanished before it was identified")
    );
    let status = child.try_wait()?.ok_or("the child was not reaped")?;
    assert_eq!(status.signal(), Some(KILLED), "killed, not outlived");
    Ok(())
}

/// A read that claims more bytes than its buffer holds is refused, never trusted (CWE-130).
#[test]
fn sim_a_drain_read_that_lies_about_its_length_is_refused() -> TestResult {
    let mut rng = Seeded::from_seed(0x5EED_0000_0000_0051);
    for _ in 0..16 {
        let over = usize::try_from(rng.below(1 << 20)?)?.saturating_add(1);
        let refused = drain(
            |chunk| Ok(chunk.len().saturating_add(over)),
            || Duration::ZERO,
            &AtomicBool::new(false),
        )
        .map(|drained| drained.captured.bytes)
        .map_err(|cause| cause.to_string());
        assert_eq!(
            refused,
            Err("a read reported more bytes than its buffer holds".to_owned()),
            "over by {over}"
        );
    }
    Ok(())
}

/// A grace the clock cannot represent reads as passed: the stream is reported open at once and
/// never awaited without a bound (CWE-835, larry#334).
#[test]
fn sim_a_grace_past_the_clock_ends_the_drain_at_once() -> TestResult {
    let settled = AtomicBool::new(true);
    let reads = std::cell::Cell::new(0_u32);
    let drained = drain(
        |_chunk| {
            reads.set(reads.get().saturating_add(1));
            Err(io::Error::from(io::ErrorKind::WouldBlock))
        },
        || Duration::MAX,
        &settled,
    )?;
    assert!(drained.open, "a held stream is reported open");
    assert_eq!(reads.get(), 0, "nothing is read past an impossible grace");
    assert!(settled.load(Ordering::Relaxed));
    Ok(())
}

/// Byte counts that would pass `u64` are refused, never wrapped into a short count (CWE-190,
/// larry#319).
#[test]
fn a_stream_count_that_would_wrap_is_refused() {
    let mut full = Captured {
        bytes: u64::MAX,
        ..Captured::default()
    };
    assert_eq!(
        full.push(b"x").map_err(|cause| cause.to_string()),
        Err("the stream byte count overflowed".to_owned())
    );
    let mut dropped = Captured {
        tail: vec![b't'; TAIL_BYTES.saturating_add(1)],
        dropped: u64::MAX,
        ..Captured::default()
    };
    assert_eq!(
        dropped.trim().map_err(|cause| cause.to_string()),
        Err("the dropped byte count overflowed".to_owned())
    );
}

/// A reader that fails is the command's spawn failure with its own cause, never a short stream
/// reported whole (CWE-252, larry#334).
#[test]
fn sim_a_failed_reader_is_a_typed_failure_with_its_cause() -> TestResult {
    let refused = std::thread::scope(|scope| {
        let reader = scope.spawn(|| Err(io::Error::other("the socket broke")));
        spawn_refusal(join_reader(reader, "cat"))
    })?;
    assert_eq!(refused, ("cat".to_owned(), "the socket broke".to_owned()));
    Ok(())
}
