//! What ends the frame besides the person (larry#573, ADR 0139): SIGTERM, SIGHUP and the end of
//! input. crossterm 0.29 loops on a read of zero bytes, so on a hung-up terminal `event::poll`
//! never returns and the frame never reads its stop flag. The watcher waits on its own threads
//! and puts a quiet socket over stdin, whose read answers "nothing yet", so the poll returns
//! inside its own timeout. The signals and the input watch are Unix; elsewhere the watcher only
//! waits for the frame to end.

use std::io;
use std::sync::atomic::AtomicBool;
#[cfg(unix)]
use std::{
    os::fd::{AsFd, OwnedFd},
    os::unix::net::UnixStream,
    sync::atomic::Ordering,
};

use lgwks_bot::rt::sync::CancellationToken;
#[cfg(unix)]
use rustix::{
    event::{PollFd, PollFlags, poll},
    io::Errno,
};

/// What ended the watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stop {
    /// SIGTERM from a service manager, or SIGHUP from a closed terminal or supervisor.
    Signal,
    /// Standard input hung up: its far end is gone and no key will come again.
    InputEnded,
    /// The frame ended first.
    Done,
}

/// What the watch found, with the real stdin still to be put back.
pub(super) struct Watched {
    /// What ended the watch, or why watching failed.
    stop: io::Result<Stop>,
    /// The stdin the watch may have covered.
    #[cfg(unix)]
    quiet: Quiet,
}

impl Watched {
    /// Puts the real stdin back and returns what ended the watch.
    pub(super) fn restore(self) -> io::Result<Stop> {
        #[cfg(unix)]
        self.quiet.restore()?;
        self.stop
    }
}

/// The watcher's registrations, made before the terminal is entered so a refusal paints nothing.
#[cfg(unix)]
pub(super) struct Watch {
    /// SIGTERM and SIGHUP.
    signals: Signals,
    /// A duplicate of stdin, watched for a hang-up.
    input: OwnedFd,
    /// What the input watch also waits on: it closes when the signal watch ends.
    woken: UnixStream,
    /// The far end of `woken`.
    waker: UnixStream,
    /// The quiet stdin put over the real one once the watch ends the frame.
    quiet: Quiet,
}

#[cfg(unix)]
impl Watch {
    /// Registers the signals and the input watch.
    pub(super) fn register() -> io::Result<Self> {
        let signals = Signals::register()?;
        let input = io::stdin().as_fd().try_clone_to_owned()?;
        let (woken, waker) = UnixStream::pair()?;
        Ok(Self {
            signals,
            input,
            woken,
            waker,
            quiet: Quiet::new()?,
        })
    }

    /// Waits for what ends the frame; unless the frame ended first, marks `stopped` and quiets
    /// stdin so a loop held inside a read returns to see the mark. The input is watched on a
    /// thread of its own, joined before this returns.
    pub(super) fn wait(self, stopped: &AtomicBool, done: &CancellationToken) -> Watched {
        let Self {
            signals,
            input,
            woken,
            waker,
            quiet,
        } = self;
        let stop = std::thread::scope(|scope| {
            let watching = scope.spawn(|| {
                let ended = hung_up(&input, &woken);
                if !matches!(ended, Ok(false)) {
                    stopped.store(true, Ordering::Release);
                    quiet.cover()?;
                }
                ended
            });
            let signalled = signals.wait(stopped, done);
            let covered = if signalled { quiet.cover() } else { Ok(()) };
            drop(waker);
            let ended = match watching.join() {
                Ok(ended) => ended,
                Err(panic) => Err(io::Error::other(crate::panic_text(&*panic))),
            };
            covered.and(ended).map(|input_ended| {
                if input_ended {
                    Stop::InputEnded
                } else if signalled {
                    Stop::Signal
                } else {
                    Stop::Done
                }
            })
        });
        Watched { stop, quiet }
    }
}

/// Blocks until `input` hangs up (`true`) or `wake` closes (`false`). No byte is read from
/// either, and keys waiting to be read are not an end: only a hang-up is asked for.
#[cfg(unix)]
fn hung_up(input: &impl AsFd, wake: &impl AsFd) -> io::Result<bool> {
    let gone = PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL;
    loop {
        let mut watched = [
            PollFd::new(input, PollFlags::HUP),
            PollFd::new(wake, PollFlags::IN),
        ];
        match poll(&mut watched, None) {
            Ok(_ready) => {}
            // A signal, a window resize among them, interrupts the wait and ends nothing.
            Err(Errno::INTR) => continue,
            Err(cause) => return Err(cause.into()),
        }
        let [ref on_input, ref on_wake] = watched;
        if on_input.revents().intersects(gone) {
            return Ok(true);
        }
        if !on_wake.revents().is_empty() {
            return Ok(false);
        }
    }
}

/// The signals that end a terminal also end its owned session.
#[cfg(unix)]
struct Signals {
    /// SIGTERM from a service manager.
    terminate: lgwks_bot::rt::signal::unix::Signal,
    /// SIGHUP from a closed controlling terminal or supervisor.
    hangup: lgwks_bot::rt::signal::unix::Signal,
}

#[cfg(unix)]
impl Signals {
    /// Registers the signals before the terminal starts.
    fn register() -> io::Result<Self> {
        use lgwks_bot::rt::signal::unix::{SignalKind, signal};
        lgwks_bot::rt::runtime::block_on(async {
            Ok(Self {
                terminate: signal(SignalKind::terminate())?,
                hangup: signal(SignalKind::hangup())?,
            })
        })
    }

    /// Waits for either signal, marking `stopped` and answering `true`, or for the frame to
    /// end, answering `false`.
    fn wait(mut self, stopped: &AtomicBool, done: &CancellationToken) -> bool {
        let signalled = lgwks_bot::rt::runtime::block_on(async {
            lgwks_bot::rt::select! {
                _ = self.terminate.recv() => true,
                _ = self.hangup.recv() => true,
                () = done.cancelled() => false,
            }
        });
        if signalled {
            stopped.store(true, Ordering::Release);
        }
        signalled
    }
}

/// A stdin with nothing to say, and the real one it stands over.
#[cfg(unix)]
struct Quiet {
    /// The real stdin, put back before the screen is restored.
    original: OwnedFd,
    /// The nonblocking end read in its place: a read answers "would block", never end of file.
    reader: UnixStream,
    /// The far end, held open so the reader never sees an end of file.
    _writer: UnixStream,
}

#[cfg(unix)]
impl Quiet {
    /// Makes the quiet stdin and keeps a handle on the real one.
    fn new() -> io::Result<Self> {
        let original = io::stdin().as_fd().try_clone_to_owned()?;
        let (reader, writer) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        Ok(Self {
            original,
            reader,
            _writer: writer,
        })
    }

    /// Puts the quiet end on descriptor 0.
    fn cover(&self) -> io::Result<()> {
        Ok(rustix::stdio::dup2_stdin(&self.reader)?)
    }

    /// Puts the real stdin back on descriptor 0.
    fn restore(&self) -> io::Result<()> {
        Ok(rustix::stdio::dup2_stdin(&self.original)?)
    }
}

/// No Unix signal or descriptor watch exists on this target.
#[cfg(not(unix))]
pub(super) struct Watch;

#[cfg(not(unix))]
impl Watch {
    /// Nothing to register.
    pub(super) fn register() -> io::Result<Self> {
        Ok(Self)
    }

    /// Ends when the frame has ended; no platform signal is consumed here.
    pub(super) fn wait(self, _stopped: &AtomicBool, done: &CancellationToken) -> Watched {
        lgwks_bot::rt::runtime::block_on(done.cancelled());
        Watched {
            stop: Ok(Stop::Done),
        }
    }
}

/// The frame's terminal signals and its end of input (larry#252, larry#573).
#[cfg(all(test, unix))]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{RecvTimeoutError, sync_channel};
    use std::time::Duration;

    use lgwks_bot::rt::sync::CancellationToken;
    use lgwks_macros::Error;

    use super::{Quiet, Signals, hung_up};

    /// What every test here returns.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// How long a watch that must end is given before the test fails.
    const BOUND: Duration = Duration::from_secs(5);

    /// Why a call that had to answer gave no answer.
    #[derive(Debug, Error)]
    enum Unanswered {
        /// Its thread could not start.
        #[error("the thread did not start: {0}")]
        Spawn(#[source] std::io::Error),
        /// It was still running at `BOUND`, or it panicked.
        #[error("no answer: {0}")]
        Silent(#[source] RecvTimeoutError),
    }

    /// Runs `call` on its own thread and returns its answer, failing if none comes in `BOUND`:
    /// a watch or a read that never returns fails the test instead of hanging it. The channel
    /// holds the one answer, so the thread's send cannot block.
    fn within_bound<T: Send + 'static>(
        call: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, Unanswered> {
        let (sender, answer) = sync_channel(1);
        let thread = std::thread::Builder::new()
            .spawn(move || sender.send(call()))
            .map_err(Unanswered::Spawn)?;
        let answered = answer.recv_timeout(BOUND).map_err(Unanswered::Silent)?;
        drop(thread);
        Ok(answered)
    }

    /// Sends `signal` to this test's own process and holds that the watcher calls it a stop.
    fn the_signal_stops_the_frame(signal: &str) -> TestResult {
        let signals = Signals::register()?;
        let stopped = Arc::<AtomicBool>::default();
        let done = CancellationToken::new();
        let flag = Arc::clone(&stopped);
        let me = std::process::id().to_string();
        let sent = std::process::Command::new("kill")
            .args([signal, &me])
            .status()?;
        assert!(sent.success(), "kill {signal} reached the test process");
        let signalled = within_bound(move || signals.wait(&flag, &done))?;
        assert!(signalled, "{signal} is a signal stop");
        assert!(stopped.load(Ordering::Acquire), "{signal} sets the flag");
        Ok(())
    }

    /// SIGHUP ends the terminal: a hangup the watcher owns sets the stop flag, so the frame
    /// finishes its worker instead of running on. Source: POSIX `SIGHUP`.
    #[test]
    fn a_hangup_sets_the_frame_stop_flag() -> TestResult {
        the_signal_stops_the_frame("-HUP")
    }

    /// SIGTERM, which the orphans of larry#573 caught and ignored, stops the frame the same
    /// way. Source: POSIX `SIGTERM`.
    #[test]
    fn a_termination_sets_the_frame_stop_flag() -> TestResult {
        the_signal_stops_the_frame("-TERM")
    }

    /// The stdin put over a hung-up terminal answers "nothing yet" at once: a blocking read
    /// would hold the frame, and an end of file would spin crossterm. Source: POSIX `read`,
    /// `EAGAIN` on a nonblocking descriptor.
    #[test]
    fn the_quiet_stdin_never_blocks_and_never_ends() -> TestResult {
        let quiet = Quiet::new()?;
        let read = within_bound(move || (&quiet.reader).read(&mut [0_u8; 1]))?;
        let kind = read.err().map(|cause| cause.kind());
        assert_eq!(kind, Some(std::io::ErrorKind::WouldBlock));
        Ok(())
    }

    /// A far end that goes away ends the watch while its wake socket stays open. Source:
    /// POSIX `poll`, `POLLHUP` on a stream whose peer closed.
    #[test]
    fn an_ended_input_ends_the_terminal_watch() -> TestResult {
        let (near, far) = UnixStream::pair()?;
        let (woken, waker) = UnixStream::pair()?;
        drop(far);
        let ended = within_bound(move || hung_up(&near, &woken))??;
        assert!(ended, "a closed far end is the end of input");
        drop(waker);
        Ok(())
    }

    /// Keys waiting to be read are not an end of input, so a person's typing never closes the
    /// frame: the watch answers only that it was woken.
    #[test]
    fn input_with_keys_waiting_has_not_ended() -> TestResult {
        let (near, mut far) = UnixStream::pair()?;
        far.write_all(b"k")?;
        let (woken, waker) = UnixStream::pair()?;
        drop(waker);
        let ended = hung_up(&near, &woken)?;
        assert!(!ended, "an open input with a key waiting has not ended");
        Ok(())
    }
}
