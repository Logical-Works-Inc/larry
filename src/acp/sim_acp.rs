//! Seeded adversarial simulations of the ACP agent's protocol surface (larry#93),
//! through `serve_bounded` and the content-block reader, against reference models
//! written from the Agent Client Protocol v1 and JSON-RPC 2.0 section 5, never
//! from the agent's own output. Fools: monkey (nonsense order over the whole
//! surface), wrong-order (sessions and prompts before `initialize`, a second
//! `initialize`), double-clicker (initialize twice, the session bound), giant-and-empty
//! (frames at the bound and one past it), weird-bytes (invalid UTF-8, NUL,
//! right-to-left and zero-width marks, `../`, float and extreme ids), and the
//! saboteur (no reply both result and error, no update for a session never opened,
//! no log line but a cancel's, the agent never fails).

use std::io::Cursor;

use lgwks_std::json::{self, Value, json};
use lgwks_std::seeded::Seeded;

use super::{PromptFault, instruction, serve_bounded};
use crate::mcp::wire;
use crate::sim_error::SimError;

/// What every simulation returns.
type SimResult = Result<(), Box<dyn std::error::Error>>;

/// Seeds each simulation runs.
const SEEDS: u64 = 256;
/// Frames the monkey sends per seed: past the 16-session bound on most seeds.
const FRAMES: usize = 96;
/// Text pieces the weird-bytes fool draws from.
const WEIRD: [&str; 10] = [
    "a", "\0", "\u{202e}", "\u{200b}", "🦝", "../", "%00", "\n", "\"", "é",
];

/// An absolute directory every host has.
fn tmp() -> Result<String, SimError> {
    let dir = std::env::temp_dir().canonicalize()?;
    Ok(dir
        .to_str()
        .ok_or(SimError::Missing("a UTF-8 temp dir"))?
        .to_owned())
}

/// A generated text of up to `most` weird pieces.
fn weird(rng: &mut Seeded, most: u64) -> Result<String, SimError> {
    let mut out = String::new();
    for _ in 0..rng.below(most)? {
        let piece = WEIRD
            .get(rng.index(WEIRD.len())?)
            .ok_or(SimError::Missing("weird pieces"))?;
        out.push_str(piece);
    }
    Ok(out)
}

/// The reference agent: ACP v1 as Larry declares it, with refused settings.
struct Reference {
    /// Whether `initialize` was answered.
    initialized: bool,
    /// Sessions opened so far.
    opened: u64,
}

/// What the reference says one frame gets.
struct Owed {
    /// The reply's id and error code (`None`: a result), or no reply.
    reply: Option<(Value, Option<i64>)>,
    /// Whether a `session/update` precedes the reply.
    update: bool,
    /// Whether the reader logs a cancel.
    logged: bool,
}

impl Owed {
    /// A reply with no update or log.
    fn reply(id: Value, code: Option<i64>) -> Self {
        Self {
            reply: Some((id, code)),
            update: false,
            logged: false,
        }
    }

    /// No reply.
    const fn silent(logged: bool) -> Self {
        Self {
            reply: None,
            update: false,
            logged,
        }
    }
}

impl Reference {
    /// A request's answer once the envelope is valid: everything but
    /// `initialize` is refused before it.
    fn gated(&self, id: &Value, then: Option<i64>) -> Owed {
        if self.initialized {
            Owed::reply(id.clone(), then)
        } else {
            Owed::reply(id.clone(), Some(-32600))
        }
    }

    /// The session id the agent gives its `n`th session.
    fn session(n: u64) -> String {
        format!("larry-{}-{n}", std::process::id())
    }

    /// One monkey frame and what the reference owes it.
    fn draw(
        &mut self,
        rng: &mut Seeded,
        index: u64,
        cwd: &str,
    ) -> Result<(Vec<u8>, Owed), Box<dyn std::error::Error>> {
        let id = if rng.below(2)? == 0 {
            json!(index)
        } else {
            json!(format!("s{index}"))
        };
        let target = Self::session(rng.below(self.opened.saturating_add(2))?);
        let known = self.opened > 0 && (0..self.opened).any(|n| Self::session(n) == target);
        let request = |method: &str, params: Value| {
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string()
        };
        // Four of twenty draws open a session, so most seeds pass the 16-session bound.
        Ok(match rng.below(20)? {
            0 => (
                b"\xff\xfe{".to_vec(),
                Owed::reply(Value::Null, Some(-32700)),
            ),
            1 => (
                // A raw newline would end the frame, so a non-JSON line carries none.
                format!("not json {}", weird(rng, 4)?.replace('\n', " ")).into_bytes(),
                Owed::reply(Value::Null, Some(-32700)),
            ),
            2 => (
                br#"[{"jsonrpc":"2.0","id":1,"method":"initialize"}]"#.to_vec(),
                Owed::reply(Value::Null, Some(-32600)),
            ),
            3 => {
                let bad = ["null", "1.5", "true", "{}"]
                    .get(rng.index(4)?)
                    .ok_or("ids")?
                    .to_owned();
                (
                    format!(r#"{{"jsonrpc":"2.0","id":{bad},"method":"initialize"}}"#).into_bytes(),
                    Owed::reply(Value::Null, Some(-32600)),
                )
            }
            4 => (
                json!({"jsonrpc":"1.0","id":id,"method":"initialize"})
                    .to_string()
                    .into_bytes(),
                Owed::reply(id.clone(), Some(-32600)),
            ),
            5 => (
                br#"{"jsonrpc":"2.0","method":"notes/whatever","params":{}}"#.to_vec(),
                Owed::silent(false),
            ),
            6 => (
                json!({"jsonrpc":"2.0","id":id,"result":{}})
                    .to_string()
                    .into_bytes(),
                Owed::silent(false),
            ),
            7 => (
                br#"{"jsonrpc":"2.0","method":"session/cancel","params":[1]}"#.to_vec(),
                Owed::silent(false),
            ),
            8 => (
                request("session/new", json!("x")).into_bytes(),
                Owed::reply(id.clone(), Some(-32602)),
            ),
            9 => {
                let owed = if self.initialized {
                    Owed::reply(id.clone(), Some(-32600))
                } else {
                    self.initialized = true;
                    Owed::reply(id.clone(), None)
                };
                (
                    request("initialize", json!({"protocolVersion": rng.below(5)?})).into_bytes(),
                    owed,
                )
            }
            10 => {
                let version = ["-1", "\"1\"", "1.5"]
                    .get(rng.index(3)?)
                    .ok_or("versions")?
                    .to_owned();
                let owed = if self.initialized {
                    Owed::reply(id.clone(), Some(-32600))
                } else {
                    Owed::reply(id.clone(), Some(-32602))
                };
                let text = format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"method":"initialize","params":{{"protocolVersion":{version}}}}}"#
                );
                (text.into_bytes(), owed)
            }
            11 => (
                request("session/load", json!({})).into_bytes(),
                self.gated(&id, Some(-32601)),
            ),
            12 => (
                request("authenticate", json!({"methodId":"x"})).into_bytes(),
                self.gated(&id, Some(-32602)),
            ),
            13 | 17..=19 => {
                let owed = match (self.initialized, self.opened < 16) {
                    (false, _) => Owed::reply(id.clone(), Some(-32600)),
                    (true, false) => Owed::reply(id.clone(), Some(-32600)),
                    (true, true) => {
                        self.opened = self.opened.saturating_add(1);
                        Owed::reply(id.clone(), None)
                    }
                };
                (
                    request("session/new", json!({"cwd":cwd,"mcpServers":[]})).into_bytes(),
                    owed,
                )
            }
            14 => (
                request("session/new", json!({"cwd":"rel/../x","mcpServers":[]})).into_bytes(),
                self.gated(&id, Some(-32602)),
            ),
            15 => {
                let image = rng.below(2)? == 0;
                let block = if image {
                    json!({"type":"image","data":"","mimeType":"image/png"})
                } else {
                    json!({"type":"text","text":weird(rng, 6)?})
                };
                let mut owed = match (known, image) {
                    (true, false) => self.gated(&id, None),
                    _ => self.gated(&id, Some(-32602)),
                };
                owed.update = self.initialized && known && !image;
                (
                    request(
                        "session/prompt",
                        json!({"sessionId":target,"prompt":[block]}),
                    )
                    .into_bytes(),
                    owed,
                )
            }
            _ => {
                let text = json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":target}});
                (text.to_string().into_bytes(), Owed::silent(true))
            }
        })
    }
}

/// Serves `frames` with refused settings and frame bound `max`; returns the output
/// lines and the log.
fn serve(
    frames: &[Vec<u8>],
    max: usize,
) -> Result<(Vec<Value>, String), Box<dyn std::error::Error>> {
    let mut input = Vec::new();
    for frame in frames {
        input.extend_from_slice(frame);
        input.push(b'\n');
    }
    let mut out = Vec::new();
    let mut log = Vec::new();
    let settings = Err(String::from("no executor in a simulation"));
    serve_bounded(Cursor::new(input), &mut out, max, &settings, &mut log)?;
    let mut lines = Vec::new();
    for line in String::from_utf8(out)?.lines() {
        lines.push(json::from_str(line)?);
    }
    Ok((lines, String::from_utf8(log)?))
}

/// The monkey walks the whole surface in nonsense order; every frame gets exactly
/// the reference agent's answer, in order, and the saboteur finds nothing.
#[test]
fn sim_acp_monkey_frames_get_the_reference_agents_answer_for_every_seed() -> SimResult {
    let cwd = tmp()?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let mut reference = Reference {
            initialized: false,
            opened: 0,
        };
        let mut frames = Vec::with_capacity(FRAMES);
        let mut owed = Vec::with_capacity(FRAMES);
        for index in 0..FRAMES {
            let (frame, answer) = reference.draw(&mut rng, u64::try_from(index)?, &cwd)?;
            frames.push(frame);
            owed.push(answer);
        }
        let (lines, log) = serve(&frames, 4096)?;
        let replies: Vec<(Value, Option<i64>)> = lines
            .iter()
            .filter(|line| line.get("method").is_none())
            .map(|line| (line["id"].clone(), line["error"]["code"].as_i64()))
            .collect();
        let expected: Vec<(Value, Option<i64>)> = owed
            .iter()
            .filter_map(|answer| answer.reply.clone())
            .collect();
        assert_eq!(
            replies, expected,
            "seed {seed}: every frame's answer, in order"
        );
        let updates = lines
            .iter()
            .filter(|line| line.get("method").is_some())
            .count();
        assert_eq!(
            updates,
            owed.iter().filter(|answer| answer.update).count(),
            "seed {seed}: updates"
        );
        assert_eq!(
            log.lines().count(),
            owed.iter().filter(|answer| answer.logged).count(),
            "seed {seed}: log"
        );
        for line in &lines {
            assert_eq!(line["jsonrpc"], "2.0", "seed {seed}: {line}");
            assert!(
                line.get("result").is_none() || line.get("error").is_none(),
                "seed {seed}: both"
            );
            if line.get("method").is_some() {
                let session = line["params"]["sessionId"]
                    .as_str()
                    .ok_or("an update names its session")?;
                let opened = (0..reference.opened).any(|n| Reference::session(n) == session);
                assert!(opened, "seed {seed}: an update for a session never opened");
            }
        }
        assert!(
            log.lines().all(|line| line.ends_with(" cancel requested")),
            "seed {seed}: log"
        );
    }
    Ok(())
}

/// A frame at the bound is read and answered by id; one past it is discarded with
/// the wire's error, whatever the bound.
#[test]
fn sim_acp_giant_frames_at_and_past_the_bound_get_the_rule_answer() -> SimResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed.wrapping_add(1 << 40));
        let max = usize::try_from(rng.below(4032)?.saturating_add(64))?;
        let mut frames = Vec::new();
        let mut expected = Vec::new();
        for index in 0..8_u64 {
            let head = format!(r#"{{"jsonrpc":"2.0","id":{index},"method":"session/new""#);
            let len = max.saturating_sub(3).saturating_add(rng.index(7)?);
            let pad = len.saturating_sub(head.len()).saturating_sub(1);
            frames.push(format!("{head}{}}}", " ".repeat(pad)).into_bytes());
            let id = if len <= max {
                json!(index)
            } else {
                Value::Null
            };
            expected.push((id, Some(-32600)));
        }
        let (lines, _) = serve(&frames, max)?;
        let got: Vec<(Value, Option<i64>)> = lines
            .iter()
            .map(|line| (line["id"].clone(), line["error"]["code"].as_i64()))
            .collect();
        assert_eq!(got, expected, "seed {seed}: bound {max}");
    }
    Ok(())
}

/// Weird bytes in content blocks: text and embedded resource text pass through
/// whole, a link becomes `name: uri`, and the first block that is not one of those
/// refuses the prompt with its reason.
#[test]
fn sim_acp_weird_bytes_content_blocks_read_as_the_reference_says() -> SimResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed.wrapping_add(2 << 40));
        let mut blocks = Vec::new();
        let mut parts: Vec<String> = Vec::new();
        let mut refusal: Option<PromptFault> = None;
        for _ in 0..rng.below(5)? {
            let text = weird(&mut rng, 8)?;
            let (block, part) = match rng.below(8)? {
                0 | 1 => (json!({"type":"text","text":text}), Ok(text)),
                2 => (
                    json!({"type":"resource","resource":{"uri":"file:///x","text":text}}),
                    Ok(text),
                ),
                3 => (
                    json!({"type":"resource_link","name":text,"uri":"file:///../y"}),
                    Ok(format!("{text}: file:///../y")),
                ),
                4 => (
                    json!({"type":"audio","data":"","mimeType":"audio/wav"}),
                    Err(PromptFault::Unsupported(String::from("audio"))),
                ),
                5 => (json!({"type":"text"}), Err(PromptFault::MissingText)),
                6 => (
                    json!({"type":"resource_link","name":text}),
                    Err(PromptFault::MissingText),
                ),
                _ => (json!({"text":text}), Err(PromptFault::MissingText)),
            };
            blocks.push(block);
            match part {
                Ok(piece) => parts.push(piece),
                Err(reason) => {
                    refusal.get_or_insert(reason);
                }
            }
        }
        let expected = match (blocks.is_empty(), refusal) {
            (true, _) => Err(PromptFault::Empty),
            (false, Some(reason)) => Err(reason),
            (false, None) => Ok(parts.join("\n\n")),
        };
        let params = json!({"prompt": blocks});
        assert_eq!(instruction(params.as_object()), expected, "seed {seed}");
    }
    let not_an_array = json!({"prompt": "text"});
    assert_eq!(
        instruction(not_an_array.as_object()),
        Err(PromptFault::NotAnArray),
        "a prompt that is not an array"
    );
    // Each fault's text is the -32602 message a client reads.
    for (fault, text) in [
        (
            PromptFault::NotAnArray,
            "prompt must be an array of content blocks",
        ),
        (PromptFault::Empty, "prompt must not be empty"),
        (
            PromptFault::Unsupported(String::from("image")),
            "unsupported content block: image",
        ),
        (
            PromptFault::MissingText,
            "a content block is missing its text",
        ),
    ] {
        assert_eq!(fault.to_string(), text, "{fault:?}");
    }
    Ok(())
}

/// Weird ids: a request's id is valid exactly when it is a string (any bytes) or
/// a JSON integer, extremes included; a float, null, bool or container is refused
/// with a null id.
#[test]
fn sim_acp_weird_bytes_ids_are_valid_exactly_when_string_or_integer() -> SimResult {
    let ids: [(&str, bool); 12] = [
        ("-9223372036854775808", true),
        ("9223372036854775807", true),
        ("18446744073709551615", true),
        // JSON reads `-0` as the float -0.0 (serde_json's rule), so it is no integer.
        ("-0", false),
        ("0", true),
        ("1.5", false),
        ("-0.0", false),
        ("1e3", false),
        ("null", false),
        ("true", false),
        ("[]", false),
        ("{}", false),
    ];
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed.wrapping_add(3 << 40));
        let (literal, valid) = if rng.below(3)? == 0 {
            (json::to_string(&weird(&mut rng, 6)?)?, true)
        } else {
            let (text, valid) = *ids.get(rng.index(ids.len())?).ok_or("ids")?;
            (text.to_owned(), valid)
        };
        let frame = format!(r#"{{"jsonrpc":"2.0","id":{literal},"method":"m"}}"#);
        let object = wire::object(frame.as_bytes()).map_err(|reply| reply.to_string())?;
        let parsed: Value = json::from_str(&literal)?;
        match wire::envelope(&object) {
            Ok(wire::Envelope::Request { id, method, .. }) => {
                assert!(valid, "seed {seed}: {literal} admitted");
                assert_eq!((id, method), (&parsed, "m"), "seed {seed}: the id kept");
            }
            Err(Some(reply)) => {
                assert!(!valid, "seed {seed}: {literal} refused");
                assert_eq!(reply["id"], Value::Null, "seed {seed}: null id");
                assert_eq!(
                    reply["error"]["code"], -32600,
                    "seed {seed}: invalid request"
                );
            }
            other => return Err(format!("seed {seed}: {literal} gave {other:?}").into()),
        }
    }
    Ok(())
}
