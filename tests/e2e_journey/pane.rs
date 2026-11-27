//! The artifact pane beside the chat (larry#244, larry#252): while the question about
//! a change is asked the pane already shows its diff, before the folder holds the file, and
//! while a command runs its first output line streams into the pane before it finishes.

use super::*;

/// The first `name` present under any live workspace tree, inside one deadline (larry#268).
fn await_tree_file(workspaces: &Path, name: &str, what: &str) -> Journey<PathBuf> {
    await_event(what, || {
        Ok(tree_copies(workspaces)
            .into_iter()
            .map(|tree| tree.join(name))
            .find(|path| path.exists()))
    })
}

/// The artifact pane beside the chat (larry#244; the PTY is 120 columns, past the 108 that
/// open it): while the question about the change is asked the pane already shows its file list
/// and hunk, before the folder holds the file; once the person applies it the sealed report's
/// receipt takes the pane, and Tab goes back to the command's output.
#[test]
fn tui_journey_pane_shows_diff_and_receipt() -> Journey {
    let replies = [COMMAND, r#"{"done":"wrote hello.txt"}"#];
    let ran = frame(scripted(44, &replies), |pty, _, work| {
        ask(pty)?;
        pty.send("y")?;
        pty.wait_for(REVIEW)?;
        pty.wait_for("  hello.txt  +1 -0")?;
        pty.wait_for("+hello")?;
        assert!(
            !work.join("hello.txt").exists(),
            "shown before the folder changes"
        );
        pty.send("y")?;
        pty.wait_for(APPLIED)?;
        pty.wait_for("commands  1 run, 0 declined, 0 interrupted")?;
        pty.wait_for("digest    ")?;
        pty.send("\t")?;
        pty.wait_for("$ printf")
    })?;
    assert!(ran.status.success(), "exits 0: {}", ran.status);
    Ok(())
}

/// A running command's first line reaches the artifact pane before its FIFO is released;
/// the second line cannot be a buffered completion artifact then.
#[test]
fn tui_journey_pane_streams_first_line_before_fifo_release() -> Journey {
    let replies = [
        r#"{"command":"mkfifo stream.fifo; printf '\\146\\151\\162\\163\\164\\n'; cat stream.fifo; printf '\\163\\145\\143\\157\\156\\144\\n'; rm stream.fifo"}"#,
        r#"{"done":"stream finished"}"#,
    ];
    let (state, work) = fixture()?;
    let workspaces = state.path().join("ws");
    let ran = frame_in(
        &state,
        work,
        scripted(252, &replies),
        Box::new(|pty, _, _| {
            let mut fifo_guard = FifoRelease::arm(workspaces.clone());
            ask(pty)?;
            pty.send("y")?;
            pty.wait_for("first")?;
            let fifo = await_tree_file(&workspaces, "stream.fifo", "the stream FIFO exists")?;
            fifo_guard.release(&fifo)?;
            pty.wait_for("second")?;
            pty.wait_for("stream finished")
        }),
    )?;
    assert!(ran.status.success(), "the streamed journey exits 0");
    Ok(())
}
