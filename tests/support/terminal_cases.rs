//! The person's controlling terminal as a journey (larry#323): a command a person approves must
//! not be able to draw on their screen or read their keys, so a fake approval or diff cannot be
//! painted past the sanitizer. A closed terminal ends Larry, and a test's terminal guard takes
//! the binary with it (larry#573; POSIX `termios` hang-up and `kill(2)`). These cases are part
//! of the `e2e_journey` binary and use its shared harness; they read the screen each frame left.

use std::process::Command;

use super::journey::{READY, ask, frame, frame_in, scripted};
use super::origin::fixture;
use super::pty::deadline::await_event;
use super::pty::{Journey, Launch, Pty, alive};

/// Waits for the binary `pid` to be gone. A binary that stays is killed before the failure is
/// returned, so a failing run leaves no orphan on the host.
fn await_gone(pid: &str, what: &str) -> Journey {
    let gone = await_event(what, || Ok((!alive(pid)?).then_some(())));
    if let Err(cause) = gone {
        let killed = Command::new("kill").args(["-9", pid]).status()?;
        return Err(format!("{cause}; the orphan {pid} was killed: {killed}").into());
    }
    Ok(())
}

/// Opens a frame, waits for it to be ready, closes the terminal under it and waits for the
/// binary to leave on its own.
fn close_the_terminal(launch: &Launch<'_>) -> Journey {
    let (state, work) = fixture()?;
    let env = [("LARRY_STATE_DIR", state.path().as_os_str().to_owned())];
    std::thread::scope(|owner| -> Journey {
        let mut pty = Pty::launch(owner, work.path(), &env, launch)?;
        pty.wait_for(READY)?;
        let pid = pty.hang_up()?;
        await_gone(&pid, "bare larry leaves when its terminal closes")
    })
}

/// The orphans of larry#573: the terminal is closed under a running frame. Larry must end,
/// never spin on the end of input with its stop signals unread.
#[test]
fn tui_journey_a_closed_terminal_leaves_no_orphan_larry() -> Journey {
    close_the_terminal(&Launch::Bare)
}

/// The same closed terminal with no `SIGHUP` to announce it: the end of input alone ends Larry.
#[cfg(target_os = "linux")]
#[test]
fn tui_journey_a_terminal_closed_without_a_hangup_signal_ends_larry() -> Journey {
    close_the_terminal(&Launch::NewSession)
}

/// A file on stdin while a controlling terminal exists: Larry refuses it and paints no frame,
/// rather than reading the person's terminal behind their redirect.
#[test]
fn tui_journey_a_file_on_stdin_of_a_terminal_is_refused() -> Journey {
    let (state, work) = fixture()?;
    let env = [("LARRY_STATE_DIR", state.path().as_os_str().to_owned())];
    let keys = work.path().join("keys");
    std::fs::write(&keys, "write hello.txt\r")?;
    std::thread::scope(|owner| -> Journey {
        let mut pty = Pty::launch(owner, work.path(), &env, &Launch::StdinFrom(&keys))?;
        pty.wait_for("standard input is not a terminal")?;
        let (status, _) = pty.finish()?;
        assert_eq!(status.code(), Some(1), "the refusal exits 1");
        Ok(())
    })
}

/// A journey that gives up mid-run drops its terminal: no `larry` is left behind.
#[test]
fn tui_journey_a_dropped_terminal_guard_leaves_no_orphan() -> Journey {
    let (state, work) = fixture()?;
    let env = [("LARRY_STATE_DIR", state.path().as_os_str().to_owned())];
    std::thread::scope(|owner| -> Journey {
        let mut pty = Pty::open(owner, work.path(), &env)?;
        pty.wait_for(READY)?;
        let pid = pty.pid()?;
        assert!(alive(&pid)?, "larry {pid} runs before the guard drops");
        drop(pty);
        await_gone(&pid, "the dropped guard takes larry with it")
    })
}

/// The issue's proof (larry#323): a command that writes the person's controlling terminal puts
/// no byte on their screen. The same journey runs twice in one folder and state directory, the
/// first with a trigger file that makes the command try the write; the final screens are
/// identical, so the write reached nothing. A leak would paint its bytes on the first run and
/// the screens would differ.
#[test]
fn tui_journey_a_command_never_writes_the_persons_terminal() -> Journey {
    let replies = [
        r#"{"command":"if [ -e tty-trigger ]; then ( printf 'TTY-LEAK-7a1b' > /dev/tty ) 2>/dev/null || true; fi; printf 'ran-%s-7a\n' ok"}"#,
        r#"{"done":"all finished"}"#,
    ];
    let (state, mut work) = fixture()?;
    let mut screens: Vec<String> = Vec::new();
    for attempt in 0..2 {
        let trigger = work.path().join("tty-trigger");
        if attempt == 0 {
            std::fs::write(&trigger, b"armed")?;
        } else {
            std::fs::remove_file(&trigger)?;
        }
        let ran = frame_in(
            &state,
            work,
            scripted(61, &replies),
            Box::new(|pty, _, _| {
                ask(pty)?;
                pty.send("y")?;
                pty.wait_for("ran-ok-7a")?;
                pty.wait_for("all finished")?;
                pty.wait_for("verdict")?;
                screens.push(pty.screen_text());
                Ok(())
            }),
        )?;
        assert!(
            ran.status.success(),
            "attempt {attempt}: exits 0: {}",
            ran.status
        );
        work = ran.work;
    }
    let first = screens.first().ok_or("the first run left no screen")?;
    let second = screens.get(1).ok_or("the second run left no screen")?;
    assert_eq!(
        without_receipt_digest(first),
        without_receipt_digest(second),
        "the screen is the same with and without the /dev/tty write"
    );
    Ok(())
}

/// `screen` with the receipt's per-run digest blanked: the digest covers the run id and each
/// command's measured duration, which differ between two runs of the same journey.
fn without_receipt_digest(screen: &str) -> String {
    screen
        .lines()
        .map(|line| {
            if line.contains("digest") {
                "digest"
            } else {
                line
            }
        })
        .collect::<Vec<&str>>()
        .join("\n")
}

/// A command that opens the person's controlling terminal is refused, and its own stderr says
/// so (larry#323): the shell's error names `/dev/tty:`, text that was not on screen before the
/// person answered.
#[test]
fn tui_journey_a_command_never_opens_the_persons_terminal() -> Journey {
    let replies = [
        r#"{"command":": < /dev/tty"}"#,
        r#"{"done":"all finished"}"#,
    ];
    let ran = frame(scripted(67, &replies), |pty, _, _| {
        ask(pty)?;
        pty.send("y")?;
        pty.wait_for("/dev/tty:")?;
        pty.wait_for("all finished")
    })?;
    assert!(ran.status.success(), "exits 0: {}", ran.status);
    Ok(())
}
