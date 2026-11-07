//! Journeys against the shipped `larry mcp` binary, as an MCP client lives them.
//!
//! Source: MCP specification 2025-06-18 (lifecycle, tools, transports/stdio),
//! JSON-RPC 2.0 section 5 (error codes), RFC 8032 (the receipt's ed25519) and
//! forge#158 (a denied tool call reported as success). Requirement: larry#84, a
//! stdio MCP server inside the shipped binary, bounded, typed, never simulating
//! a success; larry#123, `larry_submit_task` runs the task through the model,
//! the workspace and the receipts, and only an accepted run is a success. Every
//! case drives the real process through its public interface, against the
//! seeded mock model endpoint, so no test makes a paid model call.

#[path = "support/http_wire.rs"]
mod http_wire;
mod mcp_support;
#[path = "support/mcp_task.rs"]
mod mcp_task;
#[path = "support/mock_model.rs"]
mod mock_model;
#[path = "support/origin.rs"]
mod origin;

use lgwks_std::json::{self, Value, json};
use mcp_support::{Rng, Session, TestResult};
use mcp_task::{briefed, connected, operator, submit, verify_offline};
use mock_model::{Config, Fault, Handle};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The valid typed task the issue implies: goal, repo, files, acceptance.
fn task() -> Value {
    json!({
        "goal": "bound the retry loop",
        "repo": "Logical-Works-Inc/larry",
        "files": ["src/retry.rs"],
        "acceptance": ["cargo nextest run --locked"]
    })
}

/// A started, initialized session.
fn ready() -> TestResult<Session> {
    let mut session = Session::start()?;
    let reply = session.initialize()?;
    assert_eq!(reply["id"], 0, "initialize reply carries the request id");
    Ok(session)
}

/// initialize, initialized, tools/list, tools/call, ping, unknown method, EOF.
#[test]
fn full_lifecycle_ends_with_exit_code_zero() -> TestResult<()> {
    let mut session = Session::start()?;
    let init = session.initialize()?;
    assert_eq!(
        init["result"]["protocolVersion"], "2025-06-18",
        "version negotiated"
    );
    assert_eq!(
        init["result"]["serverInfo"]["name"], "larry",
        "server identity"
    );
    assert_eq!(
        init["result"]["capabilities"]["tools"]["listChanged"], false,
        "tools declared, static"
    );

    let list = session.request(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))?;
    assert_eq!(
        list["result"]["tools"][0]["name"], "larry_submit_task",
        "the one tool"
    );
    assert_eq!(
        list["result"]["tools"][0]["inputSchema"]["additionalProperties"], false,
        "schema is closed"
    );

    let refused = session.request(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
        "params":{"name":"larry_submit_task","arguments":task()}}))?;
    assert_eq!(refused["result"]["isError"], true, "never a fake success");
    assert_eq!(
        refused["result"]["structuredContent"]["error"]["code"], "executor_not_connected",
        "typed refusal while the executor is absent"
    );

    let invalid = session.request(&json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
        "params":{"name":"larry_submit_task","arguments":{"goal":"x"}}}))?;
    assert_eq!(
        invalid["result"]["structuredContent"]["error"]["code"], "invalid_arguments",
        "a malformed task is a tool error, not a protocol error"
    );

    let unknown_tool = session.request(&json!({"jsonrpc":"2.0","id":4,"method":"tools/call",
        "params":{"name":"nope","arguments":{}}}))?;
    assert_eq!(
        unknown_tool["error"]["code"], -32602,
        "unknown tool is invalid params"
    );

    let ping = session.request(&json!({"jsonrpc":"2.0","id":"p","method":"ping"}))?;
    assert_eq!(ping["result"], json!({}), "ping answers an empty result");
    assert_eq!(ping["id"], "p", "string ids round-trip");

    let unknown = session.request(&json!({"jsonrpc":"2.0","id":6,"method":"resources/list"}))?;
    assert_eq!(
        unknown["error"]["code"], -32601,
        "unadvertised method is not found"
    );

    let status = session.finish()?;
    assert_eq!(status.code(), Some(0), "end of input is a clean shutdown");
    Ok(())
}

/// Lifecycle order: only ping and initialize before initialize; one initialize.
/// A stateless method with neither `_meta` nor a session lacks required fields.
#[test]
fn requests_before_initialize_are_refused_and_initialize_is_once() -> TestResult<()> {
    let mut session = Session::start()?;
    let early = session.request(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))?;
    assert_eq!(
        early["error"]["code"], -32602,
        "tools/list before initialize, without _meta"
    );
    let call = session.request(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
        "params":{"name":"larry_submit_task","arguments":task()}}))?;
    assert_eq!(
        call["error"]["code"], -32602,
        "tools/call before initialize, without _meta"
    );
    let ping = session.request(&json!({"jsonrpc":"2.0","id":3,"method":"ping"}))?;
    assert_eq!(ping["result"], json!({}), "ping is allowed at any time");
    let init = session.initialize()?;
    assert!(
        init["result"].as_object().is_some(),
        "initialize still works after refusals"
    );
    let again = session.request(&json!({"jsonrpc":"2.0","id":9,"method":"initialize",
        "params":{"protocolVersion":"2025-06-18"}}))?;
    assert_eq!(
        again["error"]["code"], -32600,
        "a second initialize is refused"
    );
    assert_eq!(session.finish()?.code(), Some(0), "clean exit");
    Ok(())
}

/// Version negotiation: a known legacy revision is echoed, an unknown one gets
/// the newest legacy revision.
#[test]
fn version_negotiation_echoes_known_and_offers_newest_otherwise() -> TestResult<()> {
    for (asked, answered) in [
        ("2024-11-05", "2024-11-05"),
        ("2025-11-25", "2025-11-25"),
        ("1999-01-01", "2025-11-25"),
    ] {
        let mut session = Session::start()?;
        let reply = session.request(&json!({"jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":asked,"capabilities":{},"clientInfo":{"name":"t","version":"0"}}}))?;
        assert_eq!(
            reply["result"]["protocolVersion"], answered,
            "asked {asked}"
        );
        assert_eq!(session.finish()?.code(), Some(0), "clean exit");
    }
    let mut session = Session::start()?;
    let bad =
        session.request(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))?;
    assert_eq!(bad["error"]["code"], -32602, "protocolVersion is required");
    Ok(())
}

/// The stateless 2026-07-28 journey as a client lives it: no handshake, the
/// version and capabilities on every request, discover, list, the typed
/// refusal, a removed method, an unsupported version, end of input.
#[test]
fn stateless_journey_needs_no_handshake_and_ends_with_exit_code_zero() -> TestResult<()> {
    let meta = |version: &str| {
        json!({"io.modelcontextprotocol/protocolVersion": version,
            "io.modelcontextprotocol/clientCapabilities": {},
            "io.modelcontextprotocol/clientInfo": {"name": "journey", "version": "1"}})
    };
    let mut session = Session::start()?;
    let refused = session.request(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"larry_submit_task","arguments":task(),"_meta":meta("2026-07-28")}}))?;
    assert_eq!(
        refused["result"]["resultType"], "complete",
        "resultType on a tool result"
    );
    assert_eq!(
        refused["result"]["structuredContent"]["error"]["code"], "executor_not_connected",
        "the first request of the process is served, typed refusal intact"
    );
    let found = session.request(&json!({"jsonrpc":"2.0","id":2,"method":"server/discover",
        "params":{"_meta":meta("2026-07-28")}}))?;
    assert_eq!(
        found["result"]["supportedVersions"][0], "2026-07-28",
        "newest first"
    );
    assert_eq!(
        found["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "larry",
        "identity in result _meta"
    );
    let list = session.request(&json!({"jsonrpc":"2.0","id":3,"method":"tools/list",
        "params":{"_meta":meta("2026-07-28")}}))?;
    assert_eq!(
        list["result"]["tools"][0]["name"], "larry_submit_task",
        "the one tool"
    );
    assert_eq!(list["result"]["cacheScope"], "public", "cache scope");
    let ping = session.request(&json!({"jsonrpc":"2.0","id":4,"method":"ping",
        "params":{"_meta":meta("2026-07-28")}}))?;
    assert_eq!(ping["error"]["code"], -32601, "ping is removed");
    let old = session.request(&json!({"jsonrpc":"2.0","id":5,"method":"tools/list",
        "params":{"_meta":meta("1900-01-01")}}))?;
    assert_eq!(old["error"]["code"], -32022, "unsupported version");
    assert_eq!(
        old["error"]["data"]["supported"][0], "2026-07-28",
        "the supported list names the newest first"
    );
    assert_eq!(session.finish()?.code(), Some(0), "clean exit");
    Ok(())
}

/// A malformed line is a parse error with no id and the stream recovers.
#[test]
fn malformed_line_gets_parse_error_and_the_server_keeps_serving() -> TestResult<()> {
    let mut session = ready()?;
    session.send(b"{this is not json")?;
    let error = session.recv()?;
    assert_eq!(error["error"]["code"], -32700, "parse error");
    assert_eq!(error["id"], Value::Null, "id unknown");
    session.send(b"[1,2,3]")?;
    assert_eq!(
        session.recv()?["error"]["code"],
        -32600,
        "batches are refused"
    );
    session.send(&[0xff, 0xfe, 0x7b])?;
    assert_eq!(
        session.recv()?["error"]["code"],
        -32700,
        "invalid UTF-8 is a parse error"
    );
    let ping = session.request(&json!({"jsonrpc":"2.0","id":5,"method":"ping"}))?;
    assert_eq!(ping["id"], 5, "still serving");
    assert_eq!(session.finish()?.code(), Some(0), "clean exit");
    Ok(())
}

/// A frame over the 1 MiB bound is refused with -32600 and the server survives.
#[test]
fn oversized_frame_is_refused_and_the_server_keeps_serving() -> TestResult<()> {
    let mut session = ready()?;
    let huge = vec![b'x'; 1024 * 1024 + 1];
    session.send(&huge)?;
    let error = session.recv()?;
    assert_eq!(error["error"]["code"], -32600, "oversized frame refused");
    assert_eq!(error["id"], Value::Null, "id unknown");
    let ping = session.request(&json!({"jsonrpc":"2.0","id":8,"method":"ping"}))?;
    assert_eq!(ping["id"], 8, "framing recovered after the discarded line");
    assert_eq!(session.finish()?.code(), Some(0), "clean exit");
    Ok(())
}

/// Replies are compact single lines, and a final unterminated frame is served.
#[test]
fn replies_are_single_lines_and_an_unterminated_last_frame_is_served() -> TestResult<()> {
    let mut session = ready()?;
    session.send(br#"{"jsonrpc":"2.0","id":12,"method":"ping"}"#)?;
    let line = session.recv_line()?;
    assert_eq!(
        line.iter().filter(|byte| **byte == b'\n').count(),
        1,
        "one line"
    );
    assert!(!line.contains(&b' '), "compact JSON has no padding");
    session.send_raw(br#"{"jsonrpc":"2.0","id":11,"method":"ping"}"#)?;
    session.close_stdin();
    assert_eq!(
        session.recv()?["id"],
        11,
        "the final unterminated frame is answered"
    );
    assert!(
        session.recv_line().is_err(),
        "then the server closes stdout"
    );
    assert_eq!(session.finish()?.code(), Some(0), "clean exit");
    Ok(())
}

/// kill -9 loses nothing because the server holds nothing: a restart serves at once.
#[test]
fn kill_dash_nine_then_restart_serves_a_fresh_session() -> TestResult<()> {
    let mut rng = Rng(0x9);
    let id = rng.next();
    let salt = rng.below(1000);
    let lane = rng.index(3)?;
    let mut first = ready()?;
    let ping = first.request(&json!({"jsonrpc":"2.0","id":id,"method":"ping"}))?;
    assert_eq!(
        ping["id"], id,
        "served before the kill (salt {salt}, lane {lane})"
    );
    let status = first.kill()?;
    assert!(status.code().is_none(), "SIGKILL leaves no exit code");
    let mut second = ready()?;
    let list = second.request(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))?;
    assert_eq!(
        list["result"]["tools"][0]["name"], "larry_submit_task",
        "fresh session serves"
    );
    assert_eq!(second.finish()?.code(), Some(0), "clean exit");
    Ok(())
}

/// Anything but `mcp` or nothing is refused without touching stdout.
#[test]
fn unknown_arguments_exit_nonzero_with_empty_stdout() -> TestResult<()> {
    for args in [vec!["mcp", "extra"], vec!["serve"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_larry"))
            .args(&args)
            .stdin(Stdio::null())
            .output()?;
        assert_eq!(output.status.code(), Some(1), "args {args:?}");
        assert!(
            output.stdout.is_empty(),
            "nothing on the protocol stream for {args:?}"
        );
    }
    Ok(())
}

/// The `percent`th percentile of a sorted, non-empty list of samples.
fn percentile(sorted: &[u128], percent: usize) -> TestResult<u128> {
    let rank = sorted
        .len()
        .saturating_mul(percent)
        .div_ceil(100)
        .saturating_sub(1);
    Ok(*sorted.get(rank).ok_or("no samples")?)
}

/// tools/list round trips over a pipe: p50/p95/p99 reported, p99 bounded.
#[test]
fn tools_list_latency_is_reported_and_bounded() -> TestResult<()> {
    const CALLS: usize = 3000;
    let mut session = ready()?;
    let mut micros: Vec<u128> = Vec::with_capacity(CALLS);
    for index in 0..CALLS {
        let started = Instant::now();
        let reply = session.request(&json!({"jsonrpc":"2.0","id":index,"method":"tools/list"}))?;
        micros.push(started.elapsed().as_micros());
        assert_eq!(
            reply["result"]["tools"][0]["name"], "larry_submit_task",
            "call {index}"
        );
    }
    micros.sort_unstable();
    let (p50, p95, p99) = (
        percentile(&micros, 50)?,
        percentile(&micros, 95)?,
        percentile(&micros, 99)?,
    );
    let report =
        format!("mcp tools/list round trips: n={CALLS} p50={p50}us p95={p95}us p99={p99}us\n");
    std::io::stderr().lock().write_all(report.as_bytes())?;
    assert!(p99 < 1_000_000, "p99 under one second on a local pipe");
    assert_eq!(session.finish()?.code(), Some(0), "clean exit");
    Ok(())
}

/// Seeds the submitted-task journey runs, each twice.
const TASK_SEEDS: u64 = 8;

/// What one accepted run left: the model trace, the sealed diff, the wall time.
struct Run {
    /// The mock endpoint's trace hash.
    trace: String,
    /// The sealed diff.
    diff: String,
    /// Wall time of the `tools/call`, request to reply.
    took: Duration,
}

/// One accepted submitted task on a fresh origin under seed `seed`.
fn accepted_run(seed: u64) -> TestResult<Run> {
    let (state, origin) = origin::fixture()?;
    mock_model::run(Config::seeded(seed), |handle: &Handle| -> TestResult<Run> {
        let mut session = connected(&operator(state.path(), handle.addr(), &[]))?;
        let task = briefed(
            origin.path(),
            &["README", "src/new.rs"],
            &["test -s src/new.rs", "test -s README"],
        );
        let started = Instant::now();
        let result = submit(&mut session, &task)?;
        let took = started.elapsed();
        assert_eq!(
            result["isError"], false,
            "seed {seed}: accepted is a success: {result}"
        );
        let structured = &result["structuredContent"];
        assert_eq!(structured["accepted"], true, "seed {seed}: the verdict");
        let text = result["content"][0]["text"]
            .as_str()
            .ok_or("a text block")?;
        assert_eq!(
            &json::from_str::<Value>(text)?,
            structured,
            "the text block is the structured content"
        );
        let report = &structured["report"];
        let diff = report["diff"].as_str().ok_or("a sealed diff")?.to_owned();
        assert!(
            diff.contains("-origin readme"),
            "seed {seed}: README was rewritten: {diff}"
        );
        assert!(
            diff.contains("+++ b/src/new.rs"),
            "seed {seed}: src/new.rs was written"
        );
        assert_eq!(
            report["authored"][0]["path"], "README",
            "seed {seed}: README is recorded"
        );
        assert_eq!(
            report["authored"][0]["existed"], true,
            "seed {seed}: README existed"
        );
        assert_eq!(
            report["authored"][1]["existed"], false,
            "seed {seed}: src/new.rs is new"
        );
        for index in 0..2 {
            assert_eq!(
                report["results"][index]["exit_code"], 0,
                "seed {seed}: command {index} saw the files"
            );
        }
        assert_eq!(handle.served(), 2, "seed {seed}: one completion per file");
        let first: Value = json::from_slice(handle.requests().first().ok_or("a model request")?)?;
        let turn = first["messages"][1]["content"]
            .as_str()
            .ok_or("a user turn")?;
        assert!(
            turn.contains("replace the readme")
                && turn.contains("File: README")
                && turn.contains("origin readme"),
            "seed {seed}: the goal, the path and the current content reached the model: {turn}"
        );
        assert!(
            handle
                .authorizations()
                .iter()
                .all(|sent| sent.as_deref() == Some("Bearer mcp-key")),
            "seed {seed}: the operator's key, and only it, reached the gateway"
        );
        let (code, verdict) = verify_offline(state.path(), report)?;
        assert_eq!(
            code,
            Some(0),
            "seed {seed}: the receipt verifies offline as accepted"
        );
        assert_eq!(
            verdict["task_id"], report["task_id"],
            "seed {seed}: the verdict names the task"
        );
        assert_eq!(session.finish()?.code(), Some(0), "clean exit");
        Ok(Run {
            trace: handle.trace_hash(),
            diff,
            took,
        })
    })
}

/// larry#123: one task over MCP against the mock model, its receipt verified,
/// for 8 seeds, each replayed to the same model trace and the same diff.
#[test]
fn a_submitted_task_runs_against_the_mock_model_and_its_receipt_verifies() -> TestResult<()> {
    let mut samples: Vec<Duration> = Vec::new();
    for seed in 0..TASK_SEEDS {
        let first = accepted_run(seed)?;
        let again = accepted_run(seed)?;
        assert_eq!(
            first.trace, again.trace,
            "seed {seed}: one seed, one model trace"
        );
        assert_eq!(
            first.diff, again.diff,
            "seed {seed}: one seed, one sealed diff"
        );
        samples.push(first.took);
        samples.push(again.took);
    }
    samples.sort_unstable();
    let micros: Vec<u128> = samples.iter().map(Duration::as_micros).collect();
    let line = format!(
        "mcp submitted task (clone, 2 completions, 2 commands, seal): n={} p50={}us p95={}us p99={}us\n",
        micros.len(),
        percentile(&micros, 50)?,
        percentile(&micros, 95)?,
        percentile(&micros, 99)?
    );
    std::io::stderr().lock().write_all(line.as_bytes())?;
    Ok(())
}

/// The tool error code and message of a failed `result`.
fn failure_of(result: &Value) -> TestResult<(String, String)> {
    assert_eq!(
        result["isError"], true,
        "a failure is never a success: {result}"
    );
    let error = &result["structuredContent"]["error"];
    let code = error["code"].as_str().ok_or("an error code")?.to_owned();
    let message = error["message"]
        .as_str()
        .ok_or("an error message")?
        .to_owned();
    Ok((code, message))
}

/// How many workspaces a state directory holds; none before its first run.
fn workspaces(state: &Path) -> TestResult<usize> {
    match fs::read_dir(state.join("ws")) {
        Ok(entries) => Ok(entries.count()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

/// Submits `task` with `script` faults and `extra` settings on a fresh state directory;
/// returns the failed result's code and message and the completions served.
fn refused_with(
    task: &Value,
    script: Vec<Option<Fault>>,
    extra: &[(&'static str, &str)],
) -> TestResult<(String, String, u64)> {
    let state = origin::TempDir::new()?;
    let config = Config {
        script,
        ..Config::seeded(21)
    };
    let outcome = mock_model::run(
        config,
        |handle: &Handle| -> TestResult<(String, String, u64)> {
            let mut session = connected(&operator(state.path(), handle.addr(), extra))?;
            let result = submit(&mut session, task)?;
            handle.release_delayed();
            assert!(
                result["structuredContent"].get("report").is_none(),
                "a refusal seals nothing: {result}"
            );
            let (code, message) = failure_of(&result)?;
            assert_eq!(session.finish()?.code(), Some(0), "clean exit");
            Ok((code, message, handle.served()))
        },
    )?;
    assert_eq!(
        workspaces(state.path())?,
        0,
        "a refused task leaves no workspace behind"
    );
    Ok(outcome)
}

/// larry#123, forge#158: a rejected run, every model fault, a symlink escape, a
/// missing model key and a bad repository are failed tool results, never success.
#[test]
fn a_rejected_or_refused_task_is_a_failed_tool_result_never_success() -> TestResult<()> {
    let (state, origin) = origin::fixture()?;
    let files = ["README"];
    let rejected = mock_model::run(Config::seeded(20), |handle: &Handle| -> TestResult<Value> {
        let mut session = connected(&operator(state.path(), handle.addr(), &[]))?;
        let result = submit(&mut session, &briefed(origin.path(), &files, &["exit 3"]))?;
        assert_eq!(session.finish()?.code(), Some(0), "clean exit");
        Ok(result)
    })?;
    let (code, _) = failure_of(&rejected)?;
    assert_eq!(code, "task_rejected", "a failing command is a rejected run");
    let report = &rejected["structuredContent"]["report"];
    assert_eq!(report["accepted"], false, "sealed as rejected");
    assert_eq!(
        verify_offline(state.path(), report)?.0,
        Some(2),
        "its receipt verifies as rejected"
    );

    let task = briefed(origin.path(), &files, &["true"]);
    let once = [("LARRY_MODEL_ATTEMPTS", "1")];
    let faults = [
        (Fault::ServerError { status: 502 }, "status 502"),
        (
            Fault::RateLimit {
                retry_after_secs: 30,
            },
            "retry after 30s",
        ),
        (Fault::HostileRefusal { status: 401 }, "status 401"),
    ];
    for (fault, needle) in faults {
        let (code, message, served) = refused_with(&task, vec![Some(fault)], &once)?;
        assert_eq!(code, "task_refused", "{fault:?} refuses the task");
        assert!(
            message.contains(needle),
            "{fault:?}: the planner sees why: {message:.160}"
        );
        assert!(
            !message.chars().any(char::is_control),
            "{fault:?}: no terminal control reaches the planner"
        );
        assert_eq!(served, 1, "{fault:?}: one attempt was configured");
    }
    let slow = [("LARRY_MODEL_TIMEOUT_MS", "200")];
    let (code, message, _) = refused_with(&task, vec![Some(Fault::Delay)], &slow)?;
    assert_eq!(
        (code.as_str(), message.contains("timed out")),
        ("task_refused", true),
        "{message}"
    );

    // A zero-wait retry goes ahead, and a padded OpenRouter reply still parses.
    let flaky = Config {
        script: vec![
            Some(Fault::ServerError { status: 503 }),
            Some(Fault::PaddedDone),
        ],
        ..Config::seeded(22)
    };
    let retried = origin::TempDir::new()?;
    mock_model::run(flaky, |handle: &Handle| -> TestResult<()> {
        let mut session = connected(&operator(retried.path(), handle.addr(), &[]))?;
        let result = submit(&mut session, &task)?;
        assert_eq!(
            result["isError"], false,
            "a 503 then a padded reply is accepted: {result}"
        );
        assert_eq!(handle.served(), 2, "the 503 was retried once");
        assert_eq!(session.finish()?.code(), Some(0), "clean exit");
        Ok(())
    })?;

    // A symlink the origin carries is never written through.
    let outside = origin::TempDir::new()?;
    std::os::unix::fs::symlink(outside.path(), origin.path().join("escape"))?;
    origin::commit_all(origin.path(), "add a symlink")?;
    let escape = briefed(origin.path(), &["escape/x.rs"], &["true"]);
    let (code, message, served) = refused_with(&escape, Vec::new(), &[])?;
    assert_eq!(code, "task_refused", "a symlink escape is refused");
    assert!(
        message.contains("symlink"),
        "the planner sees why: {message}"
    );
    assert_eq!(served, 0, "the model is never asked");
    assert_eq!(
        fs::read_dir(outside.path())?.count(),
        0,
        "nothing was written outside the clone"
    );

    // No model key: the executor is not connected; a non-repository is refused.
    let keyless = origin::TempDir::new()?;
    let mut session = connected(&[("LARRY_STATE_DIR", keyless.path().as_os_str().to_owned())])?;
    let (code, message) = failure_of(&submit(&mut session, &task)?)?;
    assert_eq!(code, "executor_not_connected", "no key, no executor");
    assert!(
        message.contains("LARRY_MODEL_KEY"),
        "the message names the variable: {message}"
    );
    assert_eq!(session.finish()?.code(), Some(0), "clean exit");
    let empty = origin::TempDir::new()?;
    let (code, message, served) =
        refused_with(&briefed(empty.path(), &files, &["true"]), Vec::new(), &[])?;
    assert_eq!(
        (code.as_str(), served),
        ("task_refused", 0),
        "a non-repository is refused before the model"
    );
    assert!(message.contains("could not clone"), "{message}");
    Ok(())
}
