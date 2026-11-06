//! Test-only fixtures shared by this crate's unit tests (authority, replay).

use std::time::{Duration, SystemTime};

/// `secs` seconds after the Unix epoch: the pinned instants the contract
/// tests reason about.
pub(crate) fn instant(secs: u64) -> SystemTime {
    let duration = Duration::from_secs(secs);
    let instant = match SystemTime::UNIX_EPOCH.checked_add(duration) {
        Some(instant) => instant,
        // Never taken for the second counts these tests use; the assertion
        // below names the fixture if a platform ever cannot represent one.
        None => SystemTime::UNIX_EPOCH,
    };
    assert_eq!(
        instant
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|error| error.to_string()),
        Ok(duration),
        "fixture instant must be representable"
    );
    instant
}
