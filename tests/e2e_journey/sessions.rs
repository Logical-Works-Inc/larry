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

/// Kills the terminal's `larry` with `kill -9`, which runs no destructor, and reaps it.
fn kill_hard(pty: Pty<'_>) -> Journey {
    let pid = pty.pid()?;
    let killed = Command::new("kill").args(["-9", &pid]).status()?;
    assert!(killed.success(), "kill -9 {pid} was delivered");
    let (status, _) = pty.finish()?;
    assert!(!status.success(), "kill -9 left a success: {status}");
    Ok(())
}

/// Runs `body` with the operator's settings for a mock gateway that follows `config`.
fn against(
    state: &Path,
    config: Config,
    body: impl FnOnce(&[(&'static str, std::ffi::OsString)]) -> Journey,
) -> Journey {
    mock_model::run(config, |mock| body(&operator(state, mock)))
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
            kill_hard(pty)?;
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

/// Adds `held` rows beside the one row under `state`, same tenant and folder, 2 KiB each.
fn fill(state: &Path, held: u32) -> Journey {
    let added = Connection::open(state.join("executor.db"))?.execute(
        "WITH RECURSIVE held(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM held WHERE n < ?1) \
         INSERT INTO tui_sessions (tenant, id, name, folder, model, updated, transcript) \
         SELECT tenant, printf('row-%05d', n), printf('held %05d', n), folder, model, n, \
         printf('%.2048c', 'x') FROM held, (SELECT tenant, folder, model FROM tui_sessions LIMIT 1)",
        [held],
    )?;
    assert_eq!(added, usize::try_from(held)?, "every row was added");
    Ok(())
}

/// `/resume` over a full tenant, ten thousand sessions of one folder, through the shipped
/// binary: keys to painted listing p50/p95/p99 and the RSS it held, printed and never a pass
/// condition.
#[test]
fn tui_journey_a_full_session_store_lists_and_reports_latency_and_rss() -> Journey {
    let (state, work) = fixture()?;
    against(state.path(), scripted(10, &[]), |env| -> Journey {
        std::thread::scope(|owner| -> Journey {
            let mut first = Pty::open(owner, work.path(), env)?;
            first.wait_for(READY)?;
            first.send(QUIT)?;
            assert_exited(&first.finish()?.0, "Ctrl+C exits 0");
            // With the next terminal's own row the tenant is at its bound of 10,000.
            fill(state.path(), 9_998)?;
            let mut pty = Pty::open(owner, work.path(), env)?;
            pty.wait_for(READY)?;
            let pid = pty.pid()?;
            let mut latencies = Vec::with_capacity(ROUND_TRIPS);
            let mut peak = 0;
            for round in 0..ROUND_TRIPS {
                let sent = Instant::now();
                pty.send("/resume\r")?;
                // 9,999 other rows, the newest 20 listed.
                pty.wait_for("9979 older;")?;
                latencies.push(sent.elapsed());
                peak = peak.max(sample(&pid)?.1);
                // The echo of what is typed after /clear shows the listing is gone.
                pty.send("/clear\r")?;
                let token = format!("zq{round}x");
                pty.send(&token)?;
                pty.wait_for(&token)?;
                pty.send(CLEAR)?;
                pty.wait_for(READY)?;
            }
            pty.send(QUIT)?;
            assert_exited(&pty.finish()?.0, "Ctrl+C exits 0");
            print_figures("/resume over 10000 sessions", "list", &mut latencies, peak)
        })
    })
}

/// Runs the mock's first task to its applied change and its painted receipt.
fn apply_a_task(pty: &mut Pty<'_>) -> Journey {
    ask(pty)?;
    pty.send("y")?;
    pty.wait_for(REVIEW)?;
    pty.send("y")?;
    pty.wait_for(APPLIED)?;
    pty.wait_for("digest    ")
}

/// The first receipt digest the one row under `state` holds.
fn stored_digest(state: &Path) -> Journey<String> {
    let (_, _, transcript) = rows(state)?.into_iter().next().ok_or("the row")?;
    let held: lgwks_std::json::Value = lgwks_std::json::from_str(&transcript)?;
    held.get("receipts")
        .and_then(|receipts| receipts.get(0))
        .and_then(|receipt| receipt.get("digest"))
        .and_then(lgwks_std::json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("the row holds no receipt digest: {transcript}").into())
}

/// Keys to painted line of `/export`, each to a new file, and the RSS it held: printed, never a
/// pass condition.
fn export_figures(pty: &mut Pty<'_>) -> Journey {
    let pid = pty.pid()?;
    let mut latencies = Vec::with_capacity(ROUND_TRIPS);
    let mut peak = 0;
    for round in 0..ROUND_TRIPS {
        let sent = Instant::now();
        pty.send(&format!("/export round{round}.md\r"))?;
        pty.wait_for(&format!("exported to round{round}.md"))?;
        latencies.push(sent.elapsed());
        peak = peak.max(sample(&pid)?.1);
    }
    print_figures(
        "/export of a one task session",
        "export",
        &mut latencies,
        peak,
    )
}

/// One line of latency percentiles and the sampled peak RSS on standard error.
fn print_figures(scope: &str, name: &str, latencies: &mut [Duration], peak: u64) -> Journey {
    latencies.sort();
    let figures = format!(
        "{{\"scope\":\"shipped debug binary, {scope}\",\"samples\":{},\"{name}_p50_ms\":{:.1},\
         \"{name}_p95_ms\":{:.1},\"{name}_p99_ms\":{:.1},\"sampled_max_rss_kib\":{peak}}}\n",
        latencies.len(),
        nearest_rank(latencies, 50)?,
        nearest_rank(latencies, 95)?,
        nearest_rank(latencies, 99)?,
    );
    Ok(std::io::stderr().lock().write_all(figures.as_bytes())?)
}

/// `/recap` sums the session with no further model call, `/export` writes it inside the folder
/// as Markdown with every receipt digest, and it never overwrites a file, follows a link or
/// leaves the folder (larry#609). Source: `CommonMark` 0.31 section 4.5 (fenced code), CWE-22
/// (path traversal), CWE-59 (link following).
#[test]
fn tui_journey_export_writes_the_session_and_recap_sums_it() -> Journey {
    use std::os::unix::fs::PermissionsExt as _;
    let (state, work) = fixture()?;
    let (_, elsewhere) = fixture()?;
    let replies = [COMMAND, r#"{"done":"wrote hello.txt"}"#];
    against(state.path(), scripted(609, &replies), |env| -> Journey {
        std::thread::scope(|owner| -> Journey {
            let mut pty = Pty::open(owner, work.path(), env)?;
            apply_a_task(&mut pty)?;
            pty.wait_for("wrote hello.txt")?;
            std::os::unix::fs::symlink(elsewhere.path(), work.path().join("out"))?;

            pty.send("/recap\r")?;
            pty.wait_for("recap: 1 task, 1 command")?;
            pty.wait_for("last answer: wrote hello.txt")?;
            pty.send("/export\r")?;
            pty.wait_for("exported to session-1.md")?;
            let written = work.path().join("session-1.md");
            let exported = std::fs::read_to_string(&written)?;
            let digest = stored_digest(state.path())?;
            for held in [&digest, "write hello.txt", "printf 'hello", "applied"] {
                assert!(exported.contains(held), "{held:?} is missing: {exported}");
            }
            let mode = std::fs::metadata(&written)?.permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "a transcript is the person's alone");

            pty.send("/export\r")?;
            pty.wait_for("session-1.md already exists")?;
            assert_eq!(std::fs::read_to_string(&written)?, exported);
            for (typed, said) in [
                ("../escape.md", "../escape.md is outside"),
                ("out/escape.md", "never follows a link"),
                (".git/escape.md", ".git/escape.md is inside"),
            ] {
                pty.send(&format!("/export {typed}\r"))?;
                pty.wait_for(said)?;
            }
            pty.send("/export notes.md\r")?;
            pty.wait_for("exported to notes.md")?;
            export_figures(&mut pty)?;
            pty.send(QUIT)?;
            assert_exited(&pty.finish()?.0, "Ctrl+C exits 0");

            assert!(work.path().join("notes.md").exists());
            let above = work.path().parent().ok_or("the folder has a parent")?;
            for refused in [
                above.join("escape.md"),
                elsewhere.path().join("escape.md"),
                work.path().join(".git/escape.md"),
            ] {
                assert!(!refused.exists(), "{} was written", refused.display());
            }
            Ok(())
        })
    })
}

/// The issue's proof (larry#256, larry#603): a task runs, the terminal is killed with
/// kill -9, and a fresh `larry` lists the session under `/resume`, brings its transcript
/// and pane back, and forks it: the fork takes the later turns and the original row keeps
/// what it held. `/new` then starts an empty session beside both.
#[test]
fn tui_journey_a_killed_terminal_resumes_its_session_and_forks_it() -> Journey {
    let (state, work) = fixture()?;
    let replies = [COMMAND, r#"{"done":"wrote hello.txt"}"#];
    against(state.path(), scripted(603, &replies), |env| -> Journey {
        std::thread::scope(|owner| -> Journey {
            let mut pty = Pty::open(owner, work.path(), env)?;
            apply_a_task(&mut pty)?;
            kill_hard(pty)?;

            let mut fresh = Pty::open(owner, work.path(), env)?;
            fresh.wait_for(READY)?;
            // The listing leaves out this terminal's own empty row, so 1 is the killed one.
            fresh.send("/resume\r")?;
            fresh.wait_for("1. Session 1")?;
            fresh.send("/resume 1\r")?;
            fresh.wait_for("resumed Session 1.")?;
            fresh.wait_for("write hello.txt")?;
            fresh.wait_for("commands  1 run, 0 declined, 0 interrupted")?;
            fresh.wait_for("digest    ")?;
            fresh.send("/rename demo\r")?;
            fresh.wait_for("renamed to demo.")?;
            fresh.send("/fork\r")?;
            fresh.wait_for("forked demo as demo fork.")?;
            fresh.send("/help\r")?;
            fresh.wait_for("/quit leaves Larry")?;
            fresh.send("/new\r")?;
            fresh.wait_for("started Session 2.")?;
            fresh.send(QUIT)?;
            assert_exited(&fresh.finish()?.0, "Ctrl+C exits 0");

            let kept = rows(state.path())?;
            let held = |name: &str| {
                kept.iter()
                    .find(|row| row.0 == name)
                    .map(|row| row.2.as_str())
                    .ok_or_else(|| format!("no row {name}: {kept:?}"))
            };
            let (original, fork, started) = (held("demo")?, held("demo fork")?, held("Session 2")?);
            assert!(original.contains("write hello.txt") && fork.contains("write hello.txt"));
            assert!(
                fork.contains("/quit leaves Larry") && !original.contains("/quit leaves Larry"),
                "the fork diverged and the original did not: {original}"
            );
            assert!(
                !started.contains("write hello.txt"),
                "a new session starts empty: {started}"
            );
            Ok(())
        })
    })
}
