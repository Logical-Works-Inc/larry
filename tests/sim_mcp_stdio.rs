//! Seeded simulations against the shipped `rocco mcp` binary.
//!
//! Source: JSON-RPC 2.0 section 5 (error objects, notifications get no reply),
//! MCP specification 2025-06-18 (lifecycle, stdio framing) and the fuzzing
//! practice of seed-replayed malformed frames. Requirement: larry#84, the
//! server answers every frame exactly as the spec says, never panics, never
//! replies to a notification, and stays isolated per session. One seed drives
//! every generated frame; the same seed must give the same reply bytes.

mod mcp_support;

use lgwks_std::json::{self, Value, json};
use mcp_support::{Rng, Session, TestResult};

/// The server's frame bound; the oversized frames are built from it.
const MAX_FRAME: usize = 1024 * 1024;
/// Frames per seeded session.
const STEPS: usize = 48;

/// FNV-1a over every reply byte: the trace hash of a session.
struct Fnv(u64);

impl Fnv {
    /// Folds `bytes` into the hash.
    fn feed(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// What the spec says the server must answer.
#[derive(Clone, Copy)]
enum Want {
    /// A success result.
    Ok,
    /// A result with `isError` true and this typed code.
    ToolError(&'static str),
    /// A JSON-RPC error with this code.
    Error(i64),
    /// No reply at all.
    Silent,
}

/// One generated frame and the reply it must draw.
struct Step {
    /// The frame bytes, without the newline.
    bytes: Vec<u8>,
    /// The id the reply must carry.
    id: Value,
    /// The reply class.
    want: Want,
}

/// A fresh request id: an integer or a string, chosen by the seed.
fn pick_id(rng: &mut Rng, index: usize) -> Value {
    if rng.below(2) == 0 {
        json!(index)
    } else {
        json!(format!("id-{}", rng.next()))
    }
}

/// A `tools/call` frame for the one tool.
fn call_frame(id: &Value, arguments: &Value) -> TestResult<Vec<u8>> {
    Ok(json::to_vec(
        &json!({"jsonrpc":"2.0","id":id,"method":"tools/call",
        "params":{"name":"larry_submit_task","arguments":arguments}}),
    )?)
}

/// Builds one frame of the 16 kinds the seed selects from.
fn make_step(rng: &mut Rng, index: usize) -> TestResult<Step> {
    let id = pick_id(rng, index);
    let request = |method: &str| json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method}));
    let (bytes, reply_id, want): (Vec<u8>, Value, Want) = match rng.below(16) {
        0 => (request("ping")?, id.clone(), Want::Ok),
        1 => (request("tools/list")?, id.clone(), Want::Ok),
        2 => (
            call_frame(
                &id,
                &json!({"goal":"g","repo":"r","files":["a/b.rs"],"acceptance":["c"]}),
            )?,
            id.clone(),
            Want::ToolError("executor_not_connected"),
        ),
        3 => {
            let arguments = match rng.below(4) {
                0 => json!({}),
                1 => json!({"goal":"g","repo":"r","acceptance":[]}),
                2 => json!({"goal":"g","repo":"r","acceptance":["c"],"files":["../x"]}),
                _ => json!({"goal":rng.next(),"repo":"r","acceptance":["c"]}),
            };
            (
                call_frame(&id, &arguments)?,
                id.clone(),
                Want::ToolError("invalid_arguments"),
            )
        }
        4 => (
            request(&format!("x/{}", rng.next()))?,
            id.clone(),
            Want::Error(-32601),
        ),
        5 => {
            let whole = request("ping")?;
            let cut = rng
                .index(u32::try_from(whole.len().saturating_sub(2))?)?
                .saturating_add(1);
            (
                whole.get(..cut).ok_or("cut inside the frame")?.to_vec(),
                Value::Null,
                Want::Error(-32700),
            )
        }
        6 => (
            json::to_vec(&json!({"jsonrpc":"1.0","id":id,"method":"ping"}))?,
            id.clone(),
            Want::Error(-32600),
        ),
        7 => (
            br#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#.to_vec(),
            Value::Null,
            Want::Error(-32600),
        ),
        8 => (
            br#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#.to_vec(),
            Value::Null,
            Want::Error(-32600),
        ),
        9 => (
            json::to_vec(
                &json!({"jsonrpc":"2.0","method":format!("notifications/n{}", rng.next())}),
            )?,
            Value::Null,
            Want::Silent,
        ),
        10 => {
            let mut junk = vec![0xff, 0xfe];
            for _ in 0..rng.below(64) {
                let byte = u8::try_from(rng.below(256))?;
                junk.push(if byte == b'\n' { 0x0b } else { byte });
            }
            (junk, Value::Null, Want::Error(-32700))
        }
        11 => (
            vec![b'['; rng.index(200)?.saturating_add(130)],
            Value::Null,
            Want::Error(-32700),
        ),
        12 => (
            json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":[1]}))?,
            id.clone(),
            Want::Error(-32602),
        ),
        13 => (Vec::new(), Value::Null, Want::Error(-32700)),
        14 => (
            json::to_vec(&json!({"jsonrpc":"2.0","id":id,"result":{}}))?,
            Value::Null,
            Want::Silent,
        ),
        _ => (
            br#"{"jsonrpc":"2.0","id":1.5,"method":"ping"}"#.to_vec(),
            Value::Null,
            Want::Error(-32600),
        ),
    };
    Ok(Step {
        bytes,
        id: reply_id,
        want,
    })
}

/// Asserts `reply` is what `step` demands.
fn check(seed: u64, index: usize, step: &Step, reply: &Value) -> TestResult<()> {
    assert_eq!(reply["id"], step.id, "seed {seed} step {index}: id");
    match step.want {
        Want::Ok => assert!(
            reply["result"].is_object() && reply["result"]["isError"].is_null(),
            "seed {seed} step {index}: a plain result, got {reply}"
        ),
        Want::ToolError(code) => {
            assert_eq!(
                reply["result"]["isError"], true,
                "seed {seed} step {index}: isError"
            );
            assert_eq!(
                reply["result"]["structuredContent"]["error"]["code"], code,
                "seed {seed} step {index}: tool error code"
            );
        }
        Want::Error(code) => {
            assert_eq!(
                reply["error"]["code"], code,
                "seed {seed} step {index}: error code"
            );
        }
        Want::Silent => {
            return Err(format!("seed {seed} step {index}: a silent frame was answered").into());
        }
    }
    Ok(())
}

/// Runs one seeded session against the binary and returns its reply trace hash.
fn run_seed(seed: u64) -> TestResult<u64> {
    let mut rng = Rng(seed);
    let mut session = Session::start()?;
    session.initialize()?;
    let mut trace = Fnv(0xcbf2_9ce4_8422_2325);
    for index in 0..STEPS {
        let step = if index == 17 && seed & 7 == 0 {
            Step {
                bytes: vec![b'a'; MAX_FRAME.saturating_add(1).saturating_add(rng.index(4096)?)],
                id: Value::Null,
                want: Want::Error(-32600),
            }
        } else {
            make_step(&mut rng, index)?
        };
        session.send(&step.bytes)?;
        if matches!(step.want, Want::Silent) {
            continue;
        }
        let line = session.recv_line()?;
        trace.feed(&line);
        check(seed, index, &step, &json::from_slice(&line)?)?;
    }
    session.send(br#"{"jsonrpc":"2.0","id":"barrier","method":"ping"}"#)?;
    let barrier = session.recv()?;
    assert_eq!(
        barrier["id"], "barrier",
        "seed {seed}: no notification or response was answered"
    );
    assert_eq!(session.finish()?.code(), Some(0), "seed {seed}: clean exit");
    Ok(trace.0)
}

/// Same seed, same reply bytes; different seeds explore different traces.
#[test]
fn seeded_frame_fuzz_replays_bit_identically() -> TestResult<()> {
    let mut hashes = Vec::new();
    for seed in 0..24 {
        let first = run_seed(seed)?;
        let second = run_seed(seed)?;
        assert_eq!(first, second, "seed {seed}: replay diverged");
        hashes.push(first);
    }
    hashes.sort_unstable();
    hashes.dedup();
    assert!(
        hashes.len() > 20,
        "seeds produce distinct traces, got {}",
        hashes.len()
    );
    Ok(())
}

/// 100 sessions at once: every reply reaches its own session, kill -9 of a
/// tenth of them disturbs no other, the rest exit 0. The host allows far more
/// descriptors than 100 sessions need (ulimit -n 1048576); 100 is the level reached.
#[test]
fn one_hundred_concurrent_sessions_stay_isolated() -> TestResult<()> {
    const SESSIONS: usize = 100;
    const ROUNDS: usize = 10;
    let mut rng = Rng(0x1090);
    let mut sessions = Vec::with_capacity(SESSIONS);
    for _ in 0..SESSIONS {
        let mut session = Session::start()?;
        session.initialize()?;
        sessions.push(session);
    }
    for round in 0..ROUNDS {
        let mut asked = Vec::with_capacity(SESSIONS);
        for (index, session) in sessions.iter_mut().enumerate() {
            let id = index.saturating_mul(1000).saturating_add(round);
            let list = rng.below(2) == 0;
            let method = if list { "tools/list" } else { "ping" };
            session.send(&json::to_vec(
                &json!({"jsonrpc":"2.0","id":id,"method":method}),
            )?)?;
            asked.push((id, list));
        }
        for (session, (id, list)) in sessions.iter_mut().zip(asked) {
            let reply = session.recv()?;
            assert_eq!(reply["id"], id, "round {round}: reply crossed sessions");
            assert_eq!(
                reply["result"]["tools"].is_array(),
                list,
                "round {round}: result shape for id {id}"
            );
        }
    }
    for (index, session) in sessions.into_iter().enumerate() {
        if index % 10 == 0 {
            let status = session.kill()?;
            assert!(
                status.code().is_none(),
                "session {index} was killed by signal"
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
