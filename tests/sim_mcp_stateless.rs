//! Seeded simulations of the stateless 2026-07-28 era against the shipped `rocco mcp`.
//!
//! Source: MCP specification 2026-07-28 (Basic: `_meta` per-request fields and
//! error codes; Versioning and Compatibility; Server: Discovery, Tools,
//! Caching, Logging) and `schema/2026-07-28/schema.json`, vendored under
//! tests/fixtures/mcp and checked reply by reply. Requirement: larry#110, the
//! stdio server serves the stateless revision with no handshake, in any order,
//! with the error codes the schema allocates. One seed drives every choice; the
//! same seed must give the same reply bytes. Every test drives the binary
//! through its public interface; nothing here calls server code.

#[path = "mcp_support/eras.rs"]
mod eras;
mod mcp_support;
#[path = "mcp_support/schema.rs"]
mod schema;

use eras::{LEVELS, MODERN, SERVED, ask, frame_at, meta_at, pick, task};
use lgwks_std::json::{Value, json};
use mcp_support::{Rng, Session, TestResult};
use schema::Schema;

/// A fresh request id, an integer or a string, chosen by the seed.
fn pick_id(rng: &mut Rng, index: usize) -> Value {
    if rng.below(2) == 0 {
        json!(index)
    } else {
        json!(format!("r{}", rng.next()))
    }
}

/// The reply to `id` is a complete 2026-07-28 result naming this server.
fn assert_complete(reply: &Value, id: &Value, context: &str) {
    assert_eq!(&reply["id"], id, "{context}: id");
    let result = &reply["result"];
    assert_eq!(result["resultType"], "complete", "{context}: resultType");
    let info = &result["_meta"]["io.modelcontextprotocol/serverInfo"];
    assert_eq!(info["name"], "larry", "{context}: serverInfo name");
    assert_eq!(
        info["version"],
        env!("CARGO_PKG_VERSION"),
        "{context}: serverInfo version"
    );
}

/// What one generated request must draw.
#[derive(Clone, Copy)]
enum Want {
    /// `server/discover` result.
    Discover,
    /// `tools/list` result.
    List,
    /// A tool result with `isError` true and this typed code.
    Refused(&'static str),
    /// A JSON-RPC error with this code.
    Error(i64),
}

/// One seeded stateless session with no initialize, requests in any order.
/// Returns every reply line, the session's trace.
fn run_any_order(seed: u64, schema: &Schema) -> TestResult<Vec<Vec<u8>>> {
    let mut rng = Rng(seed);
    let mut session = Session::start()?;
    let mut trace = Vec::new();
    for index in 0..16 {
        let id = pick_id(&mut rng, index);
        let (method, params, want) = match rng.below(5) {
            0 => ("server/discover", json!({}), Want::Discover),
            1 => ("tools/list", json!({}), Want::List),
            2 => (
                "tools/call",
                json!({"name": "larry_submit_task", "arguments": task(&mut rng)}),
                Want::Refused("executor_not_connected"),
            ),
            3 => (
                "tools/call",
                json!({"name": "larry_submit_task", "arguments": {"goal": "x"}}),
                Want::Refused("invalid_arguments"),
            ),
            _ => (
                "tools/call",
                json!({"name": "nope", "arguments": {}}),
                Want::Error(-32602),
            ),
        };
        let frame = frame_at(&mut rng, &id, method, params, MODERN)?;
        let (line, reply) = ask(&mut session, schema, &frame)?;
        let context = format!("seed {seed} step {index} {method}");
        match want {
            Want::Discover => {
                assert_complete(&reply, &id, &context);
                assert_eq!(
                    reply["result"]["supportedVersions"],
                    json!(SERVED),
                    "{context}: versions"
                );
                assert_eq!(
                    reply["result"]["capabilities"]["tools"]["listChanged"], false,
                    "{context}: capabilities"
                );
            }
            Want::List => {
                assert_complete(&reply, &id, &context);
                assert_eq!(
                    reply["result"]["tools"][0]["name"], "larry_submit_task",
                    "{context}: tool"
                );
                assert_eq!(reply["result"]["cacheScope"], "public", "{context}: scope");
                assert!(reply["result"]["ttlMs"].is_u64(), "{context}: ttlMs");
            }
            Want::Refused(code) => {
                assert_complete(&reply, &id, &context);
                assert_eq!(reply["result"]["isError"], true, "{context}: isError");
                assert_eq!(
                    reply["result"]["structuredContent"]["error"]["code"], code,
                    "{context}: code"
                );
                assert!(
                    reply["result"]["ttlMs"].is_null(),
                    "{context}: a tool result is not cacheable"
                );
            }
            Want::Error(code) => {
                assert_eq!(reply["error"]["code"], code, "{context}: error code");
                assert_eq!(reply["id"], id, "{context}: error id");
            }
        }
        trace.push(line);
    }
    assert_eq!(session.finish()?.code(), Some(0), "seed {seed}: clean exit");
    Ok(trace)
}

/// Any order, no prior call, every reply inside the schema, same seed same bytes.
#[test]
fn sim_stateless_requests_in_any_order_need_no_prior_call() -> TestResult<()> {
    let schema = Schema::vendored(MODERN)?;
    let mut firsts = Vec::new();
    for seed in 0..24 {
        let first = run_any_order(seed, &schema)?;
        assert_eq!(
            first,
            run_any_order(seed, &schema)?,
            "seed {seed}: replay diverged"
        );
        firsts.push(first);
    }
    firsts.sort_unstable();
    firsts.dedup();
    assert!(firsts.len() > 20, "seeds explore distinct traces");
    Ok(())
}

/// `server/discover` first: the advertised versions are the served ones, and a
/// request at the version it names is answered at once.
#[test]
fn sim_discover_first_then_a_request_at_a_listed_version() -> TestResult<()> {
    let schema = Schema::vendored(MODERN)?;
    for seed in 0..8 {
        let mut rng = Rng(seed);
        let mut session = Session::start()?;
        let frame = frame_at(&mut rng, &json!("d"), "server/discover", json!({}), MODERN)?;
        let (_, found) = ask(&mut session, &schema, &frame)?;
        let listed: Vec<&str> = found["result"]["supportedVersions"]
            .as_array()
            .ok_or("supportedVersions")?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(listed, SERVED, "seed {seed}: versions");
        assert!(
            found["result"]["ttlMs"].as_u64().is_some_and(|ttl| ttl > 0),
            "seed {seed}: a static server is cacheable"
        );
        let chosen = listed.first().ok_or("no versions")?;
        let call = frame_at(&mut rng, &json!(1), "tools/list", json!({}), chosen)?;
        let (_, list) = ask(&mut session, &schema, &call)?;
        assert_complete(&list, &json!(1), &format!("seed {seed}"));
    }
    Ok(())
}

/// A version the server does not serve per request draws -32022 with the full
/// supported list and the requested version echoed, whatever the method.
#[test]
fn sim_version_mismatch_returns_unsupported_protocol_version() -> TestResult<()> {
    let schema = Schema::vendored(MODERN)?;
    let pool = [
        "1900-01-01",
        "2025-11-25",
        "2025-06-18",
        "2027-01-01",
        "",
        "latest",
        "2026-07-28 ",
        "2026-07-29",
    ];
    let methods = ["tools/list", "tools/call", "server/discover", "ping", "x/y"];
    let mut rng = Rng(0x2275);
    let mut session = Session::start()?;
    for index in 0..40 {
        let asked = *pick(&mut rng, &pool)?;
        let method = *pick(&mut rng, &methods)?;
        let frame = frame_at(&mut rng, &json!(index), method, json!({}), asked)?;
        let (_, reply) = ask(&mut session, &schema, &frame)?;
        assert_eq!(reply["error"]["code"], -32022, "{asked:?} on {method}");
        assert_eq!(reply["error"]["data"]["requested"], asked, "echoed");
        assert_eq!(reply["error"]["data"]["supported"], json!(SERVED), "list");
    }
    let mut wrong_type = frame_at(&mut rng, &json!("t"), "tools/list", json!({}), MODERN)?;
    wrong_type["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!(20260728);
    let (_, reply) = ask(&mut session, &schema, &wrong_type)?;
    assert_eq!(
        reply["error"]["code"], -32602,
        "a numeric version is malformed"
    );
    Ok(())
}

/// A request missing a required `_meta` field, or carrying a malformed one, is
/// -32602 and the session serves the next request; an unknown method stays -32601.
#[test]
fn sim_missing_or_malformed_meta_is_invalid_params() -> TestResult<()> {
    let schema = Schema::vendored(MODERN)?;
    let version = "io.modelcontextprotocol/protocolVersion";
    let caps = "io.modelcontextprotocol/clientCapabilities";
    let bad_metas = [
        json!({}),
        json!({"progressToken": 1}),
        json!({ version: MODERN }),
        json!({ version: MODERN, caps: [] }),
        json!({ version: 5, caps: {} }),
        json!({ version: MODERN, caps: {}, "io.modelcontextprotocol/clientInfo": {"name": "n"} }),
        json!({ version: MODERN, caps: {}, "io.modelcontextprotocol/logLevel": "LOUD" }),
        json!({ version: MODERN, caps: {}, "io.modelcontextprotocol/logLevel": null }),
        json!("not an object"),
    ];
    let methods = ["tools/list", "tools/call", "server/discover"];
    let mut rng = Rng(0x3e7a);
    let mut session = Session::start()?;
    for index in 0..36 {
        let method = *pick(&mut rng, &methods)?;
        let mut params = json!({"name": "larry_submit_task"});
        params["_meta"] = pick(&mut rng, &bad_metas)?.clone();
        let frame = json!({"jsonrpc": "2.0", "id": index, "method": method, "params": params});
        let (_, reply) = ask(&mut session, &schema, &frame)?;
        assert_eq!(
            reply["error"]["code"], -32602,
            "{method} with {}",
            frame["params"]
        );
    }
    for bare in [
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {}}),
    ] {
        let (_, reply) = ask(&mut session, &schema, &bare)?;
        assert_eq!(
            reply["error"]["code"], -32602,
            "no params, no session: {bare}"
        );
    }
    let unknown = json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"});
    let (_, reply) = ask(&mut session, &schema, &unknown)?;
    assert_eq!(
        reply["error"]["code"], -32601,
        "an unknown method is not found"
    );
    let good = frame_at(&mut rng, &json!(4), "tools/list", json!({}), MODERN)?;
    let (_, reply) = ask(&mut session, &schema, &good)?;
    assert_complete(&reply, &json!(4), "after the refusals");
    Ok(())
}

/// `ping` and `logging/setLevel` are removed on 2026-07-28 and so are every
/// other method this server never had: all are -32601.
#[test]
fn sim_removed_methods_are_method_not_found() -> TestResult<()> {
    let schema = Schema::vendored(MODERN)?;
    let methods = [
        "ping",
        "logging/setLevel",
        "resources/list",
        "prompts/list",
        "roots/list",
        "notifications/roots/list_changed",
        "initialize/again",
    ];
    let mut rng = Rng(0x7e11);
    let mut session = Session::start()?;
    for index in 0..30 {
        let method = *pick(&mut rng, &methods)?;
        let params = json!({"level": *pick(&mut rng, &LEVELS)?});
        let frame = frame_at(&mut rng, &json!(index), method, params, MODERN)?;
        let (_, reply) = ask(&mut session, &schema, &frame)?;
        assert_eq!(reply["error"]["code"], -32601, "{method} is not found");
    }
    Ok(())
}

/// The per-request log level is validated and honoured: no `notifications/message`
/// is ever written, so a run of requests with and without a level is answered one
/// line each, in order, and a barrier line is the last thing on the wire.
#[test]
fn sim_log_level_is_honoured_and_no_message_is_sent_unasked() -> TestResult<()> {
    let schema = Schema::vendored(MODERN)?;
    for seed in 0..8 {
        let mut rng = Rng(seed);
        let mut session = Session::start()?;
        let count = rng.index(20)?.saturating_add(8);
        let mut ids = Vec::new();
        for index in 0..count {
            let id = json!(format!("l{seed}-{index}"));
            let mut frame = frame_at(&mut rng, &id, "tools/list", json!({}), MODERN)?;
            if rng.below(2) == 0 {
                frame["params"]["_meta"]["io.modelcontextprotocol/logLevel"] =
                    json!(*pick(&mut rng, &LEVELS)?);
            } else if let Some(meta) = frame["params"]["_meta"].as_object_mut() {
                meta.remove("io.modelcontextprotocol/logLevel");
            }
            session.send(&lgwks_std::json::to_vec(&frame)?)?;
            ids.push(id);
        }
        for id in &ids {
            let reply = session.recv()?;
            assert!(
                reply.get("method").is_none(),
                "seed {seed}: a notification leaked"
            );
            assert_eq!(
                &reply["id"], id,
                "seed {seed}: one reply per request, in order"
            );
            schema.check_reply("tools/list", &reply)?;
        }
        let barrier = frame_at(
            &mut rng,
            &json!("barrier"),
            "server/discover",
            json!({}),
            MODERN,
        )?;
        let (_, last) = ask(&mut session, &schema, &barrier)?;
        assert_eq!(
            last["id"], "barrier",
            "seed {seed}: nothing was queued behind"
        );
    }
    Ok(())
}

/// The tool list is byte-identical across requests, sessions and client
/// metadata, carries valid cache hints, and its schema is plain JSON Schema 2020-12.
#[test]
fn sim_tool_list_is_deterministic_cacheable_and_valid_2020_12() -> TestResult<()> {
    let schema = Schema::vendored(MODERN)?;
    let mut reference: Option<Value> = None;
    for seed in 0..12 {
        let mut rng = Rng(seed);
        let mut session = Session::start()?;
        for index in 0..4 {
            let frame = frame_at(&mut rng, &json!(index), "tools/list", json!({}), MODERN)?;
            let (_, reply) = ask(&mut session, &schema, &frame)?;
            let tools = reply["result"]["tools"].clone();
            let first = reference.get_or_insert_with(|| tools.clone());
            assert_eq!(
                &tools, first,
                "seed {seed}: the list varied with the request"
            );
            assert!(
                reply["result"]["ttlMs"].as_u64().is_some(),
                "seed {seed}: ttlMs is a non-negative integer"
            );
            assert!(
                matches!(
                    reply["result"]["cacheScope"].as_str(),
                    Some("public" | "private")
                ),
                "seed {seed}: cacheScope is public or private"
            );
        }
    }
    let tools = reference.ok_or("no list")?;
    let listed = tools.as_array().ok_or("tools is an array")?;
    let names: Vec<&str> = listed
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted, "name order");
    for tool in listed {
        check_dialect(&tool["inputSchema"], "inputSchema")?;
        assert!(
            tool["inputSchema"]["$schema"].is_null()
                || tool["inputSchema"]["$schema"] == "https://json-schema.org/draft/2020-12/schema",
            "the dialect is the default or 2020-12 stated"
        );
    }
    Ok(())
}

/// Walks a tool schema: every keyword is a JSON Schema 2020-12 one the server
/// uses, types are valid names, `required` names declared properties and bounds
/// are consistent integers.
fn check_dialect(node: &Value, at: &str) -> TestResult<()> {
    let known = [
        "type",
        "properties",
        "required",
        "additionalProperties",
        "items",
        "enum",
        "const",
        "minLength",
        "maxLength",
        "minItems",
        "maxItems",
        "minimum",
        "maximum",
        "description",
        "title",
        "$schema",
    ];
    let object = node
        .as_object()
        .ok_or_else(|| format!("{at}: not an object"))?;
    for key in object.keys() {
        assert!(
            known.contains(&key.as_str()),
            "{at}: {key} is not a keyword used here"
        );
    }
    if let Some(kind) = node.get("type") {
        let valid = [
            "string", "number", "integer", "boolean", "object", "array", "null",
        ];
        assert!(
            kind.as_str().is_some_and(|name| valid.contains(&name)),
            "{at}: type {kind}"
        );
    }
    let declared = node.get("properties").and_then(Value::as_object);
    for name in node
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let key = name.as_str().ok_or("required entry is not a string")?;
        assert!(
            declared.is_some_and(|map| map.contains_key(key)),
            "{at}: required {key} undeclared"
        );
    }
    for pair in [("minLength", "maxLength"), ("minItems", "maxItems")] {
        if let (Some(low), Some(high)) = (node.get(pair.0), node.get(pair.1)) {
            assert!(
                low.as_u64() <= high.as_u64(),
                "{at}: {} above {}",
                pair.0,
                pair.1
            );
        }
    }
    for (name, sub) in declared.into_iter().flatten() {
        check_dialect(sub, &format!("{at}.{name}"))?;
    }
    if let Some(items) = node.get("items") {
        check_dialect(items, &format!("{at}[]"))?;
    }
    Ok(())
}

/// Statelessness: kill -9 and restart, and the very same request bytes draw the
/// very same reply bytes with no handshake in between.
#[test]
fn sim_kill_nine_then_restart_answers_the_same_request_identically() -> TestResult<()> {
    let schema = Schema::vendored(MODERN)?;
    for seed in 0..6 {
        let mut rng = Rng(seed);
        let frame = frame_at(&mut rng, &json!(seed), "tools/list", json!({}), MODERN)?;
        let mut first = Session::start()?;
        let (before, _) = ask(&mut first, &schema, &frame)?;
        assert!(
            first.kill()?.code().is_none(),
            "seed {seed}: killed by signal"
        );
        let mut second = Session::start()?;
        let (after, _) = ask(&mut second, &schema, &frame)?;
        assert_eq!(
            before, after,
            "seed {seed}: the restart answers byte for byte"
        );
    }
    Ok(())
}

/// A legacy `initialize` on the same process does not make the stateless era
/// stateful: the next `_meta` request is answered exactly as before it, and a
/// removed method stays removed (2026-07-28 Versioning, a dual-era server).
#[test]
fn sim_a_legacy_session_leaves_stateless_requests_stateless() -> TestResult<()> {
    let schema = Schema::vendored(MODERN)?;
    for seed in 0..6 {
        let mut rng = Rng(seed);
        let mut session = Session::start()?;
        let list = frame_at(&mut rng, &json!("b"), "tools/list", json!({}), MODERN)?;
        let (_, before) = ask(&mut session, &schema, &list)?;
        let init = session.initialize()?;
        assert!(
            init["result"]["protocolVersion"].is_string(),
            "seed {seed}: initialize answered"
        );
        let (_, after) = ask(&mut session, &schema, &list)?;
        assert_eq!(
            before["result"], after["result"],
            "seed {seed}: initialize changed a stateless answer"
        );
        let ping = frame_at(&mut rng, &json!("p"), "ping", json!({}), MODERN)?;
        let refused = session.request(&ping)?;
        schema.check_reply("ping", &refused)?;
        assert_eq!(
            refused["error"]["code"], -32601,
            "seed {seed}: ping with _meta stays removed"
        );
    }
    Ok(())
}

/// 100 stateless sessions at once with no handshake: every reply reaches its own
/// session and validates, and kill -9 of a tenth disturbs no other. The host
/// allows far more descriptors than 100 sessions need; 100 is the level reached.
#[test]
fn sim_one_hundred_stateless_sessions_stay_isolated() -> TestResult<()> {
    const SESSIONS: usize = 100;
    let schema = Schema::vendored(MODERN)?;
    let mut rng = Rng(0x1100);
    let mut sessions = Vec::with_capacity(SESSIONS);
    for _ in 0..SESSIONS {
        sessions.push(Session::start()?);
    }
    for round in 0..6 {
        let mut asked = Vec::with_capacity(SESSIONS);
        for (index, session) in sessions.iter_mut().enumerate() {
            let id = json!(index.saturating_mul(1000).saturating_add(round));
            let method = if rng.below(2) == 0 {
                "tools/list"
            } else {
                "server/discover"
            };
            let meta = meta_at(&mut rng, MODERN)?;
            let frame = json!({"jsonrpc": "2.0", "id": id, "method": method,
                "params": {"_meta": meta}});
            session.send(&lgwks_std::json::to_vec(&frame)?)?;
            asked.push((id, method));
        }
        for (session, (id, method)) in sessions.iter_mut().zip(asked) {
            let reply = session.recv()?;
            assert_eq!(reply["id"], id, "round {round}: reply crossed sessions");
            schema.check_reply(method, &reply)?;
        }
    }
    for (index, session) in sessions.into_iter().enumerate() {
        if index % 10 == 0 {
            assert!(
                session.kill()?.code().is_none(),
                "session {index}: killed by signal"
            );
        } else {
            assert_eq!(
                session.finish()?.code(),
                Some(0),
                "session {index}: clean exit"
            );
        }
    }
    Ok(())
}
