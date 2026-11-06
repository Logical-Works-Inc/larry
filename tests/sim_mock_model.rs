//! Seeded deterministic simulations of the mock model endpoint (larry#82).
//!
//! Source: the OpenAI chat-completions wire contract the forge client speaks
//! (`srinji-kaggss/forge-harness` `sdk/forge-providers`), HTTP/1.1 (RFC 9110)
//! for the 413, 429 with `Retry-After`, 503 and 411 semantics, and the estate's
//! seeded-simulation rule (AGENTS.md: one seed, same seed same trace hash).
//! Requirement proved: one seed names one byte-identical response sequence and
//! one trace hash over a real TCP endpoint; every fault the model client (#83)
//! must survive is reachable and visible from a plain `TcpStream`; the body and
//! connection bounds refuse instead of growing.

#[path = "support/http_wire.rs"]
mod http_wire;
#[path = "support/mock_model.rs"]
mod mock_model;

use std::error::Error;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use lgwks_std::json::{self, Value};
use mock_model::{Config, Fault};

/// What every test here returns.
type TestResult = Result<(), Box<dyn Error>>;

/// Client read bound for exchanges that must complete.
const PATIENT: Duration = Duration::from_secs(5);
/// Client read bound for the exchange the gate holds back.
const IMPATIENT: Duration = Duration::from_millis(150);

/// One parsed HTTP response.
#[derive(Debug, PartialEq, Eq)]
struct Reply {
    /// Status code.
    status: u16,
    /// Header name and value pairs, in order.
    headers: Vec<(String, String)>,
    /// Body bytes actually received.
    body: Vec<u8>,
}

impl Reply {
    /// The first header named `name`, case-insensitively.
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|header| header.0.eq_ignore_ascii_case(name))
            .map(|header| header.1.as_str())
    }
}

/// Split raw response bytes into a [`Reply`].
fn parse(raw: &[u8]) -> Result<Reply, Box<dyn Error>> {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("response has no head terminator")?;
    let head = std::str::from_utf8(raw.get(..split).ok_or("head slice")?)?;
    let start = split.checked_add(4).ok_or("offset overflow")?;
    let body = raw.get(start..).ok_or("body slice")?.to_vec();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .ok_or("no status line")?
        .split_whitespace()
        .nth(1)
        .ok_or("no status code")?
        .parse()?;
    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or("header without a colon")?;
        headers.push((String::from(name.trim()), String::from(value.trim())));
    }
    Ok(Reply {
        status,
        headers,
        body,
    })
}

/// Send `raw` and read the connection to its end, bounded by `timeout`.
fn exchange_for(addr: SocketAddr, raw: &[u8], timeout: Duration) -> io::Result<Vec<u8>> {
    let mut stream = TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.write_all(raw)?;
    let mut out = Vec::new();
    stream.read_to_end(&mut out)?;
    Ok(out)
}

/// A POST to `path` with a correct `Content-Length`.
fn post_raw(path: &str, body: &[u8]) -> Vec<u8> {
    let mut raw = format!(
        "POST {path} HTTP/1.1\r\nHost: mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    raw.extend_from_slice(body);
    raw
}

/// A chat-completions request body in the shape the forge client sends.
fn chat_body(tag: u32) -> Vec<u8> {
    format!(
        "{{\"model\":\"deepseek-v4-flash\",\"max_tokens\":256,\"temperature\":0.0,\"messages\":[{{\"role\":\"user\",\"content\":\"task {tag}\"}}]}}"
    )
    .into_bytes()
}

/// One chat-completions exchange.
fn chat(addr: SocketAddr, tag: u32) -> Result<Reply, Box<dyn Error>> {
    parse(&exchange_for(
        addr,
        &post_raw("/v1/chat/completions", &chat_body(tag)),
        PATIENT,
    )?)
}

/// Run `count` chat requests against a fresh server for `config`; return every
/// reply and the final trace hash.
fn session(config: Config, count: u32) -> Result<(Vec<Reply>, String), Box<dyn Error>> {
    mock_model::run(config, |handle| {
        let mut replies = Vec::new();
        for tag in 0..count {
            replies.push(chat(handle.addr(), tag)?);
        }
        Ok((replies, handle.trace_hash()))
    })
}

/// The `content` of a good reply.
fn content(reply: &Reply) -> Result<String, Box<dyn Error>> {
    let value: Value = json::from_slice(&reply.body)?;
    Ok(String::from(
        value["choices"][0]["message"]["content"]
            .as_str()
            .ok_or("no content")?,
    ))
}

#[test]
fn sim_same_seed_replays_byte_identical_responses_and_trace() -> TestResult {
    let first = session(Config::seeded(0x5eed_0082), 25)?;
    let second = session(Config::seeded(0x5eed_0082), 25)?;
    assert_eq!(first.0, second.0, "same seed, same requests, same bytes");
    assert_eq!(first.1, second.1, "same seed, same trace hash");
    assert_eq!(
        first.1.len(),
        64,
        "the trace hash is a 32-byte digest in hex"
    );
    assert!(
        first.0.iter().all(|reply| reply.status == 200),
        "a fault-free server answers every request"
    );
    assert!(
        !content(first.0.first().ok_or("no replies")?)?.is_empty(),
        "a reply carries content"
    );
    Ok(())
}

#[test]
fn sim_different_seed_changes_responses_and_trace() -> TestResult {
    let one = session(Config::seeded(1), 10)?;
    let two = session(Config::seeded(2), 10)?;
    assert_ne!(one.1, two.1, "another seed must hash differently");
    assert_ne!(one.0, two.0, "another seed must answer differently");
    Ok(())
}

#[test]
fn sim_the_trace_hash_binds_the_request_sequence() -> TestResult {
    let straight = mock_model::run(Config::seeded(9), |handle| {
        chat(handle.addr(), 1)?;
        chat(handle.addr(), 2)?;
        Ok::<_, Box<dyn Error>>(handle.trace_hash())
    })?;
    let swapped = mock_model::run(Config::seeded(9), |handle| {
        chat(handle.addr(), 2)?;
        chat(handle.addr(), 1)?;
        Ok::<_, Box<dyn Error>>(handle.trace_hash())
    })?;
    assert_ne!(straight, swapped, "request order is part of the trace");
    Ok(())
}

#[test]
fn sim_seeded_faults_replay_and_every_kind_is_reachable() -> TestResult {
    let config = Config {
        fault_per_mille: 600,
        ..Config::seeded(0xfa17_0082)
    };
    let first = session(config.clone(), 200)?;
    let second = session(config, 200)?;
    assert_eq!(first.0, second.0, "drawn faults replay byte for byte");
    assert_eq!(first.1, second.1, "drawn faults replay to one trace hash");
    let mut limited = false;
    let mut failed = false;
    let mut malformed = false;
    let mut truncated = false;
    let mut clean = false;
    for reply in &first.0 {
        let declared: usize = reply.header("content-length").ok_or("no length")?.parse()?;
        match reply.status {
            429 => limited = true,
            500 | 502 | 503 | 504 => failed = true,
            200 if reply.body.len() < declared => truncated = true,
            200 if json::from_slice::<Value>(&reply.body).is_err() => malformed = true,
            200 => clean = true,
            other => return Err(format!("unexpected status {other}").into()),
        }
    }
    assert!(
        limited && failed && malformed && truncated && clean,
        "200 requests at 600 per mille must reach every kind: {limited} {failed} {malformed} {truncated} {clean}"
    );
    Ok(())
}

#[test]
fn sim_scripted_faults_are_visible_to_a_plain_client_and_replay() -> TestResult {
    let script = vec![
        Some(Fault::RateLimit {
            retry_after_secs: 3,
        }),
        Some(Fault::MalformedJson),
        Some(Fault::TruncatedBody),
        Some(Fault::ServerError { status: 503 }),
        None,
    ];
    let config = Config {
        script,
        ..Config::seeded(77)
    };
    let (replies, trace) = session(config.clone(), 5)?;
    assert_eq!(replies.len(), 5, "one reply per request");
    let at = |slot: usize| replies.get(slot).ok_or("missing reply");
    let (limited, malformed, truncated, failed, clean) = (at(0)?, at(1)?, at(2)?, at(3)?, at(4)?);
    assert_eq!(limited.status, 429, "rate limit status");
    assert_eq!(
        limited.header("retry-after"),
        Some("3"),
        "Retry-After header"
    );
    assert_eq!(malformed.status, 200, "malformed body is still a 200");
    assert!(
        json::from_slice::<Value>(&malformed.body).is_err(),
        "malformed body must not parse as JSON"
    );
    let declared: usize = truncated
        .header("content-length")
        .ok_or("no length")?
        .parse()?;
    assert!(
        truncated.body.len() < declared,
        "truncated body ({}) must be shorter than declared ({declared})",
        truncated.body.len()
    );
    assert_eq!(failed.status, 503, "5xx status");
    assert_eq!(clean.status, 200, "the request past the script is clean");
    assert!(!content(clean)?.is_empty(), "the clean reply parses");
    let (_, again) = session(config, 5)?;
    assert_eq!(trace, again, "a scripted run replays to one trace hash");
    Ok(())
}

#[test]
fn sim_a_delayed_reply_waits_on_the_gate_not_the_clock() -> TestResult {
    let config = Config {
        script: vec![Some(Fault::Delay)],
        ..Config::seeded(5)
    };
    mock_model::run(config, |handle| {
        let held = exchange_for(
            handle.addr(),
            &post_raw("/v1/chat/completions", &chat_body(0)),
            IMPATIENT,
        );
        let kind = held.err().map(|failure| failure.kind());
        assert!(
            matches!(
                kind,
                Some(io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
            ),
            "a delayed reply must outlast the client timeout, got {kind:?}"
        );
        handle.release_delayed();
        assert_eq!(
            chat(handle.addr(), 1)?.status,
            200,
            "later requests are clean"
        );
        assert_eq!(handle.served(), 2, "both requests were planned");
        Ok(())
    })
}

#[test]
fn sim_an_oversized_body_is_refused_and_never_planned() -> TestResult {
    let config = Config {
        max_body_bytes: 1024,
        ..Config::seeded(3)
    };
    mock_model::run(config, |handle| {
        let big = vec![b'x'; 4096];
        let full = parse(&exchange_for(
            handle.addr(),
            &post_raw("/v1/chat/completions", &big),
            PATIENT,
        )?)?;
        assert_eq!(full.status, 413, "a body over the cap is refused");
        let head_only = parse(&exchange_for(
            handle.addr(),
            b"POST /v1/chat/completions HTTP/1.1\r\nContent-Length: 10485760\r\n\r\n",
            PATIENT,
        )?)?;
        assert_eq!(
            head_only.status, 413,
            "a huge declared length is refused unread"
        );
        assert_eq!(handle.served(), 0, "refused bodies never reach the ledger");
        assert_eq!(chat(handle.addr(), 0)?.status, 200, "the server survives");
        Ok(())
    })
}

#[test]
fn sim_the_connection_cap_answers_503() -> TestResult {
    let config = Config {
        max_connections: 1,
        script: vec![Some(Fault::Delay)],
        ..Config::seeded(6)
    };
    mock_model::run(config, |handle| {
        let mut holder = TcpStream::connect(handle.addr())?;
        holder.set_read_timeout(Some(PATIENT))?;
        holder.write_all(&post_raw("/v1/chat/completions", &chat_body(0)))?;
        let refused = chat(handle.addr(), 1)?;
        assert_eq!(refused.status, 503, "a second connection past the cap");
        assert_eq!(
            refused.header("retry-after"),
            Some("1"),
            "cap refusal retry hint"
        );
        handle.release_delayed();
        let mut raw = Vec::new();
        holder.read_to_end(&mut raw)?;
        assert_eq!(parse(&raw)?.status, 200, "the held request completes");
        Ok(())
    })
}

#[test]
fn sim_hostile_requests_get_typed_refusals() -> TestResult {
    mock_model::run(Config::seeded(8), |handle| {
        let addr = handle.addr();
        let status = |raw: &[u8]| -> Result<u16, Box<dyn Error>> {
            Ok(parse(&exchange_for(addr, raw, PATIENT)?)?.status)
        };
        assert_eq!(status(&post_raw("/v1/other", b"{}"))?, 404, "unknown route");
        assert_eq!(
            status(b"GET /v1/chat/completions HTTP/1.1\r\n\r\n")?,
            405,
            "wrong method"
        );
        assert_eq!(
            status(&post_raw("/v1/chat/completions", b"{not json"))?,
            400,
            "body that is not JSON"
        );
        assert_eq!(
            status(&post_raw("/v1/chat/completions", b"{\"messages\":[]}"))?,
            400,
            "missing model"
        );
        assert_eq!(
            status(&post_raw(
                "/v1/chat/completions",
                b"{\"model\":\"m\",\"messages\":[]}"
            ))?,
            400,
            "empty messages"
        );
        assert_eq!(
            status(&post_raw(
                "/v1/chat/completions",
                b"{\"model\":\"m\",\"messages\":[{}],\"stream\":true}"
            ))?,
            400,
            "streaming is not offered"
        );
        assert_eq!(
            status(b"POST /v1/chat/completions HTTP/1.1\r\n\r\n")?,
            411,
            "a body without Content-Length"
        );
        assert_eq!(
            status(b"POST /v1/chat/completions HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n")?,
            411,
            "chunked framing is not offered"
        );
        assert_eq!(status(b"not http at all\r\n\r\n")?, 400, "garbage head");
        Ok(())
    })
}

#[test]
fn the_model_and_temperature_a_client_sends_reach_the_endpoint() -> TestResult {
    mock_model::run(Config::seeded(4), |handle| {
        let reply = chat(handle.addr(), 12)?;
        let echoed: Value = json::from_slice(&reply.body)?;
        assert_eq!(
            echoed["model"], "deepseek-v4-flash",
            "the pinned model is echoed"
        );
        let seen = handle.requests();
        let sent: Value = json::from_slice(seen.first().ok_or("no recorded request")?)?;
        assert_eq!(sent["temperature"], 0.0, "temperature reached the gateway");
        assert_eq!(
            sent["model"], "deepseek-v4-flash",
            "model reached the gateway"
        );
        assert_eq!(
            handle.authorizations(),
            [None],
            "a request without a key is recorded as keyless, aligned with its body"
        );
        Ok(())
    })
}
