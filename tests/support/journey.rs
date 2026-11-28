//! The shared journey harness (larry#323): one shipped `larry` frame under a scripted mock
//! gateway on a real PTY, so two journey files drive the binary the same way. Included by
//! `tests/e2e_journey.rs` and `tests/terminal_journey.rs` after their support modules.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use super::mock_model::{Config, Handle};
use super::origin::{TempDir, fixture};
use super::pty::deadline::await_event;
use super::pty::{Journey, Pty};

/// Ctrl+C: quits from an empty composer.
pub(crate) const QUIT: &str = "\x03";

/// The composer's placeholder: the frame is up and takes a task.
pub(crate) const READY: &str = "Ask Larry to do something";

/// The question Larry asks before a command.
pub(crate) const ASK: &str = "Run this command?";

/// The operator's settings for a terminal session against `mock` in `state`: the default
/// policy, which holds no `model.command`, because each command runs in an isolated copy of the
/// folder (larry#243).
pub(crate) fn operator(state: &Path, mock: &Handle) -> Vec<(&'static str, OsString)> {
    vec![
        ("LARRY_STATE_DIR", state.as_os_str().to_owned()),
        (
            "LARRY_MODEL_URL",
            format!("http://{}/v1", mock.addr()).into(),
        ),
        ("LARRY_MODEL", OsString::from("mock-model")),
        ("LARRY_MODEL_KEY", OsString::from("tui-key")),
        ("LARRY_MODEL_BACKOFF_MS", OsString::from("0")),
    ]
}

/// A mock whose replies are `contents`, in order.
pub(crate) fn scripted(seed: u64, contents: &[&str]) -> Config {
    Config {
        contents: contents.iter().map(|text| (*text).to_owned()).collect(),
        ..Config::seeded(seed)
    }
}

/// Types a task and waits for Larry's question about the model's first command.
pub(crate) fn ask(pty: &mut Pty<'_>) -> Journey {
    pty.wait_for(READY)?;
    pty.send("write hello.txt\r")?;
    pty.wait_for(ASK)
}

/// Types a task and waits for the frame to show the session working.
pub(crate) fn working(pty: &mut Pty<'_>) -> Journey {
    pty.wait_for(READY)?;
    pty.send("write hello.txt\r")?;
    pty.wait_for("Working")
}

/// What one frame left once Ctrl+C closed it.
pub(crate) struct Ran {
    /// The binary's exit status.
    pub(crate) status: ExitStatus,
    /// Every byte it wrote to the terminal.
    pub(crate) wire: String,
    /// The folder the session ran in.
    pub(crate) work: TempDir,
}

/// The driver of one frame: types keys, waits for text, reads the mock.
pub(crate) type Body<'body> = Box<dyn FnOnce(&mut Pty<'_>, &Handle, &Path) -> Journey + 'body>;

/// One frame against a mock gateway in fresh folders.
pub(crate) fn frame<'body>(
    config: Config,
    body: impl FnOnce(&mut Pty<'_>, &Handle, &Path) -> Journey + 'body,
) -> Journey<Ran> {
    let (state, work) = super::origin::fixture()?;
    frame_in(&state, work, config, Box::new(body))
}

/// One frame in `state` and `work`: `body` drives it, then Ctrl+C closes it and it is reaped.
pub(crate) fn frame_in(
    state: &TempDir,
    work: TempDir,
    config: Config,
    body: Body<'_>,
) -> Journey<Ran> {
    let (status, raw) = super::mock_model::run(config, |mock| -> Journey<(ExitStatus, Vec<u8>)> {
        std::thread::scope(|owner| {
            let mut pty = Pty::open(owner, work.path(), &operator(state.path(), mock))?;
            body(&mut pty, mock, work.path())?;
            pty.send(QUIT)?;
            pty.finish()
        })
    })?;
    let wire = String::from_utf8_lossy(&raw).into_owned();
    Ok(Ran { status, wire, work })
}

/// The command with a background member both group-kill journeys approve (larry#242,
/// larry#262): a sleep that plants its background pid in `bg.pid`, then sleeps.
pub(crate) const BACKGROUND_SLEEP: &str = r#"{"command":"sleep 30 & echo $! > bg.pid; sleep 30"}"#;

/// The background member's pid: waits for `bg.pid` in the session's copy `workspaces`,
/// where the confined command wrote it. The copy goes with an interrupted session, so the
/// pid is read before the stop, and an empty read flaked (PR #251). Shared by the Esc kill
/// journey (larry#242) and `/stop` (larry#262).
pub(crate) fn background_pid(workspaces: &Path) -> Journey<String> {
    await_event("the background member wrote bg.pid", || {
        Ok(std::fs::read_dir(workspaces)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path().join("tree").join("bg.pid"))
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .map(|text| text.trim().to_owned())
            .find(|text| !text.is_empty()))
    })
}

/// Fresh state and work driving the background-sleep mock, with the session-copy
/// workspaces its pid is read from: the frame both group-kill journeys approve and
/// stop (larry#242, larry#262).
pub(crate) fn background_kill_frame() -> Journey<(TempDir, TempDir, PathBuf, Config)> {
    let (state, work) = fixture()?;
    let workspaces = state.path().join("ws");
    let replies = [BACKGROUND_SLEEP];
    let config = scripted(31, &replies);
    Ok((state, work, workspaces, config))
}

/// Checks the background member `pid` died with its whole process group and the mock served
/// one model call: the group kill Esc (larry#242) and `/stop` (larry#262) each prove, `why`
/// naming the stop for the failure line.
pub(crate) fn background_group_died(pid: &str, mock: &Handle, why: &str) -> Journey {
    assert!(!pid.is_empty(), "bg.pid holds the background pid");
    let probe = Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()?;
    assert!(
        !probe.success(),
        "the background member {pid} died with its group"
    );
    assert_eq!(mock.served(), 1, "no second model call after {why}");
    Ok(())
}

/// Approves the background-sleep command and proves `stop` kills its whole process group:
/// waits for the pid file in the session copy (read before the stop: the copy goes with an
/// interrupted session, and an empty read flaked, PR #251), runs `stop`, and checks the member
/// died with one model call served. Esc (larry#242) sends its key with timing; `/stop`
/// (larry#262) lists with `/ps` first. `why` names the stop for the failure line.
pub(crate) fn background_kill_journey(
    stop: impl FnOnce(&mut Pty<'_>) -> Journey,
    why: &str,
) -> Journey<Ran> {
    let (state, work, workspaces, config) = background_kill_frame()?;
    let ran = frame_in(
        &state,
        work,
        config,
        Box::new(|pty, mock, _| {
            ask(pty)?;
            pty.send("y")?;
            pty.wait_for("esc to interrupt")?;
            let pid = background_pid(&workspaces)?;
            stop(pty)?;
            background_group_died(&pid, mock, why)?;
            Ok(())
        }),
    )?;
    Ok(ran)
}
