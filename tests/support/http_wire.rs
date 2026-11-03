//! Minimal HTTP/1.1 framing for the mock model endpoint: one request per
//! connection, length-delimited bodies only, every read bounded. Test support,
//! never linked into a shipped binary (INVARIANTS: test doubles never ship).

use std::io::Read;

/// Largest request head (request line plus headers) the mock will buffer.
const MAX_HEAD_BYTES: usize = 16_384;

/// One parsed request.
#[derive(Debug)]
pub(crate) struct Request {
    /// The request method, as sent.
    pub(crate) method: String,
    /// The request target, as sent.
    pub(crate) path: String,
    /// The body, at most the configured cap.
    pub(crate) body: Vec<u8>,
}

/// Why a request was refused before it reached routing.
#[derive(Debug)]
pub(crate) enum Refusal {
    /// The peer closed before sending a byte.
    Closed,
    /// The head was unreadable, oversized or not HTTP.
    Head,
    /// A body without a `Content-Length` (or with chunked framing).
    LengthRequired,
    /// The declared body exceeds the cap; nothing past the head was read.
    TooLarge {
        /// The `Content-Length` the client declared.
        declared: usize,
    },
    /// The socket failed or the client stalled past the read timeout.
    Io,
}

/// Read one request from `stream`, never buffering more than the head limit
/// plus `max_body` bytes.
pub(crate) fn read_request(stream: &mut impl Read, max_body: usize) -> Result<Request, Refusal> {
    let mut head: Vec<u8> = Vec::with_capacity(512);
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_HEAD_BYTES {
            return Err(Refusal::Head);
        }
        match stream.read(&mut byte) {
            Ok(0) if head.is_empty() => return Err(Refusal::Closed),
            Ok(0) => return Err(Refusal::Head),
            Ok(_) => head.extend_from_slice(&byte),
            Err(_) => return Err(Refusal::Io),
        }
    }
    let text = std::str::from_utf8(&head).map_err(|_| Refusal::Head)?;
    let mut lines = text.split("\r\n");
    let mut request_line = lines.next().ok_or(Refusal::Head)?.split_whitespace();
    let method = String::from(request_line.next().ok_or(Refusal::Head)?);
    let path = String::from(request_line.next().ok_or(Refusal::Head)?);
    if !request_line
        .next()
        .is_some_and(|version| version.starts_with("HTTP/"))
    {
        return Err(Refusal::Head);
    }
    let mut declared: Option<usize> = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("transfer-encoding") {
            return Err(Refusal::LengthRequired);
        }
        if name.trim().eq_ignore_ascii_case("content-length") {
            declared = Some(value.trim().parse().map_err(|_| Refusal::Head)?);
        }
    }
    let length = match declared {
        Some(length) => length,
        None if method.eq_ignore_ascii_case("POST") => return Err(Refusal::LengthRequired),
        None => 0,
    };
    if length > max_body {
        return Err(Refusal::TooLarge { declared: length });
    }
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).map_err(|_| Refusal::Io)?;
    Ok(Request { method, path, body })
}

/// The reason phrase for the statuses the mock emits.
fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Unknown",
    }
}

/// Render a response whose head declares `declared_len` body bytes and whose
/// body carries `body`: a `body` shorter than `declared_len` is a truncation.
/// Every response closes its connection.
pub(crate) fn render(
    status: u16,
    headers: &[(&'static str, String)],
    body: &[u8],
    declared_len: usize,
) -> Vec<u8> {
    let mut lines = vec![format!("HTTP/1.1 {status} {}", reason(status))];
    lines.push(String::from("Content-Type: application/json"));
    lines.push(format!("Content-Length: {declared_len}"));
    lines.push(String::from("Connection: close"));
    lines.extend(
        headers
            .iter()
            .map(|header| format!("{}: {}", header.0, header.1)),
    );
    let mut out = lines.join("\r\n").into_bytes();
    out.extend_from_slice(b"\r\n\r\n");
    out.extend_from_slice(body);
    out
}
