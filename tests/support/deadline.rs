//! One failure deadline for the journeys' waits (larry#268).
//!
//! A wait ends when the event it needs arrives: a pipe a command opens, a row the executor
//! inserts, a file a command writes. A count of yields is not that bound, because on a
//! loaded host it ends early and on an idle host it ends late; a wait that never gets its
//! event fails inside [`PATIENCE`], naming what it waited for. A read refused by the wait's own
//! transient condition, such as a database lock, is retried inside the same bound so the last
//! cause is reported when it passes (larry#294). Included by path into
//! `tests/support/exec_cli.rs`, `tests/support/pty.rs` and `tests/governance_journey.rs`, and
//! every helper here is exercised by the tests at the end of this file.

use std::error::Error;
use std::fs::File;
use std::path::Path;
use std::sync::mpsc::{RecvTimeoutError, sync_channel};
use std::time::{Duration, Instant};

/// The bound a wait may take before the host is called broken: a failure deadline, never
/// the pace of a working one. The gates run every journey binary at once, so a child's
/// clone and its first command can wait seconds for a core.
pub const PATIENCE: Duration = Duration::from_secs(30);

/// A pid no live process holds: the largest positive `i32` less one, so the group it names
/// is already gone.
const DEAD_PID: i32 = 2_147_483_646;

/// Waits until `probe` answers `Some`, yielding between checks, and returns its answer. A
/// probe that answers `None` for `within` fails with `what` and the bound it passed.
pub fn await_until<T>(
    what: &str,
    within: Duration,
    mut probe: impl FnMut() -> Result<Option<T>, Box<dyn Error>>,
) -> Result<T, Box<dyn Error>> {
    let started = Instant::now();
    loop {
        if let Some(value) = probe()? {
            return Ok(value);
        }
        if started.elapsed() >= within {
            return Err(format!("{what} did not happen within {within:?}").into());
        }
        std::thread::yield_now();
    }
}

/// [`await_until`] within [`PATIENCE`].
pub fn await_event<T>(
    what: &str,
    probe: impl FnMut() -> Result<Option<T>, Box<dyn Error>>,
) -> Result<T, Box<dyn Error>> {
    await_until(what, PATIENCE, probe)
}

/// Waits until `probe` answers `Some`, retrying a refusal inside `within` and returning its
/// answer. A read of a database another writer holds the lock on answers `SQLITE_BUSY` for a
/// moment, so that refusal is the wait's own condition and not a failed journey (larry#294). A
/// probe that never answers fails at the deadline, naming the event, the bound and the last
/// refusal, so a permanent fault is reported rather than retried away.
pub fn await_retrying<T>(
    what: &str,
    within: Duration,
    mut probe: impl FnMut() -> Result<Option<T>, Box<dyn Error>>,
) -> Result<T, Box<dyn Error>> {
    let started = Instant::now();
    let mut last: Option<Box<dyn Error>> = None;
    loop {
        match probe() {
            Ok(Some(value)) => return Ok(value),
            Ok(None) => {}
            Err(cause) => last = Some(cause),
        }
        if started.elapsed() >= within {
            return Err(match last {
                Some(cause) => format!("{what} did not happen within {within:?}: {cause}").into(),
                None => format!("{what} did not happen within {within:?}").into(),
            });
        }
        std::thread::yield_now();
    }
}

/// Waits until process group `pgid` is gone, within [`PATIENCE`]. Killed members may linger
/// briefly as zombies, so the group's own exit is the event and the deadline only bounds a
/// failure.
pub fn await_group_exit(pgid: i32) -> Result<(), Box<dyn Error>> {
    await_event(&format!("process group {pgid} is gone"), || {
        match lgwks_std::process::process_group_exists(pgid)? {
            true => Ok(None),
            false => Ok(Some(())),
        }
    })
}

/// Opens `fifo` for writing on a thread of its own and returns the handle within [`PATIENCE`].
///
/// The open blocks until a reader opens the pipe, so the pipe is the event itself: a command
/// that runs `cat < fifo` has started, and the handle this returns keeps it blocked until the
/// caller drops it. A command that never reaches the read fails the wait inside the bound
/// instead of hanging the journey; on that failure a reader is opened to release the thread.
pub fn await_fifo_writer(fifo: &Path) -> Result<File, Box<dyn Error>> {
    let path = fifo.to_path_buf();
    let (sender, opened) = sync_channel(1);
    let writer = std::thread::Builder::new()
        .name(String::from("fifo-writer"))
        .spawn(move || {
            let handle = File::options().write(true).open(&path);
            drop(sender.send(handle));
        })?;
    match opened.recv_timeout(PATIENCE) {
        Ok(Ok(handle)) => {
            writer
                .join()
                .map_err(|_| "the fifo writer thread panicked")?;
            Ok(handle)
        }
        Ok(Err(cause)) => {
            drop(writer.join());
            Err(format!(
                "{} could not be opened for writing: {cause}",
                fifo.display()
            )
            .into())
        }
        Err(RecvTimeoutError::Timeout) => {
            // A reader unblocks the waiting open, so the thread ends instead of staying parked
            // on a pipe nobody will use; whether that reader opened is part of the failure.
            let released = match File::open(fifo) {
                Ok(release) => format!("a reader released the waiting open: {release:?}"),
                Err(cause) => format!("the waiting open could not be released: {cause}"),
            };
            drop(writer.join());
            Err(format!(
                "no command opened {} for reading within {:?}; {released}",
                fifo.display(),
                PATIENCE
            )
            .into())
        }
        Err(RecvTimeoutError::Disconnected) => {
            Err(format!("{} never answered its open", fifo.display()).into())
        }
    }
}

/// Every wait this module offers, against the events and refusals each meets: an answer that
/// is there at once comes back, an answer that never comes fails naming its bound, a pipe that
/// cannot be opened fails typed, and a group no process holds is already gone.
#[test]
fn every_wait_answers_on_its_event_or_fails_by_name() -> Result<(), Box<dyn Error>> {
    let immediate = await_event("the immediate answer", || Ok(Some(7_u32)))?;
    assert_eq!(immediate, 7, "a probe that answers at once is returned");

    let bound = Duration::from_millis(50);
    let message = match await_until::<u32>("the answer that never comes", bound, || Ok(None)) {
        Ok(value) => return Err(format!("a wait with no event answered {value}").into()),
        Err(cause) => cause.to_string(),
    };
    assert!(
        message.contains("the answer that never comes") && message.contains("did not happen"),
        "the failure names the event and its bound: {message}"
    );

    let missing = std::env::temp_dir()
        .join("larry-no-such-directory-for-a-fifo")
        .join("hold");
    let refusal = match await_fifo_writer(&missing) {
        Ok(_) => return Err("a pipe under a directory that does not exist opened".into()),
        Err(cause) => cause.to_string(),
    };
    assert!(
        refusal.contains("larry-no-such-directory-for-a-fifo"),
        "the refusal names the pipe: {refusal}"
    );

    await_group_exit(DEAD_PID)?;
    Ok(())
}

/// A read refused every time fails the wait at its deadline naming the last refusal: a refusal
/// the wait's own writer causes is retried, and a permanent one is reported, never swallowed
/// (larry#294).
#[test]
fn a_wait_whose_reads_keep_failing_names_the_last_cause() -> Result<(), Box<dyn Error>> {
    let bound = Duration::from_millis(50);
    let message = match await_retrying::<u32>("the ledger row", bound, || {
        Err("database is locked".into())
    }) {
        Ok(value) => return Err(format!("a wait whose reads all failed answered {value}").into()),
        Err(cause) => cause.to_string(),
    };
    assert!(
        message.contains("did not happen within") && message.contains("database is locked"),
        "the failure names the wait, its bound and the last refusal: {message}"
    );
    Ok(())
}
