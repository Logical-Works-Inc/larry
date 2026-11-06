//! Seeded simulations of backward compatibility and schema conformance for `rocco mcp`.
//!
//! Source: MCP specification 2026-07-28, Versioning and Compatibility
//! ("Backward Compatibility with Initialization-Based Versions", the
//! compatibility matrix: a dual-era server answers `initialize` with the legacy
//! lifecycle and serves per-request `_meta` statelessly, concurrently), the
//! 2025-11-25, 2025-06-18 and 2025-03-26 Lifecycle pages, and the official
//! schema.json of 2026-07-28 and 2025-11-25 vendored under tests/fixtures/mcp.
//! Requirement: larry#110, today's clients keep working and every reply the
//! server emits is valid for its era. One seed drives each run; the same seed
//! must give the same reply bytes.

#[path = "mcp_support/eras.rs"]
mod eras;
mod mcp_support;
#[path = "mcp_support/schema.rs"]
mod schema;

use eras::{LEVELS, MODERN, SERVED, ask, frame_at, meta_at, pick, task};
use lgwks_std::json::{self, Value, json};
use mcp_support::{Rng, Session, TestResult};
use schema::Schema;

/// The legacy schema tag every legacy revision is checked against.
const LEGACY: &str = "2025-11-25";
/// The server's frame bound; the oversized frames are built from it.
const MAX_FRAME: usize = 1024 * 1024;

/// Opens a legacy session at `revision` and returns the initialize reply.
fn open(session: &mut Session, legacy: &Schema, revision: &str) -> TestResult<Value> {
    let frame = json!({"jsonrpc": "2.0", "id": "init", "method": "initialize",
        "params": {"protocolVersion": revision, "capabilities": {},
            "clientInfo": {"name": "old-client", "version": "1"}}});
    let (_, reply) = ask(session, legacy, &frame)?;
    session.send(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)?;
    Ok(reply)
}

/// A legacy request: no `_meta`, the shape every pre-2026 client sends.
fn legacy_frame(id: &Value, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

/// A client that opens with `initialize` at each served legacy revision gets
/// that revision back and the legacy lifecycle: same tools, legacy result
/// shapes (no `resultType`, no cache hints), `ping` still answered.
#[test]
fn sim_old_client_initialize_fallback_serves_every_legacy_revision() -> TestResult<()> {
    let legacy = Schema::vendored(LEGACY)?;
    for revision in &SERVED[1..] {
        let mut rng = Rng(0x01d);
        let mut session = Session::start()?;
        let init = open(&mut session, &legacy, revision)?;
        assert_eq!(
            init["result"]["protocolVersion"], *revision,
            "{revision} echoed"
        );
        assert_eq!(
            init["result"]["serverInfo"]["name"], "larry",
            "{revision}: identity"
        );
        let list = legacy_frame(&json!(1), "tools/list", json!({}));
        let (_, listed) = ask(&mut session, &legacy, &list)?;
        assert_eq!(
            listed["result"]["tools"][0]["name"], "larry_submit_task",
            "{revision}: tool"
        );
        assert!(
            listed["result"]["resultType"].is_null(),
            "{revision}: legacy shape"
        );
        assert!(
            listed["result"]["ttlMs"].is_null(),
            "{revision}: no cache hints"
        );
        let call = legacy_frame(
            &json!(2),
            "tools/call",
            json!({"name": "larry_submit_task", "arguments": task(&mut rng)}),
        );
        let (_, called) = ask(&mut session, &legacy, &call)?;
        assert_eq!(
            called["result"]["structuredContent"]["error"]["code"], "executor_not_connected",
            "{revision}: the typed refusal survives"
        );
        assert!(
            called["result"]["resultType"].is_null(),
            "{revision}: legacy call shape"
        );
        let ping = legacy_frame(&json!(3), "ping", json!({}));
        let pong = session.request(&ping)?;
        legacy.check_reply("ping", &pong)?;
        assert_eq!(pong["result"], json!({}), "{revision}: ping");
        let discover = legacy_frame(&json!(4), "server/discover", json!({}));
        let (_, absent) = ask(&mut session, &legacy, &discover)?;
        assert_eq!(
            absent["error"]["code"], -32601,
            "{revision}: no discover without _meta"
        );
        assert_eq!(session.finish()?.code(), Some(0), "{revision}: clean exit");
    }
    Ok(())
}

/// One process serves both eras at once (2026-07-28 Versioning): before
/// `initialize` only stateless requests work; after it both do, each in its own
/// shape, in any order, and the era is chosen by how each request opens.
#[test]
fn sim_both_eras_interleave_on_one_process() -> TestResult<()> {
    let (modern, legacy) = (Schema::vendored(MODERN)?, Schema::vendored(LEGACY)?);
    for seed in 0..8 {
        let mut rng = Rng(seed);
        let mut session = Session::start()?;
        let opened_at = rng.index(6)?;
        for index in 0..14 {
            if index == opened_at {
                let init = session.initialize()?;
                legacy.check_reply("initialize", &init)?;
            }
            let id = json!(index);
            if rng.below(2) == 0 {
                let frame = frame_at(&mut rng, &id, "tools/list", json!({}), MODERN)?;
                let (_, reply) = ask(&mut session, &modern, &frame)?;
                assert_eq!(
                    reply["result"]["resultType"], "complete",
                    "seed {seed}: stateless"
                );
                assert!(
                    reply["result"]["ttlMs"].as_u64().is_some(),
                    "seed {seed}: cache hint"
                );
            } else if index >= opened_at {
                let list = legacy_frame(&id, "tools/list", json!({}));
                let (_, reply) = ask(&mut session, &legacy, &list)?;
                assert!(
                    reply["result"]["tools"].as_array().is_some(),
                    "seed {seed}: legacy list"
                );
                assert!(
                    reply["result"]["resultType"].is_null(),
                    "seed {seed}: legacy shape"
                );
            }
        }
        if seed & 1 == 0 {
            assert_eq!(session.finish()?.code(), Some(0), "seed {seed}: clean exit");
        } else {
            let status = session.kill()?;
            assert!(status.code().is_none(), "seed {seed}: kill -9 mid-session");
        }
    }
    Ok(())
}

/// One generated frame and the era whose schema its reply must satisfy.
struct Step {
    /// The frame bytes, without the newline.
    bytes: Vec<u8>,
    /// The request method, or empty for a frame that is not a request.
    method: &'static str,
    /// Whether the reply is a legacy-era shape.
    legacy: bool,
    /// Whether no reply is due.
    silent: bool,
}

impl Step {
    /// A request whose reply is due, in the legacy or stateless era.
    fn request(frame: &Value, method: &'static str, legacy: bool) -> TestResult<Self> {
        Ok(Self {
            bytes: json::to_vec(frame)?,
            method,
            legacy,
            silent: false,
        })
    }

    /// A frame that is not a request: garbage, a batch or an oversized line.
    fn raw(bytes: Vec<u8>, silent: bool) -> Self {
        Self {
            bytes,
            method: "",
            legacy: false,
            silent,
        }
    }
}

/// Builds frame `index` of seed `seed`'s mixed-era run.
fn make_step(rng: &mut Rng, index: usize, seed: u64) -> TestResult<Step> {
    let id = json!(index);
    let bad_version = *pick(rng, &["2025-11-25", "1900-01-01", "2027-01-01"])?;
    let valid = json!({"name": "larry_submit_task", "arguments": task(rng)});
    let invalid = json!({"name": "larry_submit_task", "arguments": {"goal": 1}});
    let level = json!({"level": *pick(rng, &LEVELS)?});
    let initialize = json!({"protocolVersion": *pick(rng, &SERVED)?});
    let (frame, method, legacy) = match rng.below(15) {
        0 => (
            frame_at(rng, &id, "server/discover", json!({}), MODERN)?,
            "server/discover",
            false,
        ),
        1 | 2 => (
            frame_at(rng, &id, "tools/list", json!({}), MODERN)?,
            "tools/list",
            false,
        ),
        3 => (
            frame_at(rng, &id, "tools/call", valid, MODERN)?,
            "tools/call",
            false,
        ),
        4 => (
            frame_at(rng, &id, "tools/call", invalid, MODERN)?,
            "tools/call",
            false,
        ),
        5 => (
            frame_at(rng, &id, "tools/list", json!({}), bad_version)?,
            "tools/list",
            false,
        ),
        6 => (
            legacy_frame(&id, "tools/list", json!({})),
            "tools/list",
            true,
        ),
        7 => (
            frame_at(rng, &id, "ping", json!({}), MODERN)?,
            "ping",
            false,
        ),
        8 => (
            legacy_frame(&id, "initialize", initialize),
            "initialize",
            true,
        ),
        9 => (legacy_frame(&id, "ping", json!({})), "ping", true),
        10 => (legacy_frame(&id, "tools/call", valid), "tools/call", true),
        11 => return Ok(Step::raw(b"{not json".to_vec(), false)),
        12 => {
            return Ok(Step::raw(
                br#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#.to_vec(),
                false,
            ));
        }
        13 => {
            let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
            return Ok(Step::raw(json::to_vec(&note)?, true));
        }
        _ if index == 17 && seed & 7 == 0 => {
            let size = MAX_FRAME.saturating_add(1).saturating_add(rng.index(512)?);
            return Ok(Step::raw(vec![b'a'; size], false));
        }
        _ => (
            frame_at(rng, &id, "logging/setLevel", level, MODERN)?,
            "logging/setLevel",
            false,
        ),
    };
    Step::request(&frame, method, legacy)
}

/// One mixed-era seeded session; every reply is checked against the schema of
/// its era and the shape of that era. Returns the reply lines.
fn run_mixed(seed: u64, modern: &Schema, legacy: &Schema) -> TestResult<Vec<Vec<u8>>> {
    let mut rng = Rng(seed);
    let mut session = Session::start()?;
    let mut trace = Vec::new();
    for index in 0..48 {
        let step = make_step(&mut rng, index, seed)?;
        session.send(&step.bytes)?;
        if step.silent {
            continue;
        }
        let line = session.recv_line()?;
        let reply: Value = json::from_slice(&line)?;
        let context = format!("seed {seed} step {index} {}", step.method);
        let schemas: &[&Schema] = if step.method.is_empty() {
            &[modern, legacy]
        } else if step.legacy {
            &[legacy]
        } else {
            &[modern]
        };
        for schema in schemas {
            schema
                .check_reply(step.method, &reply)
                .map_err(|why| format!("{context}: {why}; {reply}"))?;
        }
        if reply["result"].as_object().is_some() && step.method.starts_with("tools/") {
            assert_eq!(
                reply["result"]["resultType"].is_null(),
                step.legacy,
                "{context}: the shape follows the era of the request"
            );
        }
        trace.push(line);
    }
    session.send(br#"{"jsonrpc":"2.0","id":"barrier","method":"ping"}"#)?;
    assert_eq!(
        session.recv()?["id"],
        "barrier",
        "seed {seed}: no stray line was queued"
    );
    assert_eq!(session.finish()?.code(), Some(0), "seed {seed}: clean exit");
    Ok(trace)
}

/// Every reply of a seeded mixed-era run, malformed and oversized frames
/// included, validates against the official schema.json of its era, uses only
/// the allocated error codes, and replays byte for byte.
#[test]
fn sim_every_reply_validates_against_the_official_schema_for_its_era() -> TestResult<()> {
    let (modern, legacy) = (Schema::vendored(MODERN)?, Schema::vendored(LEGACY)?);
    for seed in 0..20 {
        let first = run_mixed(seed, &modern, &legacy)?;
        assert_eq!(
            first,
            run_mixed(seed, &modern, &legacy)?,
            "seed {seed}: replay diverged"
        );
        assert!(
            first.len() > 10,
            "seed {seed}: the run exercised the server"
        );
    }
    Ok(())
}

/// A pristine reply plus the corruption table for the validator check.
type Corruption<'a> = (&'a str, &'a Value, &'a str, Option<Value>);

/// The validator itself: a pristine reply passes and each seeded corruption of
/// it (a required field gone, a type or enum broken, a code off the allocation,
/// a null id) is refused, so a conformant run proves something.
#[test]
fn sim_the_schema_check_refuses_a_broken_reply() -> TestResult<()> {
    let (modern, legacy) = (Schema::vendored(MODERN)?, Schema::vendored(LEGACY)?);
    let mut rng = Rng(0xbad);
    let mut session = Session::start()?;
    let params = json!({"name": "larry_submit_task", "arguments": {"goal": "g"}});
    let mut get = |method: &str, version: &str| -> TestResult<Value> {
        let frame = frame_at(&mut rng, &json!(1), method, params.clone(), version)?;
        Ok(ask(&mut session, &modern, &frame)?.1)
    };
    let list = get("tools/list", MODERN)?;
    let discover = get("server/discover", MODERN)?;
    let call = get("tools/call", MODERN)?;
    let mismatch = get("tools/list", "1900-01-01")?;
    let cases: [Corruption; 15] = [
        ("tools/list", &list, "/result/resultType", None),
        ("tools/list", &list, "/result/ttlMs", Some(json!(-1))),
        ("tools/list", &list, "/result/ttlMs", Some(json!("1"))),
        (
            "tools/list",
            &list,
            "/result/cacheScope",
            Some(json!("world")),
        ),
        ("tools/list", &list, "/result/tools", None),
        ("tools/list", &list, "/result/tools/0/name", None),
        (
            "tools/list",
            &list,
            "/result/tools/0/inputSchema/type",
            Some(json!("array")),
        ),
        ("tools/list", &list, "/id", Some(Value::Null)),
        ("tools/list", &list, "/jsonrpc", Some(json!("1.0"))),
        (
            "server/discover",
            &discover,
            "/result/supportedVersions",
            Some(json!("2026-07-28")),
        ),
        ("server/discover", &discover, "/result/capabilities", None),
        ("tools/call", &call, "/result/content", None),
        ("tools/call", &call, "/result/isError", Some(json!("yes"))),
        ("tools/list", &mismatch, "/error/data/supported", None),
        ("tools/list", &mismatch, "/error/code", Some(json!(-32023))),
    ];
    let mut shuffle = Rng(0xbad);
    let start = shuffle.index(15)?;
    for offset in 0..cases.len() {
        let at = offset.saturating_add(start) % cases.len();
        let case = cases.get(at).ok_or("case index")?;
        let (method, pointer) = (case.0, case.2);
        modern.check_reply(method, case.1)?;
        let mut broken = case.1.clone();
        let (parent, key) = pointer.rsplit_once('/').ok_or("pointer")?;
        let holder = pointer_mut(&mut broken, parent).ok_or("parent")?;
        let members = holder.as_object_mut().ok_or("object")?;
        match case.3.as_ref() {
            Some(value) => {
                members.insert(key, value.clone());
            }
            None => {
                members.remove(key);
            }
        }
        assert!(
            modern.check_reply(method, &broken).is_err(),
            "{method} {pointer} was accepted"
        );
    }
    let bad_legacy = json!({"jsonrpc": "2.0", "id": 1, "result": {"tools": [{"name": 5}]}});
    assert!(
        legacy.check_reply("tools/list", &bad_legacy).is_err(),
        "a legacy tool needs a string name"
    );
    let meta = meta_at(&mut shuffle, MODERN)?;
    assert!(
        meta.as_object().is_some(),
        "the builder returns a _meta object"
    );
    Ok(())
}

/// Follows a `/`-separated JSON Pointer mutably, `~1` and `~0` unescaped.
fn pointer_mut<'a>(root: &'a mut Value, path: &str) -> Option<&'a mut Value> {
    if path.is_empty() {
        return Some(root);
    }
    path.strip_prefix('/')?
        .split('/')
        .try_fold(root, |node, segment| {
            let token = segment.replace("~1", "/").replace("~0", "~");
            let is_array = matches!(*node, Value::Array(_));
            if is_array {
                node.get_mut(token.parse::<usize>().ok()?)
            } else {
                node.get_mut(token.as_str())
            }
        })
}
