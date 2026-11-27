//! The terminal lifecycle across a kill -9 (larry#256): a kill -9 while Larry
//! waits leaves nothing run, and a renamed session with transcript survives
//! the same kill — a fresh terminal lists it under `/resume`, continues it
//! with the old transcript and pane back, exports it as Markdown, and forks
//! it; the fork is renamed while the original row keeps its name, so the fork
//! diverged and the original is unchanged.

use super::*;

/// A kill -9 while Larry waits on the person leaves nothing run, and a fresh terminal starts
/// clean and quits 0.
#[test]
fn tui_journey_kill9_then_fresh_start_restores() -> Journey {
    let (state, work) = fixture()?;
    mock_model::run(scripted(3, &[COMMAND]), |mock| -> Journey {
        std::thread::scope(|owner| -> Journey {
            let env = operator(state.path(), mock);
            let mut pty = Pty::open(owner, work.path(), &env)?;
            ask(&mut pty)?;
            let pid = pty.pid()?;
            let killed = Command::new("kill").args(["-9", &pid]).status()?;
            assert!(killed.success(), "kill -9 {pid} was delivered");
            let (status, _) = pty.finish()?;
            assert!(!status.success(), "kill -9 left a success: {status}");
            assert_group_gone(&pid, "the killed binary")?;
            assert!(
                !work.path().join("hello.txt").exists(),
                "a killed ask runs nothing"
            );
            let mut fresh = Pty::open(owner, work.path(), &env)?;
            fresh.wait_for(READY)?;
            fresh.send(QUIT)?;
            assert_exited(
                &fresh.finish()?.0,
                "a fresh terminal starts clean and quits 0",
            );
            Ok(())
        })
    })
}

/// The issue's proof (larry#256): a renamed session survives a kill -9;
/// a fresh terminal resumes it, exports it and forks it, and the fork
/// diverges while the original is unchanged.
#[test]
fn tui_journey_resume_restores_transcript_and_pane() -> Journey {
    let (state, work) = fixture()?;
    mock_model::run(scripted(3, &[]), |mock| -> Journey {
        std::thread::scope(|owner| -> Journey {
            let env = operator(state.path(), mock);
            let mut pty = Pty::open(owner, work.path(), &env)?;
            pty.wait_for(READY)?;
            pty.send("/rename demo\r")?;
            pty.wait_for("renamed to demo")?;
            pty.send("/status\r")?;
            pty.wait_for("model: mock-model")?;
            let pid = pty.pid()?;
            let killed = Command::new("kill").args(["-9", &pid]).status()?;
            assert!(killed.success(), "kill -9 {pid} was delivered");
            let (status, _) = pty.finish()?;
            assert!(!status.success(), "kill -9 left a success: {status}");
            let mut fresh = Pty::open(owner, work.path(), &env)?;
            fresh.wait_for(READY)?;
            fresh.send("/resume\r")?;
            fresh.wait_for("demo")?;
            fresh.send("/resume demo\r")?;
            fresh.wait_for("resumed demo.")?;
            fresh.wait_for("model: mock-model")?;
            fresh.send("/export\r")?;
            fresh.wait_for("exported to")?;
            fresh.send("/fork\r")?;
            fresh.wait_for("forked demo as demo fork.")?;
            fresh.send("/rename forked-one\r")?;
            fresh.wait_for("renamed to forked-one.")?;
            fresh.send("/resume\r")?;
            fresh.wait_for("1. forked-one")?;
            fresh.wait_for("2. demo")?;
            fresh.send(QUIT)?;
            assert_exited(&fresh.finish()?.0, "Ctrl+C exits 0");
            let exported = std::fs::read_to_string(work.path().join("demo.md"))?;
            assert!(exported.contains("# demo"), "the export heads the session");
            assert!(
                exported.contains("## receipts"),
                "the export names the receipts"
            );
            Ok(())
        })
    })
}
