//! The terminal's session row across a kill -9 (larry#256, larry#578): every
//! turn is held in the state database, so a killed terminal leaves its
//! transcript behind. Source: POSIX `kill(2)`; `SQLite` atomic commit.

use lgwks_deps::storefront::rusqlite::Connection;

use super::journey::{background_kill_frame, background_pid};
use super::*;

/// One stored row: its name, model and snapshot JSON.
type Row = (String, String, String);

/// The rows `tui_sessions` holds in the state database under `state`.
fn rows(state: &Path) -> Journey<Vec<Row>> {
    let store = Connection::open(state.join("executor.db"))?;
    let mut statement = store.prepare("SELECT name, model, transcript FROM tui_sessions")?;
    let mapped = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    Ok(mapped.collect::<Result<_, _>>()?)
}

/// `/quit` during a command ends it and leaves once the session ends: the turn
/// that ends it is the one that quits, and it is still held in the row.
#[test]
fn tui_journey_a_quit_during_a_command_saves_its_last_turn() -> Journey {
    let (state, work, workspaces, config) = background_kill_frame()?;
    mock_model::run(config, |mock| -> Journey {
        std::thread::scope(|owner| -> Journey {
            let mut pty = Pty::open(owner, work.path(), &operator(state.path(), mock))?;
            ask(&mut pty)?;
            pty.send("y")?;
            pty.wait_for("esc to interrupt")?;
            background_pid(&workspaces)?;
            pty.send("/quit\r")?;
            assert_exited(&pty.finish()?.0, "/quit exits 0");
            let kept = rows(state.path())?;
            let (_, _, transcript) = kept.first().cloned().ok_or("the row")?;
            assert!(
                transcript.contains("Interrupted. Nothing more runs."),
                "the row holds the turn that quit: {transcript}"
            );
            Ok(())
        })
    })
}

/// A turn is on disk before the terminal dies: kill -9 leaves one row holding
/// the transcript, and the next start adds its own row beside it.
#[test]
fn tui_journey_a_killed_terminal_keeps_its_session_row() -> Journey {
    let (state, work) = fixture()?;
    mock_model::run(scripted(3, &[]), |mock| -> Journey {
        std::thread::scope(|owner| -> Journey {
            let env = operator(state.path(), mock);
            let mut pty = Pty::open(owner, work.path(), &env)?;
            pty.wait_for(READY)?;
            pty.send("/status\r")?;
            pty.wait_for("model: mock-model")?;
            // The frame is stored after each turn's input, so the first turn is
            // on disk by the time the second one paints.
            pty.send("/help\r")?;
            pty.wait_for("/quit leaves Larry")?;
            let pid = pty.pid()?;
            let killed = Command::new("kill").args(["-9", &pid]).status()?;
            assert!(killed.success(), "kill -9 {pid} was delivered");
            let (status, _) = pty.finish()?;
            assert!(!status.success(), "kill -9 left a success: {status}");
            let kept = rows(state.path())?;
            assert_eq!(kept.len(), 1, "one terminal, one row: {kept:?}");
            let (name, model, transcript) = kept.first().cloned().ok_or("the row")?;
            assert_eq!((name.as_str(), model.as_str()), ("Session 1", "mock-model"));
            assert!(
                transcript.contains("model: mock-model"),
                "the row holds the turn that ran before the kill: {transcript}"
            );
            let mut fresh = Pty::open(owner, work.path(), &env)?;
            fresh.wait_for(READY)?;
            fresh.send(QUIT)?;
            assert_exited(&fresh.finish()?.0, "Ctrl+C exits 0");
            assert_eq!(rows(state.path())?.len(), 2, "each start holds its own row");
            Ok(())
        })
    })
}
