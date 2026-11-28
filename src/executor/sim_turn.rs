//! Seeded-shape simulations of a terminal session's turn loop (larry#586,
//! larry#566): a declined step lets the turn end, an approved step is recorded as
//! the model saw it, a hostile reply is refused with the exact reason the model
//! retries from, a gateway fault at any step stops the turn with what ran, and a
//! cancel at any point runs nothing more. Each test drives the real [`Turn`]
//! against a scripted loopback gateway, with a scripted observer answering for
//! the person, so the loop's stops and records are what the shipped binary
//! seals. Source: the Agent Client Protocol prompt turn (one reply, one step,
//! every step only after a yes), CWE-20 (hostile input) and CWE-770 (every
//! bound holds exactly).

use lgwks_std::json::Value;

use super::super::run::Place;
use super::super::session::{Approval, Stop};
use super::super::sim_gateway::{Cancel, Reply, Scripted, StubGateway, TestResult};
use super::super::sim_tree::scratch;
use super::super::{ExecError, Ledger, Settings};
use super::{Steps, Turn};
use crate::model::{ModelClient, ModelConfig};

/// What one driven turn left behind.
struct Driven {
    /// What the turn returned.
    steps: Result<Steps, ExecError>,
    /// What the person saw and answered.
    observer: Scripted,
    /// Every request the gateway answered, in order.
    requests: Vec<Value>,
}

/// Runs one turn of `replies` against the stub gateway, the person answering as
/// `observer`, in a fresh state directory over a work directory `setup` fills.
/// `extra` variables override the table the settings and the model read.
fn drive_with(
    label: &str,
    replies: Vec<Reply>,
    mut observer: Scripted,
    extra: &[(&str, &str)],
    setup: impl FnOnce(&std::path::Path) -> TestResult<()>,
) -> TestResult<Driven> {
    let root = scratch(label)?;
    let state = root.join("state");
    let work = root.join("work");
    std::fs::create_dir_all(&work)?;
    setup(&work)?;
    let gateway = StubGateway::bind()?;
    let mut environment: Vec<(String, String)> = extra
        .iter()
        .map(|&(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    let state_text = state.to_str().ok_or("the state directory is not text")?;
    for (name, value) in [
        ("LARRY_STATE_DIR", state_text),
        ("PATH", "/usr/bin:/bin"),
        ("LARRY_MODEL_KEY", "sim-turn"),
        ("LARRY_MODEL_URL", gateway.url()),
        ("LARRY_MODEL", "sim-turn"),
        ("LARRY_MODEL_ATTEMPTS", "1"),
    ] {
        environment.push((name.to_owned(), value.to_owned()));
    }
    let read = move |name: &str| {
        environment
            .iter()
            .find(|entry| entry.0.as_str() == name)
            .map(|entry| entry.1.clone())
    };
    let settings = Settings::from_env(read.clone())?;
    let client = ModelClient::new(
        ModelConfig::from_env(read)?,
        larry_runtime::live_clock().map_err(|cause| format!("clock: {cause}"))?,
        |_| false,
    );
    let ledger = Ledger::open(&state)?;
    let mut turn = Turn {
        client: &client,
        settings: &settings,
        ledger: &ledger,
        task_id: "sim-turn",
        place: Place::Open(&work),
        shown: String::from("."),
        system: String::from(super::SYSTEM),
        observer: &mut observer,
    };
    let (steps, requests) = gateway.serve_while(replies, || Ok(turn.run("a simulated task")))?;
    Ok(Driven {
        steps,
        observer,
        requests,
    })
}

/// [`drive_with`] for a run that must not fail, with the default environment.
fn drive(
    label: &str,
    replies: Vec<Reply>,
    approvals: Vec<Approval>,
    setup: impl FnOnce(&std::path::Path) -> TestResult<()>,
) -> TestResult<(Steps, Scripted, usize)> {
    let driven = drive_with(label, replies, Scripted::new(approvals), &[], setup)?;
    Ok((driven.steps?, driven.observer, driven.requests.len()))
}

/// The text of message `index` of the request `number`, both from 0.
fn message(requests: &[Value], number: usize, index: usize) -> TestResult<&str> {
    Ok(requests
        .get(number)
        .and_then(|request| request.get("messages"))
        .and_then(Value::as_array)
        .and_then(|messages| messages.get(index))
        .and_then(|entry| entry.get("content"))
        .and_then(Value::as_str)
        .ok_or("the request holds no such message")?)
}

/// The refusal a turn stopped with.
fn refusal(stop: &Stop) -> TestResult<&str> {
    match *stop {
        Stop::Refusal(ref line) => Ok(line),
        ref other => Err(format!("the turn did not refuse: {other:?}").into()),
    }
}

/// A declined command lets the turn end instead of cancelling it: the model
/// is asked again, says done, and the stop is `EndTurn` with one decline and
/// no interruption.
#[test]
fn sim_a_declined_command_lets_the_turn_end() -> TestResult<()> {
    let replies = vec![
        Reply::from("{\"command\":\"echo hi\"}"),
        Reply::from("{\"done\":\"left it alone\"}"),
    ];
    let (steps, _, served) = drive("sim-turn-decline", replies, vec![Approval::Decline], |_| {
        Ok(())
    })?;
    assert_eq!(served, 2, "the model is asked again after the decline");
    assert!(
        matches!(steps.stop, Stop::EndTurn),
        "a decline ends the turn, never cancels it"
    );
    assert_eq!(steps.tally.declined, 1, "one decline is counted");
    assert_eq!(
        steps.tally.interrupted, 0,
        "a decline is never an interruption"
    );
    assert!(
        steps.tally.results.is_empty(),
        "a declined command never runs"
    );
    Ok(())
}

/// An approved command that exits zero passes its observation: the report
/// keeps the model's view of the run, and the pass flag says it succeeded.
#[test]
fn sim_an_approved_command_passes_its_observation_on_exit_zero() -> TestResult<()> {
    let replies = vec![
        Reply::from("{\"command\":\"echo hi\"}"),
        Reply::from("{\"done\":\"said hi\"}"),
    ];
    let (steps, observer, served) =
        drive("sim-turn-command", replies, vec![Approval::Run], |_| Ok(()))?;
    assert_eq!(served, 2, "the command, then the summary");
    assert!(
        matches!(steps.stop, Stop::EndTurn),
        "the turn ends at the model's done"
    );
    assert_eq!(steps.tally.results.len(), 1, "one command is recorded");
    assert_eq!(
        observer.passed,
        vec![true],
        "an exit-zero command passes its observation"
    );
    Ok(())
}

/// An approved edit that names the digest on disk lands and is recorded with
/// both digests; a stale one is refused with its record and writes nothing; a
/// declined one never runs. None of them is a command.
#[test]
fn sim_an_approved_edit_records_both_digests() -> TestResult<()> {
    let before = "alpha\n";
    let after = "beta\n";
    let digest = lgwks_std::hash::blake3(before.as_bytes()).to_hex();
    let next = lgwks_std::hash::blake3(after.as_bytes()).to_hex();
    let edit = format!(
        "{{\"edit\":{{\"path\":\"a.txt\",\"old\":\"alpha\",\"new\":\"beta\",\"expect_digest\":\"{digest}\"}}}}"
    );
    let replies = vec![
        Reply::from(edit.clone()),
        Reply::from(edit.clone()),
        Reply::from(edit),
        Reply::from("{\"done\":\"edited\"}"),
    ];
    let approvals = vec![Approval::Run, Approval::Run, Approval::Decline];
    let (steps, observer, served) = drive("sim-turn-edit", replies, approvals, |work| {
        std::fs::write(work.join("a.txt"), before)?;
        Ok(())
    })?;
    assert_eq!(
        served, 4,
        "an edit, a stale one, a decline, then the summary"
    );
    assert_eq!(observer.passed, vec![true, false], "each edit ends once");
    assert_eq!(
        observer.observations.first(),
        Some(&format!("edited\ndigest {digest} -> {next}")),
        "the model is shown the digest its next edit names"
    );
    assert_eq!((steps.tally.edits, steps.tally.reads), (2, 0));
    assert_eq!(steps.tally.declined, 1, "the third edit was declined");
    assert!(steps.tally.results.is_empty(), "an edit is not a command");
    assert_eq!(
        steps.tally.changes,
        vec![
            lgwks_std::json::json!({
                "step": 0, "kind": "edit", "path": "a.txt",
                "before_digest": digest, "after_digest": next,
            }),
            lgwks_std::json::json!({
                "step": 1, "kind": "edit", "path": "a.txt",
                "refusal": format!("edit file a.txt changed since it was read: its digest is {next}"),
            }),
        ]
    );
    Ok(())
}

/// An approved read records the file's digest, which the model is shown; a read
/// of a path outside the tree is a typed refusal, and a declined read runs not
/// at all. A long file is cut for the model but digested whole.
#[test]
fn sim_an_approved_read_shows_the_digest() -> TestResult<()> {
    let before = "pub fn origin() {}\n";
    let replies = vec![
        Reply::from("{\"read\":\"lib.rs\"}"),
        Reply::from("{\"read\":\"../outside\"}"),
        Reply::from("{\"read\":\"lib.rs\"}"),
        Reply::from("{\"done\":\"read it\"}"),
    ];
    let approvals = vec![Approval::Run, Approval::Run, Approval::Decline];
    let (steps, observer, served) = drive("sim-turn-read", replies, approvals, |work| {
        std::fs::write(work.join("lib.rs"), before)?;
        Ok(())
    })?;
    assert_eq!(served, 4, "two reads, a decline, then the summary");
    assert_eq!(steps.tally.changes.len(), 2, "two reads ran");
    assert_eq!(steps.tally.declined, 1, "the third read was declined");
    assert_eq!(observer.passed, vec![true, false], "each read ends once");
    let digest = lgwks_std::hash::blake3(before.as_bytes()).to_hex();
    assert_eq!(
        observer.observations.first(),
        Some(&format!("read\ndigest {digest} (19 bytes)\n{before}")),
        "the model is shown the digest, the length and the file"
    );
    assert_eq!(
        steps
            .tally
            .changes
            .get(1)
            .and_then(|record| record.get("step")),
        Some(&lgwks_std::json::json!(1)),
        "each read takes the next step number"
    );
    let first = steps
        .tally
        .changes
        .first()
        .ok_or("the read earns a record")?;
    assert_eq!(
        first.get("digest").and_then(lgwks_std::json::Value::as_str),
        Some(lgwks_std::hash::blake3(before.as_bytes()).to_hex().as_str()),
        "the record carries the file's digest"
    );
    let second = steps
        .tally
        .changes
        .get(1)
        .and_then(|record| record.get("refusal"))
        .and_then(lgwks_std::json::Value::as_str)
        .ok_or("the second read is refused")?;
    assert_eq!(second, "edit path ../outside escapes the snapshot");
    Ok(())
}

/// Every hostile reply is refused with the exact reason the model is asked to
/// retry from, and nothing runs (CWE-20): the next request carries the reply as
/// the conversation keeps it and the parser's sentence. A reply of exactly the
/// bound is kept whole; one byte past it only its length is kept, so it cannot
/// grow the next request (CWE-770).
#[test]
fn sim_a_refused_reply_tells_the_model_its_exact_fault() -> TestResult<()> {
    let at_bound = format!("{:<8192}", "[1]");
    let past_bound = "x".repeat(8193);
    let long_command = format!("{{\"command\":\"{}\"}}", "y".repeat(4097));
    let cases: [(&str, &str); 10] = [
        (&at_bound, "the reply is not a JSON object"),
        (&past_bound, "the reply is over 8192 bytes"),
        ("[\"ls\"]", "the reply is not a JSON object"),
        (
            "{\"command\":\"a\",\"done\":\"b\"}",
            "the object must hold exactly one key, command, done, read or edit",
        ),
        (
            "{\"run\":\"ls\"}",
            "the object must hold exactly one key, command, done, read or edit",
        ),
        ("{\"command\":7}", "the value must be a string"),
        ("{\"command\":\"  \"}", "the command is empty"),
        (
            &long_command,
            "the command, summary, path or edit text is over 4096 bytes",
        ),
        (
            "{\"command\":\"a\\u0000b\"}",
            "the command, path or edit text holds a NUL byte",
        ),
        (
            "{\"edit\":{\"path\":\"a\"}}",
            "the edit must hold exactly path, old, new and expect_digest",
        ),
    ];
    for (reply, fault) in cases {
        let driven = drive_with(
            "sim-turn-fault",
            vec![Reply::from(reply), Reply::from("{\"done\":\"gave up\"}")],
            Scripted::new(Vec::new()),
            &[],
            |_| Ok(()),
        )?;
        let steps = driven.steps?;
        assert_eq!(
            driven.requests.len(),
            2,
            "{fault}: the model is asked again"
        );
        assert!(
            matches!(steps.stop, Stop::EndTurn),
            "{fault}: {:?}",
            steps.stop
        );
        assert!(
            driven.observer.started.is_empty(),
            "{fault}: nothing started"
        );
        let kept = if reply.len() > 8192 {
            format!(
                "(a reply of {} bytes, over the bound, was dropped)",
                reply.len()
            )
        } else {
            reply.to_owned()
        };
        assert_eq!(
            message(&driven.requests, 1, 2)?,
            kept,
            "{fault}: the conversation keeps exactly this"
        );
        assert_eq!(
            message(&driven.requests, 1, 3)?,
            format!(
                "Your reply was refused: {fault}. Reply with exactly one JSON object: {{\"command\": \"...\"}} or {{\"done\": \"...\"}}."
            ),
            "the model is told why"
        );
    }
    let replies = vec![Reply::from("run ls"), Reply::from("{\"done\":\"ok\"}")];
    let driven = drive_with(
        "sim-turn-not-json",
        replies,
        Scripted::new(Vec::new()),
        &[],
        |_| Ok(()),
    )?;
    let told = message(&driven.requests, 1, 3)?;
    assert!(
        told.starts_with("Your reply was refused: the reply is not JSON ("),
        "a reply that is not JSON names the parser's reason: {told}"
    );
    Ok(())
}

/// Two refused replies and then a good one reset the count; three in a row end
/// the turn, the person hears the line, and the model is not asked a fourth time.
#[test]
fn sim_only_refusals_in_a_row_end_the_turn() -> TestResult<()> {
    let bad = || Reply::from("{\"command\":7}");
    let replies = vec![
        bad(),
        bad(),
        Reply::from("{\"command\":\"true\"}"),
        bad(),
        bad(),
        bad(),
        Reply::from("{\"done\":\"never asked\"}"),
    ];
    let (steps, observer, served) =
        drive("sim-turn-three", replies, vec![Approval::Run], |_| Ok(()))?;
    assert_eq!(
        served, 6,
        "the good reply resets the count; the third ends it"
    );
    let line = "model: 3 replies in a row refused: the value must be a string";
    assert_eq!(refusal(&steps.stop)?, line);
    assert_eq!(observer.messages, vec![line.to_owned()]);
    assert_eq!(steps.tally.results.len(), 1, "the one good command ran");
    Ok(())
}

/// A gateway fault at any step (a refused request or a hung-up connection)
/// stops the turn naming the model, keeps every command that already ran and
/// says the same line to the person: no step is lost and none is invented.
#[test]
fn sim_a_gateway_fault_at_any_step_stops_the_turn_with_what_ran() -> TestResult<()> {
    let faults = [
        Reply::Refused {
            status: 400,
            body: String::from("{\"error\":{\"message\":\"scripted refusal\"}}"),
        },
        Reply::Refused {
            status: 503,
            body: String::from("not even JSON"),
        },
        Reply::Dropped,
    ];
    for fault in faults {
        for ran in 0..3_usize {
            let mut replies = vec![Reply::from("{\"command\":\"true\"}"); ran];
            replies.push(fault.clone());
            let (steps, observer, served) = drive(
                "sim-turn-gateway",
                replies,
                vec![Approval::Run; ran],
                |_| Ok(()),
            )?;
            assert_eq!(
                served,
                ran.saturating_add(1),
                "{fault:?}: no further request"
            );
            let line = refusal(&steps.stop)?;
            assert!(line.starts_with("model: "), "{fault:?}: {line}");
            assert_eq!(observer.messages, vec![line.to_owned()], "{fault:?}");
            assert_eq!(
                steps.tally.results.len(),
                ran,
                "{fault:?}: what ran is kept"
            );
            assert_eq!(
                steps.usage.len(),
                ran,
                "{fault:?}: one usage per completion"
            );
        }
    }
    Ok(())
}

/// A gateway that keeps refusing the window ends the turn: a window no
/// compaction fits under is the irreducible refusal, and a gateway that refuses
/// nine times though the window never shrinks is stopped at the shrink bound.
#[test]
fn sim_a_refused_window_ends_the_turn_naming_the_bound() -> TestResult<()> {
    let overflow = |limit: u64| Reply::Refused {
        status: 400,
        body: format!(
            "{{\"error\":{{\"code\":\"context_length_exceeded\",\"message\":\"This model's maximum context length is {limit} tokens.\"}}}}"
        ),
    };
    let (steps, observer, served) = drive(
        "sim-turn-window",
        vec![overflow(10)],
        Vec::new(),
        |_| Ok(()),
    )?;
    assert_eq!(
        served, 1,
        "nothing fits under nine bytes, so nothing is resent"
    );
    let line = refusal(&steps.stop)?;
    assert!(
        line.starts_with("model: compacted as far as it goes, the request is ")
            && line.ends_with(" bytes, over the model's window of 9"),
        "{line}"
    );
    assert_eq!(observer.messages, vec![line.to_owned()]);

    let (steps, _, served) = drive(
        "sim-turn-shrinks",
        vec![overflow(1_000_000_000); 9],
        Vec::new(),
        |_| Ok(()),
    )?;
    assert_eq!(served, 9, "eight shrinks, and the ninth refusal ends it");
    let line = refusal(&steps.stop)?;
    assert!(
        line.starts_with("model: ")
            && line.ends_with("; the gateway refused the model's window 8 times"),
        "{line}"
    );
    assert_eq!(
        steps.compaction.get("shrinks"),
        Some(&lgwks_std::json::json!(8))
    );
    Ok(())
}

/// A cancel before the first call, after a reply, or while the person decides on
/// a command, a read or an edit ends the turn cancelled with nothing run
/// (larry#242).
#[test]
fn sim_a_cancel_before_a_step_runs_nothing() -> TestResult<()> {
    let command = "{\"command\":\"true\"}";
    let read = "{\"read\":\"a.txt\"}";
    let edit =
        "{\"edit\":{\"path\":\"a.txt\",\"old\":\"a\",\"new\":\"b\",\"expect_digest\":\"d\"}}";
    let cases = [
        (command, Cancel::AfterAsks(0), 0),
        (command, Cancel::AfterAsks(1), 1),
        (command, Cancel::OnApprove, 1),
        (read, Cancel::OnApprove, 1),
        (edit, Cancel::OnApprove, 1),
    ];
    for (reply, cancel, asked) in cases {
        let driven = drive_with(
            "sim-turn-cancel",
            vec![Reply::from(reply)],
            Scripted::cancelling(vec![Approval::Run], cancel),
            &[],
            |_| Ok(()),
        )?;
        let steps = driven.steps?;
        assert_eq!(
            driven.requests.len(),
            asked,
            "{reply}: no request after the cancel"
        );
        assert!(
            matches!(steps.stop, Stop::Cancelled),
            "{reply}: {:?}",
            steps.stop
        );
        assert!(
            driven.observer.started.is_empty(),
            "{reply}: nothing started"
        );
        assert_eq!(steps.tally.step(), 0, "{reply}: nothing is recorded");
        assert_eq!(steps.tally.declined, 0, "{reply}: a cancel is no decline");
    }
    Ok(())
}

/// A cancel that lands while a command runs kills its group, records it
/// interrupted and never passed, and ends the turn cancelled (larry#242).
#[test]
fn sim_a_cancel_during_a_command_records_it_interrupted() -> TestResult<()> {
    let driven = drive_with(
        "sim-turn-interrupt",
        vec![Reply::from("{\"command\":\"sleep 30\"}")],
        Scripted::cancelling(vec![Approval::Run], Cancel::OnStart),
        &[],
        |_| Ok(()),
    )?;
    let steps = driven.steps?;
    assert!(matches!(steps.stop, Stop::Cancelled), "{:?}", steps.stop);
    assert_eq!(steps.tally.interrupted, 1);
    assert_eq!(
        steps.tally.results.len(),
        1,
        "the interrupted command is kept"
    );
    assert_eq!(
        driven.observer.observations,
        vec![String::from(
            "interrupted by the person; its process group was killed\nstdout:\n\nstderr:\n"
        )]
    );
    assert_eq!(driven.observer.passed, vec![false]);
    Ok(())
}

/// Every way a command can end shows the model its exact status line and both
/// streams, and only exit zero passes (larry#93).
#[test]
fn sim_each_command_ending_shows_the_model_its_exact_status() -> TestResult<()> {
    let cut = format!(
        "exit status 0 (output past 4096 bytes of a stream was cut)\nstdout:\n{}\nstderr:\n",
        "x".repeat(4096)
    );
    let cases = [
        (
            "printf out; printf err >&2",
            "exit status 0\nstdout:\nout\nstderr:\nerr",
            true,
        ),
        ("exit 3", "exit status 3\nstdout:\n\nstderr:\n", false),
        (
            "kill -9 $$",
            "ended by a signal\nstdout:\n\nstderr:\n",
            false,
        ),
        ("head -c 9000 /dev/zero | tr '\\000' x", cut.as_str(), true),
    ];
    for (command, shown, passed) in cases {
        let reply = lgwks_std::json::json!({ "command": command }).to_string();
        let replies = vec![Reply::from(reply), Reply::from("{\"done\":\"ran\"}")];
        let (steps, observer, _) =
            drive("sim-turn-status", replies, vec![Approval::Run], |_| Ok(()))?;
        assert_eq!(observer.observations, vec![shown.to_owned()], "{command}");
        assert_eq!(observer.passed, vec![passed], "{command}");
        assert_eq!(steps.tally.interrupted, 0, "{command}: no interrupt");
    }
    let driven = drive_with(
        "sim-turn-timeout",
        vec![
            Reply::from("{\"command\":\"sleep 30\"}"),
            Reply::from("{\"done\":\"ran\"}"),
        ],
        Scripted::new(vec![Approval::Run]),
        &[("LARRY_EXEC_TIMEOUT_MS", "200")],
        |_| Ok(()),
    )?;
    assert_eq!(
        driven.observer.observations,
        vec![String::from(
            "timed out; its process group was killed\nstdout:\n\nstderr:\n"
        )]
    );
    assert_eq!(driven.observer.passed, vec![false]);
    assert!(
        matches!(driven.steps?.stop, Stop::EndTurn),
        "a timeout is a step"
    );
    Ok(())
}

/// A declined read or edit is answered as not run, records nothing and touches
/// nothing (larry#586).
#[test]
fn sim_a_declined_read_or_edit_is_told_to_the_model() -> TestResult<()> {
    let replies = vec![
        Reply::from("{\"read\":\"a.txt\"}"),
        Reply::from(
            "{\"edit\":{\"path\":\"a.txt\",\"old\":\"a\",\"new\":\"b\",\"expect_digest\":\"d\"}}",
        ),
        Reply::from("{\"done\":\"left it\"}"),
    ];
    let driven = drive_with(
        "sim-turn-declined",
        replies,
        Scripted::new(vec![Approval::Decline, Approval::Decline]),
        &[],
        |_| Ok(()),
    )?;
    let steps = driven.steps?;
    assert_eq!(
        message(&driven.requests, 1, 3)?,
        "not run: the person declined this read\n"
    );
    assert_eq!(
        message(&driven.requests, 2, 5)?,
        "not run: the person declined this edit\n"
    );
    assert_eq!(steps.tally.declined, 2);
    assert!(
        steps.tally.changes.is_empty(),
        "a declined step leaves no record"
    );
    assert!(driven.observer.started.is_empty());
    Ok(())
}

/// The session takes exactly 256 steps, declines counted: the 257th reply is
/// never asked for, and one step fewer still ends at the model's done
/// (CWE-770).
#[test]
fn sim_the_step_bound_stops_at_exactly_its_count() -> TestResult<()> {
    let command = Reply::from("{\"command\":\"true\"}");
    let (steps, _, served) = drive(
        "sim-turn-bound",
        vec![command.clone(); 300],
        Vec::new(),
        |_| Ok(()),
    )?;
    assert_eq!(served, 256, "the bound stops the asking");
    assert!(
        matches!(steps.stop, Stop::MaxTurnRequests),
        "{:?}",
        steps.stop
    );
    assert_eq!(steps.tally.declined, 256);

    let mut replies = vec![command; 255];
    replies.push(Reply::from("{\"done\":\"just inside\"}"));
    let (steps, _, served) = drive("sim-turn-inside", replies, Vec::new(), |_| Ok(()))?;
    assert_eq!(served, 256);
    assert!(matches!(steps.stop, Stop::EndTurn), "{:?}", steps.stop);
    Ok(())
}

/// A command the host cannot start is a typed error out of the turn, never a
/// step the model is told about (larry#598).
#[test]
fn sim_a_shell_the_host_lacks_is_a_typed_error() -> TestResult<()> {
    let empty = scratch("sim-turn-nopath")?;
    let path = empty.to_str().ok_or("the path is not text")?;
    let driven = drive_with(
        "sim-turn-spawn",
        vec![Reply::from("{\"command\":\"true\"}")],
        Scripted::new(vec![Approval::Run]),
        &[("PATH", path)],
        |_| Ok(()),
    )?;
    assert!(
        matches!(driven.steps, Err(ExecError::Spawn { ref argv0, .. }) if argv0 == "sh"),
        "the missing shell is named"
    );
    assert!(driven.observer.passed.is_empty(), "no step ended");
    Ok(())
}

/// A file longer than the observation bound is digested and measured whole and
/// shown cut at the bound, so an edit's digest still names every byte
/// (larry#586, CWE-770).
#[test]
fn sim_a_long_file_is_digested_whole_and_shown_cut() -> TestResult<()> {
    let whole = "z".repeat(5000);
    let digest = lgwks_std::hash::blake3(whole.as_bytes()).to_hex();
    let replies = vec![
        Reply::from("{\"read\":\"long.txt\"}"),
        Reply::from("{\"done\":\"read it\"}"),
    ];
    let (_, observer, _) = drive("sim-turn-long", replies, vec![Approval::Run], |work| {
        std::fs::write(work.join("long.txt"), &whole)?;
        Ok(())
    })?;
    assert_eq!(
        observer.observations,
        vec![format!(
            "read\ndigest {digest} (5000 bytes), only the first 4096 bytes shown\n{}",
            "z".repeat(4096)
        )]
    );
    Ok(())
}
