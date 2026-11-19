//! The ACP agent's protocol surface, driven through `serve_bounded` with no
//! executor behind it (larry#93). Source: the Agent Client Protocol v1
//! (initialization, session setup, prompt turn, stdio transport) and JSON-RPC 2.0
//! section 5.1: every malformed frame gets its exact error and the agent keeps serving.

use std::io::Cursor;

use lgwks_std::json::{self, Value};

use super::serve_bounded;
use crate::executor::ExecError;
use crate::json_at::At;

/// Serves `lines` with refused settings and returns every reply line and the log.
fn replies(lines: &[&str], max: usize) -> Result<(Vec<Value>, String), Box<dyn std::error::Error>> {
    let mut input = lines.join("\n");
    input.push('\n');
    let mut out: Vec<u8> = Vec::new();
    let mut log: Vec<u8> = Vec::new();
    let settings = Err(ExecError::StateDirUnset);
    serve_bounded(
        Cursor::new(input.into_bytes()),
        &mut out,
        max,
        &settings,
        &mut log,
    )?;
    let mut parsed = Vec::new();
    for line in String::from_utf8(out)?.lines() {
        parsed.push(json::from_str(line)?);
    }
    Ok((parsed, String::from_utf8(log)?))
}

/// An absolute directory that exists on every host the tests run on.
fn tmp() -> Result<String, Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().canonicalize()?;
    Ok(dir.to_str().ok_or("temp dir is not UTF-8")?.to_owned())
}

/// The opening every session needs.
const INIT: &str = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}}"#;

/// Version 1 is answered whatever is asked, with what Larry offers and nothing more.
#[test]
fn initialize_answers_version_one_and_its_capabilities() -> Result<(), Box<dyn std::error::Error>> {
    let (got, _) = replies(
        &[r#"{"jsonrpc":"2.0","id":"a","method":"initialize","params":{"protocolVersion":7}}"#],
        1_048_576,
    )?;
    let reply = got.first().ok_or("no reply")?;
    let result = reply.at("/result")?;
    assert_eq!(*reply.at("/id")?, "a", "id echoed");
    assert_eq!(
        *result.at("/protocolVersion")?,
        1,
        "the one version Larry speaks"
    );
    assert_eq!(
        *result.at("/agentCapabilities/loadSession")?,
        false,
        "no load"
    );
    assert_eq!(
        *result.at("/agentCapabilities/promptCapabilities/image")?,
        false,
        "no image input"
    );
    assert_eq!(
        *result.at("/authMethods")?,
        json::json!([]),
        "no auth method"
    );
    assert_eq!(*result.at("/agentInfo/name")?, "larry", "agent info");
    Ok(())
}

/// Every malformed frame and every refused request gets its exact error, and the
/// agent answers the next frame.
#[test]
fn every_malformed_frame_gets_its_exact_error() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tmp()?;
    let new_relative = r#"{"jsonrpc":"2.0","id":9,"method":"session/new","params":{"cwd":"rel/dir","mcpServers":[]}}"#;
    let new_no_servers = format!(
        r#"{{"jsonrpc":"2.0","id":10,"method":"session/new","params":{{"cwd":{}}}}}"#,
        json::to_string(&dir)?
    );
    let lines = [
        r#"{"jsonrpc":"2.0","id":1,"method":"session/new","params":{}}"#,
        "not json",
        "[1,2]",
        r#"{"jsonrpc":"2.0","id":null,"method":"x"}"#,
        r#"{"jsonrpc":"1.0","id":2,"method":"x"}"#,
        r#"{"jsonrpc":"2.0","id":3}"#,
        r#"{"jsonrpc":"2.0","id":4,"method":"initialize","params":[]}"#,
        r#"{"jsonrpc":"2.0","method":"session/cancel","params":[]}"#,
        r#"{"jsonrpc":"2.0","id":5,"method":"initialize","params":{"protocolVersion":"1"}}"#,
        INIT,
        INIT,
        r#"{"jsonrpc":"2.0","id":6,"method":"authenticate","params":{"methodId":"x"}}"#,
        r#"{"jsonrpc":"2.0","id":7,"method":"session/load","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":8,"method":"session/prompt","params":{"sessionId":"nope","prompt":[]}}"#,
        new_relative,
        &new_no_servers,
        r#"{"jsonrpc":"2.0","result":{},"id":11}"#,
    ];
    let (got, _) = replies(&lines, 1_048_576)?;
    // `Value::Null` is an id the reply omits because the request's was unknown (`wire::error`).
    let codes = got
        .iter()
        .map(super::sim::id_and_code)
        .collect::<Result<Vec<_>, _>>()?;
    let expected: Vec<(Value, Option<i64>)> = [
        (json::json!(1), Some(-32600)),
        (Value::Null, Some(-32700)),
        (Value::Null, Some(-32600)),
        (Value::Null, Some(-32600)),
        (json::json!(2), Some(-32600)),
        (json::json!(3), Some(-32600)),
        (json::json!(4), Some(-32602)),
        (json::json!(5), Some(-32602)),
        (json::json!(0), None),
        (json::json!(0), Some(-32600)),
        (json::json!(6), Some(-32602)),
        (json::json!(7), Some(-32601)),
        (json::json!(8), Some(-32602)),
        (json::json!(9), Some(-32602)),
        (json::json!(10), Some(-32602)),
    ]
    .to_vec();
    assert_eq!(codes, expected, "each frame's exact answer, in order");
    assert_eq!(
        *got.first()
            .ok_or("no reply at that index")?
            .at("/error/message")?,
        "initialize must come first",
        "a session before the handshake"
    );
    assert_eq!(
        *got.get(13)
            .ok_or("no reply at that index")?
            .at("/error/message")?,
        "cwd must be an absolute path",
        "ACP: cwd MUST be absolute"
    );
    Ok(())
}

/// The frame bound and its error are `larry mcp`'s own wire, and serving goes on.
#[test]
fn the_frame_bound_and_errors_are_the_mcp_wires_own() -> Result<(), Box<dyn std::error::Error>> {
    // The bound sits above the initialize frame (about 100 bytes) and below the long one.
    let long = "x".repeat(200);
    let (got, _) = replies(&[&long, INIT], 160)?;
    assert_eq!(
        *got.first()
            .ok_or("no reply at that index")?
            .at("/error/code")?,
        -32600,
        "oversized"
    );
    assert_eq!(
        *got.first()
            .ok_or("no reply at that index")?
            .at("/error/message")?,
        "frame exceeds the size limit and was discarded",
        "the wire's message"
    );
    assert_eq!(
        *got.get(1)
            .ok_or("no reply at that index")?
            .at("/result/protocolVersion")?,
        1,
        "still serving"
    );
    Ok(())
}

/// A prompt on a process whose settings were refused ends as a refusal that says why,
/// and a cancel is acknowledged on the log, never on the protocol stream.
#[test]
fn a_prompt_without_settings_is_a_refusal_and_a_cancel_is_logged()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tmp()?;
    let new = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"session/new","params":{{"cwd":{},"mcpServers":[]}}}}"#,
        json::to_string(&dir)?
    );
    let session = format!("larry-{}-0", std::process::id());
    let cancel = format!(
        r#"{{"jsonrpc":"2.0","method":"session/cancel","params":{{"sessionId":"{session}"}}}}"#
    );
    let prompt = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{{"sessionId":"{session}","prompt":[{{"type":"text","text":"do it"}}]}}}}"#
    );
    let image = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{{"sessionId":"{session}","prompt":[{{"type":"image","data":"","mimeType":"image/png"}}]}}}}"#
    );
    let (got, log) = replies(&[INIT, &new, &cancel, &prompt, &image], 1_048_576)?;
    assert_eq!(
        *got.get(1)
            .ok_or("no reply at that index")?
            .at("/result/sessionId")?,
        session.as_str(),
        "session id"
    );
    assert_eq!(
        *got.get(2).ok_or("no reply at that index")?.at("/method")?,
        "session/update",
        "the refusal is said first"
    );
    assert_eq!(
        *got.get(2)
            .ok_or("no reply at that index")?
            .at("/params/update/content/text")?,
        ExecError::StateDirUnset.to_string().as_str(),
        "the reason is the refused settings' own message"
    );
    assert_eq!(
        *got.get(3)
            .ok_or("no reply at that index")?
            .at("/result/stopReason")?,
        "refusal",
        "stop reason"
    );
    assert_eq!(
        *got.get(4)
            .ok_or("no reply at that index")?
            .at("/error/code")?,
        -32602,
        "an image block is refused"
    );
    assert_eq!(got.len(), 5, "the cancel notification gets no reply");
    assert_eq!(
        log,
        format!("larry acp: session {session} cancel requested\n"),
        "the cancel is acknowledged on the log"
    );
    Ok(())
}

/// The seventeenth session is refused; the sixteen before it are not.
#[test]
fn a_process_holds_at_most_sixteen_sessions() -> Result<(), Box<dyn std::error::Error>> {
    let dir = json::to_string(&tmp()?)?;
    let opens: Vec<String> = (1..=17)
        .map(|id| {
            format!(
                r#"{{"jsonrpc":"2.0","id":{id},"method":"session/new","params":{{"cwd":{dir},"mcpServers":[]}}}}"#
            )
        })
        .collect();
    let mut lines: Vec<&str> = vec![INIT];
    lines.extend(opens.iter().map(String::as_str));
    let (got, _) = replies(&lines, 1_048_576)?;
    let opened = got
        .iter()
        .filter(|reply| matches!(reply.member("/result/sessionId"), Some(Value::String(..))))
        .count();
    assert_eq!(opened, 16, "sixteen sessions");
    assert_eq!(
        *got.get(17)
            .ok_or("no reply at that index")?
            .at("/error/code")?,
        -32600,
        "the seventeenth is refused"
    );
    Ok(())
}
