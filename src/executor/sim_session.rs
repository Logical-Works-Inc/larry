//! Seeded simulations of a terminal session's reply parser and output clip
//! (larry#93), and of the person's cancel before, during and after a command
//! (larry#242). Each case is built from a seed together with the answer the rule
//! gives it, so the parser is judged against the rule, never against itself. The
//! cancel simulation drives `run_command` itself with its cancel closure and
//! flips the flag from a data witness (the ledger lists the group), never a timer.
//! Source: the Agent Client Protocol prompt turn (one model reply, one tool call)
//! and CWE-20 (improper input validation): every malformed reply gets its exact
//! refusal and nothing runs.

use std::collections::BTreeSet;
use std::io;
use std::mem::discriminant;
use std::num::TryFromIntError;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use lgwks_macros::Error;
use lgwks_std::process::process_group_exists;
use lgwks_std::seeded::{DrawError, Seeded};

use super::super::run::{CommandResult, GROUP_PROBE_DEADLINE, Place, clip, run_command};
use super::{
    ExecError, Ledger, MAX_INSTRUCTION_BYTES, OBSERVED_BYTES, Reply, ReplyFault, SessionError,
    check, parse_reply,
};
use crate::json_at::At;
use crate::sim_error::SimError;
use crate::sim_lanes::{self, LaneError};

/// What every simulation returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Seeds each simulation runs.
const SEEDS: u64 = 2048;

/// Characters a generated command draws from: ASCII, a quote, a backslash, a
/// newline, two-, three- and four-byte UTF-8.
const ALPHABET: [&str; 10] = ["a", "Z", "0", " ", "'", "\\", "\n", "é", "→", "🦝"];

/// A generated text of `len` characters; the compaction simulations draw theirs here too.
pub(in crate::executor) fn text(rng: &mut Seeded, len: usize) -> Result<String, SimError> {
    let mut out = String::new();
    for _ in 0..len {
        let pick = ALPHABET
            .get(rng.index(ALPHABET.len())?)
            .ok_or(SimError::Missing("alphabet"))?;
        out.push_str(pick);
    }
    Ok(out)
}

/// A JSON string literal for `value`.
fn quoted(value: &str) -> Result<String, SimError> {
    Ok(lgwks_std::json::to_string(value)?)
}

/// One generated reply and what the rule says of it.
fn case(seed: u64) -> Result<(String, Result<Reply, ReplyFault>), SimError> {
    let mut rng = Seeded::from_seed(seed);
    let kind = rng.below(11)?;
    let len = usize::try_from(rng.below(40)?.saturating_add(1))?;
    let body = format!("x{}", text(&mut rng, len)?);
    let pad = if rng.below(2)? == 0 { "" } else { "  \n" };
    Ok(match kind {
        0 => (
            format!("{pad}{{\"command\":{}}}{pad}", quoted(&body)?),
            Ok(Reply::Command(body)),
        ),
        1 => (
            format!("{{\"done\":{}}}", quoted(&body)?),
            Ok(Reply::Done(body)),
        ),
        2 => (
            format!("```json\n{{\"command\":{}}}\n```", quoted(&body)?),
            Ok(Reply::Command(body)),
        ),
        3 => (
            format!("{{\"command\":{0},\"done\":{0}}}", quoted(&body)?),
            Err(ReplyFault::Keys),
        ),
        4 => (
            format!("{{\"run\":{}}}", quoted(&body)?),
            Err(ReplyFault::Keys),
        ),
        5 => (
            format!("{{\"command\":{len}}}"),
            Err(ReplyFault::NotAString),
        ),
        6 => (
            format!("run {body}"),
            Err(ReplyFault::NotJson(String::new())),
        ),
        7 => (
            format!("[{}]", quoted(&body)?),
            Err(ReplyFault::NotAnObject),
        ),
        8 => (
            format!("{{\"command\":{}}}", quoted(&"y".repeat(4097))?),
            Err(ReplyFault::FieldTooLong),
        ),
        9 => (
            format!("{{\"command\":{}}}", quoted(&format!("{body}\0"))?),
            Err(ReplyFault::Nul),
        ),
        _ => (
            format!("{{\"command\":{}}}", quoted(&" ".repeat(len))?),
            Err(ReplyFault::EmptyCommand),
        ),
    })
}

/// Whether `got` is the rule's answer: the same reply, or the same refusal kind
/// (a parse refusal carries the parser's own reason, which the rule does not fix).
fn agrees(got: &Result<Reply, ReplyFault>, want: &Result<Reply, ReplyFault>) -> bool {
    match *want {
        Ok(ref reply) => got.as_ref().ok() == Some(reply),
        Err(ReplyFault::NotJson(_)) => {
            matches!(*got, Err(ReplyFault::NotJson(ref reason)) if !reason.is_empty())
        }
        Err(ref fault) => got
            .as_ref()
            .err()
            .is_some_and(|refused| discriminant(refused) == discriminant(fault)),
    }
}

/// Every generated reply gets exactly the rule's answer, for every seed.
#[test]
fn sim_reply_parser_matches_the_reference_for_every_seed() -> TestResult {
    for seed in 0..SEEDS {
        let (reply, expected) = case(seed)?;
        let got = parse_reply(&reply);
        assert!(
            agrees(&got, &expected),
            "seed {seed}: {reply:?} gave {got:?}, the rule says {expected:?}"
        );
    }
    Ok(())
}

/// The whole reply is bounded before it is parsed, and an empty object or an
/// unknown sole key is refused.
#[test]
fn an_oversized_or_keyless_reply_is_refused_before_anything_runs() {
    let long = format!("{{\"done\":\"{}\"}}", "z".repeat(16_400));
    assert_eq!(
        parse_reply(&long),
        Err(ReplyFault::TooLong),
        "the reply bound"
    );
    assert_eq!(parse_reply("{}"), Err(ReplyFault::Keys), "no key");
    assert_eq!(
        parse_reply("{\"done\":\"\"}"),
        Ok(Reply::Done(String::new())),
        "empty summary"
    );
}

/// The fat-finger hands a session a blank, giant, NUL-carrying or ordinary
/// instruction and a directory, a file or nothing; the refusal is the first rule
/// broken in the documented order (blank, size, NUL, directory), checked before
/// anything runs.
#[test]
fn sim_session_fat_finger_instruction_and_directory_get_the_rule_refusal() -> TestResult {
    let dir = std::env::temp_dir();
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed.wrapping_add(2 << 32));
        let len = match rng.below(4)? {
            0 => 0,
            1 => usize::try_from(rng.below(64)?)?,
            2 => MAX_INSTRUCTION_BYTES,
            _ => MAX_INSTRUCTION_BYTES.saturating_add(usize::try_from(rng.below(3)?)?),
        };
        let mut instruction: String = if rng.below(3)? == 0 {
            [' ', '\n', '\t'].iter().cycle().take(len).collect()
        } else {
            "x".repeat(len)
        };
        if rng.below(4)? == 0 && !instruction.is_empty() {
            instruction.replace_range(..1, "\0");
        }
        let missing = dir.join(format!("larry-no-such-dir-{seed}"));
        let cwd = match rng.below(3)? {
            0 => dir.as_path(),
            1 => file.as_path(),
            _ => missing.as_path(),
        };
        let expected = if instruction.trim().is_empty() {
            Err(SessionError::EmptyInstruction)
        } else if instruction.len() > MAX_INSTRUCTION_BYTES {
            Err(SessionError::InstructionTooLarge {
                len: instruction.len(),
            })
        } else if instruction.contains('\0') {
            Err(SessionError::NulInInstruction)
        } else if cwd == dir.as_path() {
            Ok(())
        } else {
            Err(SessionError::NotADirectory)
        };
        assert_eq!(check(&instruction, cwd), expected, "seed {seed}: len {len}");
    }
    Ok(())
}

/// A stream past the observed bound is cut on a character boundary at or under
/// the bound and marked cut; a stream at the bound is kept whole.
#[test]
fn sim_observations_keep_their_bound_on_char_boundaries_for_every_seed() -> TestResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed.wrapping_add(1 << 32));
        let target = OBSERVED_BYTES
            .saturating_sub(8)
            .saturating_add(usize::try_from(rng.below(24)?)?);
        let mut stream = String::new();
        while stream.len() < target {
            stream.push_str(text(&mut rng, 1)?.as_str());
        }
        let original = stream.clone();
        let cut = clip(&mut stream, OBSERVED_BYTES);
        assert_eq!(
            cut,
            original.len() > OBSERVED_BYTES,
            "seed {seed}: cut flag"
        );
        assert!(stream.len() <= OBSERVED_BYTES, "seed {seed}: bound");
        assert!(original.starts_with(&stream), "seed {seed}: a prefix");
        if cut {
            assert!(
                OBSERVED_BYTES.saturating_sub(stream.len()) < 4,
                "seed {seed}: at most one character short of the bound"
            );
        }
    }
    Ok(())
}

/// Seeds the cancel simulation sweeps.
const CANCEL_SEEDS: u64 = 32;

/// The timeout every cancel run is given: far past every cancel, so it only ends a
/// run that failed to be cancelled, and the failure then reports itself.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// When the person's cancel lands against the command (larry#242).
#[derive(Debug, Clone, Copy)]
enum When {
    /// The closure is true from its first call: the command is killed at once.
    Before,
    /// The flag flips once the ledger lists the command's group.
    During,
    /// The command exits on its own; the flag flips only after `run_command` returns.
    After,
}

impl When {
    /// Every schedule, in the order a seed runs them.
    const ALL: [When; 3] = [When::Before, When::During, When::After];

    /// The schedule's name in a trace.
    const fn name(self) -> &'static str {
        match self {
            When::Before => "before",
            When::During => "during",
            When::After => "after",
        }
    }

    /// The shell script of the schedule: a long sleep to interrupt (with a
    /// background member for the group to take down) or a command that exits 0.
    const fn script(self) -> &'static str {
        match self {
            When::Before => "sleep 30",
            When::During => "sleep 30 & sleep 30",
            When::After => "true",
        }
    }
}

/// A fresh scratch directory under the system temp dir, one per run: empty when it is made and
/// removed when its guard drops, so a process id the system reuses never lands a run in a stale
/// tree (larry#267).
fn scratch(tag: &str) -> Result<crate::executor::sim_tree::Scratch, SimError> {
    crate::executor::sim_tree::scratch(&format!("larry-esc-{tag}"))
}

/// Sets the witness's `finished` flag when it drops, so the watcher ends even when
/// `run_command` unwinds: without it a panic leaves the watcher spinning and the scope can
/// never join (larry#294).
struct FinishedOnDrop<'a> {
    /// The flag the ledger witness reads.
    finished: &'a AtomicBool,
}

impl Drop for FinishedOnDrop<'_> {
    fn drop(&mut self) {
        self.finished.store(true, Ordering::Release);
    }
}

/// Whether the ledger under `state` lists a group before the run finished: the wait is the
/// ledger's own row, a data witness, and it ends when the row appears, when the run drops its
/// [`FinishedOnDrop`] guard, or at [`GROUP_PROBE_DEADLINE`] past the first reading of the
/// runtime's live clock, so a witness that never arrives cannot outlive the run (larry#268,
/// larry#294).
fn group_is_listed(state: &Path, finished: &AtomicBool) -> Result<bool, ExecError> {
    let clock = larry_runtime::live_clock().map_err(ExecError::Gate)?;
    let started = clock.now();
    let ledger = Ledger::open(state)?;
    loop {
        if !ledger.live_tasks()?.is_empty() {
            return Ok(true);
        }
        if finished.load(Ordering::Acquire) {
            return Ok(false);
        }
        if clock.now().saturating_sub(started) >= GROUP_PROBE_DEADLINE {
            return Ok(false);
        }
        std::thread::yield_now();
    }
}

/// Flips `cancel` once the ledger lists the command's group (a data witness) and
/// says whether it did. The flag flips on every path out, so a witness that fails
/// ends the command at once and the run reports the failure.
fn flip_when_listed(
    state: &Path,
    cancel: &AtomicBool,
    finished: &AtomicBool,
) -> Result<bool, ExecError> {
    let listed = group_is_listed(state, finished);
    cancel.store(true, Ordering::Release);
    listed
}

/// Why one cancel run could not be driven to its assertions.
#[derive(Debug, Error)]
enum CancelSimError {
    /// The marker text could not be built.
    #[error("marker text: {0}")]
    Text(#[from] SimError),
    /// The seeded stream refused a draw.
    #[error("seeded draw refused: {0}")]
    Draw(#[from] DrawError),
    /// A drawn number does not fit its type.
    #[error("drawn number out of range: {0}")]
    Range(#[from] TryFromIntError),
    /// The executor refused the run or the ledger.
    #[error("executor: {0}")]
    Exec(#[from] ExecError),
    /// A scratch directory or a group probe failed.
    #[error("scratch or probe: {0}")]
    Io(#[from] io::Error),
    /// The ledger witness thread panicked.
    #[error("the ledger witness thread panicked")]
    WitnessPanicked,
    /// The command's record lacks a field the run needs.
    #[error("missing: {0}")]
    Missing(&'static str),
    /// A tree still present after its guard dropped, named so the leak can be found.
    #[error("the tree {} outlived its guard", __field_0.display())]
    Leaked(PathBuf),
    /// A lane of the seed sweep failed outright.
    #[error("seed sweep: {0}")]
    Lane(#[from] LaneError),
}

/// One schedule of one seed against `run_command`: asserts what the result and the
/// ledger must say and returns the facts that do not vary by run, the trace the
/// sweep hashes. The seed draws the marker text in the command.
fn drive(
    when: When,
    seed: u64,
    path_env: &str,
    state: &Path,
    work: &Path,
) -> Result<String, CancelSimError> {
    let at = when.name();
    std::fs::create_dir_all(work)?;
    let mut rng = Seeded::from_seed(seed.wrapping_add(3 << 32));
    let marker_len = usize::try_from(rng.below(48)?)?.saturating_add(1);
    let marker = text(&mut rng, marker_len)?.replace('\n', " ");
    let argv = [
        String::from("sh"),
        String::from("-c"),
        format!("{} # seed {seed} {marker}", when.script()),
    ];
    let ledger = Ledger::open(state)?;
    let cancel = AtomicBool::new(matches!(when, When::Before));
    // Set once `run_command` returns, so the data witness stops with the run: its wait is
    // bounded by the command's own deadline and reads no clock of its own (larry#268).
    let finished = AtomicBool::new(false);
    let task_id = format!("sim-esc-{seed}-{at}");
    let (result, witnessed) = std::thread::scope(
        |scope| -> Result<(CommandResult, Option<bool>), CancelSimError> {
            let watcher = matches!(when, When::During)
                .then(|| scope.spawn(|| flip_when_listed(state, &cancel, &finished)));
            // The guard sets `finished` on unwind too, so a panic inside `run_command` ends the
            // watcher instead of leaving the scope waiting on it for good (larry#294).
            let result = {
                let _finished = FinishedOnDrop {
                    finished: &finished,
                };
                run_command(
                    &argv,
                    Place::Open(work),
                    path_env,
                    COMMAND_TIMEOUT,
                    &ledger,
                    &task_id,
                    &|| cancel.load(Ordering::Acquire),
                )
            };
            finished.store(true, Ordering::Release);
            cancel.store(true, Ordering::Release);
            let witnessed = match watcher {
                Some(watcher) => {
                    let Ok(listed) = watcher.join() else {
                        return Err(CancelSimError::WitnessPanicked);
                    };
                    Some(listed?)
                }
                None => None,
            };
            Ok((result?, witnessed))
        },
    )?;
    let (exit_code, timed_out, interrupted) =
        (result.exit_code(), result.timed_out(), result.interrupted());
    let record = result.into_json();
    let pgid = match record.at("/pgid").map(|pgid| pgid.as_i64()) {
        Ok(Some(pgid)) => i32::try_from(pgid)?,
        Ok(None) | Err(crate::json_at::Missing { .. }) => {
            return Err(CancelSimError::Missing("the record's pgid"));
        }
    };
    let gone = !process_group_exists(pgid)?;
    let unlisted = ledger.live_tasks()?.is_empty();
    assert!(!timed_out, "seed {seed} {at}: a cancel is never a timeout");
    assert!(gone, "seed {seed} {at}: process group {pgid} is gone");
    assert!(unlisted, "seed {seed} {at}: the ledger lists no group");
    match when {
        When::Before | When::During => {
            assert!(interrupted, "seed {seed} {at}: the command was interrupted");
            assert_eq!(exit_code, None, "seed {seed} {at}: a kill is no exit code");
        }
        When::After => {
            assert!(!interrupted, "seed {seed} {at}: a finished command is not");
            assert_eq!(exit_code, Some(0), "seed {seed} {at}: a normal exit");
        }
    }
    if matches!(when, When::During) {
        assert_eq!(
            witnessed,
            Some(true),
            "seed {seed} {at}: the ledger showed the group before the cancel"
        );
    }
    Ok(format!(
        "{at} argv {argv:?} exit {exit_code:?} timed {timed_out} interrupted {interrupted} gone {gone} unlisted {unlisted}\n"
    ))
}

/// One schedule of one seed in a fresh state directory and workspace, both removed
/// afterwards whatever the outcome.
fn run_one(when: When, seed: u64, path_env: &str) -> Result<String, CancelSimError> {
    let state = scratch("state")?;
    let work = scratch("work")?;
    let outcome = drive(when, seed, path_env, &state, &work);
    let (state_path, work_path) = (state.to_path_buf(), work.to_path_buf());
    drop(state);
    drop(work);
    let trace = outcome?;
    leaked_tree([state_path, work_path])?;
    Ok(trace)
}

/// Refuses with the first of `paths` still there, naming it: a tree the run's guards left behind
/// says which tree leaked (larry#296).
fn leaked_tree(paths: [PathBuf; 2]) -> Result<(), CancelSimError> {
    match paths.into_iter().find(|path| path.exists()) {
        Some(path) => Err(CancelSimError::Leaked(path)),
        None => Ok(()),
    }
}

/// Every seed's trace hash, in seed order.
fn sweep(path_env: &str) -> Result<Vec<String>, CancelSimError> {
    // Each seed owns its state directory and workspace, so the seeds run on bounded lanes and
    // come back in seed order. A seed mostly waits on its command, so lanes pay here (ADR 0049:
    // 5.6 s to 1.8 s on the 4-core CI runner).
    let hashes = sim_lanes::sweep(
        CANCEL_SEEDS,
        |_lane, seed| -> Result<String, CancelSimError> {
            let mut trace = String::new();
            for when in When::ALL {
                trace.push_str(&run_one(when, seed, path_env)?);
            }
            let mut hasher = lgwks_std::hash::Hasher::new();
            hasher.update(trace.as_bytes());
            Ok(hasher.finalize().to_hex())
        },
    )?;
    hashes.into_iter().collect()
}

/// The timeout the timeout run is given: short, so the deadline and not a cancel ends it.
const SHORT_TIMEOUT: Duration = Duration::from_millis(200);

/// Seeds the timeout run draws its marker text for.
const TIMEOUT_SEEDS: u64 = 4;

/// A command left alone past its timeout ends as a timeout and never as an interrupt:
/// the cancel closure is false throughout, the group is killed and gone, the ledger
/// is empty and no exit code is reported (larry#242).
#[test]
fn sim_run_timeout_is_not_an_interrupt() -> TestResult {
    let path_env = std::env::var("PATH")?;
    for seed in 0..TIMEOUT_SEEDS {
        let state = scratch("timeout-state")?;
        let work = scratch("timeout-work")?;
        let mut rng = Seeded::from_seed(seed.wrapping_add(4 << 32));
        let marker_len = usize::try_from(rng.below(48)?)?.saturating_add(1);
        let marker = text(&mut rng, marker_len)?.replace('\n', " ");
        let argv = [
            String::from("sh"),
            String::from("-c"),
            format!("sleep 30 & sleep 30 # seed {seed} {marker}"),
        ];
        let ledger = Ledger::open(&state)?;
        let outcome = run_command(
            &argv,
            Place::Open(&work),
            &path_env,
            SHORT_TIMEOUT,
            &ledger,
            "sim-timeout",
            &|| false,
        );
        let listed = ledger.live_tasks();
        let (state_path, work_path) = (state.to_path_buf(), work.to_path_buf());
        drop(ledger);
        drop(state);
        drop(work);
        let trees_removed = !state_path.exists() && !work_path.exists();
        let result = outcome?;
        assert!(result.timed_out(), "seed {seed}: the deadline ended it");
        assert!(
            !result.interrupted(),
            "seed {seed}: a timeout is not an interrupt"
        );
        assert_eq!(result.exit_code(), None, "seed {seed}: a kill is no exit");
        let record = result.into_json();
        let pgid = i32::try_from(record.at("/pgid")?.as_i64().ok_or("no pgid")?)?;
        assert!(
            !process_group_exists(pgid)?,
            "seed {seed}: process group {pgid} is gone"
        );
        assert!(listed?.is_empty(), "seed {seed}: the ledger lists no group");
        assert!(trees_removed, "seed {seed}: the fixture removes its trees");
    }
    Ok(())
}

/// Esc before, during and after a command, driven through `run_command` and its
/// cancel closure with no model and no session: before kills the command at once,
/// during kills a group with a background member once the ledger lists it, after
/// leaves a normal exit untouched; the group is gone and unlisted each time, and
/// the same 32 seeds swept twice give the same trace hashes (larry#242).
#[test]
fn sim_esc_before_during_after_command() -> TestResult {
    let path_env = std::env::var("PATH")?;
    let first = sweep(&path_env)?;
    let second = sweep(&path_env)?;
    assert_eq!(first, second, "the same seeds give the same trace hashes");
    let distinct: BTreeSet<&String> = first.iter().collect();
    assert_eq!(
        distinct.len(),
        first.len(),
        "each seed's marker gives its own trace"
    );
    Ok(())
}

/// The seed the panic simulation draws its marker and poll from.
const PANIC_SEED: u64 = 0;

/// Drives one run whose cancel closure fails inside `run_command` while the ledger witness is
/// still waiting for its row: the watcher ends when the run unwinds, so the scope joins and the
/// panic reaches the caller instead of hanging it (larry#294). The witness reads its own empty
/// ledger, which is the window where the run unwinds before any row is listed.
fn panic_inside_run(seed: u64) -> Result<(), CancelSimError> {
    let run_state = scratch("panic-run")?;
    let watch_state = scratch("panic-watch")?;
    let work = scratch("panic-work")?;
    let path_env = std::env::var("PATH").map_err(io::Error::other)?;
    let mut rng = Seeded::from_seed(seed.wrapping_add(5 << 32));
    let marker_len = usize::try_from(rng.below(48)?)?.saturating_add(1);
    let marker = text(&mut rng, marker_len)?.replace('\n', " ");
    let fails_at = rng.below(3)?.saturating_add(1);
    let argv = [
        String::from("sh"),
        String::from("-c"),
        format!("sleep 1 # seed {seed} {marker}"),
    ];
    let ledger = Ledger::open(&run_state)?;
    let cancel = AtomicBool::new(false);
    let finished = AtomicBool::new(false);
    let polls = AtomicU64::new(0);
    std::thread::scope(|scope| -> Result<(), CancelSimError> {
        let watcher = scope.spawn(|| flip_when_listed(&watch_state, &cancel, &finished));
        let result = {
            let _finished = FinishedOnDrop {
                finished: &finished,
            };
            run_command(
                &argv,
                Place::Open(&work),
                &path_env,
                COMMAND_TIMEOUT,
                &ledger,
                "sim-panic",
                &|| {
                    let poll = polls.fetch_add(1, Ordering::AcqRel).saturating_add(1);
                    assert!(
                        poll > fails_at,
                        "seed {seed}: the injected fault inside run_command"
                    );
                    false
                },
            )
        };
        // Only a run that never reached its cancel poll gets here: end the watcher, then say
        // the run did not panic, so the test fails instead of passing on a fault it missed.
        finished.store(true, Ordering::Release);
        cancel.store(true, Ordering::Release);
        let Ok(witnessed) = watcher.join() else {
            return Err(CancelSimError::WitnessPanicked);
        };
        witnessed?;
        result?;
        Err(CancelSimError::Missing("the panic inside run_command"))
    })
}

/// A seeded run panics inside `run_command` while its witness is still waiting for a row: the
/// scope joins and the test reports the panic, never a hang (larry#294).
#[test]
#[should_panic(expected = "the injected fault inside run_command")]
fn sim_a_panic_inside_run_command_ends_the_witness_and_reports_the_panic() {
    let driven = panic_inside_run(PANIC_SEED);
    assert!(
        driven.is_err(),
        "the injected fault inside run_command never fired: the run returned instead of unwinding"
    );
    if let Err(cause) = driven {
        assert!(
            cause.to_string().is_empty(),
            "the run must reach the injected fault inside run_command, not fail at setup: {cause}"
        );
    }
}

/// A tree still present after its guard dropped is refused with its own path in the message, so
/// a leaked run says which tree to look at (larry#296). The tree is the crate's scratch guard,
/// which removes it when the test ends.
#[test]
fn a_tree_still_present_after_its_guard_names_its_path_in_the_refusal() -> TestResult {
    let tree = scratch("leak")?;
    let absent = tree.join("absent");
    let refusal = match leaked_tree([tree.to_path_buf(), absent]) {
        Err(cause) => cause,
        Ok(()) => return Err("the check passed over a tree that is still there".into()),
    };
    let named = tree.to_string_lossy();
    assert!(
        refusal.to_string().contains(named.as_ref()),
        "the refusal names the leaked tree {named}: {refusal}"
    );
    Ok(())
}
