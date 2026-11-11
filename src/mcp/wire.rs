//! Newline-delimited stdio framing and JSON-RPC 2.0 message shapes, shared by
//! `larry mcp` and `larry acp` (larry#93).
//!
//! Written from the MCP specification (2026-07-28 and 2025-11-25, "Transports:
//! stdio") and the Agent Client Protocol's stdio transport, which agree: one
//! JSON-RPC message per line, UTF-8, no embedded newlines. The frame bound is
//! Larry's own: a line longer than the bound is discarded as it streams in, so a
//! peer that never sends a newline cannot grow the server's memory.

use lgwks_std::json::{self, Map, Value, json};
use std::io::{self, BufRead, ErrorKind};

/// Largest accepted frame in bytes, excluding the terminating newline.
pub(crate) const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// JSON-RPC 2.0: the frame was not valid JSON.
pub(crate) const PARSE_ERROR: i64 = -32700;
/// JSON-RPC 2.0: the frame was JSON but not a valid request.
pub(crate) const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC 2.0: the method does not exist.
pub(crate) const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC 2.0: the parameters are invalid.
pub(crate) const INVALID_PARAMS: i64 = -32602;
/// JSON-RPC 2.0: the server failed on its own side.
pub(crate) const INTERNAL_ERROR: i64 = -32603;
/// MCP 2026-07-28 `UnsupportedProtocolVersionError`: inside the range the spec
/// reserves for itself (-32020 to -32099); the only code from it this server emits.
pub(super) const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

/// One JSON-RPC 2.0 message, validated.
#[derive(Debug, PartialEq)]
pub(crate) enum Envelope<'frame> {
    /// A request: it gets exactly one reply.
    Request {
        /// Its id, a string or an integer.
        id: &'frame Value,
        /// The method.
        method: &'frame str,
        /// The parameters, when the request carries them.
        params: Option<&'frame Map>,
    },
    /// A notification: it gets no reply.
    Notification {
        /// The method.
        method: &'frame str,
        /// The parameters, when the notification carries them.
        params: Option<&'frame Map>,
    },
    /// A response to a request this side never sends; it gets no reply.
    Response,
}

/// Parses one frame into its JSON object, or the error reply JSON-RPC owes it.
pub(crate) fn object(bytes: &[u8]) -> Result<Map, Value> {
    match json::from_slice::<Value>(bytes) {
        Err(cause) => Err(error(
            &Value::Null,
            PARSE_ERROR,
            &format!("parse error: {cause}"),
        )),
        Ok(Value::Object(frame)) => Ok(frame),
        Ok(_) => Err(error(
            &Value::Null,
            INVALID_REQUEST,
            "a frame must be one JSON object; batches are not supported",
        )),
    }
}

/// Validates one JSON-RPC object. An invalid message is `Err` with the reply it
/// is owed, or with `None` when JSON-RPC answers it with silence (a notification
/// whose params are not an object).
pub(crate) fn envelope(frame: &Map) -> Result<Envelope<'_>, Option<Value>> {
    let id = match frame.get("id") {
        None => None,
        Some(value) if is_valid_id(value) => Some(value),
        Some(_) => {
            return Err(Some(error(
                &Value::Null,
                INVALID_REQUEST,
                "id must be a string or an integer",
            )));
        }
    };
    let reply_id = match id {
        Some(value) => value,
        None => &Value::Null,
    };
    if frame.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(Some(error(
            reply_id,
            INVALID_REQUEST,
            "jsonrpc must be \"2.0\"",
        )));
    }
    let Some(method_value) = frame.get("method") else {
        if frame.contains_key("result") || frame.contains_key("error") {
            return Ok(Envelope::Response);
        }
        return Err(Some(error(reply_id, INVALID_REQUEST, "method is required")));
    };
    let Some(method) = method_value.as_str() else {
        return Err(Some(error(
            reply_id,
            INVALID_REQUEST,
            "method must be a string",
        )));
    };
    let params = match frame.get("params") {
        None => None,
        Some(value) => match value.as_object() {
            Some(map) => Some(map),
            None => {
                return Err(id.map(|request_id| {
                    error(request_id, INVALID_PARAMS, "params must be an object")
                }));
            }
        },
    };
    Ok(match id {
        Some(id) => Envelope::Request { id, method, params },
        None => Envelope::Notification { method, params },
    })
}

/// Whether `value` is a legal request id: a string or an integer, never null.
fn is_valid_id(value: &Value) -> bool {
    value.is_string() || value.as_i64().is_some() || value.as_u64().is_some()
}

/// A notification frame: `method` with `params`, no id.
pub(crate) fn notification(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}
/// The JSON-RPC 2.0 internal error (-32603) reply, sent only when a value cannot be
/// encoded, which the shapes built here never hit.
const FALLBACK_REPLY: &[u8] =
    br#"{"jsonrpc":"2.0","error":{"code":-32603,"message":"internal error"}}"#;

/// One unit read from the peer.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Frame {
    /// A complete line with its newline and one trailing carriage return removed.
    Line(Vec<u8>),
    /// A line over the bound; its bytes were discarded without being stored.
    Oversized,
    /// End of input with nothing pending.
    End,
}

/// Reads one frame, holding at most `max` payload bytes.
///
/// A final line without a newline is served like a terminated one. Memory use
/// is bounded by `max` plus one buffer chunk whatever the peer sends.
pub(crate) fn read_frame<R: BufRead>(reader: &mut R, max: usize) -> io::Result<Frame> {
    let mut line: Vec<u8> = Vec::new();
    let mut oversized = false;
    let mut saw_bytes = false;
    loop {
        let chunk = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if chunk.is_empty() {
            return Ok(match (oversized, saw_bytes) {
                (true, _) => Frame::Oversized,
                (false, true) => Frame::Line(finish(line)),
                (false, false) => Frame::End,
            });
        }
        saw_bytes = true;
        let (payload_len, consumed, terminated) = match chunk.iter().position(|byte| *byte == b'\n')
        {
            Some(at) => (at, at.saturating_add(1), true),
            None => (chunk.len(), chunk.len(), false),
        };
        let Some(payload) = chunk.get(..payload_len) else {
            return Err(io::Error::new(ErrorKind::InvalidData, "frame bounds"));
        };
        if !oversized {
            if line.len().saturating_add(payload.len()) > max {
                oversized = true;
                line = Vec::new();
            } else {
                line.extend_from_slice(payload);
            }
        }
        reader.consume(consumed);
        if terminated {
            return Ok(if oversized {
                Frame::Oversized
            } else {
                Frame::Line(finish(line))
            });
        }
    }
}

/// Drops one trailing carriage return.
fn finish(mut line: Vec<u8>) -> Vec<u8> {
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    line
}

/// A success reply for request `id`.
pub(crate) fn result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// An error reply. `id` is `Value::Null` when the request id is unknown, and
/// the reply then omits `id`: the MCP schemas (2026-07-28 and 2025-11-25,
/// `JSONRPCErrorResponse`) type `id` as optional and string or integer, so a
/// null id fails a schema-checking client where an absent one is valid.
pub(crate) fn error(id: &Value, code: i64, message: &str) -> Value {
    with_id(
        id,
        json!({ "jsonrpc": "2.0", "error": { "code": code, "message": message } }),
    )
}

/// An error reply carrying a `data` member, as `UnsupportedProtocolVersionError` does.
pub(super) fn error_with_data(id: &Value, code: i64, message: &str, data: Value) -> Value {
    with_id(
        id,
        json!({ "jsonrpc": "2.0", "error": { "code": code, "message": message, "data": data } }),
    )
}

/// `frame` with `id` set, unless the id is unknown (`Value::Null`).
fn with_id(id: &Value, mut frame: Value) -> Value {
    if !id.is_null()
        && let Some(map) = frame.as_object_mut()
    {
        map.insert(String::from("id"), id.clone());
    }
    frame
}

/// Encodes one reply as a newline-terminated compact line.
///
/// Compact JSON never contains a raw newline.
pub(crate) fn encode(value: &Value) -> Vec<u8> {
    let mut bytes = match json::to_vec(value) {
        Ok(encoded) => encoded,
        Err(_) => FALLBACK_REPLY.to_vec(),
    };
    bytes.push(b'\n');
    bytes
}

#[cfg(test)]
mod tests {
    use super::{Frame, encode, error, read_frame};
    use lgwks_std::json::Value;
    use std::io::{BufReader, Cursor};

    /// A two-byte buffer forces the reader across many `fill_buf` chunks.
    fn tiny(input: &[u8]) -> BufReader<Cursor<Vec<u8>>> {
        BufReader::with_capacity(2, Cursor::new(input.to_vec()))
    }

    /// A line over the bound is discarded and the next line is still served.
    #[test]
    fn oversized_line_is_discarded_and_framing_recovers() {
        let mut reader = tiny(b"aaaaaaaaaa\nok\n");
        assert_eq!(
            read_frame(&mut reader, 4).ok(),
            Some(Frame::Oversized),
            "ten bytes against a bound of four"
        );
        assert_eq!(
            read_frame(&mut reader, 4).ok(),
            Some(Frame::Line(b"ok".to_vec())),
            "the next line survives"
        );
        assert_eq!(
            read_frame(&mut reader, 4).ok(),
            Some(Frame::End),
            "clean end of input"
        );
    }

    /// The bound is inclusive and counts the bytes before the newline, a
    /// carriage return among them; the carriage return is then trimmed.
    #[test]
    fn line_at_the_bound_is_kept_and_crlf_is_trimmed() {
        let mut reader = tiny(b"abcd\r\nabcde\nabcdef");
        assert_eq!(
            read_frame(&mut reader, 5).ok(),
            Some(Frame::Line(b"abcd".to_vec())),
            "four bytes and a carriage return fit a bound of five"
        );
        assert_eq!(
            read_frame(&mut reader, 5).ok(),
            Some(Frame::Line(b"abcde".to_vec())),
            "exactly the bound passes"
        );
        assert_eq!(
            read_frame(&mut reader, 5).ok(),
            Some(Frame::Oversized),
            "an unterminated line over the bound is refused at end of input"
        );
    }

    /// Source: schema.json (2026-07-28 and 2025-11-25) `JSONRPCErrorResponse`:
    /// `id` is optional and never null, so an unknown id is left out and a known one kept.
    #[test]
    fn an_unknown_id_is_omitted_and_a_known_one_is_kept() {
        let unknown = error(&Value::Null, -32700, "no");
        assert!(
            unknown
                .as_object()
                .is_some_and(|map| !map.contains_key("id")),
            "no id key for an unreadable request"
        );
        let known = error(&Value::from(7), -32601, "no");
        assert_eq!(known["id"], 7, "the request id comes back");
    }

    /// Encoded replies are one line and carry the id back.
    #[test]
    fn replies_are_single_newline_terminated_lines() {
        let bytes = encode(&error(&Value::from(7), -32601, "no"));
        assert_eq!(bytes.last(), Some(&b'\n'), "terminated");
        assert_eq!(
            bytes.iter().filter(|byte| **byte == b'\n').count(),
            1,
            "exactly one newline"
        );
    }
}
