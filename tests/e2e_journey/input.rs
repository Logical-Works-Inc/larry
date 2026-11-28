//! Terminal input the keyboard does not send (larry#239, larry#576): a paste, focus and mouse
//! reports, a window resize and an environment the settings refuse. Sources: xterm control
//! sequences (bracketed paste mode 2004, focus mode 1004, SGR mouse mode 1006) and POSIX
//! `SIGWINCH`. Requirement: what a terminal reports never submits or types anything the person
//! did not, and a refused setting is shown without its value.

use std::ffi::OsString;
use std::fs::File;
use std::os::unix::ffi::OsStringExt as _;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

use super::journey::{QUIT, READY, frame, operator, scripted};
use super::mock_model;
use super::origin::fixture;
use super::pty::{Journey, Pty};

/// fat-finger and hostile-paste (CWE-150): a pasted block that holds a carriage return and an
/// OSC title sequence waits in the composer. Nothing is submitted, the paste's escape never
/// reaches the terminal and Ctrl+C clears it. Catches a paste dropped, a pasted newline read as
/// Enter, and pasted bytes written to the person's terminal.
#[test]
fn tui_journey_a_pasted_block_waits_in_the_composer() -> Journey {
    let ran = frame(scripted(71, &[]), |pty, mock, _| {
        pty.wait_for(READY)?;
        pty.send("\x1b[200~pasta-9c\rbravo \x1b]0;owned\x07tail-9c\x1b[201~")?;
        pty.wait_for("tail-9c")?;
        let screen = pty.screen_text();
        assert!(
            screen.contains("pasta-9c") && !screen.contains("Working"),
            "the whole paste waits in the composer: {screen}"
        );
        pty.send(QUIT)?;
        pty.wait_for(READY)?;
        assert_eq!(mock.served(), 0, "a pasted newline submits nothing");
        Ok(())
    })?;
    assert!(ran.status.success(), "exits 0: {}", ran.status);
    assert!(
        !ran.wire.contains("\x1b]0;owned"),
        "the pasted title sequence never reaches the terminal"
    );
    Ok(())
}

/// noisy-terminal: focus in, focus out and an SGR mouse press and release arrive before the
/// person types. The composer holds only what was typed. Catches a report typed into the task.
#[test]
fn tui_journey_focus_and_mouse_reports_type_nothing() -> Journey {
    let ran = frame(scripted(73, &[]), |pty, _, _| {
        pty.wait_for(READY)?;
        pty.send("\x1b[I\x1b[O\x1b[<0;5;5M\x1b[<0;5;5mafter-focus-9c")?;
        pty.wait_for("after-focus-9c")?;
        let screen = pty.screen_text();
        let row = screen
            .lines()
            .find(|row| row.contains("after-focus-9c"))
            .ok_or("no composer row")?;
        for report in ["[<", "0;5;5", "[I", "[O"] {
            assert!(!row.contains(report), "{report:?} was typed: {row:?}");
        }
        pty.send(QUIT)?;
        pty.wait_for(READY)
    })?;
    assert!(ran.status.success(), "exits 0: {}", ran.status);
    Ok(())
}

/// Sets the width of the terminal `pid` runs on; the kernel then sends it `SIGWINCH`.
fn narrow(pid: &str, columns: u16) -> Journey {
    let found = Command::new("ps")
        .args(["-o", "tty=", "-p", pid])
        .stderr(Stdio::null())
        .output()?;
    let name = String::from_utf8(found.stdout)?;
    let device = File::open(Path::new("/dev").join(name.trim()))?;
    let set = Command::new("stty")
        .args(["cols", &columns.to_string()])
        .stdin(device)
        .status()?;
    assert!(set.success(), "stty set {columns} columns on {name:?}");
    Ok(())
}

/// small-window (POSIX `SIGWINCH`): at 120 columns the chat is a 44-cell column beside the
/// artifact pane, so a 51-cell line wraps; shrunk to 60 columns the pane is gone and the chat
/// takes the whole width, so the line fits one row. Catches a resize that is ignored and a frame
/// that keeps painting at the old width.
#[test]
fn tui_journey_a_narrowed_window_is_laid_out_again() -> Journey {
    let ran = frame(scripted(77, &[]), |pty, _, _| {
        pty.wait_for(READY)?;
        let line = format!("{}-end-9c", "w".repeat(44));
        pty.send(&line)?;
        pty.wait_for("-end-9c")?;
        let wide = pty.screen_text();
        assert!(
            wide.contains("Receipt") && !wide.contains(&line),
            "at 120 columns the pane shows and the line wraps: {wide}"
        );
        narrow(&pty.pid()?, 60)?;
        pty.wait_until("laid out at 60 columns", |screen| {
            screen.contains(&line) && !screen.contains("Receipt")
        })?;
        pty.send(QUIT)?;
        pty.wait_for(READY)
    })?;
    assert!(ran.status.success(), "exits 0: {}", ran.status);
    Ok(())
}

/// broken-environment (CWE-176, CWE-532): a setting that is not UTF-8 is named in the welcome
/// box and again when a task is submitted; its value is never shown and no model is called.
/// Catches a refused setting read as unset, or its bytes echoed to the terminal.
#[test]
fn tui_journey_a_setting_that_is_not_text_is_named_and_runs_nothing() -> Journey {
    let (state, work) = fixture()?;
    let (status, raw) = mock_model::run(
        scripted(79, &[]),
        |mock| -> Journey<(ExitStatus, Vec<u8>)> {
            std::thread::scope(|owner| {
                let mut env = operator(state.path(), mock);
                let value = OsString::from_vec(b"secret-9c\xff".to_vec());
                env.push(("LARRY_POLICY", value));
                let mut pty = Pty::open(owner, work.path(), &env)?;
                pty.wait_for("LARRY_POLICY")?;
                pty.send("write hello.txt\r")?;
                pty.wait_until("refused the task by name", |screen| {
                    screen.matches("LARRY_POLICY").count() >= 2
                })?;
                assert_eq!(mock.served(), 0, "no model call under a refused setting");
                pty.send(QUIT)?;
                pty.finish()
            })
        },
    )?;
    assert!(status.success(), "exits 0: {status}");
    let wire = String::from_utf8_lossy(&raw);
    assert!(
        !wire.contains("secret-9c"),
        "the refused value is never shown"
    );
    Ok(())
}
