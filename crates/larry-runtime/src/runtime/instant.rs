//! The tick's one observed instant, sampled from the estate's declared clock (E30, larry#74).
//!
//! Larry keeps no clock: the time source is `lgwks_bot::clock::Clock`. That clock carries no
//! epoch (logicalworks-crates#845 asks for one it can record), so the convention is fixed here,
//! once: a reading is elapsed time since 1970-01-01T00:00Z. A virtual clock placed at a recorded
//! reading therefore replays the same instants on any host, and a live host's wall clock
//! (`Clock::wall_at`) follows monotonic time, so a tick never pins an instant earlier than the
//! one before it.

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

#[cfg(test)]
mod tests {
    use super::{Clock, RuntimeError, SystemTime, sample};
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
