//! Journeys against what a command leaves behind (larry#334): a workspace whose
//! directory lost its permissions, a child that left the process group and kept a
//! stream open, and a journal whose lock cannot be probed.
//!
//! Source: POSIX process groups and `setsid(2)` (a session leader leaves its group),
//! POSIX file modes (a directory without write permission cannot lose an entry),
//! Linux/Darwin `stat(2)` returning `EACCES` for a file a mode hides, and the
//! estate journal's one liveness proof, its lock (`JournalError::Locked`).
//! Requirement proved: one command's leftover never stops a later run; the output
//! drain ends on the command's status and the cancel input, and a stream a child
//! kept open is a typed refusal; a probe error is never read as dead or absent.
//! Each wait ends on its event or on one deadline (larry#268); the pipe holder is
//! a fifo the test controls, so no wait here is a sleep.

use std::error::Error;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Output};
use std::time::Duration;

#[path = "support/deadline.rs"]
mod deadline;
#[path = "support/fixed_probe.rs"]
mod fixed_probe;
#[path = "support/larry_cli.rs"]
mod larry_cli;
#[path = "support/origin.rs"]
mod origin;

use deadline::{await_fifo_writer, await_until};
use fixed_probe::FIXED_PROBE;
use larry_cli::{spawn_larry_args, stderr_of};
use origin::{TempDir, fixture};

/// What every test here returns.
type TestResult = Result<(), Box<dyn Error>>;

/// A task document naming `repo`, `base` and the acceptance commands as raw JSON.
fn task_json(repo: &Path, base: &str, acceptance: &str) -> String {
    format!(
        r#"{{"repo":"{}","base":"{}","acceptance":{}}}"#,
        repo.display(),
        base,
        acceptance
    )
}

/// The acceptance of a task that only needs the next start to run: one command that passes,
/// because a task names 1 to 16 (larry#395).
const QUICK: &str = r#"[["true"]]"#;

/// Creates a named pipe at `path`.
fn make_fifo(path: &Path) -> Result<(), Box<dyn Error>> {
    let status = Command::new("mkfifo").arg(path).status()?;
    assert!(status.success(), "mkfifo {} must succeed", path.display());
    Ok(())
}

/// Starts `larry exec` with `task` on stdin and the state directory set. The
/// fixed probe rides every run (larry#274), so the verdict never pays a live
/// probe's dozen spawns.
fn spawn_exec(state: &Path, task: &str) -> Result<Child, Box<dyn Error>> {
    spawn_larry_args(
        &["exec"],
        &[
            ("LARRY_STATE_DIR", state.as_os_str()),
            ("LARRY_PROBE", std::ffi::OsStr::new(FIXED_PROBE)),
        ],
        task.as_bytes(),
    )
}

/// Runs `larry exec` to its end and returns its output.
fn run_exec(state: &Path, task: &str) -> Result<Output, Box<dyn Error>> {
    Ok(spawn_exec(state, task)?.wait_with_output()?)
}

/// Asserts the run printed a sealed report: exit 0 for accepted, 2 for rejected, never the
/// exit 1 of a refusal.
fn expect_report(output: &Output) {
    let code = output.status.code();
    assert!(
        matches!(code, Some(0 | 2)),
        "larry exec must print a sealed report (exit 0 or 2), got {code:?}; stderr: {}",
        stderr_of(output)
    );
}

/// A shell script that starts `cat` reading `$1` in a session of its own, so it outlives the
/// leader's process group and keeps the command's output stream open. The leader forks a child
/// that is not a group leader, so `POSIX::setsid` succeeds there, and the leader exits only
/// after that child has detached: the holder can never be killed with the leader's group, and
/// no wait here is a sleep. Perl ships with both macOS and the Ubuntu runners.
fn detached_cat(scratch: &Path) -> Result<std::path::PathBuf, Box<dyn Error>> {
    let script = scratch.join("detach.sh");
    let program = r#"my $c1 = fork();
die "fork: $!" unless defined $c1;
if ($c1 == 0) {
    POSIX::setsid() or die "setsid: $!";
    my $c2 = fork();
    die "fork: $!" unless defined $c2;
    exit 0 if $c2;
    exec q{sh}, q{-c}, q{exec cat < "$1"}, @ARGV;
}
waitpid($c1, 0);
exit 0;"#;
    fs::write(
        &script,
        format!("#!/bin/sh\nexec perl -MPOSIX -e '{program}' sh \"$1\"\n"),
    )?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700))?;
    Ok(script)
}

/// How long the test waits for `larry exec` to end before calling the run hung:
/// a hang guard, never the verdict. The verdict is the exact typed refusal once the
/// call returned; a loaded host only delays that return, so no working run may fail
/// here for being slow (larry#274, fifth pass: the gate's reach base failed this
/// test at 25.9 s and 33.8 s on a loaded Mac against a 10 s verdict bound).
const HANG_GUARD: Duration = Duration::from_secs(180);

/// The program the refusal names: the session launcher where the host has one, else
/// the acceptance command's own `sh`. The word is the platform's, the refusal is the
/// test's: `Spawn` with `an output stream stayed open after the command ended`.
fn launcher() -> &'static str {
    if Path::new("/usr/bin/setsid").is_file() {
        "/usr/bin/setsid"
    } else {
        "sh"
    }
}

/// A child that left the process group and kept the command's output stream open cannot hold
/// the run: the drain ends on the command's status and the cancel input, and a stream still
/// open is a typed refusal, never a join with no deadline.
#[test]
fn a_child_that_left_the_group_and_holds_the_pipe_ends_the_run() -> TestResult {
    let (state, origin) = fixture()?;
    let scratch = TempDir::new()?;
    let hold = scratch.path().join("hold");
    make_fifo(&hold)?;
    let script = detached_cat(scratch.path())?;
    let acceptance = format!(r#"[["sh","{}","{}"]]"#, script.display(), hold.display());
    let task = task_json(origin.path(), "HEAD", &acceptance);
    let mut child = spawn_exec(state.path(), &task)?;
    // The holder opens the fifo's read end only once it runs in its own session; the writer here
    // is the test's control of it, held until the run has ended.
    let _writer = await_fifo_writer(&hold)?;
    let ended = await_until(
        "larry exec ends with its command's output stream still open",
        HANG_GUARD,
        || Ok(child.try_wait()?),
    );
    let status = match ended {
        Ok(status) => status,
        Err(cause) => {
            child.kill()?;
            child.wait()?;
            return Err(cause);
        }
    };
    let output = child.wait_with_output()?;
    let stderr = stderr_of(&output);
    assert_eq!(
        status.code(),
        Some(1),
        "a stream still open past the command ends the run typed; stderr: {stderr:?}"
    );
    let refusal = format!(
        "exec could not run {}: an output stream stayed open after the command ended",
        launcher()
    );
    assert_eq!(
        stderr.trim(),
        refusal,
        "the run ends with the drain's exact typed refusal, naming the stream left open"
    );
    Ok(())
}

/// A command that leaves a directory without write permission cannot make its tree
/// undeletable: the workspace resets its directories before removal, so the next start runs
/// instead of refusing every run.
#[test]
fn a_command_that_leaves_a_directory_read_only_never_stops_the_next_start() -> TestResult {
    let (state, origin) = fixture()?;
    let acceptance = r#"[["sh","-c","mkdir -p sealed && touch sealed/file && chmod 500 sealed"]]"#;
    expect_report(&run_exec(
        state.path(),
        &task_json(origin.path(), "HEAD", acceptance),
    )?);
    expect_report(&run_exec(
        state.path(),
        &task_json(origin.path(), "HEAD", QUICK),
    )?);
    Ok(())
}

/// A workspace a killed executor left, whose directory lost its permissions, is swept by the
/// next start: the sweep resets each directory to owner rwx before removal.
#[test]
fn a_stranded_workspace_that_lost_its_permissions_is_swept() -> TestResult {
    let (state, origin) = fixture()?;
    let stranded = state.path().join("ws").join("stranded");
    let sealed = stranded.join("sealed");
    fs::create_dir_all(&sealed)?;
    fs::write(sealed.join("file"), b"x")?;
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o000))?;
    expect_report(&run_exec(
        state.path(),
        &task_json(origin.path(), "HEAD", QUICK),
    )?);
    assert!(
        !stranded.exists(),
        "the sweep removed the stranded workspace after resetting its directories"
    );
    Ok(())
}

/// A journal whose lock cannot be probed is never read as dead: the sweep keeps the run's
/// workspace, reports the typed refusal and the new run goes on.
#[test]
fn a_journal_lock_probe_that_errors_keeps_the_runs_workspace() -> TestResult {
    let (state, origin) = fixture()?;
    let id = "018f0000-0000-7000-8000-00000000d334";
    let workspace = state.path().join("ws").join(id);
    fs::create_dir_all(&workspace)?;
    let journals = state.path().join("journals");
    fs::create_dir_all(&journals)?;
    let journal = journals.join(format!("{id}.journal"));
    fs::write(&journal, b"not a journal")?;
    fs::set_permissions(&journal, fs::Permissions::from_mode(0o000))?;
    expect_report(&run_exec(
        state.path(),
        &task_json(origin.path(), "HEAD", QUICK),
    )?);
    assert!(
        workspace.is_dir(),
        "a workspace whose liveness probe errors is kept, never removed as dead"
    );
    Ok(())
}
