//! Seeded simulations of the group wait's one deadline (larry#268).
//!
//! Requirement proved: both group waits end on one reading of the clock their caller
//! injected, never on a count of yields, so the same seed gives the same trace on a loaded
//! host and on an idle one. Source: `kill(2)` and `wait(2)` on a process group (POSIX), and
//! the Darwin `EPERM` refusal of a group whose remaining members are zombies (larry#131).
//!
//! A virtual clock moves only when this file advances it, so a wait that ended early, late
//! or on the probe count is visible in the trace instead of hidden behind the scheduler.

use std::io;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;
use std::sync::mpsc;
use std::time::Duration;

use lgwks_bot::kernel::clock::{Clock, ClockError};
use lgwks_macros::Error;
use lgwks_std::seeded::{DrawError, Seeded};

use super::{ExecError, GROUP_PROBE_DEADLINE, await_gone, settle_with, status_after_kill};

/// What every test here returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Seeds each group-wait simulation sweeps.
const SEEDS: u64 = 64;

/// The largest virtual step a seed draws, in milliseconds: small enough that a wait walks to
/// the 5 s bound in a few thousand probes, and never zero, so time always moves.
const MAX_STEP_MS: u64 = 50;

/// The most refusals or still-there probes a seed draws.
const MAX_REFUSALS: u64 = 600;

/// The kernel's error number for "no such process", as [`super::ESRCH`].
const ESRCH: i32 = super::ESRCH;

/// The kernel's error number for "operation not permitted", as [`super::EPERM`].
const EPERM: i32 = super::EPERM;

/// The kernel's error number for "input/output error": here, a refusal that is neither the
/// group being gone nor a Darwin zombie, so it is typed rather than retried.
const EIO: i32 = 5;

/// Why one group-wait simulation could not be driven to its assertions.
#[derive(Debug, Error)]
enum GroupWaitSimError {
    /// The seeded stream refused a draw.
    #[error("seeded draw refused: {0}")]
    Draw(#[from] DrawError),
    /// A drawn number does not fit the type its use needs.
    #[error("drawn number out of range: {0}")]
    Range(#[from] std::num::TryFromIntError),
    /// A drawn multiplication left the range a duration holds.
    #[error("a drawn duration overflowed")]
    Overflow,
    /// The virtual clock refused the step this simulation asked of it.
    #[error("the virtual clock refused a step: {0}")]
    Clock(#[from] ClockError),
    /// The leader's status could not be queued for the wait.
    #[error("the status could not be queued: {0}")]
    Send(#[from] mpsc::SendError<io::Result<ExitStatus>>),
    /// The wait answered with the waiter's own fault where a status was expected.
    #[error("the waiter's own fault: {0}")]
    Status(#[source] io::Error),
    /// The wait refused with an error this simulation does not expect.
    #[error("the wait refused: {0}")]
    Exec(#[source] ExecError),
}

/// The 5 s bound in whole milliseconds, compared the way the wait reads it.
fn bound_ms() -> Result<u64, std::num::TryFromIntError> {
    u64::try_from(GROUP_PROBE_DEADLINE.as_millis())
}

/// A seed's virtual clock placement and step: time starts at zero and moves one step per probe.
fn clock_and_step(seeded: &mut Seeded) -> Result<(Clock, Duration, u64), GroupWaitSimError> {
    let step_ms = seeded.below(MAX_STEP_MS)?.saturating_add(1);
    Ok((
        Clock::virtual_at(Duration::ZERO),
        Duration::from_millis(step_ms),
        step_ms,
    ))
}

/// BLAKE3 of a trace, as 64 hex characters: one seed must give one trace hash.
fn trace_hash(trace: &str) -> String {
    let mut hasher = lgwks_std::hash::Hasher::new();
    hasher.update(trace.as_bytes());
    hasher.finalize().to_hex()
}

/// The group-gone wait for one seed: a probe that reports the group still there for the drawn
/// number of probes, then gone. Returns the trace of what the oracle observed.
fn gone_trace(seed: u64) -> Result<String, GroupWaitSimError> {
    let mut seeded = Seeded::from_seed(seed);
    let (clock, step, step_ms) = clock_and_step(&mut seeded)?;
    let there_for = seeded.below(MAX_REFUSALS)?;
    let elapsed_ms = there_for
        .checked_mul(step_ms)
        .ok_or(GroupWaitSimError::Overflow)?;
    // The rule, worked out here and not by the wait: after each still-there probe the wait
    // asks the clock, so the group is found gone only while the reading before that probe is
    // inside the bound.
    let expected = elapsed_ms < bound_ms()?;
    let mut probes = 0_u64;
    // The probe answers the way `process_group_exists` does: `true` means still there.
    let outcome = await_gone(&clock, || -> Result<bool, ClockError> {
        probes = probes.saturating_add(1);
        clock.advance(step)?;
        Ok(probes <= there_for)
    })?;
    assert_eq!(
        outcome,
        expected,
        "seed {seed}: {there_for} still-there probes of {step_ms} ms is {} the 5 s bound",
        if expected { "inside" } else { "past" }
    );
    let reached = step
        .checked_mul(u32::try_from(probes)?)
        .ok_or(GroupWaitSimError::Overflow)?;
    assert_eq!(
        clock.now(),
        reached,
        "seed {seed}: the wait read the clock once per probe and advanced it by one step"
    );
    if outcome {
        assert_eq!(
            probes,
            there_for.saturating_add(1),
            "seed {seed}: the wait ends on the first probe that finds the group gone"
        );
        let before = clock.now().saturating_sub(step);
        assert!(
            before < GROUP_PROBE_DEADLINE,
            "seed {seed}: the wait gives up before it can see the group gone, read {before:?}"
        );
    } else {
        let before = clock.now().saturating_sub(step);
        assert!(
            clock.now() >= GROUP_PROBE_DEADLINE && before < GROUP_PROBE_DEADLINE,
            "seed {seed}: the wait gives up at the first reading past the bound, read {:?}",
            clock.now()
        );
    }
    Ok(format!(
        "there_for={there_for} step_ms={step_ms} probes={probes} elapsed_ms={} still_there={}",
        clock.now().as_millis(),
        !outcome
    ))
}

/// The group-gone wait ends on the injected clock, never on a count of yields: for every seed,
/// a group gone before the bound returns `true` inside it, and a group still there at the
/// bound returns `false` at the first reading past it (larry#268).
#[test]
fn sim_the_group_gone_wait_ends_on_the_injected_clock() -> TestResult {
    for seed in 0..SEEDS {
        let first = trace_hash(&gone_trace(seed)?);
        let second = trace_hash(&gone_trace(seed)?);
        assert_eq!(first, second, "seed {seed}: one seed gives one trace hash");
    }
    Ok(())
}

/// A group that never goes away is still bounded: the wait stops at the first reading past the
/// bound even when the probe never reports it gone (larry#268).
#[test]
fn sim_a_group_that_never_goes_away_stops_at_the_bound() -> TestResult {
    for seed in 0..SEEDS {
        let mut seeded = Seeded::from_seed(seed.wrapping_add(1 << 40));
        let (clock, step, _step_ms) = clock_and_step(&mut seeded)?;
        let outcome = await_gone(&clock, || -> Result<bool, ClockError> {
            clock.advance(step)?;
            Ok(true)
        })?;
        assert!(!outcome, "seed {seed}: the group never went away");
        let before = clock.now().saturating_sub(step);
        assert!(
            clock.now() >= GROUP_PROBE_DEADLINE && before < GROUP_PROBE_DEADLINE,
            "seed {seed}: the wait stops at the first reading past the bound, read {:?}",
            clock.now()
        );
    }
    Ok(())
}

/// Longest pause the backoff may sleep between two probes on a real clock.
const PAUSE_MAX: Duration = super::PROBE_PAUSE_MAX;

/// Most probes a group that never goes away may cost across the whole 5 s bound: 500 pauses at
/// the cap plus the doubling steps below it. The yield loop this replaces spun millions.
const MAX_PROBES: u64 = 520;

/// The group waits pause on a bounded backoff on a real clock, never a yield loop (larry#427):
/// walking the schedule a wall-clock wait sleeps, a group that never goes away costs at most
/// [`MAX_PROBES`] probes over the 5 s bound and no pause passes the deadline left, and for every
/// seed a group gone at a drawn instant is seen no later than one capped pause after it and no
/// later than its own age plus the first pause. Source: crossbeam `Backoff` (spin, then park),
/// and the 1 to 2 vCPU budget host the waiter competes on.
#[test]
fn sim_the_group_wait_backoff_probes_a_bounded_number_of_times() -> TestResult {
    let mut waited = Duration::ZERO;
    let mut probes = 0_u64;
    let mut attempt = 0_u32;
    while waited < GROUP_PROBE_DEADLINE {
        let left = GROUP_PROBE_DEADLINE.saturating_sub(waited);
        let pause = super::probe_pause(attempt, left);
        assert!(
            pause <= PAUSE_MAX,
            "attempt {attempt}: {pause:?} passes the cap"
        );
        assert!(
            pause <= left,
            "attempt {attempt}: {pause:?} passes the deadline"
        );
        assert!(!pause.is_zero(), "attempt {attempt}: a zero pause spins");
        waited = waited.saturating_add(pause);
        probes = probes.saturating_add(1);
        attempt = attempt.saturating_add(1);
    }
    assert!(
        probes <= MAX_PROBES,
        "a group that never goes away cost {probes} probes over the bound"
    );
    assert_eq!(
        waited, GROUP_PROBE_DEADLINE,
        "the last pause ends on the bound"
    );
    for seed in 0..SEEDS {
        let mut seeded = Seeded::from_seed(seed.wrapping_add(3 << 40));
        let gone_at = Duration::from_micros(seeded.below(4_000_000)?);
        let (mut at, mut attempt) = (Duration::ZERO, 0_u32);
        while at < gone_at {
            at = at.saturating_add(super::probe_pause(attempt, Duration::MAX));
            attempt = attempt.saturating_add(1);
        }
        let lag = at.saturating_sub(gone_at);
        assert!(
            lag <= PAUSE_MAX,
            "seed {seed}: gone at {gone_at:?}, seen {lag:?} late"
        );
        assert!(
            lag <= gone_at.saturating_add(super::PROBE_PAUSE_FIRST),
            "seed {seed}: gone at {gone_at:?}, seen {lag:?} late, past its age plus one pause"
        );
    }
    Ok(())
}

/// The signal schedules one seed drives, each asserted against the rule and returned as a trace.
fn settle_trace(seed: u64) -> Result<String, GroupWaitSimError> {
    let mut seeded = Seeded::from_seed(seed.wrapping_add(2 << 40));
    let (_, step, step_ms) = clock_and_step(&mut seeded)?;
    let refusals = seeded.below(MAX_REFUSALS)?;
    let refusals_ms = refusals
        .checked_mul(step_ms)
        .ok_or(GroupWaitSimError::Overflow)?;
    let inside = refusals_ms < bound_ms()?;

    // The Darwin zombie: EPERM for the drawn number of refusals, then the signal lands.
    let clock = Clock::virtual_at(Duration::ZERO);
    let mut calls = 0_u64;
    let landed = settle_with(&clock, || {
        calls = calls.saturating_add(1);
        if calls > refusals {
            return Ok(());
        }
        clock.advance(step).map(drop).map_err(io::Error::other)?;
        Err(io::Error::from_raw_os_error(EPERM))
    });
    assert!(
        landed.is_ok(),
        "seed {seed}: an EPERM retry still settles the group"
    );
    if inside {
        assert_eq!(
            calls,
            refusals.saturating_add(1),
            "seed {seed}: the retry reaches the group as soon as the refusals end"
        );
        assert!(
            clock.now() < GROUP_PROBE_DEADLINE,
            "seed {seed}: the retry lands inside the bound"
        );
    } else {
        let before = clock.now().saturating_sub(step);
        assert!(
            clock.now() >= GROUP_PROBE_DEADLINE && before < GROUP_PROBE_DEADLINE,
            "seed {seed}: a refusal past the bound stops at the first reading past it, read {:?}",
            clock.now()
        );
    }
    let first_calls = calls;
    let first_ms = clock.now().as_millis();

    // A refusal that outlasts the bound is handed to the group-gone probe, never retried past it.
    let clock = Clock::virtual_at(Duration::ZERO);
    let mut calls = 0_u64;
    let outlasted = settle_with(&clock, || {
        calls = calls.saturating_add(1);
        clock.advance(step).map(drop).map_err(io::Error::other)?;
        Err(io::Error::from_raw_os_error(EPERM))
    });
    assert!(
        outlasted.is_ok(),
        "seed {seed}: a refusal at the bound is not a typed failure"
    );
    assert!(
        clock.now() >= GROUP_PROBE_DEADLINE,
        "seed {seed}: the outlasted refusal runs exactly to the bound"
    );
    let outlasted_calls = calls;
    let outlasted_ms = clock.now().as_millis();

    // A group the kernel says is gone settles on the first signal.
    let clock = Clock::virtual_at(Duration::ZERO);
    let mut calls = 0_u64;
    let gone = settle_with(&clock, || {
        calls = calls.saturating_add(1);
        Err(io::Error::from_raw_os_error(ESRCH))
    });
    assert!(gone.is_ok(), "seed {seed}: a gone group is settled");
    assert_eq!(calls, 1, "seed {seed}: a gone group is not signalled twice");
    assert_eq!(
        clock.now(),
        Duration::ZERO,
        "seed {seed}: a gone group costs no time"
    );
    let gone_calls = calls;

    // Any other refusal is typed, never retried and never presumed settled.
    let clock = Clock::virtual_at(Duration::ZERO);
    let refused = settle_with(&clock, || Err(io::Error::from_raw_os_error(EIO)));
    assert!(
        matches!(refused, Err(ExecError::Reap(ref cause)) if cause.raw_os_error() == Some(EIO)),
        "seed {seed}: a refusal that is neither EPERM nor ESRCH is the typed reap error"
    );
    Ok(format!(
        "refusals={refusals} calls={first_calls} elapsed_ms={first_ms} outlasted_calls={outlasted_calls} outlasted_ms={outlasted_ms} gone_calls={gone_calls} refused=reap"
    ))
}

/// The pid the status wait is told its group is: nothing is signalled, so any positive id serves.
const SIM_PGID: i32 = 4242;

/// The wait status of a leader a SIGKILL ended: a signal, so no exit code.
const KILLED: i32 = 9;

/// The real time one slice of the status wait sleeps here: tiny, so a seed that re-signals fifty
/// times costs milliseconds, while the virtual clock alone decides the bound.
const SLICE: Duration = Duration::from_micros(50);

/// The most re-signals a seed draws before its group exists: past the 26 to 50 a step of 100 to
/// 199 ms fits in the bound, so some seeds land the kill and some give up.
const MAX_ABSENT: u64 = 100;

/// One seed's status wait: the group does not exist for `absent` signals (the `setsid` that makes
/// it has not run), the signal that finds it lands the kill, and the leader's status follows. The
/// rule is worked out here from the bound alone: the wait signals once per slice and gives up at
/// the first reading past the bound, so the kill lands when `absent` re-signals fit inside it.
fn status_trace(seed: u64) -> Result<String, GroupWaitSimError> {
    let mut seeded = Seeded::from_seed(seed.wrapping_add(4 << 40));
    let step_ms = seeded.below(100)?.saturating_add(100);
    let step = Duration::from_millis(step_ms);
    let absent = seeded.below(MAX_ABSENT)?;
    let fits = bound_ms()?.div_ceil(step_ms);
    let lands = absent <= fits;
    let clock = Clock::virtual_at(Duration::ZERO);
    let (sender, receiver) = mpsc::sync_channel(1);
    let killed = ExitStatus::from_raw(KILLED);
    if absent == 0 {
        sender.send(Ok(killed))?;
    }
    let mut calls = 0_u64;
    let outcome = status_after_kill(&clock, &receiver, SIM_PGID, SLICE, || {
        calls = calls.saturating_add(1);
        clock
            .advance(step)
            .map(drop)
            .map_err(|cause| ExecError::Reap(io::Error::other(cause)))?;
        if calls == absent {
            sender
                .send(Ok(killed))
                .map_err(|cause| ExecError::Reap(io::Error::other(cause)))?;
        }
        Ok(())
    });
    match outcome {
        Ok(Ok(status)) => {
            assert!(lands, "seed {seed}: {absent} re-signals are past the bound");
            assert_eq!(
                status.signal(),
                Some(KILLED),
                "seed {seed}: the kill's status"
            );
            assert_eq!(
                calls, absent,
                "seed {seed}: one signal per slice until it landed"
            );
        }
        Ok(Err(cause)) => return Err(GroupWaitSimError::Status(cause)),
        Err(ExecError::GroupSurvived { pgid }) => {
            assert!(!lands, "seed {seed}: {absent} re-signals fit the bound");
            assert_eq!(pgid, SIM_PGID, "seed {seed}: the refusal names the group");
            assert_eq!(
                calls, fits,
                "seed {seed}: it gives up at the first reading past the bound"
            );
            assert!(
                clock.now() >= GROUP_PROBE_DEADLINE,
                "seed {seed}: it gives up only once the bound has passed"
            );
        }
        Err(other) => return Err(GroupWaitSimError::Exec(other)),
    }
    Ok(format!(
        "absent={absent} step_ms={step_ms} calls={calls} lands={lands} elapsed_ms={}",
        clock.now().as_millis()
    ))
}

/// A cancel or a deadline that kills before the command's group exists is not the end of the
/// run: the Linux `setsid` wrapper makes the group after `spawn` returns, so the first signal can
/// find nothing (`ESRCH` settles it), and the wait signals again each slice until the leader's
/// status arrives, or gives up typed at the bound on the injected clock (larry#442). Seeded over
/// 64 groups that appear after 0 to 99 signals; one seed gives one trace hash.
#[test]
fn sim_a_group_not_yet_made_is_signalled_again_until_the_status_arrives() -> TestResult {
    for seed in 0..SEEDS {
        let first = trace_hash(&status_trace(seed)?);
        let second = trace_hash(&status_trace(seed)?);
        assert_eq!(first, second, "seed {seed}: one seed gives one trace hash");
    }
    Ok(())
}

/// A waiter that ended without a status is its own fault and is never answered with a signal.
#[test]
fn sim_a_leader_whose_waiter_ended_reports_the_fault_and_signals_nothing() -> TestResult {
    let clock = Clock::virtual_at(Duration::ZERO);
    let (sender, receiver) = mpsc::sync_channel::<io::Result<ExitStatus>>(1);
    drop(sender);
    let mut calls = 0_u64;
    let outcome = status_after_kill(&clock, &receiver, SIM_PGID, SLICE, || {
        calls = calls.saturating_add(1);
        Ok(())
    })?;
    assert!(outcome.is_err(), "a closed channel is the waiter's fault");
    assert_eq!(calls, 0, "nothing is signalled for a waiter that is gone");
    Ok(())
}

/// The EPERM retry ends on the injected clock, never on a count of yields: for every seed the
/// retry lands inside the bound when the refusals end there, stops at the first reading past
/// the bound when they do not, and a gone group or another refusal is answered at once
/// (larry#268, larry#131).
#[test]
fn sim_the_eperm_retry_ends_on_the_injected_clock() -> TestResult {
    for seed in 0..SEEDS {
        let first = trace_hash(&settle_trace(seed)?);
        let second = trace_hash(&settle_trace(seed)?);
        assert_eq!(first, second, "seed {seed}: one seed gives one trace hash");
    }
    Ok(())
}
