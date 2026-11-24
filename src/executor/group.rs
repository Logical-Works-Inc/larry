//! Settling a command's process group: the kill that survives Darwin's zombie `EPERM`, and the
//! wait until the kernel says the group is gone, both bounded by one reading of the clock the
//! caller injected (larry#214, larry#268) and pausing on a backoff, never spinning (larry#427).

use std::io;
use std::time::Duration;

use lgwks_bot::kernel::clock::{Clock, TimeSource};
use lgwks_std::process::{kill_process_group, process_group_exists};

use super::ExecError;

/// The kernel's error number for "no such process".
const ESRCH: i32 = 3;

/// The kernel's error number for "operation not permitted", the same on Linux and Darwin.
const EPERM: i32 = 1;

/// Time a process group is given before it counts as surviving (larry#214, larry#268): killed
/// members are briefly zombies, and a count of yields measured the scheduler rather than time,
/// so both group waits end on this one bound read from the clock their caller injected.
pub(in crate::executor) const GROUP_PROBE_DEADLINE: Duration = Duration::from_secs(5);

/// First pause between two group probes on a real clock (larry#427): a killed group is most
/// often gone within it, so the common wait costs one short sleep and no spin.
const PROBE_PAUSE_FIRST: Duration = Duration::from_micros(100);

/// Longest pause between two group probes on a real clock (larry#427): it bounds how late a
/// gone group is seen, and over the 5 s bound keeps a waiter near 500 probes, where the yield
/// loop it replaced spun a core on a 1 to 2 vCPU host against the processes it waited on.
const PROBE_PAUSE_MAX: Duration = Duration::from_millis(10);

/// Whether `clock` has reached [`GROUP_PROBE_DEADLINE`] past the wait's one start reading.
/// This is the only bound either group wait reads (larry#268): one reading of the injected
/// clock, never a count of yields.
fn past_deadline(clock: &Clock, started: Duration) -> bool {
    clock.now().saturating_sub(started) >= GROUP_PROBE_DEADLINE
}

/// The pause before probe `attempt + 1` of a group wait on a real clock: [`PROBE_PAUSE_FIRST`]
/// doubled per attempt, capped at [`PROBE_PAUSE_MAX`] and at `left`, the time the bound still
/// gives, so no sleep carries the wait past its deadline (larry#427).
fn probe_pause(attempt: u32, left: Duration) -> Duration {
    let doubled = match 1_u32.checked_shl(attempt) {
        Some(factor) => PROBE_PAUSE_FIRST.saturating_mul(factor),
        None => PROBE_PAUSE_MAX,
    };
    doubled.min(PROBE_PAUSE_MAX).min(left)
}

/// Waits between two group probes on the clock itself: a wall clock waits [`probe_pause`] on
/// `Clock::sleep`, the estate's one timed wait, through the process driver
/// `lgwks_bot::rt::runtime::block_on` already owns, so a waiter costs a few hundred timer wakes over
/// the whole bound rather than a core (larry#427). Every caller runs on the executor's own
/// blocking thread, never an async worker. The group has no kernel event to wait on: a pidfd
/// or `NOTE_EXIT` watches one pid, and the ledger reaps groups that are not this process's
/// children. A virtual clock moves only when its driver advances it, and the seeded sims
/// advance it inside the probe, so its sleep would wait on a move that never comes; the
/// thread yields instead and the sims spend no real time.
fn pause(clock: &Clock, started: Duration, attempt: u32) {
    if clock.source() == TimeSource::Virtual {
        std::thread::yield_now();
        return;
    }
    let left = GROUP_PROBE_DEADLINE.saturating_sub(clock.now().saturating_sub(started));
    lgwks_bot::rt::runtime::block_on(clock.sleep(probe_pause(attempt, left)));
}

/// SIGKILLs a process group; a group that is already gone is settled.
///
/// macOS refuses a signal to a group whose remaining members are zombies with
/// `EPERM` (openvax/mhctools#620), and BSD `kill(2)` sends nothing to any member
/// when one target fails the check, so one refusal can leave live members
/// unsignalled. The kill is therefore repeated while `EPERM` persists, until
/// [`GROUP_PROBE_DEADLINE`] has passed on `clock`, so it lands once the zombie is
/// reaped; refusing at the first `EPERM` failed 7 of 24 parallel runs of the timeout
/// journey (MEASURED, larry#131). A refusal that outlasts the bound is handed to
/// [`await_group_gone`], which never presumes the group gone and ends a member
/// that really refuses in [`ExecError::GroupSurvived`].
pub(super) fn settle_group(clock: &Clock, pgid: i32) -> Result<(), ExecError> {
    settle_with(clock, || kill_process_group(pgid))
}

/// [`settle_group`] over a signal the caller supplies, so the refusal schedule can be
/// driven: an `EPERM` is repeated until [`GROUP_PROBE_DEADLINE`] has passed on `clock`, a
/// group the kernel says is gone settles at once, and any other refusal is typed.
fn settle_with(clock: &Clock, mut signal: impl FnMut() -> io::Result<()>) -> Result<(), ExecError> {
    let started = clock.now();
    let mut attempt = 0_u32;
    loop {
        match signal() {
            Ok(()) => return Ok(()),
            Err(error) if error.raw_os_error() == Some(ESRCH) => return Ok(()),
            Err(error) if error.raw_os_error() == Some(EPERM) => {
                if past_deadline(clock, started) {
                    return Ok(());
                }
                pause(clock, started, attempt);
                attempt = attempt.saturating_add(1);
            }
            Err(error) => return Err(ExecError::Reap(error)),
        }
    }
}

/// Probes the group until it is gone, pausing between probes; fails once
/// [`GROUP_PROBE_DEADLINE`] has passed on `clock` with the group still there.
pub(super) fn await_group_gone(clock: &Clock, pgid: i32) -> Result<(), ExecError> {
    if await_gone(clock, || {
        process_group_exists(pgid).map_err(ExecError::Reap)
    })? {
        return Ok(());
    }
    Err(ExecError::GroupSurvived { pgid })
}

/// Waits for `still_there` to answer that the group is gone, pausing between probes, and
/// answers whether it did: `false` once [`GROUP_PROBE_DEADLINE`] has passed on `clock` with
/// the group still there. `still_there` is the probe's own answer, so the caller passes
/// `process_group_exists` and a `true` keeps waiting (larry#268); both decisions are the
/// probe's and one clock reading, so the seeded sim can drive them to the deadline and
/// before it on a virtual clock. The error type is the probe's, so a sim can report a
/// refused clock advance as itself. Visible to the executor so the ledger's reap and its
/// simulation share this one wait instead of a second copy (larry#308).
pub(in crate::executor) fn await_gone<E>(
    clock: &Clock,
    mut still_there: impl FnMut() -> Result<bool, E>,
) -> Result<bool, E> {
    let started = clock.now();
    let mut attempt = 0_u32;
    while still_there()? {
        if past_deadline(clock, started) {
            return Ok(false);
        }
        pause(clock, started, attempt);
        attempt = attempt.saturating_add(1);
    }
    Ok(true)
}

/// Seeded simulations of the group wait's one deadline and its backoff (larry#268, larry#427).
#[cfg(test)]
#[path = "sim_group.rs"]
mod sim_group;
