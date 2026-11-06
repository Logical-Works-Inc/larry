//! The pinned time source.
//!
//! E30/D36 require that expiry is a *pinned input*: an expired approval is a
//! typed denial, never consent by timeout, and a worker's own observation
//! instant can never extend a lease. The runtime therefore never reads the
//! wall clock from inside a decision. It samples one instant per tick from a
//! [`Clock`] and threads that value through every authorization made in that
//! tick.
//!
//! Replay uses a [`FixedClock`]; [`SystemClock`] exists for a live host and
//! must never be used to re-derive a recorded decision.

use std::time::{Duration, SystemTime};

/// A source of the observed instant.
pub trait Clock {
    /// The current observed instant.
    fn now(&self) -> SystemTime;
}

/// A deterministic clock: a fixed instant advanced by an explicit step.
///
/// This is the clock every test and every replay uses. Identical start and
/// step always yield an identical instant sequence, which is what makes a
/// replayed authorization reproducible.
#[derive(Debug, Clone)]
pub struct FixedClock {
    /// Time returned by the next observation.
    now: SystemTime,
    /// Deterministic increment per observation.
    step: Duration,
}

impl FixedClock {
    /// A clock pinned at `at` that does not advance.
    #[must_use]
    pub const fn new(at: SystemTime) -> Self {
        Self {
            now: at,
            step: Duration::ZERO,
        }
    }

    /// A clock pinned at `at` that advances by `step` on every
    /// [`FixedClock::advance`].
    #[must_use]
    pub const fn with_step(at: SystemTime, step: Duration) -> Self {
        Self { now: at, step }
    }

    /// The instant this clock currently reports, without advancing.
    #[must_use]
    pub const fn peek(&self) -> SystemTime {
        self.now
    }

    /// Advances the clock by its step and returns the new instant.
    pub fn advance(&mut self) -> SystemTime {
        // A step past the platform's last representable instant holds the clock
        // there: time never runs backwards and never wraps.
        if let Some(next) = self.now.checked_add(self.step) {
            self.now = next;
        }
        self.now
    }
}

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.now
    }
}

/// The host wall clock.
///
/// A live runtime samples this once per tick. It must never be the source of
/// a replayed decision: replaying against `SystemClock` cannot be
/// bit-identical, so [`crate::runtime::Runtime::replay`] takes only a
/// journal and never consults this type.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}
