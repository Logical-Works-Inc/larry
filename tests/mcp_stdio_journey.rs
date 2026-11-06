//! Journeys against the shipped `rocco mcp` binary, as an MCP client lives them.
//!
//! Source: MCP specification 2025-06-18 (lifecycle, tools, transports/stdio)
//! and JSON-RPC 2.0 section 5 (error codes). Requirement: larry#84, a stdio
//! MCP server inside the shipped binary, bounded, typed, never simulating a
//! success. Every case drives the real process through its public interface.

mod mcp_support;

use lgwks_std::json::{Value, json};
use mcp_support::{Rng, Session, TestResult};
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Instant;

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
        let output = Command::new(env!("CARGO_BIN_EXE_rocco"))
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
