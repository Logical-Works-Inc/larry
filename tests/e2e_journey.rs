//! The shipped `larry` binary driven through its user journeys on a real PTY (larry#239).
//!
//! A person types a task, Larry shows the command the model chose and waits; `y` runs it and
//! `n` declines it. The seeded mock model stands in for the gateway, so no test makes a paid
//! call. Source: ECMA-48 (CSI, OSC), the DEC 2026 synchronized-output spec, POSIX `kill(2)`.
//! Requirement: the frame paints, asks before every command, never runs a declined one,
//! survives kill -9 with a clean next start, writes nothing while idle, and exits 0 on Ctrl+C.

#[path = "support/http_wire.rs"]
mod http_wire;
#[path = "support/journey.rs"]
mod journey;
#[path = "support/mock_model.rs"]
mod mock_model;
#[path = "support/origin.rs"]
mod origin;
#[path = "support/ps.rs"]
mod ps;
#[path = "support/pty.rs"]
mod pty;
#[path = "support/terminal_cases.rs"]
mod terminal_cases;

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use journey::{ASK, QUIT, READY, Ran, ask, frame, frame_in, operator, scripted, working};
use mock_model::{Config, Fault};
use origin::{fixture, git_command};
use pty::deadline::await_event;
use pty::{Journey, Pty};

/// The idle window the probe watches.
const IDLE: Duration = Duration::from_secs(5);
/// Input round trips the probe times.
const ROUND_TRIPS: usize = 20;
/// The question Larry asks once the session ends with a change in its copy of the folder.
const REVIEW: &str = "Apply this change";
/// What the frame says once that change is in the person's folder.
const APPLIED: &str = "applied to your folder";
/// What the model's command writes.
const COMMAND: &str = r#"{"command":"printf 'hello\n' > hello.txt"}"#;
/// What Larry tells the model about a declined command (`executor::session::DECLINED`).
const DECLINED: &str = "not run: the person declined this command";
/// Ctrl+U: clears the composer.
const CLEAR: &str = "\x15";

/// A digest of every entry beneath `root`, `.git` included: path, kind and bytes or link target.
fn tree_hash(root: &Path) -> Journey<String> {
    let mut hasher = lgwks_std::hash::Hasher::new();
    let mut pending = vec![std::path::PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(root.join(&relative))? {
            names.push(entry?.file_name());
        }
        names.sort();
        for name in names {
            let path = relative.join(&name);
            let full = root.join(&path);
            let meta = std::fs::symlink_metadata(&full)?;
            hasher.update(path.to_string_lossy().as_bytes());
            if meta.is_dir() {
                pending.push(path);
            } else if meta.is_symlink() {
                hasher.update(std::fs::read_link(&full)?.to_string_lossy().as_bytes());
            } else {
                hasher.update(&std::fs::read(&full)?);
            }
        }
    }
    Ok(hasher.finalize().to_hex())
}

/// Whether the model's command wrote its file in `ran`'s folder.
fn wrote(ran: &Ran) -> bool {
    ran.work.path().join("hello.txt").exists()
}

/// The journey: a task typed into the frame, the model's command shown and approved with `y`,
/// the file written in the working directory, the model's summary painted, Ctrl+C exits 0.
#[test]
fn tui_journey_runs_task_with_mock_gateway() -> Journey {
    let replies = [COMMAND, r#"{"done":"wrote hello.txt"}"#];
    let ran = frame(scripted(7, &replies), |pty, mock, work| {
        ask(pty)?;
        assert!(
            !work.join("hello.txt").exists(),
            "nothing runs before the person answers"
        );
        pty.send("y")?;
        // The command ran in a copy: the person's folder changes only at the review.
        pty.wait_for(REVIEW)?;
        assert!(
            !work.join("hello.txt").exists(),
            "an approved command never writes the folder itself"
        );
        pty.send("y")?;
        pty.wait_for(APPLIED)?;
        pty.wait_for("wrote hello.txt")?;
        assert_eq!(mock.served(), 2, "the command, then the summary");
        Ok(())
    })?;
    assert!(
        ran.status.success(),
        "Ctrl+C on an empty box exits 0: {}",
        ran.status
    );
    assert_eq!(
        std::fs::read_to_string(ran.work.path().join("hello.txt"))?,
        "hello\n"
    );
    assert!(ran.wire.contains("\x1b[?2026h"), "frames are synchronized");
    assert!(
        ran.wire.contains("\x1b[?1049l"),
        "the main screen is restored"
    );
    Ok(())
}

/// A first-timer's bare `larry` in a folder its state never provisioned still
/// reaches the approval prompt for the model's first command: the local door
/// stays open until the first root row binds it, so no failure line appears
/// first (larry#207).
#[test]
fn tui_journey_unprovisioned_local_reaches_the_first_approval() -> Journey {
    let state = origin::TempDir::new()?;
    let work = origin::TempDir::new()?;
    std::fs::write(work.path().join("notes.txt"), "a first-timer's project\n")?;
    let replies = [COMMAND, r#"{"done":"left it alone"}"#];
    let ran = frame_in(
        &state,
        work,
        scripted(7, &replies),
        Box::new(|pty, mock, _| {
            ask(pty)?;
            pty.send("n")?;
            pty.wait_for("left it alone")?;
            let told = mock
                .requests()
                .get(1)
                .is_some_and(|body| String::from_utf8_lossy(body).contains(DECLINED));
            assert!(told, "the model is told the command was declined");
            Ok(())
        }),
    )?;
    assert!(
        ran.status.success(),
        "Ctrl+C on an empty box exits 0: {}",
        ran.status
    );
    assert!(!wrote(&ran), "a declined command never runs");
    Ok(())
}

/// A declined command never runs: `n` at the question, the model is told, the file never
/// appears, and the model's next request carries the decline.
#[test]
fn sim_session_declined_command_never_runs() -> Journey {
    for seed in 0..4 {
        let replies = [COMMAND, r#"{"done":"left it alone"}"#];
        let ran = frame(scripted(seed, &replies), |pty, mock, _| {
            ask(pty)?;
            pty.send("n")?;
            pty.wait_for("left it alone")?;
            let requests = mock.requests();
            let told = requests.get(1).ok_or("the model was asked again")?;
            assert!(
                String::from_utf8_lossy(told).contains(DECLINED),
                "seed {seed}: the model is told the command was declined"
            );
            Ok(())
        })?;
        assert!(ran.status.success(), "seed {seed}: exits 0: {}", ran.status);
        assert!(!wrote(&ran), "seed {seed}: a declined command ran");
    }
    Ok(())
}

/// A key typed before the question is painted never answers it, and Enter alone declines
/// (larry#325): the model call is held, so the `y` is read while the frame still shows Working,
/// the frame shows it in the box, and the next request tells the model the command was declined.
#[test]
fn tui_journey_a_key_typed_before_the_question_never_runs_the_command() -> Journey {
    let config = Config {
        script: vec![Some(Fault::Delay)],
        ..scripted(53, &[COMMAND, r#"{"done":"left it alone"}"#])
    };
    let ran = frame(config, |pty, mock, _| {
        working(pty)?;
        // The box takes "zzy" while the model is held, so its `y` answers no question.
        pty.send("zzy")?;
        pty.wait_for("zzy")?;
        mock.release_delayed();
        pty.wait_for(ASK)?;
        pty.send("\r")?;
        pty.wait_for("left it alone")?;
        let told = mock
            .requests()
            .get(1)
            .is_some_and(|body| String::from_utf8_lossy(body).contains(DECLINED));
        assert!(told, "the model is told the command was declined");
        pty.send(CLEAR)?;
        pty.wait_for(READY)?;
        Ok(())
    })?;
    assert!(ran.status.success(), "exits 0: {}", ran.status);
    assert!(!wrote(&ran), "a key typed before the question ran it");
    Ok(())
}

/// A gateway that answers 401 with escapes, a bell and 4 KiB of text (`Fault::HostileRefusal`,
/// larry#128): the frame shows a notice, no byte of the hostile text reaches the terminal as an
/// escape or a bell, nothing runs, and the operator's key went as its bearer token.
#[test]
fn sim_tui_gateway_refusal_shows_typed_notice() -> Journey {
    let config = Config {
        script: vec![Some(Fault::HostileRefusal { status: 401 })],
        ..scripted(5, &[COMMAND])
    };
    let ran = frame(config, |pty, mock, _| {
        working(pty)?;
        pty.wait_for("expired")?;
        let first = mock.authorizations().into_iter().next().flatten();
        assert_eq!(first.as_deref(), Some("Bearer tui-key"), "the route's key");
        Ok(())
    })?;
    assert!(
        ran.status.success(),
        "a refusal leaves the frame usable: {}",
        ran.status
    );
    assert!(
        !ran.wire.contains("\x1b[31m"),
        "the gateway's colour never reaches the wire"
    );
    assert!(
        !ran.wire.contains('\u{7}'),
        "the gateway's bell never reaches the wire"
    );
    assert!(!wrote(&ran), "a refused session runs nothing");
    Ok(())
}

/// larry#396: a reply the gateway cut at its token limit ends the session with a notice naming
/// `finish_reason`; an empty reply is a refused reply the model is told about, and once three in
/// a row are refused the session ends. Neither runs anything, and the frame stays usable.
#[test]
fn sim_tui_cut_or_empty_reply_runs_nothing() -> Journey {
    for (fault, notice) in [
        (Fault::Finish { reason: "length" }, "finish_reason"),
        (Fault::EmptyContent, "refused"),
    ] {
        let config = Config {
            script: vec![Some(fault)],
            ..scripted(6, &[COMMAND])
        };
        let ran = frame(config, |pty, _, _| {
            working(pty)?;
            pty.wait_for(notice)
        })?;
        assert!(
            ran.status.success(),
            "{fault:?}: the frame stays usable: {}",
            ran.status
        );
        assert!(!wrote(&ran), "{fault:?}: an unfinished reply runs nothing");
    }
    Ok(())
}

/// Esc while the model call is held (`Fault::Delay`) stops the session: the reply that then
/// arrives is never run, the frame says so, and Ctrl+C exits 0 with the worker joined.
#[test]
fn sim_tui_esc_cancels_and_joins_worker() -> Journey {
    let config = Config {
        script: vec![Some(Fault::Delay)],
        ..scripted(9, &[COMMAND])
    };
    let ran = frame(config, |pty, mock, _| {
        working(pty)?;
        pty.send("\x1b")?;
        pty.wait_for("Stopping")?;
        mock.release_delayed();
        pty.wait_for("Interrupted")
    })?;
    assert!(
        ran.status.success(),
        "the worker is joined and the frame exits 0: {}",
        ran.status
    );
    assert!(!wrote(&ran), "an interrupted reply never runs");
    Ok(())
}

/// Esc ends a running command, not only the next step (larry#242): the mock model asks for a
/// command with a background member, the person approves it, Esc kills the whole group, the
/// frame says Interrupted, the background pid is gone, and Ctrl+C exits 0. The whole path is
/// printed as `esc_to_interrupted_ms`; it is MEASURED, never a pass condition, because a host
/// with every core busy moves it (larry#268).
#[test]
fn tui_journey_esc_kills_running_command() -> Journey {
    let replies = [r#"{"command":"sleep 30 & echo $! > bg.pid; sleep 30"}"#];
    let (state, work) = fixture()?;
    let workspaces = state.path().join("ws");
    let config = scripted(31, &replies);
    let ran = frame_in(
        &state,
        work,
        config,
        Box::new(|pty, mock, _| {
            ask(pty)?;
            pty.send("y")?;
            pty.wait_for("esc to interrupt")?;
            // Esc is pressed only once the background member is up, so its death is the group's,
            // not a race with the shell's own start. The command runs in the session's copy.
            // The shell creates bg.pid before `echo` writes into it, so the wait is for the file
            // to hold the pid, not only to exist (an empty read flaked on Linux, PR #251). The
            // copy goes with the interrupted session, so the pid is read before Esc, and a pid
            // that never appears fails the wait inside one deadline (larry#268).
            let pid = await_event("the background member wrote bg.pid", || {
                Ok(std::fs::read_dir(&workspaces)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|entry| entry.path().join("tree").join("bg.pid"))
                    .filter_map(|path| std::fs::read_to_string(path).ok())
                    .map(|text| text.trim().to_owned())
                    .find(|text| !text.is_empty()))
            })?;
            let pressed = Instant::now();
            pty.send("\x1b")?;
            pty.wait_for("Interrupted")?;
            let esc_to_interrupted = pressed.elapsed();
            assert!(!pid.is_empty(), "bg.pid holds the background pid");
            let probe = Command::new("kill")
                .args(["-0", &pid])
                .stderr(Stdio::null())
                .status()?;
            assert!(
                !probe.success(),
                "the background member {pid} died with its group"
            );
            assert_eq!(mock.served(), 1, "no second model call after the interrupt");
            let mut figure = String::new();
            writeln!(
                figure,
                "{{\"esc_to_interrupted_ms\":{:.0}}}",
                esc_to_interrupted.as_secs_f64() * 1000.0
            )?;
            std::io::stderr().lock().write_all(figure.as_bytes())?;
            Ok(())
        }),
    )?;
    assert!(
        ran.status.success(),
        "Ctrl+C on an empty box exits 0: {}",
        ran.status
    );
    Ok(())
}

/// One seed, one conversation: the approve journey run twice in the same folders against an
/// OpenRouter-style padded reply (`Fault::PaddedDone`) gives the gateway the same requests in
/// the same order.
#[test]
fn sim_tui_same_seed_same_trace_hash_through_the_binary() -> Journey {
    let mut hashes = Vec::new();
    let (state, mut work) = fixture()?;
    for _ in 0..2 {
        let config = Config {
            script: vec![Some(Fault::PaddedDone)],
            ..scripted(21, &[COMMAND, r#"{"done":"wrote hello.txt"}"#])
        };
        let ran = frame_in(
            &state,
            work,
            config,
            Box::new(|pty, mock, _| {
                ask(pty)?;
                pty.send("y")?;
                pty.wait_for(REVIEW)?;
                pty.send("y")?;
                pty.wait_for("wrote hello.txt")?;
                hashes.push(mock.trace_hash());
                Ok(())
            }),
        )?;
        assert!(ran.status.success() && wrote(&ran));
        std::fs::remove_file(ran.work.path().join("hello.txt"))?;
        work = ran.work;
    }
    assert_eq!(hashes.first(), hashes.get(1), "same seed, same trace");
    Ok(())
}

/// The issue's proof (larry#243): the model asks for a write outside the workspace, the person
/// approves it, the kernel refuses it, the person discards the rest, and the folder is
/// byte-identical, `.git` included.
#[test]
fn tui_journey_write_outside_workspace_leaves_origin_identical() -> Journey {
    let (state, work) = fixture()?;
    let escape = format!(
        r#"{{"command":"printf x > {}/escape.txt; printf y > inside.txt"}}"#,
        work.path().display()
    );
    let replies = [escape.as_str(), r#"{"done":"kept out"}"#];
    let before = tree_hash(work.path())?;
    let ran = frame_in(
        &state,
        work,
        scripted(41, &replies),
        Box::new(|pty, _, origin| {
            ask(pty)?;
            pty.send("y")?;
            pty.wait_for(REVIEW)?;
            // The write outside the copy never reached it: only the inside write is offered.
            pty.wait_for("inside.txt  new  +1 -0")?;
            assert!(
                !origin.join("escape.txt").exists() && !origin.join("inside.txt").exists(),
                "nothing reaches the folder before the person answers"
            );
            pty.send("n")?;
            pty.wait_for("discarded")?;
            pty.wait_for("kept out")
        }),
    )?;
    assert!(ran.status.success(), "exits 0: {}", ran.status);
    assert_eq!(
        tree_hash(ran.work.path())?,
        before,
        "the origin tree is byte-identical"
    );
    Ok(())
}

/// An approved change lands in the folder exactly as shown: the file is there with its bytes,
/// and git sees that one untracked file and nothing else changed.
#[test]
fn tui_journey_apply_writes_the_diff_into_the_folder() -> Journey {
    let replies = [COMMAND, r#"{"done":"wrote hello.txt"}"#];
    let ran = frame(scripted(43, &replies), |pty, _, work| {
        ask(pty)?;
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

/// The terminal's shell gets the PATH built from where its tools resolve, never the host's
/// (larry#269, larry#296): the model's command writes `$PATH` inside the confined copy and the
/// review shows exactly that PATH, so a host tool that shadows `git`, `sh` or `sleep` cannot
/// change what a journey runs.
#[test]
fn the_terminal_gets_the_fixed_path_not_the_host() -> Journey {
    let expected = pty::test_path()?;
    let replies = [
        r#"{"command":"printf '%s' \"$PATH\" > path.txt"}"#,
        r#"{"done":"read the path"}"#,
    ];
    let ran = frame(scripted(47, &replies), |pty, _, _| {
        ask(pty)?;
        pty.send("y")?;
        pty.wait_for(REVIEW)?;
        pty.wait_for(expected)?;
        pty.send("n")?;
        pty.wait_for("read the path")
    })?;
    assert!(ran.status.success(), "Ctrl+C exits 0: {}", ran.status);
    Ok(())
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

/// Files in the bulk folders of the overhead measurement, and their size in bytes.
const BULK_FILES: usize = 600;
/// Bytes of each bulk file.
const BULK_BYTES: usize = 2048;
/// Rounds of the overhead measurement.
const BULK_ROUNDS: usize = 5;

/// Time from the person's Enter to Larry's question about the first command, which includes the
/// workspace snapshot and the confinement probe, against a plain `cp -R` of the same folder (the
/// rival): the whole path, with its PTY and its gateway, over `BULK_ROUNDS` fresh folders of
/// `BULK_FILES` files. Prints the figures; asserts only that every round was measured.
#[test]
fn tui_journey_probe_reports_isolation_overhead_against_a_plain_copy() -> Journey {
    let mut isolated = Vec::new();
    let mut plain = Vec::new();
    for round in 0..BULK_ROUNDS {
        let (state, work) = fixture()?;
        for file in 0..BULK_FILES {
            let bucket = file.checked_div(100).ok_or("a bucket")?;
            let folder = work.path().join(format!("bulk{bucket}"));
            std::fs::create_dir_all(&folder)?;
            std::fs::write(folder.join(format!("f{file}.txt")), "x".repeat(BULK_BYTES))?;
        }
        let rival = state.path().join("plain-copy");
        let started = Instant::now();
        let copied = Command::new("cp")
            .arg("-R")
            .arg(work.path())
            .arg(&rival)
            .status()?;
        plain.push(started.elapsed());
        assert!(copied.success(), "the rival copy ran");
        let replies = [COMMAND, r#"{"done":"left it alone"}"#];
        let config = scripted(u64::try_from(round)?, &replies);
        let ran = frame_in(
            &state,
            work,
            config,
            Box::new(|pty, _, _| {
                pty.wait_for(READY)?;
                let sent = Instant::now();
                pty.send("write hello.txt\r")?;
                pty.wait_for(ASK)?;
                isolated.push(sent.elapsed());
                pty.send("n")?;
                pty.wait_for("left it alone")
            }),
        )?;
        assert!(
            ran.status.success(),
            "round {round}: exits 0: {}",
            ran.status
        );
    }
    isolated.sort();
    plain.sort();
    let mut figures = String::new();
    writeln!(
        figures,
        "{{\"scope\":\"debug binary, PTY, mock gateway, {BULK_FILES} files of {BULK_BYTES} bytes\",\
         \"rounds\":{BULK_ROUNDS},\"enter_to_question_ms\":{{\"p50\":{:.1},\"p95\":{:.1},\"p99\":{:.1}}},\
         \"plain_cp_r_ms\":{{\"p50\":{:.1},\"p95\":{:.1},\"p99\":{:.1}}}}}",
        nearest_rank(&isolated, 50)?,
        nearest_rank(&isolated, 95)?,
        nearest_rank(&isolated, 99)?,
        nearest_rank(&plain, 50)?,
        nearest_rank(&plain, 95)?,
        nearest_rank(&plain, 99)?,
    )?;
    std::io::stderr().lock().write_all(figures.as_bytes())?;
    assert_eq!((isolated.len(), plain.len()), (BULK_ROUNDS, BULK_ROUNDS));
    Ok(())
}

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
            assert!(
                !status.success(),
                "a killed binary does not report success: {status}"
            );
            let probe = Command::new("kill")
                .args(["-0", &pid])
                .stderr(Stdio::null())
                .status()?;
            assert!(!probe.success(), "the killed binary {pid} was reaped");
            assert!(
                !work.path().join("hello.txt").exists(),
                "a killed ask runs nothing"
            );
            let mut fresh = Pty::open(owner, work.path(), &env)?;
            fresh.wait_for(READY)?;
            fresh.send(QUIT)?;
            let (status, _) = fresh.finish()?;
            assert!(
                status.success(),
                "a fresh terminal starts clean and quits 0: {status}"
            );
            Ok(())
        })
    })
}

/// Two terminals at once, each with its own state directory, gateway and folder: each runs
/// its own command and seals its own report, and neither sees the other's.
#[test]
fn tui_journey_two_state_dirs_keep_their_own_reports() -> Journey {
    let (state_a, work_a) = fixture()?;
    let (state_b, work_b) = fixture()?;
    let replies = [COMMAND, r#"{"done":"wrote hello.txt"}"#];
    mock_model::run(scripted(11, &replies), |mock_a| -> Journey {
        mock_model::run(scripted(12, &replies), |mock_b| -> Journey {
            std::thread::scope(|owner| -> Journey {
                let mut first = Pty::open(owner, work_a.path(), &operator(state_a.path(), mock_a))?;
                let mut second =
                    Pty::open(owner, work_b.path(), &operator(state_b.path(), mock_b))?;
                ask(&mut first)?;
                ask(&mut second)?;
                first.send("y")?;
                second.send("y")?;
                first.wait_for(REVIEW)?;
                second.wait_for(REVIEW)?;
                first.send("y")?;
                second.send("y")?;
                first.wait_for("wrote hello.txt")?;
                second.wait_for("wrote hello.txt")?;
                first.send(QUIT)?;
                second.send(QUIT)?;
                assert!(first.finish()?.0.success() && second.finish()?.0.success());
                assert_eq!(
                    (mock_a.served(), mock_b.served()),
                    (2, 2),
                    "one gateway each"
                );
                for (state, other) in [(&state_a, &state_b), (&state_b, &state_a)] {
                    let mine = std::fs::read_dir(state.path())?.count();
                    assert!(mine > 0, "each state directory holds its own records");
                    assert_ne!(state.path(), other.path());
                }
                assert!(work_a.path().join("hello.txt").exists());
                assert!(work_b.path().join("hello.txt").exists());
                Ok(())
            })
        })
    })
}

/// CPU seconds and RSS KiB of `pid`, from the probe's own `ps` sample (larry#269).
fn sample(pid: &str) -> Journey<(f64, u64)> {
    ps::sample(pid).ok_or_else(|| format!("ps reported nothing for {pid}").into())
}

/// The `rank`th percentile of `sorted`, by nearest rank, in milliseconds.
fn nearest_rank(sorted: &[Duration], rank: usize) -> Journey<f64> {
    let position = sorted
        .len()
        .saturating_mul(rank)
        .div_ceil(100)
        .saturating_sub(1);
    let value = sorted.get(position).ok_or("no samples")?;
    Ok(value.as_secs_f64() * 1000.0)
}

/// First paint, key-to-paint p50/p95/p99 over typed text, an idle window that writes nothing,
/// peak RSS, and Ctrl+C to exit.
#[test]
fn tui_journey_probe_reports_p50_p95_p99_and_rss() -> Journey {
    let (state, work) = fixture()?;
    let env = [("LARRY_STATE_DIR", state.path().as_os_str().to_owned())];
    std::thread::scope(|owner| -> Journey {
        let started = Instant::now();
        let mut pty = Pty::open(owner, work.path(), &env)?;
        pty.wait_for(READY)?;
        let first_paint = started.elapsed().as_secs_f64() * 1000.0;
        pty.drain(Duration::from_millis(100))?;
        let mut latencies = Vec::with_capacity(ROUND_TRIPS);
        for round in 0..ROUND_TRIPS {
            let sent = Instant::now();
            let token = format!("zq{round}x");
            pty.send(&token)?;
            pty.wait_for(&token)?;
            latencies.push(sent.elapsed());
            pty.send(CLEAR)?;
            pty.wait_for(READY)?;
        }
        latencies.sort();
        let pid = pty.pid()?;
        let (cpu_before, rss_before) = sample(&pid)?;
        let idle_started = Instant::now();
        let idle_bytes = pty.drain(IDLE)?;
        let idle_seconds = idle_started.elapsed().as_secs_f64();
        let (cpu_after, rss_after) = sample(&pid)?;
        let quit = Instant::now();
        pty.send(QUIT)?;
        let (status, _) = pty.finish()?;
        let quit_ms = quit.elapsed().as_secs_f64() * 1000.0;
        assert!(status.success(), "Ctrl+C exits 0: {status}");
        // `idle_output_bytes` below is a MEASURED figure, never a pass condition: a frame a
        // loaded host paints late can land inside the window (larry#268).
        // `sim_tui_unchanged_frame_writes_nothing` proves deterministically that a repaint of an
        // unchanged frame writes nothing.
        let figures = format!(
            "{{\"scope\":\"debug binary, 120x32 PTY, no model route\",\"os\":\"{}\",\
             \"first_paint_ms\":{first_paint:.1},\"input_samples\":{ROUND_TRIPS},\
             \"input_p50_ms\":{:.1},\"input_p95_ms\":{:.1},\"input_p99_ms\":{:.1},\
             \"idle_seconds\":{idle_seconds:.2},\"idle_cpu_percent\":{:.2},\
             \"idle_output_bytes\":{idle_bytes},\"sampled_max_rss_kib\":{},\
             \"quit_to_exit_ms\":{quit_ms:.1}}}\n",
            std::env::consts::OS,
            nearest_rank(&latencies, 50)?,
            nearest_rank(&latencies, 95)?,
            nearest_rank(&latencies, 99)?,
            (cpu_after - cpu_before) / idle_seconds * 100.0,
            rss_before.max(rss_after),
        );
        std::io::stderr().lock().write_all(figures.as_bytes())?;
        Ok(())
    })
}
