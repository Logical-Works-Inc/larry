//! A session thread that panics still ends for the frame (larry#412). Before this, the worker
//! sent `Ended` only when `run_session` returned, so a panic left the frame showing a running
//! task forever and `Drop` kept the panic only as `join().is_err()`. The worker's one read now
//! joins a thread whose channel closed with no `Ended` and names its panic, and `finish` hands
//! the panic of a session still running at quit to the caller, typed.
//!
//! Each job hands its step to the test over a channel of one, so every read below sees a fixed
//! channel state: no sleep, no clock, and the same events for every run.

use std::sync::mpsc::sync_channel;
use std::time::Duration;

use super::super::TuiError;
use super::super::worker::{Ended, Event, Relay, Worker};
use crate::executor::{ExecError, Observer, Stop};

/// The bound one event wait may take. The wait is the channel's own; the bound only turns a
/// session that never answers into a named failure.
const PATIENCE: Duration = Duration::from_secs(120);

/// What a test job returns once it did not break.
type Outcome = Result<Stop, ExecError>;

/// A session thread that panics mid-step still ends: the event it sent first arrives through
/// the frame's own `next`, a running session with nothing new reads as nothing, then
/// `Ended::Failed` naming the panic is the last event and nothing follows it. Catches a frame
/// that shows a running task forever because no `Ended` came, and a panic message lost to a
/// bare `join().is_err()`.
#[test]
fn sim_tui_worker_a_panicked_session_ends_failed_with_its_message() -> Result<(), String> {
    let (said_tx, said) = sync_channel::<()>(1);
    let (go, go_rx) = sync_channel::<()>(1);
    let mut worker = Worker::start(false, move |relay: &mut Relay| -> Outcome {
        relay.message("step one");
        // Both ends live: the step breaks. A test that went away ends the turn instead.
        let live = said_tx.send(()).is_ok() && go_rx.recv().is_ok();
        assert!(!live, "the step broke after step one");
        Ok(Stop::EndTurn)
    })
    .map_err(|cause| cause.to_string())?;
    said.recv_timeout(PATIENCE)
        .map_err(|cause| format!("the job never said its step: {cause}"))?;
    assert_eq!(
        worker.next(),
        Some(Event::Said(String::from("step one"))),
        "the event in the channel is read at once"
    );
    assert_eq!(
        worker.next(),
        None,
        "a session blocked on its step has nothing new"
    );
    go.send(())
        .map_err(|cause| format!("the job is gone: {cause}"))?;
    assert_eq!(
        worker.wait(PATIENCE),
        Some(Event::Ended(Ended::Failed(String::from(
            "the session stopped on a panic: the step broke after step one"
        )))),
        "the panic is the last event, named"
    );
    assert_eq!(worker.wait(PATIENCE), None, "nothing follows the end");
    assert_eq!(worker.next(), None, "a reaped worker stays quiet");
    Ok(())
}

/// A worker dropped while its session waits for the person's answer wakes the session to a
/// decline and joins its thread before the drop returns, so no session outlives the frame that
/// started it. The job holds one end of a channel until its thread ends, so the test sees the
/// join as that channel closed the moment `drop` returns. Catches a drop that only cancels.
#[test]
fn sim_tui_worker_drop_joins_the_session_before_it_returns() -> Result<(), String> {
    let (alive_tx, alive) = sync_channel::<bool>(1);
    let (asked_tx, asked) = sync_channel::<()>(1);
    let worker = Worker::start(false, move |relay: &mut Relay| -> Outcome {
        let held = alive_tx;
        let told = asked_tx.send(()).is_ok();
        let answer = relay.approve(0, "ls");
        assert!(told, "the test went away before the question");
        // The drop cancels before it closes the answer channel, so the woken session reads
        // the cancel at once, before any later send of its own could set it.
        assert!(
            held.try_send(relay.cancelled()).is_ok(),
            "the test reads the end, not a send"
        );
        assert!(
            matches!(answer, crate::executor::Approval::Decline),
            "a dropped frame declines"
        );
        Ok(Stop::EndTurn)
    })
    .map_err(|cause| cause.to_string())?;
    asked
        .recv_timeout(PATIENCE)
        .map_err(|cause| format!("the session never asked: {cause}"))?;
    drop(worker);
    assert_eq!(
        alive.try_recv(),
        Ok(true),
        "the session ran to its decline and saw the cancel"
    );
    assert_eq!(
        alive.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected),
        "the thread had ended when the drop returned"
    );
    Ok(())
}

/// A session that ends its turn says `Ended::Done` once and nothing after it, and its `finish`
/// names no panic; a session that panics before its first event is reaped by `finish`, which
/// names the panic, so a quit with a broken session reports it as `TuiError::Session`.
#[test]
fn sim_tui_worker_finish_names_a_panic_and_only_a_panic() -> Result<(), String> {
    let mut clean = Worker::start(true, |relay: &mut Relay| -> Outcome {
        relay.message("all good");
        Ok(Stop::EndTurn)
    })
    .map_err(|cause| cause.to_string())?;
    assert_eq!(
        clean.wait(PATIENCE),
        Some(Event::Said(String::from("all good")))
    );
    assert_eq!(clean.wait(PATIENCE), Some(Event::Ended(Ended::Done)));
    assert_eq!(clean.wait(PATIENCE), None, "a clean end adds no failure");
    assert_eq!(clean.finish(), None, "a clean session names no panic");

    let broken = Worker::start(false, |_relay: &mut Relay| -> Outcome {
        assert!(!std::hint::black_box(true), "the step broke before a word");
        Ok(Stop::EndTurn)
    })
    .map_err(|cause| cause.to_string())?;
    let why = broken
        .finish()
        .ok_or("finish named no panic for a session that panicked")?;
    assert_eq!(
        why,
        "the session stopped on a panic: the step broke before a word"
    );
    let quit = TuiError::Session(why);
    assert_eq!(
        quit.to_string(),
        "session: the session stopped on a panic: the step broke before a word"
    );
    assert!(
        std::error::Error::source(&quit).is_none(),
        "a panic has no source error"
    );
    Ok(())
}
