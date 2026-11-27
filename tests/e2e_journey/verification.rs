//! The typed verification gate through the shipped `larry` frame on a real PTY
//! (larry#271), split from `tests/e2e_journey.rs` so each file stays one
//! responsibility: the model writes, then runs a read-only check, then sends
//! the finish pair naming that check.

use crate::journey::{ask, frame, scripted};
use crate::origin::git_command;
use crate::pty::Journey;
use crate::{APPLIED, CHECK_HELLO, COMMAND, FINISH_HELLO, REVIEW};

#[test]
fn tui_journey_requires_the_typed_verification_finish() -> Journey {
    let check = r#"{"command":"test -f hello.txt"}"#;
    let finish = r#"{"finish":{"summary":"verified hello.txt","check":"test -f hello.txt","artifacts":["hello.txt"],"state":["the workspace copy is ready for review"],"servers":["none"]}}"#;
    let replies = [COMMAND, check, finish, finish];
    let ran = frame(scripted(271, &replies), |pty, mock, work| {
        ask(pty)?;
        pty.send("y")?;
        pty.wait_for("$ test -f hello.txt")?;
        pty.send("y")?;
        pty.wait_for(REVIEW)?;
        assert!(!work.join("hello.txt").exists(), "review precedes handoff");
        pty.send("y")?;
        pty.wait_for(APPLIED)?;
        pty.wait_for("verified hello.txt")?;
        assert!(
            mock.requests().iter().any(|request| {
                String::from_utf8_lossy(request).contains("verification checklist")
            }),
            "the first finish returns its checklist to the model"
        );
        assert_eq!(mock.served(), 4, "write, check and two typed finishes");
        Ok(())
    })?;
    assert!(ran.status.success(), "the gated journey exits cleanly");
    Ok(())
}

/// An approved change lands in the folder exactly as shown: the file is there with its bytes,
/// and git sees that one untracked file and nothing else changed.
#[test]
fn tui_journey_apply_writes_the_diff_into_the_folder() -> Journey {
    let replies = [COMMAND, CHECK_HELLO, FINISH_HELLO, FINISH_HELLO];
    let ran = frame(scripted(43, &replies), |pty, _, work| {
        ask(pty)?;
        pty.send("y")?;
        pty.wait_for("$ grep -q hello hello.txt")?;
        pty.send("y")?;
        pty.wait_for(REVIEW)?;
        pty.wait_for("hello.txt  new  +1 -0")?;
        assert!(!work.join("hello.txt").exists());
        pty.send("y")?;
        pty.wait_for(APPLIED)?;
        pty.wait_for("wrote hello.txt")
    })?;
    assert!(ran.status.success(), "exits 0: {}", ran.status);
    assert_eq!(
        std::fs::read_to_string(ran.work.path().join("hello.txt"))?,
        "hello\n"
    );
    let status = git_command(ran.work.path())
        .args(["status", "--porcelain"])
        .output()?;
    assert_eq!(String::from_utf8(status.stdout)?, "?? hello.txt\n");
    Ok(())
}
