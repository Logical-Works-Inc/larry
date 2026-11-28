//! What `cargo local-ci` does when git, its log, its receipt or its own process fails (larry#574).
//! Source: CWE-252 (an unchecked return value), CWE-755 (improper handling of an exceptional
//! condition), POSIX.1-2024 `write` (`EPIPE` on a pipe with no reader) and `kill` (`SIGKILL` runs
//! no destructor), `getcwd(3)` (`ENOENT` for an unlinked directory), and the larry#440 incident:
//! separate checkouts writing one receipt directory. Requirement: every such fault is a typed
//! refusal with exit 1, a receipt is only ever whole, and no receipt vouches for a run that did
//! not get its answers.

use std::fs;
use std::io::{self, Read};
use std::process::{Command, Stdio};

use crate::Check;
use crate::origin::{TempDir, git_command};
use crate::rig::{
    ASK_BRANCH, ASK_HEAD, ASK_ROOT, Held, LANES, RECEIPT, Rig, attests, entries, exactly, lane_log,
    whole_run,
};

/// The refusal of a log whose reader is gone.
const BROKEN_LOG: &str = "local-ci: log unwritable: Broken pipe (os error 32)\n";

/// A refusal before any lane: exit 1, this line alone, and only git was asked, if at all.
fn refused_before_the_lanes(rig: &Rig, command: &mut Command, line: &str, asked: &[&str]) -> Check {
    let output = command.output()?;
    exactly(&output, 1, "", line);
    assert_eq!(rig.take_ran()?, asked, "no lane ran");
    assert!(!rig.default_receipts().exists(), "no receipt");
    Ok(())
}

/// Holds a run inside each lane in turn over an earlier head's receipt, lets `interrupt` act on the
/// held run, and asserts that receipt still stands, alone in its directory.
fn holding_each_lane(rig: &Rig, interrupt: &dyn Fn(&str, Held) -> Check) -> Check {
    for (name, line) in LANES {
        rig.write_earlier_receipt()?;
        rig.plan(&[(line, "wait")])?;
        interrupt(name, rig.hold_at(line)?)?;
        rig.earlier_receipt_stands(name)?;
    }
    Ok(())
}

/// A git that cannot name the checkout stops the run before any lane: outside a repository, with an
/// empty answer, with an answer and a failing status, or with a root that is not there.
#[test]
fn a_git_that_cannot_name_the_checkout_stops_before_any_lane() -> Check {
    let rig = Rig::new()?;
    let unnamed = "local-ci: git could not report the checkout root\n";
    let outside = rig.beside("outside");
    fs::create_dir(&outside)?;
    let mut command = rig.command(&outside, &[]);
    command.env("GIT_CEILING_DIRECTORIES", fs::canonicalize(rig.beside(""))?);
    refused_before_the_lanes(&rig, &mut command, unnamed, &[ASK_ROOT])?;
    for how in ["say:", "say:   ", "saydie:/"] {
        rig.plan(&[(ASK_ROOT, how)])?;
        let mut command = rig.command(rig.root(), &[]);
        refused_before_the_lanes(&rig, &mut command, unnamed, &[ASK_ROOT])?;
    }
    let gone = format!("say:{}", rig.beside("never made").display());
    rig.plan(&[(ASK_ROOT, &gone)])?;
    let missing = io::Error::from_raw_os_error(2);
    let line = format!("local-ci: lane root did not start: {missing}\n");
    let mut command = rig.command(rig.root(), &[]);
    refused_before_the_lanes(&rig, &mut command, &line, &[ASK_ROOT])
}

/// No git on `PATH` is the typed spawn refusal naming git.
#[test]
fn a_missing_git_is_refused_naming_git() -> Check {
    let rig = Rig::new()?;
    fs::remove_file(rig.bin("git"))?;
    let missing = io::Error::from_raw_os_error(2);
    let line = format!("local-ci: lane git did not start: {missing}\n");
    let mut command = rig.command(rig.root(), &[]);
    refused_before_the_lanes(&rig, &mut command, &line, &[])
}

/// A branch or head git cannot name after green lanes means no receipt: the run never attests a
/// commit it cannot name.
#[test]
fn an_unnamed_branch_or_head_writes_no_receipt() -> Check {
    let rig = Rig::new()?;
    for (asked, what) in [(ASK_BRANCH, "the branch"), (ASK_HEAD, "the head")] {
        for how in ["1", "say:", "saydie:main"] {
            rig.plan(&[(asked, how)])?;
            let output = rig.run()?;
            let line = format!("local-ci: git could not report {what}\n");
            exactly(&output, 1, &lane_log(&|_, _| None), &line);
            assert!(!rig.default_receipts().exists(), "{how}: no receipt");
        }
    }
    Ok(())
}

/// A repository with no commit passes the root question and every lane, and still gets no receipt:
/// real git prints `HEAD` and fails, and the failing status wins over the printed line.
#[test]
fn an_unborn_branch_writes_no_receipt() -> Check {
    let rig = Rig::new()?;
    let unborn = TempDir::new()?;
    let initialised = git_command(unborn.path()).args(["init", "-q"]).status()?;
    assert!(initialised.success(), "git init");
    rig.stub_larry(&unborn.join("target"))?;
    let output = rig.command(unborn.path(), &[]).output()?;
    exactly(
        &output,
        1,
        &lane_log(&|_, _| None),
        "local-ci: git could not report the branch\n",
    );
    assert!(!unborn.join("target/local-ci").exists(), "no receipt");
    Ok(())
}

/// A commit that lands while any lane runs (CWE-367: the checked tree is not the named one, as a
/// second session committing during a long suite does): the lanes did not test the new head, so the
/// run names both heads, exits 1 and leaves the earlier receipt as it was.
#[test]
fn sim_a_head_that_moves_while_a_lane_runs_writes_no_receipt() -> Check {
    let rig = Rig::new()?;
    holding_each_lane(&rig, &|name, held| {
        let started = rig.head()?;
        rig.git(&["commit", "--allow-empty", "-qm", name])?;
        let moved = rig.head()?;
        let output = held.release()?;
        let refusal = format!(
            "local-ci: head moved from {started} to {moved} while the lanes ran; no receipt written\n"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            refusal,
            "{name}: the refusal names both heads"
        );
        assert_eq!(output.status.code(), Some(1), "{name}: exit status");
        Ok(())
    })
}

/// A head git could not name before the lanes is not made good by naming one after them: a first
/// commit made while the suite runs on an unborn branch gets no receipt.
#[test]
fn a_head_unknown_before_the_lanes_writes_no_receipt() -> Check {
    let rig = Rig::new()?;
    rig.git(&["checkout", "-q", "--orphan", "unborn"])?;
    let build = "cargo build --bins --locked";
    rig.plan(&[(build, "wait")])?;
    let held = rig.hold_at(build)?;
    rig.git(&["commit", "-qm", "first"])?;
    let output = held.release()?;
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "local-ci: git could not report the head\n",
        "the refusal"
    );
    assert_eq!(output.status.code(), Some(1), "exit status");
    assert!(!rig.default_receipts().exists(), "no receipt");
    Ok(())
}

/// A receipt directory that is a file, and a receipt name that is a directory, are each refused
/// naming the path, after every lane ran, with no temporary left behind.
#[test]
fn an_unwritable_receipt_is_refused_naming_its_path() -> Check {
    let rig = Rig::new()?;
    let blocked = rig.beside("a file");
    fs::write(&blocked, "not a directory")?;
    let taken = rig.beside("taken");
    fs::create_dir_all(taken.join(RECEIPT))?;
    // EEXIST is 17 and EISDIR is 21 on Linux and Darwin alike.
    for (directory, errno) in [(&blocked, 17), (&taken, 21)] {
        let output = rig
            .command(rig.root(), &["--receipts"])
            .arg(directory)
            .output()?;
        let cause = io::Error::from_raw_os_error(errno);
        let line = format!(
            "local-ci: receipt {} unwritable: {cause}\n",
            directory.join(RECEIPT).display()
        );
        exactly(&output, 1, &lane_log(&|_, _| None), &line);
        assert_eq!(rig.take_ran()?, whole_run(), "every lane ran first");
    }
    assert_eq!(
        fs::read_to_string(&blocked)?,
        "not a directory",
        "untouched"
    );
    assert_eq!(entries(&taken)?, [RECEIPT], "no temporary is left");
    Ok(())
}

/// A relative receipt directory under a caller directory that was removed cannot be made absolute:
/// the refusal names it and git is never asked.
#[cfg(target_os = "linux")]
#[test]
fn a_removed_caller_directory_refuses_a_relative_receipt_directory() -> Check {
    let rig = Rig::new()?;
    let removed = rig.beside("removed");
    fs::create_dir(&removed)?;
    let binary = rig.command(&removed, &[]);
    let mut shell = Command::new("/bin/sh");
    shell
        .args(["-c", "/bin/rmdir \"$PWD\" && exec \"$0\" --receipts later"])
        .arg(binary.get_program())
        .current_dir(&removed)
        .env_clear();
    for (name, value) in binary.get_envs() {
        if let Some(value) = value {
            shell.env(name, value);
        }
    }
    crate::coverage::forward(&mut shell);
    let cause = io::Error::from_raw_os_error(2);
    let line = format!("local-ci: receipt later unwritable: {cause}\n");
    refused_before_the_lanes(&rig, &mut shell, &line, &[])
}

/// A log nobody reads stops the run at its first line: no lane runs unreported.
#[test]
fn a_closed_log_runs_no_lane() -> Check {
    let rig = Rig::new()?;
    for args in [&[][..], &["--list"][..]] {
        let (reader, writer) = io::pipe()?;
        drop(reader);
        let output = rig.command(rig.root(), args).stdout(writer).output()?;
        exactly(&output, 1, "", BROKEN_LOG);
        assert!(!rig.default_receipts().exists(), "{args:?}: no receipt");
    }
    assert_eq!(
        rig.take_ran()?,
        [ASK_ROOT, ASK_HEAD],
        "the run asked for its root and head, and started no lane"
    );
    Ok(())
}

/// The log closes while a lane runs, whether that lane then passes or fails: the run stops at the
/// next line it cannot write and starts no further lane.
#[test]
fn a_log_closed_mid_run_starts_no_further_lane() -> Check {
    let rig = Rig::new()?;
    let build = "cargo build --bins --locked";
    for how in ["wait", "waitfail"] {
        rig.plan(&[(build, how)])?;
        let output = rig.hold_at(build)?.release_with_the_log_closed()?;
        exactly(&output, 1, "", BROKEN_LOG);
        assert_eq!(
            rig.take_ran()?,
            [ASK_ROOT, ASK_HEAD, build],
            "{how}: nothing after"
        );
        assert!(!rig.default_receipts().exists(), "{how}: no receipt");
    }
    Ok(())
}

/// The log closes in the last lane: the green receipt is written, whole, and the run still exits 1
/// because it could not say so.
#[test]
fn a_log_closed_at_the_last_lane_still_fails_the_run() -> Check {
    let rig = Rig::new()?;
    let last = "git diff --check";
    rig.plan(&[(last, "wait")])?;
    let output = rig.hold_at(last)?.release_with_the_log_closed()?;
    exactly(&output, 1, "", BROKEN_LOG);
    assert_eq!(rig.take_ran()?, whole_run(), "every call in order");
    attests(&rig.default_receipts(), rig.root(), "main", &rig.head()?)
}

/// A refusal nobody can read is still exit 1, never a panic's 101.
#[test]
fn a_closed_standard_error_still_exits_one() -> Check {
    let rig = Rig::new()?;
    let (reader, writer) = io::pipe()?;
    drop(reader);
    let output = rig
        .command(rig.root(), &["--nope"])
        .stderr(writer)
        .output()?;
    exactly(&output, 1, "", "");
    assert_eq!(rig.take_ran()?, Vec::<String>::new(), "nothing ran");
    Ok(())
}

/// `SIGKILL` in any lane leaves the earlier receipt byte for byte and no temporary beside it, and
/// the next run writes a whole receipt.
#[test]
fn sim_a_kill_in_any_lane_keeps_the_earlier_receipt() -> Check {
    let rig = Rig::new()?;
    holding_each_lane(&rig, &|name, mut held| {
        let input = held.child.stdin.take().ok_or("the held run has no input")?;
        held.child.kill()?;
        let status = held.child.wait()?;
        assert_eq!(status.code(), None, "{name}: a signal ended the run");
        // The orphaned stub holds the log open until its input ends, so this read is its exit.
        drop(input);
        let mut rest = String::new();
        held.log.read_to_string(&mut rest)?;
        assert_eq!(rest, "", "{name}: nothing is logged after the kill");
        Ok(())
    })?;
    rig.plan(&[])?;
    assert_eq!(rig.run()?.status.code(), Some(0), "the next run is green");
    attests(&rig.default_receipts(), rig.root(), "main", &rig.head()?)
}

/// Eight checkouts writing one receipt directory at once (larry#440): every run is green and the
/// directory holds one whole receipt that names one checkout and that checkout's own head.
#[test]
fn eight_checkouts_sharing_a_receipt_directory_leave_one_whole_receipt() -> Check {
    let rigs = (0..8).map(|_| Rig::new()).collect::<Check<Vec<_>>>()?;
    let shared = TempDir::new()?;
    let mut running = Vec::new();
    for rig in &rigs {
        let child = rig
            .command(rig.root(), &[])
            .env("LARRY_STATE_DIR", shared.path())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        running.push(child);
    }
    for child in running {
        let output = child.wait_with_output()?;
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "",
            "no run is refused"
        );
        assert_eq!(output.status.code(), Some(0), "every run is green");
    }
    let text = fs::read_to_string(shared.join(RECEIPT))?;
    let named: Vec<&Rig> = rigs
        .iter()
        .filter(|rig| text.contains(&format!("\"{}\"", rig.root().display())))
        .collect();
    assert_eq!(named.len(), 1, "the receipt names one checkout: {text}");
    let winner = named.first().ok_or("the receipt names no checkout")?;
    attests(shared.path(), winner.root(), "main", &winner.head()?)
}
