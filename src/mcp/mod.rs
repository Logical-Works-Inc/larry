//! MCP server over stdio, started by `rocco mcp`.
//!
//! Written from the Model Context Protocol specification, revision 2025-06-18:
//! the initialize/initialized lifecycle with version negotiation, `ping`,
//! `tools/list`, `tools/call` and JSON-RPC 2.0 errors, one message per line.
//! The forge-mcp copy this replaces is not the reference: its Codex-adapted
//! client framing is not ported and its 2026-07-28 stateless era is not served.
//!
//! Bounds: a frame over `wire::MAX_FRAME_BYTES` (1 MiB) is discarded as it
//! streams in and answered with `-32600`; the server keeps serving. Pending work
//! is exactly one frame, read, answered, then the next is read, so a slow peer
//! can never queue anything. End of input ends the loop with exit code 0.

mod tools;
mod wire;

use lgwks_std::json::{self, Map, Value, json};
use std::io::{self, BufRead, Write};

/// Newest revision served; the reply to an initialize that names an unknown one.
const LATEST_VERSION: &str = "2025-06-18";
/// Revisions whose lifecycle and `tools/*` shapes this server answers, newest first.
const SUPPORTED_VERSIONS: [&str; 3] = [LATEST_VERSION, "2025-03-26", "2024-11-05"];
/// Reply text for a request that needs a completed initialize.
const NOT_INITIALIZED: &str = "server not initialized: send initialize first";

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

/// Per-connection state: the whole lifecycle is one flag.
#[derive(Default)]
struct Session {
    /// Set when an initialize request has been answered.
    initialized: bool,
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
    fn handle_object(&mut self, frame: &Map<String, Value>) -> Option<Value> {
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

    /// Answers one request.
    fn request(&mut self, id: &Value, method: &str, params: Option<&Map<String, Value>>) -> Value {
        match method {
            "ping" => wire::result(id, json!({})),
            "initialize" => self.initialize(id, params),
            "tools/list" | "tools/call" if !self.initialized => {
                wire::error(id, wire::INVALID_REQUEST, NOT_INITIALIZED)
            }
            "tools/list" => tools_list(id, params),
            "tools/call" => tools_call(id, params),
            _ => wire::error(id, wire::METHOD_NOT_FOUND, "method not found"),
        }
    }

    /// The lifecycle handshake: negotiate a revision and open the session.
    fn initialize(&mut self, id: &Value, params: Option<&Map<String, Value>>) -> Value {
        if self.initialized {
            return wire::error(id, wire::INVALID_REQUEST, "already initialized");
        }
        let Some(requested) = params
            .and_then(|map| map.get("protocolVersion"))
            .and_then(Value::as_str)
        else {
            return wire::error(
                id,
                wire::INVALID_PARAMS,
                "initialize requires params.protocolVersion as a string",
            );
        };
        let negotiated = match SUPPORTED_VERSIONS
            .iter()
            .copied()
            .find(|version| *version == requested)
        {
            Some(version) => version,
            None => LATEST_VERSION,
        };
        self.initialized = true;
        wire::result(
            id,
            json!({
                "protocolVersion": negotiated,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": {
                    "name": "larry",
                    "title": "Larry",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": "Hand Larry a typed task with larry_submit_task."
            }),
        )
    }
}

/// Whether `value` is a legal request id: a string or an integer, never null.
fn is_valid_id(value: &Value) -> bool {
    value.is_string() || value.is_i64() || value.is_u64()
}

/// `tools/list`: the declared tools; no cursor is ever issued, so none is valid.
fn tools_list(id: &Value, params: Option<&Map<String, Value>>) -> Value {
    if params.is_some_and(|map| map.contains_key("cursor")) {
        return wire::error(id, wire::INVALID_PARAMS, "invalid cursor");
    }
    wire::result(id, tools::list())
}

/// `tools/call`: an unknown tool or a malformed call is a protocol error; a
/// tool's own failure comes back as a result with `isError` true.
fn tools_call(id: &Value, params: Option<&Map<String, Value>>) -> Value {
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
        Some(outcome) => wire::result(id, outcome),
        None => wire::error(id, wire::INVALID_PARAMS, "unknown tool"),
    }
}

#[cfg(test)]
mod tests {
    use super::serve_bounded;
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

    /// The lifecycle, in order, with the notification silent.
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
        assert_eq!(got[2]["result"], json::json!({}), "ping is an empty result");
        Ok(())
    }

    /// A tool request before initialize is refused, an oversized line is
    /// refused, and the server keeps serving after both.
    #[test]
    fn pre_initialize_and_oversized_are_refused_then_serving_continues()
    -> Result<(), Box<dyn std::error::Error>> {
        let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n\
                     xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n\
                     {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n";
        let got = replies(input, 64)?;
        assert_eq!(got[0]["error"]["code"], -32600, "not initialized");
        assert_eq!(got[1]["error"]["code"], -32600, "oversized");
        assert_eq!(got[2]["id"], 2, "still serving");
        Ok(())
    }
}
