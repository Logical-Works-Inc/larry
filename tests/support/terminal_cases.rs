//! The person's controlling terminal as a journey (larry#323): a command a person approves must
//! not be able to draw on their screen or read their keys, so a fake approval or diff cannot be
//! painted past the sanitizer. These cases are part of the `e2e_journey` binary and use its
//! shared harness; they read the screen each frame left.

use super::journey::{ask, frame, frame_in, scripted};
use super::origin::fixture;
use super::pty::Journey;

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
