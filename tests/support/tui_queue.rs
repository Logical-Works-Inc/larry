//! Queued turns through the shipped `larry` binary on a real PTY (larry#262).
//! Included by `e2e_journey` so the PTY harness it needs compiles once: a
//! second target over the same support would leave most helpers unused. The
//! mock model stands in for the gateway, so no test makes a paid call.

use super::{ASK, READY, frame, scripted};
use crate::pty::Journey;

/// Queued turns send in order: while the model's command runs, two more
/// tasks queue from the composer and each starts when the last ends.
#[test]
fn tui_journey_queued_turns_send_in_order() -> Journey {
    let replies = [
        r#"{"command":"sleep 2"}"#,
        r#"{"done":"first done"}"#,
        r#"{"done":"second done"}"#,
        r#"{"done":"third done"}"#,
    ];
    let ran = frame(scripted(7, &replies), |pty, mock, _| {
        pty.wait_for(READY)?;
        pty.send("first task\r")?;
        pty.wait_for(ASK)?;
        pty.send("y")?;
        pty.wait_for("esc to interrupt")?;
        pty.send("second task\r")?;
        pty.send("third task\r")?;
        pty.wait_for("queued 1: second task")?;
        pty.wait_for("first done")?;
        pty.wait_for("second done")?;
        pty.wait_for("third done")?;
        assert_eq!(mock.served(), 4, "the command, then one done per turn");
        Ok(())
    })?;
    assert!(
        ran.status.success(),
        "Ctrl+C on an empty box exits 0: {}",
        ran.status
    );
    Ok(())
}
