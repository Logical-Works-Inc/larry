//! Seeded simulations of the admission lease on an injected clock (larry#436, SD-4).
//!
//! Requirement proved: the admission expiry is one configured lease past the reading of the
//! clock the gate was given, never past the wall, so a virtual clock placed at any instant gives
//! one expiry and one trace hash on every host. Source: the runtime's own convention that a
//! reading is elapsed time since 1970-01-01T00:00Z (`larry_runtime::live_clock`), and OWASP ASVS
//! 4.0.3 section 4.1.3 (a lease that cannot be replayed on a stopped clock is not testable).

use std::fmt;
use std::num::TryFromIntError;
use std::time::{Duration, SystemTime, SystemTimeError};

use larry_runtime::error::RuntimeError;
use lgwks_bot::kernel::clock::{Clock, ClockError};
use lgwks_std::hash::Hasher;
use lgwks_std::seeded::{DrawError, Seeded};

use super::{default_policy, lease_start, task_settings, with_expiry};

/// What every test here returns.
type TestResult = Result<(), ExpirySimError>;

/// Seeds each simulation sweeps.
const SEEDS: u64 = 64;

/// The latest instant a seed places the clock at, in whole seconds: about the year 2096.
const MAX_START_SECS: u64 = 4_000_000_000;

/// The longest step a seed moves the clock by, in nanoseconds: one hour.
const MAX_STEP_NS: u64 = 3_600_000_000_000;

/// Why one expiry simulation could not be driven to its assertions.
#[derive(Debug)]
enum ExpirySimError {
    /// The seeded stream refused a draw.
    Draw(DrawError),
    /// A drawn number does not fit the type its use needs.
    Range(TryFromIntError),
    /// The virtual clock refused the step this simulation asked of it.
    Clock(ClockError),
    /// The runtime refused to start a lease.
    Lease(RuntimeError),
    /// The expiry moved backwards across the step.
    Backwards(SystemTimeError),
    /// A drawn sum left the range an instant holds.
    Overflow,
}

impl fmt::Display for ExpirySimError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Draw(ref cause) => write!(out, "seeded draw refused: {cause}"),
            Self::Range(ref cause) => write!(out, "drawn number out of range: {cause}"),
            Self::Clock(ref cause) => write!(out, "the virtual clock refused a step: {cause}"),
            Self::Lease(ref cause) => write!(out, "the lease was refused: {cause}"),
            Self::Backwards(ref cause) => write!(out, "the expiry moved backwards: {cause}"),
            Self::Overflow => out.write_str("a drawn instant overflowed"),
        }
    }
}

impl std::error::Error for ExpirySimError {}

impl From<DrawError> for ExpirySimError {
    fn from(cause: DrawError) -> Self {
        Self::Draw(cause)
    }
}

impl From<TryFromIntError> for ExpirySimError {
    fn from(cause: TryFromIntError) -> Self {
        Self::Range(cause)
    }
}

impl From<ClockError> for ExpirySimError {
    fn from(cause: ClockError) -> Self {
        Self::Clock(cause)
    }
}

impl From<RuntimeError> for ExpirySimError {
    fn from(cause: RuntimeError) -> Self {
        Self::Lease(cause)
    }
}

impl From<SystemTimeError> for ExpirySimError {
    fn from(cause: SystemTimeError) -> Self {
        Self::Backwards(cause)
    }
}

/// BLAKE3 of a trace, as 64 hex characters: one seed must give one trace hash.
fn trace_hash(trace: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(trace.as_bytes());
    hasher.finalize().to_hex()
}

/// One seed's admission leases on a virtual clock placed at a drawn instant and then moved by a
/// drawn step: each lease must end exactly one configured lease after the clock's reading, and
/// the move must carry the expiry by exactly the step. Returns the trace of what was observed.
fn expiry_trace(seed: u64) -> Result<String, ExpirySimError> {
    let mut stream = Seeded::from_seed(seed.wrapping_add(9 << 40));
    let start = Duration::new(
        stream.below(MAX_START_SECS)?,
        u32::try_from(stream.below(1_000_000_000)?)?,
    );
    let step = Duration::from_nanos(stream.below(MAX_STEP_NS)?.saturating_add(1));
    let clock = Clock::virtual_at(start);
    let lease = task_settings(&default_policy()).lease;
    let before = with_expiry(task_settings(&default_policy()), lease_start(&clock)?)?;
    let expected = SystemTime::UNIX_EPOCH
        .checked_add(start)
        .and_then(|at| at.checked_add(lease))
        .ok_or(ExpirySimError::Overflow)?;
    assert_eq!(
        before.validity.valid_until, expected,
        "seed {seed}: the lease starts at the clock's reading, not the wall's"
    );
    clock.advance(step)?;
    let after = with_expiry(task_settings(&default_policy()), lease_start(&clock)?)?;
    let moved = after
        .validity
        .valid_until
        .duration_since(before.validity.valid_until)?;
    assert_eq!(
        moved, step,
        "seed {seed}: the expiry follows the clock by exactly the step it moved"
    );
    Ok(format!(
        "start_ns={} step_ns={} lease_ns={} moved_ns={}",
        start.as_nanos(),
        step.as_nanos(),
        lease.as_nanos(),
        moved.as_nanos()
    ))
}

/// The admission expiry is read from the injected clock, never the wall: for every seed the lease
/// is exactly one configured lease past the clock's reading, a moved clock moves the expiry by
/// the same step, and one seed gives one trace hash (larry#436, SD-4).
#[test]
fn sim_the_admission_expiry_reads_the_injected_clock() -> TestResult {
    for seed in 0..SEEDS {
        let first = trace_hash(&expiry_trace(seed)?);
        let second = trace_hash(&expiry_trace(seed)?);
        assert_eq!(first, second, "seed {seed}: one seed gives one trace hash");
    }
    Ok(())
}

/// A reading the platform cannot hold is a typed refusal that names the reading, never a wrapped
/// or clamped instant: the lease is not started on a reading it could not place (larry#436).
/// A platform whose `SystemTime` holds it places it instead, so the case holds on every target.
#[test]
fn sim_a_reading_past_the_platform_range_is_a_typed_refusal() {
    let clock = Clock::virtual_at(Duration::MAX);
    let reading = clock.now();
    let placed = SystemTime::UNIX_EPOCH.checked_add(reading);
    let started = lease_start(&clock);
    let agrees = match (placed, &started) {
        (Some(expected), &Ok(at)) => at == expected,
        (None, &Err(RuntimeError::ClockOutOfRange { elapsed })) => elapsed == reading,
        _ => false,
    };
    assert!(
        agrees,
        "a held reading is placed exactly and an unheld one is refused naming itself: {placed:?} gave {started:?}"
    );
}
