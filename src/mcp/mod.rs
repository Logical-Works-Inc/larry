//! MCP server over stdio, started by `larry mcp`.
//!
//! Written from the Model Context Protocol specification, revision 2026-07-28
//! (stateless: every request names its version and client capabilities in
//! `_meta`, `server/discover`, `resultType`, cache hints) and, for clients that
//! open with `initialize`, the initialize lifecycle of 2025-11-25, 2025-06-18,
//! 2025-03-26 and 2024-11-05. A dual-era server picks the era from how the
//! client opens (2026-07-28 Versioning): an `initialize` request selects legacy
//! semantics for this process, a request carrying the per-request version in
//! `_meta` is served statelessly, in either order, on the same process. The
//! forge-mcp copy this replaces is not the reference: its Codex-adapted client
//! framing is not ported.
//!
//! Bounds: a frame over `wire::MAX_FRAME_BYTES` (1 MiB) is discarded as it
//! streams in and answered with `-32600`; the server keeps serving. Pending work
//! is exactly one frame, read, answered, then the next is read, so a slow peer
//! can never queue anything. End of input ends the loop with exit code 0.

mod legacy;
mod modern;
mod tools;
mod wire;

use lgwks_std::json::{self, Map, Value, json};
use std::io::{self, BufRead, Write};

/// The stateless revision: newest, served per request.
const MODERN_VERSION: &str = "2026-07-28";
/// The newest legacy revision: the reply to an `initialize` that names one not served.
const LATEST_LEGACY: &str = "2025-11-25";
/// Legacy revisions served through `initialize`, newest first.
const LEGACY_VERSIONS: [&str; 4] = [LATEST_LEGACY, "2025-06-18", "2025-03-26", "2024-11-05"];
/// Guidance for the model, in `initialize` and `server/discover` results.
const INSTRUCTIONS: &str = "Hand Larry a typed task with larry_submit_task.";

/// Every revision this process serves, newest first, as `server/discover` and
/// `UnsupportedProtocolVersionError` advertise them.
fn supported_versions() -> Vec<&'static str> {
    let mut versions = vec![MODERN_VERSION];
    versions.extend(LEGACY_VERSIONS);
    versions
}

/// Who answers: the `serverInfo` of `initialize` and of every stateless result.
fn identity() -> Value {
    json!({ "name": "larry", "title": "Larry", "version": env!("CARGO_PKG_VERSION") })
}

/// What the server offers; the tool set is static, so nothing ever changes.
fn capabilities() -> Value {
    json!({ "tools": { "listChanged": false } })
}

/// Serves MCP until the reader reaches end of input.
pub(crate) fn serve<R: BufRead, W: Write>(reader: R, writer: W) -> io::Result<()> {
    serve_bounded(reader, writer, wire::MAX_FRAME_BYTES)
}

/// `serve` with an explicit frame bound, so a unit test can use a small one.
fn serve_bounded<R: BufRead, W: Write>(mut reader: R, mut writer: W, max: usize) -> io::Result<()> {
    let mut session = Session::default();
    loop {
        let reply = match wire::read_frame(&mut reader, max)? {
            wire::Frame::End => return Ok(()),
            wire::Frame::Oversized => Some(wire::error(
                &Value::Null,
                wire::INVALID_REQUEST,
                "frame exceeds the size limit and was discarded",
            )),
            wire::Frame::Line(bytes) => session.handle(&bytes),
        };
        if let Some(value) = reply {
            writer.write_all(&wire::encode(&value))?;
            writer.flush()?;
        }
    }
}

/// Per-process state: only the legacy lifecycle keeps any; the stateless era keeps none.
#[derive(Default)]
struct Session {
    /// The initialize handshake, for clients that open with `initialize`.
    lifecycle: legacy::Lifecycle,
}

impl Session {
    /// Maps one frame to its reply; `None` when JSON-RPC calls for silence.
    fn handle(&mut self, bytes: &[u8]) -> Option<Value> {
        match json::from_slice::<Value>(bytes) {
            Err(error) => Some(wire::error(
                &Value::Null,
                wire::PARSE_ERROR,
                &format!("parse error: {error}"),
            )),
            Ok(Value::Object(frame)) => self.handle_object(&frame),
            Ok(_) => Some(wire::error(
                &Value::Null,
                wire::INVALID_REQUEST,
                "a frame must be one JSON object; batches are not supported",
            )),
        }
    }

    /// Validates one JSON-RPC object and routes it.
    fn handle_object(&mut self, frame: &Map) -> Option<Value> {
        let id = match frame.get("id") {
            None => None,
            Some(value) if is_valid_id(value) => Some(value),
            Some(_) => {
                return Some(wire::error(
                    &Value::Null,
                    wire::INVALID_REQUEST,
                    "id must be a string or an integer",
                ));
            }
        };
        let reply_id = match id {
            Some(value) => value,
            None => &Value::Null,
        };
        if frame.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Some(wire::error(
                reply_id,
                wire::INVALID_REQUEST,
                "jsonrpc must be \"2.0\"",
            ));
        }
        let Some(method_value) = frame.get("method") else {
            if frame.contains_key("result") || frame.contains_key("error") {
                return None;
            }
            return Some(wire::error(
                reply_id,
                wire::INVALID_REQUEST,
                "method is required",
            ));
        };
        let Some(method) = method_value.as_str() else {
            return Some(wire::error(
                reply_id,
                wire::INVALID_REQUEST,
                "method must be a string",
            ));
        };
        let params = match frame.get("params") {
            None => None,
            Some(value) => match value.as_object() {
                Some(map) => Some(map),
                None => {
                    return id.map(|request_id| {
                        wire::error(request_id, wire::INVALID_PARAMS, "params must be an object")
                    });
                }
            },
        };
        let request_id = id?;
        Some(self.request(request_id, method, params))
    }

    /// Answers one request, choosing the era from how the request opens.
    ///
    /// `initialize` is legacy. A request carrying the per-request version in
    /// `_meta` is stateless, whatever came before. Otherwise an open legacy
    /// session serves it (and `ping` before the handshake, which the legacy
    /// lifecycle allows). A stateless method with neither `_meta` nor a session
    /// lacks required fields; any other method is unknown.
    fn request(&mut self, id: &Value, method: &str, params: Option<&Map>) -> Value {
        if method == "initialize" {
            return self.lifecycle.initialize(id, params);
        }
        if modern::is_stateless(params) {
            return modern::request(id, method, params);
        }
        if self.lifecycle.is_open() || method == "ping" {
            return legacy::request(id, method, params);
        }
        if modern::METHODS.contains(&method) {
            return modern::missing_meta(id);
        }
        wire::error(id, wire::METHOD_NOT_FOUND, "method not found")
    }
}

/// Whether `value` is a legal request id: a string or an integer, never null.
fn is_valid_id(value: &Value) -> bool {
    value.is_string() || value.as_i64().is_some() || value.as_u64().is_some()
}

/// `tools/list`: the declared tools, shaped for the era by `finish`; no cursor
/// is ever issued, so none is valid.
fn list_tools(id: &Value, params: Option<&Map>, finish: &dyn Fn(Value) -> Value) -> Value {
    if params.is_some_and(|map| map.contains_key("cursor")) {
        return wire::error(id, wire::INVALID_PARAMS, "invalid cursor");
    }
    wire::result(id, finish(tools::list()))
}

/// `tools/call`: an unknown tool or a malformed call is a protocol error; a
/// tool's own failure comes back as a result with `isError` true, shaped for
/// the era by `finish`.
fn call_tool(id: &Value, params: Option<&Map>, finish: &dyn Fn(Value) -> Value) -> Value {
    let Some(name) = params
        .and_then(|map| map.get("name"))
        .and_then(Value::as_str)
    else {
        return wire::error(
            id,
            wire::INVALID_PARAMS,
            "tools/call requires params.name as a string",
        );
    };
    let none = Map::new();
    let arguments = match params.and_then(|map| map.get("arguments")) {
        None => &none,
        Some(value) => match value.as_object() {
            Some(map) => map,
            None => {
                return wire::error(
                    id,
                    wire::INVALID_PARAMS,
                    "params.arguments must be an object",
                );
            }
        },
    };
    match tools::call(name, arguments) {
        Some(outcome) => wire::result(id, finish(outcome)),
        None => wire::error(id, wire::INVALID_PARAMS, "unknown tool"),
    }
}

#[cfg(test)]
mod tests {
    use super::{serve_bounded, supported_versions};
    use lgwks_std::json::{self, Value};
    use std::io::Cursor;

    /// Serves `input` and returns every reply line parsed.
    fn replies(input: &str, max: usize) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        let mut out = Vec::new();
        serve_bounded(Cursor::new(input.as_bytes().to_vec()), &mut out, max)?;
        let mut parsed = Vec::new();
        for line in String::from_utf8(out)?.lines() {
            parsed.push(json::from_str(line)?);
        }
        Ok(parsed)
    }

    /// The legacy lifecycle, in order, with the notification silent.
    #[test]
    fn initialize_then_list_then_ping() -> Result<(), Box<dyn std::error::Error>> {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#,
            "\n",
        );
        let got = replies(input, 1_048_576)?;
        assert_eq!(got.len(), 3, "the notification gets no reply");
        assert_eq!(
            got[0]["result"]["protocolVersion"], "2025-06-18",
            "version echoed"
        );
        assert_eq!(got[1]["id"], "a", "string ids come back as strings");
        assert_eq!(
            got[1]["result"]["tools"][0]["name"], "larry_submit_task",
            "tool listed"
        );
        assert!(
            got[1]["result"]["resultType"].is_null(),
            "a legacy result keeps the legacy shape"
        );
        assert_eq!(got[2]["result"], json::json!({}), "ping is an empty result");
        Ok(())
    }

    /// A stateless request needs no handshake and gets the 2026-07-28 shape.
    #[test]
    fn stateless_list_needs_no_handshake() -> Result<(), Box<dyn std::error::Error>> {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}"#,
            "\n",
        );
        let got = replies(input, 1_048_576)?;
        assert_eq!(got[0]["result"]["resultType"], "complete", "resultType");
        assert_eq!(got[0]["result"]["cacheScope"], "public", "cache scope");
        assert!(got[0]["result"]["ttlMs"].as_u64().is_some(), "ttlMs");
        Ok(())
    }

    /// A stateless method without `_meta` is refused, an oversized line is
    /// refused, and the server keeps serving after both.
    #[test]
    fn missing_meta_and_oversized_are_refused_then_serving_continues()
    -> Result<(), Box<dyn std::error::Error>> {
        let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n\
                     xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n\
                     {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n";
        let got = replies(input, 64)?;
        assert_eq!(got[0]["error"]["code"], -32602, "no _meta, no session");
        assert_eq!(got[1]["error"]["code"], -32600, "oversized");
        assert_eq!(got[2]["id"], 2, "still serving");
        Ok(())
    }

    /// The advertised list names the stateless revision first, then every legacy one.
    #[test]
    fn supported_versions_lead_with_the_newest() {
        assert_eq!(
            supported_versions(),
            [
                "2026-07-28",
                "2025-11-25",
                "2025-06-18",
                "2025-03-26",
                "2024-11-05"
            ],
            "newest first, legacy after"
        );
    }
}
