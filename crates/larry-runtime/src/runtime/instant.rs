//! The tick's one observed instant, sampled from the estate's declared clock (E30, larry#74).
//!
//! Larry keeps no clock: the time source is `lgwks_bot::clock::Clock`. Its readings carry no
//! epoch, so the convention is fixed here, once: a reading is elapsed time since
//! 1970-01-01T00:00Z. A virtual clock placed at a recorded reading therefore replays the same
//! instants on any host, and a live host builds its clock with [`live_clock`], which places
//! `Clock::wall_at` at the reading now and then follows monotonic time, so a tick never pins an
//! instant earlier than the one before it. Placing the clock is what anchors it: `Clock::wall`
//! starts at zero, so a tick on it would pin 1970 plus uptime while an authority expiry sat in
//! the present (larry#182).
//!
//! `lgwks_bot` 8.4.0 also ships `EpochClock` and `RecordedInstant` (logicalworks-crates#845,
//! closed); the runtime keeps `Clock` because the tick, the gate and the fence all consume it,
//! and the live placement here is the whole gap that needed closing (ADR 0016).

use std::time::SystemTime;

use lgwks_bot::clock::Clock;

use crate::error::RuntimeError;

/// The clock's reading placed after the Unix epoch.
///
/// # Errors
///
/// [`RuntimeError::ClockOutOfRange`] when the platform's `SystemTime` cannot hold the reading.
pub(super) fn sample(clock: &Clock) -> Result<SystemTime, RuntimeError> {
    let elapsed = clock.now();
    SystemTime::UNIX_EPOCH
        .checked_add(elapsed)
        .ok_or(RuntimeError::ClockOutOfRange { elapsed })
}

/// The live clock under the runtime's convention: a reading is elapsed time since
/// 1970-01-01T00:00Z, so a tick's instant and an authority expiry share one timeline.
///
/// This is the only production constructor for a live runtime clock. [`Clock::wall`] starts at
/// zero, and pairing it with an expiry read from `SystemTime::now` put the lease about 56 years
/// away (larry#182).
///
/// # Errors
///
/// [`RuntimeError::ClockBeforeEpoch`] when the system clock reads before the Unix epoch, so no
/// reading can be placed after it.
pub fn live_clock() -> Result<Clock, RuntimeError> {
    let elapsed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(RuntimeError::ClockBeforeEpoch)?;
    Ok(Clock::wall_at(elapsed))
}

#[cfg(test)]
mod tests {
    use super::{Clock, RuntimeError, SystemTime, live_clock, sample};
    use std::time::Duration;

    /// A virtual clock placed at a reading pins exactly that instant after the epoch, a read
    /// never moves it, and an advance moves the next sample by exactly the advance.
    #[test]
    fn a_sample_is_the_reading_placed_after_the_epoch() -> Result<(), Box<dyn std::error::Error>> {
        let placed = Duration::from_secs(1_700_000_000);
        let clock = Clock::virtual_at(placed);
        let first = sample(&clock)?;
        assert_eq!(
            first.duration_since(SystemTime::UNIX_EPOCH)?,
            placed,
            "the placed reading"
        );
        assert_eq!(
            sample(&clock)?,
            first,
            "a read does not move a virtual clock"
        );
        clock.advance(Duration::from_secs(60))?;
        let second = sample(&clock)?;
        assert_eq!(
            second.duration_since(first)?,
            Duration::from_secs(60),
            "exactly one advance"
        );
        Ok(())
    }

    /// Code once (larry#68 row 1): the clock lives in lgwks_bot, so this crate declares no clock
    /// module and no `Clock` trait of its own, and a second copy fails here before it ships.
    #[test]
    fn larry_keeps_no_clock_of_its_own() -> Result<(), Box<dyn std::error::Error>> {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        assert!(!src.join("clock.rs").exists(), "src/clock.rs is back");
        assert!(!src.join("clock").exists(), "src/clock/ is back");
        let lib = std::fs::read_to_string(src.join("lib.rs"))?;
        assert!(
            !lib.contains("mod clock"),
            "lib.rs declares a clock module again"
        );
        let runtime = std::fs::read_to_string(src.join("runtime.rs"))?;
        assert!(
            runtime.contains("clock: Clock") && runtime.contains("use lgwks_bot::clock::Clock;"),
            "the runtime's clock field is lgwks_bot's clock"
        );
        Ok(())
    }

    /// The live clock is anchored after 2023, not at 1970: a zero-origin `Clock::wall` reads
    /// 1970 plus uptime, which is the larry#182 defect. The typed refusal carries its source.
    #[test]
    fn a_live_clock_is_anchored_at_the_epoch_not_zero() -> Result<(), Box<dyn std::error::Error>> {
        let sampled = sample(&live_clock()?)?;
        let anchored = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(1_700_000_000))
            .ok_or("anchor instant overflow")?;
        assert!(
            sampled > anchored,
            "the live clock reads after 2023, not 1970: {sampled:?}"
        );
        let before = SystemTime::UNIX_EPOCH
            .checked_sub(Duration::from_secs(1))
            .ok_or("the platform cannot represent an instant before the epoch")?;
        let cause = match before.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(_) => return Err("a before-epoch instant must not subtract cleanly".into()),
            Err(cause) => cause,
        };
        let refusal = RuntimeError::ClockBeforeEpoch(cause);
        assert!(
            refusal.to_string().contains("before the Unix epoch"),
            "the refusal names the epoch: {refusal}"
        );
        Ok(())
    }

    /// The clock's ceiling is still an instant this platform holds, and the refusal names the
    /// reading it could not place.
    #[test]
    fn the_clock_ceiling_places_and_the_refusal_names_its_reading()
    -> Result<(), Box<dyn std::error::Error>> {
        let ceiling = Clock::elapsed_ceiling();
        let at = sample(&Clock::virtual_at(ceiling))?;
        assert_eq!(
            at.duration_since(SystemTime::UNIX_EPOCH)?,
            ceiling,
            "the ceiling is placed"
        );
        let refusal = RuntimeError::ClockOutOfRange { elapsed: ceiling };
        assert!(
            refusal.to_string().contains(&format!("{ceiling:?}")),
            "the refusal names its reading: {refusal}"
        );
        Ok(())
    }
}
