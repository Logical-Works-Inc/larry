//! Newline-delimited stdio framing and JSON-RPC 2.0 reply shapes.
//!
//! Written from the MCP specification (2025-06-18, "Transports: stdio"): one
//! JSON-RPC message per line, UTF-8, no embedded newlines. The frame bound is
//! Larry's own: a line longer than the bound is discarded as it streams in, so
//! a peer that never sends a newline cannot grow the server's memory.

use lgwks_std::json::{self, Value, json};
use std::io::{self, BufRead, ErrorKind};

/// Largest accepted frame in bytes, excluding the terminating newline.
pub(super) const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// JSON-RPC 2.0: the frame was not valid JSON.
pub(super) const PARSE_ERROR: i64 = -32700;
/// JSON-RPC 2.0: the frame was JSON but not a valid request.
pub(super) const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC 2.0: the method does not exist.
pub(super) const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC 2.0: the parameters are invalid.
pub(super) const INVALID_PARAMS: i64 = -32602;
/// The JSON-RPC 2.0 internal error (-32603) reply, sent only when a value cannot be
/// encoded, which the shapes built here never hit.
const FALLBACK_REPLY: &[u8] =
    br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"internal error"}}"#;

/// One unit read from the peer.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Frame {
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
pub(super) fn read_frame<R: BufRead>(reader: &mut R, max: usize) -> io::Result<Frame> {
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
pub(super) fn result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// An error reply. `id` is `Value::Null` when the request id is unknown.
pub(super) fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Encodes one reply as a newline-terminated compact line.
///
/// Compact JSON never contains a raw newline.
pub(super) fn encode(value: &Value) -> Vec<u8> {
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
