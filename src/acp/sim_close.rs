//! Seeded simulations of `session/close` and the session bound (larry#411), against a reference
//! written from the Agent Client Protocol v1: `session/close` (stabilized 2026-04-23) stops the
//! session's work as `session/cancel` would, frees its resources, and answers an empty result,
//! the same for a session that is unknown or already closed; the agent advertises it as
//! `agentCapabilities.sessionCapabilities.close`. Requirement: an editor that keeps one
//! `larry acp` and opens and closes chats past 16 is never refused while fewer than 16 are live.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use lgwks_std::json::{Value, json};
use lgwks_std::seeded::Seeded;

use super::super::{Agent, Book, locked, mark};
use super::{Reference, SEEDS, SimResult, id_and_code, live, request_line, serve, stops, tmp};
use crate::executor::ExecError;
use crate::json_at::At;

/// The bound on live sessions one process holds.
const BOUND: usize = 16;
/// Random frames each seed sends after the bound is reached and one slot freed.
const STEPS: u64 = 96;

/// The reference: the frames sent, the answer each is owed, and which sessions are live.
#[derive(Default)]
struct Editor {
    /// Every frame, in order.
    frames: Vec<Vec<u8>>,
    /// Each frame's reply id and error code (`None`: a result).
    owed: Vec<(Value, Option<i64>)>,
    /// The live sessions' numbers.
    live: BTreeSet<u64>,
    /// Sessions opened so far: the next one's number.
    opened: u64,
    /// Closes that name a session, each one log line from the reader.
    closes: usize,
}

impl Editor {
    /// Sends one request under the next id, owed `code`.
    fn send(&mut self, method: &str, params: &Value, code: Option<i64>) {
        let id = json!(self.owed.len());
        self.frames
            .push(request_line(&id, method, params).into_bytes());
        self.owed.push((id, code));
    }

    /// A `session/new`: a session while fewer than 16 are live, else -32600.
    fn open(&mut self, cwd: &str) {
        let code = if self.live.len() < BOUND {
            self.live.insert(self.opened);
            self.opened = self.opened.saturating_add(1);
            None
        } else {
            Some(-32600)
        };
        self.send("session/new", &json!({"cwd": cwd, "mcpServers": []}), code);
    }

    /// A `session/close` of the agent's `n`th session: an empty result, live or not.
    fn close(&mut self, n: u64) {
        self.live.remove(&n);
        self.closes = self.closes.saturating_add(1);
        let params = json!({"sessionId": Reference::session(n)});
        self.send("session/close", &params, None);
    }
}

/// An editor opens 16 chats, is refused a 17th, closes one and opens it, then opens, closes
/// and re-closes chats in seeded order: every `session/new` is admitted exactly while fewer
/// than 16 sessions are live, every close of a live, closed or unknown session gets an empty
/// result, a close that names no session is -32602, and `initialize` advertises the method.
/// Catches a book that never frees a slot, a close that frees two, and a close refused for
/// a session already gone.
#[test]
fn sim_acp_closed_sessions_free_their_slots_past_the_bound() -> SimResult {
    let cwd = tmp()?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed.wrapping_add(11 << 40));
        let mut editor = Editor::default();
        editor.send("initialize", &json!({"protocolVersion": 1}), None);
        for _ in 0..=BOUND {
            editor.open(&cwd);
        }
        let freed = *editor.live.first().ok_or("a live session")?;
        editor.close(freed);
        editor.open(&cwd);
        for _ in 0..STEPS {
            match rng.below(10)? {
                0..=3 => editor.open(&cwd),
                4..=6 if !editor.live.is_empty() => {
                    let pick = rng.index(editor.live.len())?;
                    let n = *editor.live.iter().nth(pick).ok_or("a live session")?;
                    editor.close(n);
                }
                9 => editor.send("session/close", &json!({}), Some(-32602)),
                // A session already closed, or one never opened: the same empty result.
                _ => editor.close(rng.below(editor.opened.saturating_add(4))?),
            }
        }
        let (lines, log) = serve(&editor.frames, 4096)?;
        let first = lines.first().ok_or("an initialize reply")?;
        assert_eq!(
            *first.at("/result/agentCapabilities/sessionCapabilities/close")?,
            json!({}),
            "seed {seed}: session/close is advertised"
        );
        let got = lines
            .iter()
            .map(id_and_code)
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(
            got, editor.owed,
            "seed {seed}: every frame's answer, in order"
        );
        assert!(
            editor.opened > u64::try_from(BOUND)?,
            "seed {seed}: the process opened past the bound"
        );
        let logged = log
            .lines()
            .filter(|line| line.ends_with(" close requested"))
            .count();
        assert_eq!(
            logged, editor.closes,
            "seed {seed}: the reader logs each close that names a session"
        );
    }
    Ok(())
}

/// A close binds to its session as the reader reads it: the running prompt and every prompt
/// queued behind it stop as cancelled, another session runs on, and the agent's answer then
/// removes the session from the book with an empty result. Catches a close that waits for
/// the running prompt to finish its whole bound, or one that stops another session.
#[test]
fn sim_acp_a_close_stops_the_running_prompt_and_every_queued_one() -> SimResult {
    let cwd = tmp()?;
    let mut running = live(&cwd);
    running.prompts = 3;
    running.started = 1;
    let book: Book = Mutex::new(BTreeMap::from([
        (String::from("closing"), running),
        (String::from("other"), live(&cwd)),
    ]));
    let frame = request_line(&json!(4), "session/close", &json!({"sessionId": "closing"}));
    let mut log = Vec::new();
    mark(frame.as_bytes(), &book, &mut log)?;
    for prompt_no in 1..=3 {
        assert!(
            stops(&book, "closing", prompt_no),
            "prompt {prompt_no} stops"
        );
    }
    assert!(!stops(&book, "other", 1), "another session runs on");
    assert_eq!(
        String::from_utf8(log)?,
        "larry acp: session closing close requested\n",
        "the reader acknowledges the close"
    );
    let settings = Err(ExecError::StateDirUnset);
    let agent = Agent {
        settings: &settings,
        book: &book,
        initialized: true,
        opened: 2,
    };
    let params = json!({"sessionId": "closing"});
    let reply = agent.close_session(&json!(4), params.as_object());
    assert_eq!(*reply.at("/result")?, json!({}), "an empty result");
    let left: Vec<String> = locked(&book).keys().cloned().collect();
    assert_eq!(left, ["other"], "the closed session left the book");
    Ok(())
}

/// Sixteen sessions the reader abandoned at end of input keep their slots, so a `session/new`
/// sent before that end is refused by stream order, never admitted because the reader reached
/// the end first. Catches a bound whose answer depends on which thread ran first.
#[test]
fn sim_acp_an_abandoned_session_keeps_its_slot_in_stream_order() -> SimResult {
    let cwd = tmp()?;
    let mut entries = BTreeMap::new();
    for n in 0..BOUND {
        let mut gone = live(&cwd);
        gone.abandoned = true;
        entries.insert(format!("gone-{n}"), gone);
    }
    let book: Book = Mutex::new(entries);
    let settings = Err(ExecError::StateDirUnset);
    let mut agent = Agent {
        settings: &settings,
        book: &book,
        initialized: true,
        opened: 16,
    };
    let params = json!({"cwd": cwd, "mcpServers": []});
    let reply = agent.new_session(&json!(1), params.as_object());
    assert_eq!(
        reply.member("/error/code").and_then(Value::as_i64),
        Some(-32600),
        "the 17th session is refused: {reply}"
    );
    assert_eq!(
        locked(&book).len(),
        BOUND,
        "every abandoned session keeps its slot"
    );
    Ok(())
}
