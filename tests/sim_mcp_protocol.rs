//! Seeded protocol simulations against the shipped `rocco mcp` binary.
//!
//! Source: JSON-RPC 2.0 section 5 (error codes, notifications are never
//! answered), MCP specification 2025-06-18 (lifecycle, version negotiation,
//! tools, stdio framing) and the incident class of oversized or malformed
//! stdio messages killing an MCP server (modelcontextprotocol/servers#4207).
//! Requirement: larry#84. Each test is one protocol property driven by one
//! seed through the public stdio interface; the same seed gives the same run.

mod mcp_support;

use lgwks_std::json::{self, Value, json};
use mcp_support::{Rng, Session, TestResult};

/// The server's frame bound.
const MAX_FRAME: usize = 1024 * 1024;

/// A started, initialized session.
fn ready() -> TestResult<Session> {
    let mut session = Session::start()?;
    session.initialize()?;
    Ok(session)
}

/// One request with no params, answered.
fn ask(session: &mut Session, id: &Value, method: &str) -> TestResult<Value> {
    session.request(&json!({"jsonrpc":"2.0","id":id,"method":method}))
}

/// The one tool called with `arguments`, answered.
fn submit(session: &mut Session, arguments: &Value) -> TestResult<Value> {
    session.request(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"larry_submit_task","arguments":arguments}}))
}

/// The typed tool-error code of a `tools/call` reply, or an empty string.
fn tool_code(reply: &Value) -> Option<&str> {
    reply["result"]["structuredContent"]["error"]["code"].as_str()
}

/// A JSON-RPC ping frame with a string id, as bytes.
fn ping_frame(id: &str) -> TestResult<Vec<u8>> {
    Ok(json::to_vec(
        &json!({"jsonrpc":"2.0","id":id,"method":"ping"}),
    )?)
}

/// A frame split at seeded points reaches the server as the same message.
#[test]
fn seeded_fragmented_writes_reply_like_whole_frames() -> TestResult<()> {
    for seed in 0..8 {
        let mut rng = Rng(seed);
        let arguments = json!({"goal":format!("g{}", rng.next()),"repo":"r","acceptance":["c"]});
        let frame = json::to_vec(&json!({"jsonrpc":"2.0","id":seed,"method":"tools/call",
            "params":{"name":"larry_submit_task","arguments":arguments}}))?;
        let mut whole = ready()?;
        whole.send(&frame)?;
        let expected = whole.recv_line()?;
        let mut split = ready()?;
        let mut rest = frame.as_slice();
        while !rest.is_empty() {
            let take = rng.index(7)?.saturating_add(1).min(rest.len());
            let (head, tail) = rest.split_at(take);
            split.send_raw(head)?;
            rest = tail;
        }
        split.send_raw(b"\n")?;
        assert_eq!(split.recv_line()?, expected, "seed {seed}: split reply");
    }
    Ok(())
}

/// Requests written back to back are answered one each, in order.
#[test]
fn seeded_pipelined_requests_are_answered_in_order() -> TestResult<()> {
    for seed in 0..8 {
        let mut rng = Rng(seed);
        let mut session = ready()?;
        let count = rng.index(32)?.saturating_add(8);
        for index in 0..count {
            session.send(&ping_frame(&format!("p{seed}-{index}"))?)?;
        }
        for index in 0..count {
            assert_eq!(
                session.recv()?["id"],
                format!("p{seed}-{index}"),
                "seed {seed}: reply {index} out of order"
            );
        }
        assert_eq!(session.finish()?.code(), Some(0), "seed {seed}: clean exit");
    }
    Ok(())
}

/// A storm of notifications draws no reply: the first reply is the barrier's.
#[test]
fn seeded_notification_storms_draw_no_reply() -> TestResult<()> {
    for seed in 0..8 {
        let mut rng = Rng(seed);
        let mut session = ready()?;
        for _ in 0..rng.index(40)?.saturating_add(5) {
            let method = format!("notifications/n{}", rng.next());
            session.send(&json::to_vec(
                &json!({"jsonrpc":"2.0","method":method,"params":{"k":rng.next()}}),
            )?)?;
        }
        let barrier = ask(&mut session, &json!("barrier"), "ping")?;
        assert_eq!(
            barrier["id"], "barrier",
            "seed {seed}: a notification was answered"
        );
    }
    Ok(())
}

/// Before initialize only ping works; the stateless methods without `_meta` lack
/// required fields (-32602, MCP 2026-07-28 Basic), other methods are unknown.
#[test]
fn seeded_pre_initialize_traffic_is_gated() -> TestResult<()> {
    let table = [
        ("tools/list", -32602),
        ("tools/call", -32602),
        ("server/discover", -32602),
        ("ping", 0),
        ("resources/list", -32601),
        ("completion/complete", -32601),
        ("logging/setLevel", -32601),
    ];
    for seed in 0..6 {
        let mut rng = Rng(seed);
        let mut session = Session::start()?;
        for index in 0..20 {
            let (method, code) = table[rng.index(7)?];
            let reply = ask(&mut session, &json!(index), method)?;
            if code == 0 {
                assert!(reply["result"].is_object(), "seed {seed}: {method} allowed");
            } else {
                assert_eq!(
                    reply["error"]["code"], code,
                    "seed {seed}: {method} refused"
                );
            }
        }
        session.initialize()?;
        let list = ask(&mut session, &json!("after"), "tools/list")?;
        assert!(
            list["result"]["tools"].is_array(),
            "seed {seed}: open after initialize"
        );
    }
    Ok(())
}

/// Every revision string negotiates to a served legacy revision, echoing known
/// ones; the stateless revision is not a legacy one, so it gets the newest legacy.
#[test]
fn seeded_version_strings_negotiate_to_a_supported_revision() -> TestResult<()> {
    let known = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
    let pool = [
        "2025-06-18",
        "2025-03-26",
        "2024-11-05",
        "2025-06-19",
        "2026-07-28",
        "",
        "2025-11-25",
        "2024-11-05 ",
        "latest",
    ];
    let mut rng = Rng(0x5eed);
    for _ in 0..12 {
        let asked = pool[rng.index(9)?];
        let mut session = Session::start()?;
        let reply = session.request(&json!({"jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":asked}}))?;
        let want = if known.contains(&asked) {
            asked
        } else {
            "2025-11-25"
        };
        assert_eq!(reply["result"]["protocolVersion"], want, "asked {asked:?}");
    }
    Ok(())
}

/// Request ids of every legal shape come back exactly as sent.
#[test]
fn seeded_ids_round_trip_exactly() -> TestResult<()> {
    let mut rng = Rng(0x1d5);
    let mut session = ready()?;
    let signed = i64::from_le_bytes(rng.next().to_le_bytes());
    let wide = rng.next() | 0x8000_0000_0000_0000;
    let ids = [
        json!(signed),
        json!(wide),
        json!(0),
        json!(-1),
        json!(""),
        json!("ü€😀"),
        json!("x".repeat(10_000)),
        json!("null"),
    ];
    for id in &ids {
        let reply = ask(&mut session, id, "ping")?;
        assert_eq!(&reply["id"], id, "id {id}");
    }
    Ok(())
}

/// Repo-relative paths pass; every `..`, rooted or empty path is refused.
#[test]
fn seeded_path_variants_decide_task_validity() -> TestResult<()> {
    let table = [
        ("a", true),
        ("a/b.rs", true),
        ("./a", true),
        ("a//b", true),
        ("../a", false),
        ("a/../b", false),
        ("/abs", false),
        ("a/..", false),
        ("", false),
    ];
    let mut rng = Rng(0xfa11);
    let mut session = ready()?;
    for _ in 0..24 {
        let (path, valid) = table[rng.index(9)?];
        let reply = submit(
            &mut session,
            &json!({"goal":"g","repo":"r","files":[path],"acceptance":["c"]}),
        )?;
        let want = if valid {
            "executor_not_connected"
        } else {
            "invalid_arguments"
        };
        assert_eq!(tool_code(&reply), Some(want), "path {path:?}");
    }
    Ok(())
}

/// The advertised size limits hold on both sides of their edge.
#[test]
fn seeded_task_limits_hold_on_both_sides() -> TestResult<()> {
    let mut rng = Rng(0xb0d);
    let mut session = ready()?;
    for _ in 0..18 {
        let over = rng.below(2) == 1;
        let extra = usize::from(over);
        let mut task = json!({"goal":"g","repo":"r","files":[],"acceptance":["c"]});
        match rng.below(3) {
            0 => task["goal"] = json!("g".repeat(8192_usize.saturating_add(extra))),
            1 => task["files"] = json!(vec!["f"; 256_usize.saturating_add(extra)]),
            _ => task["acceptance"] = json!(vec!["c"; 32_usize.saturating_add(extra)]),
        }
        let want = if over {
            "invalid_arguments"
        } else {
            "executor_not_connected"
        };
        assert_eq!(
            tool_code(&submit(&mut session, &task)?),
            Some(want),
            "over={over}"
        );
    }
    Ok(())
}

/// A frame of exactly the bound is served; one byte more is refused.
#[test]
fn seeded_frame_sizes_around_the_bound() -> TestResult<()> {
    let mut rng = Rng(0xb0b);
    let mut session = ready()?;
    let base = br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
    for _ in 0..4 {
        let delta = rng.index(3)?;
        let length = MAX_FRAME.saturating_sub(1).saturating_add(delta);
        let mut frame = base.to_vec();
        frame.resize(length, b' ');
        session.send(&frame)?;
        let reply = session.recv()?;
        if length <= MAX_FRAME {
            assert_eq!(reply["id"], 1, "{length} bytes fit the bound");
        } else {
            assert_eq!(reply["error"]["code"], -32600, "{length} bytes exceed it");
        }
    }
    Ok(())
}

/// Carriage-return line ends are served; a blank line is a parse error.
#[test]
fn seeded_crlf_and_blank_lines() -> TestResult<()> {
    let mut rng = Rng(0xc41f);
    let mut session = ready()?;
    for index in 0..20 {
        if rng.below(2) == 0 {
            session.send_raw(&ping_frame(&format!("c{index}"))?)?;
            session.send_raw(b"\r\n")?;
            assert_eq!(
                session.recv()?["id"],
                format!("c{index}"),
                "CRLF frame {index}"
            );
        } else {
            session.send_raw(b"\n")?;
            assert_eq!(
                session.recv()?["error"]["code"],
                -32700,
                "blank line {index}"
            );
        }
    }
    Ok(())
}

/// kill -9 at a seeded point loses nothing; the next session answers as before.
#[test]
fn seeded_kill_points_never_corrupt_a_restart() -> TestResult<()> {
    for seed in 0..6 {
        let mut rng = Rng(seed);
        let mut session = ready()?;
        for index in 0..rng.index(10)? {
            assert_eq!(
                ask(&mut session, &json!(index), "ping")?["id"],
                index,
                "seed {seed}"
            );
        }
        assert!(
            session.kill()?.code().is_none(),
            "seed {seed}: killed by signal"
        );
        let mut restarted = ready()?;
        let list = ask(&mut restarted, &json!("again"), "tools/list")?;
        assert_eq!(
            list["result"]["tools"][0]["name"], "larry_submit_task",
            "seed {seed}"
        );
    }
    Ok(())
}

/// End of input in the middle of a frame still answers that frame, then exits 0.
#[test]
fn seeded_eof_inside_a_frame_is_served_then_clean_exit() -> TestResult<()> {
    for seed in 0..6 {
        let mut rng = Rng(seed);
        let mut session = ready()?;
        for index in 0..rng.index(5)?.saturating_add(1) {
            session.send(&ping_frame(&format!("k{index}"))?)?;
            assert_eq!(session.recv()?["id"], format!("k{index}"), "seed {seed}");
        }
        session.send_raw(&ping_frame("last")?)?;
        session.close_stdin();
        assert_eq!(
            session.recv()?["id"],
            "last",
            "seed {seed}: unterminated frame"
        );
        assert!(session.recv_line().is_err(), "seed {seed}: stdout closes");
        assert_eq!(session.finish()?.code(), Some(0), "seed {seed}: clean exit");
    }
    Ok(())
}

/// Multi-byte text survives a tool call; a cut multi-byte sequence is a parse error.
#[test]
fn seeded_unicode_payloads_survive_and_broken_utf8_is_refused() -> TestResult<()> {
    let pool = [
        "日本語のタスク",
        "ünïcödé",
        "😀🚀",
        "e\u{0301}",
        "\u{202e}rtl",
    ];
    let mut rng = Rng(0xc0de);
    let mut session = ready()?;
    for _ in 0..10 {
        let goal = pool[rng.index(5)?];
        let reply = submit(
            &mut session,
            &json!({"goal":goal,"repo":"r","acceptance":["c"]}),
        )?;
        assert_eq!(
            tool_code(&reply),
            Some("executor_not_connected"),
            "goal {goal:?}"
        );
    }
    let mut broken = br#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{"x":""#.to_vec();
    broken.extend_from_slice(&[0xc3, b'"', b'}', b'}']);
    session.send(&broken)?;
    assert_eq!(
        session.recv()?["error"]["code"],
        -32700,
        "truncated sequence"
    );
    Ok(())
}

/// Nesting far under the parser limit is served; far over it is a parse error.
#[test]
fn seeded_nesting_depths_split_at_the_parser_limit() -> TestResult<()> {
    let depths = [1_usize, 10, 50, 100, 150, 200, 255];
    let mut rng = Rng(0xdee9);
    let mut session = ready()?;
    for _ in 0..14 {
        let depth = depths[rng.index(7)?];
        let frame = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"ping","params":{{"x":{}1{}}}}}"#,
            "[".repeat(depth),
            "]".repeat(depth)
        );
        session.send(frame.as_bytes())?;
        let reply = session.recv()?;
        if depth <= 100 {
            assert!(reply["result"].is_object(), "depth {depth} is served");
        } else {
            assert_eq!(reply["error"]["code"], -32700, "depth {depth} is refused");
        }
    }
    Ok(())
}

/// Only the exact method names exist; near misses are not found.
#[test]
fn seeded_near_miss_method_names_are_not_found() -> TestResult<()> {
    let mut rng = Rng(0x4e7);
    let mut session = ready()?;
    let fixed = [
        "",
        "Ping",
        "ping ",
        "tools/",
        "TOOLS/LIST",
        "notifications/initialized",
        "rpc.discover",
    ];
    for _ in 0..16 {
        let method = if rng.below(4) == 0 {
            format!("m{}", rng.next())
        } else {
            String::from(fixed[rng.index(7)?])
        };
        let reply = ask(&mut session, &json!("m"), &method)?;
        assert_eq!(reply["error"]["code"], -32601, "method {method:?}");
    }
    Ok(())
}

/// Replayed initialized notifications are silent; a second initialize is refused.
#[test]
fn seeded_repeated_initialization_is_refused_without_harm() -> TestResult<()> {
    for seed in 0..6 {
        let mut rng = Rng(seed);
        let mut session = ready()?;
        for index in 0..rng.index(5)?.saturating_add(1) {
            session.send(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)?;
            let again = session
                .request(&json!({"jsonrpc":"2.0","id":index,"method":"initialize",
                "params":{"protocolVersion":"2025-06-18"}}))?;
            assert_eq!(
                again["error"]["code"], -32600,
                "seed {seed}: initialize {index}"
            );
        }
        let list = ask(&mut session, &json!("ok"), "tools/list")?;
        assert!(
            list["result"]["tools"].is_array(),
            "seed {seed}: still serving"
        );
    }
    Ok(())
}

/// Oversized and valid frames interleaved at seeded positions each draw their own reply.
#[test]
fn seeded_oversized_and_valid_frames_interleave() -> TestResult<()> {
    for seed in 0..3 {
        let mut rng = Rng(seed);
        let mut session = ready()?;
        for index in 0..6 {
            if rng.below(2) == 0 {
                let junk = vec![b'z'; MAX_FRAME.saturating_add(1).saturating_add(rng.index(512)?)];
                session.send(&junk)?;
                assert_eq!(
                    session.recv()?["error"]["code"],
                    -32600,
                    "seed {seed} frame {index}"
                );
            } else {
                assert_eq!(
                    ask(&mut session, &json!(index), "ping")?["id"],
                    index,
                    "seed {seed} frame {index}"
                );
            }
        }
        assert_eq!(session.finish()?.code(), Some(0), "seed {seed}: clean exit");
    }
    Ok(())
}
