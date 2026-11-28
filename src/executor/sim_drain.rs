//! Seeded simulations of the bounded output drain (larry#334).
//!
//! Requirement proved: once the command's group is settled, a stream still yielding bytes is
//! read until exactly `DRAIN_GRACE` has passed on the injected clock and then reported open; a
//! stream that ends is whole; an interrupted read is retried and any other read fault is the
//! drain's answer. Source: `read(2)` (`EINTR`, `EAGAIN`) and `setsid(2)`, the call that lets a
//! child leave its group with the pipe.
//!
//! The clock moves only when a scripted read moves it, so the grace boundary is exact.

use std::cell::Cell;
use std::io;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use lgwks_std::seeded::Seeded;

use super::{DRAIN_GRACE, Drained, drain};

/// Fault injection through the run's other seams (larry#575).
#[path = "sim_run_faults.rs"]
mod faults;

/// What every test here returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Seeds the grace simulation sweeps.
const SEEDS: u64 = 64;

/// Steps every run also takes: one millisecond, a divisor of the grace, the grace itself and
/// one past it, so the boundary `at == until` is always exercised.
const FIXED_STEPS_MS: [u64; 4] = [1, 250, 2_000, 2_001];

/// How many one-byte reads a settled drain takes before it reports the stream open: the reads
/// issued while the clock, read before each, is still short of the grace.
fn reads_inside_grace(step_ms: u64) -> Result<u64, std::num::TryFromIntError> {
    let grace_ms = u64::try_from(DRAIN_GRACE.as_millis())?;
    Ok(grace_ms.div_ceil(step_ms))
}

/// A settled drain over a stream that never ends reads exactly until the grace has passed on
/// the injected clock, then reports the stream open with every byte it read counted.
fn settled_trace(step_ms: u64) -> TestResult {
    let clock = Cell::new(Duration::ZERO);
    let step = Duration::from_millis(step_ms);
    let settled = AtomicBool::new(true);
    let mut reads = 0_u64;
    let drained = drain(
        |chunk| {
            reads = reads.saturating_add(1);
            clock.set(clock.get().saturating_add(step));
            let first = chunk
                .first_mut()
                .ok_or_else(|| io::Error::other("an empty chunk"))?;
            *first = b'x';
            Ok(1)
        },
        || clock.get(),
        &settled,
    )?;
    let expected = reads_inside_grace(step_ms)?;
    assert!(
        drained.open,
        "step {step_ms} ms: a stream that never ends is open"
    );
    assert_eq!(
        reads, expected,
        "step {step_ms} ms: the drain reads while the clock is short of the grace and stops at it"
    );
    assert_eq!(
        drained.captured.bytes, expected,
        "step {step_ms} ms: every byte read is counted"
    );
    Ok(())
}

/// The grace ends at the first clock reading at or past it, for every seed's step and for the
/// steps that land exactly on it (larry#334).
#[test]
fn sim_a_settled_stream_that_keeps_writing_is_read_for_exactly_the_grace() -> TestResult {
    for step_ms in FIXED_STEPS_MS {
        settled_trace(step_ms)?;
    }
    for seed in 0..SEEDS {
        let mut seeded = Seeded::from_seed(seed);
        settled_trace(seeded.below(400)?.saturating_add(1))?;
    }
    Ok(())
}

/// A settled stream that is silent for a while and then ends is whole: a sibling command forked
/// between this pair's creation and its own `exec` holds a copy of the writer, and on a loaded
/// host it can hold it past many read timeouts (larry#454). Silence ends the drain only once the
/// grace has passed on the clock, exactly as bytes do.
#[test]
fn sim_a_settled_stream_silent_inside_the_grace_is_whole_when_it_ends() -> TestResult {
    for seed in 0..SEEDS {
        let mut seeded = Seeded::from_seed(seed);
        let step_ms = seeded.below(400)?.saturating_add(1);
        let inside = reads_inside_grace(step_ms)?;
        // Silent for up to every read inside the grace but the last two, then three bytes and the
        // end, both read inside it.
        let silent = seeded.below(inside.saturating_sub(1))?;
        let clock = Cell::new(Duration::ZERO);
        let settled = AtomicBool::new(true);
        let mut reads = 0_u64;
        let drained = drain(
            |chunk| {
                reads = reads.saturating_add(1);
                clock.set(clock.get().saturating_add(Duration::from_millis(step_ms)));
                match reads.cmp(&silent.saturating_add(1)) {
                    std::cmp::Ordering::Less => Err(io::Error::from(io::ErrorKind::WouldBlock)),
                    std::cmp::Ordering::Equal => Ok(3_usize.min(chunk.len())),
                    std::cmp::Ordering::Greater => Ok(0),
                }
            },
            || clock.get(),
            &settled,
        )?;
        assert!(
            !drained.open,
            "seed {seed}: a stream that ended inside the grace is whole"
        );
        assert_eq!(drained.captured.bytes, 3, "seed {seed}: its bytes are kept");
        // Silent past the grace: reported open after exactly the reads inside it.
        let clock = Cell::new(Duration::ZERO);
        let mut quiet = 0_u64;
        let open = drain(
            |_chunk| {
                quiet = quiet.saturating_add(1);
                clock.set(clock.get().saturating_add(Duration::from_millis(step_ms)));
                Err(io::Error::from(io::ErrorKind::WouldBlock))
            },
            || clock.get(),
            &settled,
        )?;
        assert!(
            open.open,
            "seed {seed}: a writer silent past the grace is reported open"
        );
        assert_eq!(
            quiet, inside,
            "seed {seed}: silence is read until the grace, no longer"
        );
    }
    Ok(())
}

/// One scripted read: bytes, a would-block, an interruption, or a fault.
enum Step {
    /// Yields this many bytes.
    Bytes(usize),
    /// Times out with nothing read.
    WouldBlock,
    /// Is interrupted by a signal.
    Interrupted,
    /// Fails for another reason.
    Fault,
}

/// Drains a script, never settled; the stream ends when the script runs out.
fn scripted(script: &[Step]) -> io::Result<Drained> {
    let settled = AtomicBool::new(false);
    let mut next = script.iter();
    drain(
        |chunk| match next.next() {
            Some(&Step::Bytes(count)) => Ok(count.min(chunk.len())),
            Some(&Step::WouldBlock) => Err(io::Error::from(io::ErrorKind::WouldBlock)),
            Some(&Step::Interrupted) => Err(io::Error::from(io::ErrorKind::Interrupted)),
            Some(&Step::Fault) => Err(io::Error::other("the socket failed")),
            None => Ok(0),
        },
        || Duration::ZERO,
        &settled,
    )
}

/// Before the group settles, an interrupted read and a timed-out read are retried and the
/// stream is whole at its end; any other read fault is the drain's own answer (larry#334).
#[test]
fn sim_an_unsettled_drain_retries_eintr_and_eagain_and_returns_a_fault() -> TestResult {
    let whole = scripted(&[
        Step::Interrupted,
        Step::Bytes(3),
        Step::WouldBlock,
        Step::Bytes(4),
    ])?;
    assert!(!whole.open, "a stream that ended is whole");
    assert_eq!(
        whole.captured.bytes, 7,
        "both reads around the retries are kept"
    );
    let faulted = scripted(&[Step::Bytes(3), Step::Fault, Step::Bytes(4)]);
    assert!(
        faulted.is_err(),
        "a read fault that is neither EINTR nor EAGAIN is returned, never skipped"
    );
    Ok(())
}
