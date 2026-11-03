//! Frame builders for the two MCP eras the suites drive `rocco mcp` through.
//!
//! Source: MCP 2026-07-28 Basic, "Per-request protocol fields" (`_meta` keys
//! `io.modelcontextprotocol/protocolVersion`, `clientCapabilities`,
//! `clientInfo`, `logLevel`) and the capability examples in schema.json
//! (`ClientCapabilities`). One seed picks every optional field, so a run replays.

use crate::mcp_support::{Rng, Session, TestResult};
use crate::schema::Schema;
use lgwks_std::json::{self, Value, json};

/// The stateless revision.
pub const MODERN: &str = "2026-07-28";
/// Every revision the server advertises, newest first.
pub const SERVED: [&str; 5] = [
    "2026-07-28",
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];
/// The eight `LoggingLevel` values of schema.json.
pub const LEVELS: [&str; 8] = [
    "debug",
    "info",
    "notice",
    "warning",
    "error",
    "critical",
    "alert",
    "emergency",
];

/// A pick from `pool`, by the seed.
pub fn pick<'a, T>(rng: &mut Rng, pool: &'a [T]) -> TestResult<&'a T> {
    let bound = u32::try_from(pool.len())?;
    pool.get(rng.index(bound)?)
        .ok_or_else(|| "empty pool".into())
}

/// The `_meta` of a stateless request at `version`: capabilities always, the
/// identity, a log level, a progress token and trace context when the seed says so.
pub fn meta_at(rng: &mut Rng, version: &str) -> TestResult<Value> {
    let capabilities = [
        json!({}),
        json!({"roots": {}}),
        json!({"elicitation": {"form": {}, "url": {}}}),
        json!({"sampling": {"tools": {}}, "extensions": {"io.modelcontextprotocol/ui": {}}}),
    ];
    let mut meta = json!({
        "io.modelcontextprotocol/protocolVersion": version,
        "io.modelcontextprotocol/clientCapabilities": pick(rng, &capabilities)?
    });
    if rng.below(2) == 0 {
        meta["io.modelcontextprotocol/clientInfo"] =
            json!({"name": format!("client-{}", rng.below(1000)), "version": "1.0.0"});
    }
    if rng.below(3) == 0 {
        meta["io.modelcontextprotocol/logLevel"] = json!(pick(rng, &LEVELS)?);
    }
    if rng.below(4) == 0 {
        meta["progressToken"] = json!(rng.below(1000));
    }
    if rng.below(4) == 0 {
        meta["traceparent"] = json!("00-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7-01");
    }
    Ok(meta)
}

/// A stateless request frame: `params` plus a seeded `_meta` at `version`.
pub fn frame_at(
    rng: &mut Rng,
    id: &Value,
    method: &str,
    params: Value,
    version: &str,
) -> TestResult<Value> {
    let mut params = params;
    params["_meta"] = meta_at(rng, version)?;
    Ok(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
}

/// Sends `frame`, reads the reply line and checks it against the official
/// schema of the era; returns the raw line and the parsed reply.
pub fn ask(session: &mut Session, schema: &Schema, frame: &Value) -> TestResult<(Vec<u8>, Value)> {
    session.send(&json::to_vec(frame)?)?;
    let line = session.recv_line()?;
    let reply: Value = json::from_slice(&line)?;
    let method = frame["method"].as_str().ok_or("frame has no method")?;
    schema
        .check_reply(method, &reply)
        .map_err(|why| format!("{why}; request {frame}; reply {reply}"))?;
    Ok((line, reply))
}

/// Arguments of a well-formed typed task, its text chosen by the seed.
pub fn task(rng: &mut Rng) -> Value {
    json!({
        "goal": format!("task {}", rng.below(100_000)),
        "repo": "Logical-Works-Inc/larry",
        "files": ["src/a.rs"],
        "acceptance": ["cargo nextest run --locked"]
    })
}
