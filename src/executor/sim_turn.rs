//! Seeded-shape simulations of a terminal session's turn loop (larry#586):
//! a declined command lets the turn end instead of cancelling it, an approved
//! command passes on exit zero, and an approved read shows the file's digest.
//! Each test drives the real [`Turn`] against a stub gateway on a loopback
//! socket, with a scripted observer answering for the person, so the loop's
//! stops and records are what the shipped binary seals. Source: the session
//! turn loop's contract that every step runs only after the person approves,
//! and a decline is an answer, never an interruption.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use super::super::run::Place;
use super::super::session::{Approval, Observer, Stop};
use super::super::sim_tree::scratch;
use super::super::{Ledger, Settings};
use super::Turn;
use crate::model::{ModelClient, ModelConfig};

/// What every simulation returns.
type TestResult<Outcome = ()> = Result<Outcome, Box<dyn std::error::Error + Send + Sync>>;

/// One scripted gateway reply.
type Reply = String;

/// A loopback OpenAI-compatible gateway: the test binds it, serves the
/// scripted replies on a scoped thread, and drops it once the run returned.
/// The server thread always exits: after the run the test sets `done` and
/// opens one more connection, which wakes the one accept the server may
/// still hold; the server reads EOF, answers no one, and exits, so no thread
/// outlives the test even when the run under test makes fewer requests than
/// scripted.
struct StubGateway {
    /// Where the client posts its completions.
    url: String,
    /// Replies served so far.
    served: Arc<AtomicUsize>,
    /// Set once the run returned, so the server stops after its wake-up.
    done: Arc<AtomicBool>,
    /// The listener, dropped with the gateway.
    listener: TcpListener,
}

impl StubGateway {
    /// Binds a loopback socket for the scripted replies.
    fn bind() -> TestResult<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}", listener.local_addr()?);
        Ok(Self {
            url,
            served: Arc::new(AtomicUsize::new(0)),
            done: Arc::new(AtomicBool::new(false)),
            listener,
        })
    }

    /// Serves `replies` in order on a scoped thread, then says how many
    /// replies it served while `run` drove the turn: the count the run's
    /// stops are asserted against. The thread is scoped, so it is joined
    /// before this function returns.
    fn serve_while<R>(
        &self,
        replies: Vec<Reply>,
        run: impl FnOnce() -> TestResult<R>,
    ) -> TestResult<(R, usize)> {
        let outcome = std::thread::scope(|owner| {
            let server = owner.spawn(|| {
                for reply in replies {
                    let Ok((stream, _)) = self.listener.accept() else {
                        break;
                    };
                    if self.done.load(Ordering::SeqCst) {
                        break;
                    }
                    if respond(&stream, &reply).is_err() {
                        break;
                    }
                    self.served.fetch_add(1, Ordering::SeqCst);
                }
            });
            let outcome = run();
            self.done.store(true, Ordering::SeqCst);
            let wake = TcpStream::connect(self.listener.local_addr()?)?;
            wake.shutdown(Shutdown::Both)?;
            match server.join() {
                Ok(()) => {}
                Err(panic) => return Err(format!("the gateway thread panicked: {panic:?}").into()),
            }
            outcome
        });
        let outcome = outcome?;
        Ok((outcome, self.served.load(Ordering::SeqCst)))
    }
}

/// Reads one HTTP request off `stream` and answers it with `reply` as the
/// completion's content.
fn respond(stream: &TcpStream, reply: &str) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut content_length = 0_usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
            && let Ok(length) = value.trim().parse::<usize>()
        {
            content_length = length;
        }
    }
    let mut body = vec![0_u8; content_length];
    use std::io::Read as _;
    reader.read_exact(&mut body)?;
    drop(reader);
    let document = lgwks_std::json::json!({
        "choices": [{"message": {"content": reply}, "finish_reason": "stop"}]
    })
    .to_string();
    let mut stream = stream.try_clone()?;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        document.len(),
        document
    )?;
    stream.flush()?;
    Ok(())
}

/// The person's answers, scripted: each approval is spent in order, every
/// other observation is recorded for the assertions.
struct Scripted {
    /// The approvals the run spends, in order.
    approvals: VecDeque<Approval>,
    /// Whether each finished step passed, in order.
    passed: Vec<bool>,
    /// What the model is shown for each finished step, in order.
    observations: Vec<String>,
    /// What the session said to the person, in order.
    messages: Vec<String>,
}

impl Scripted {
    /// Answers approvals from `approvals`, in order.
    fn new(approvals: Vec<Approval>) -> Self {
        Self {
            approvals: approvals.into(),
            passed: Vec::new(),
            observations: Vec::new(),
            messages: Vec::new(),
        }
    }
}

impl Observer for Scripted {
    fn approve(&mut self, _step: usize, _command: &str) -> Approval {
        match self.approvals.pop_front() {
            Some(approval) => approval,
            // An exhausted script declines: nothing runs, and the turn's
            // stop exposes the script bug instead of running unscripted work.
            None => Approval::Decline,
        }
    }

    fn finished(&mut self, _step: usize, observation: &str, passed: bool) {
        self.observations.push(observation.to_owned());
        self.passed.push(passed);
    }

    fn message(&mut self, text: &str) {
        self.messages.push(text.to_owned());
    }
}

/// The table the settings and the model read: the state directory and PATH
/// the session needs, and the stub gateway's key, URL and model name.
fn table(state: &str, url: &str) -> Vec<(String, String)> {
    vec![
        (String::from("LARRY_STATE_DIR"), state.to_owned()),
        (String::from("PATH"), String::from("/usr/bin:/bin")),
        (String::from("LARRY_MODEL_KEY"), String::from("sim-turn")),
        (String::from("LARRY_MODEL_URL"), url.to_owned()),
        (String::from("LARRY_MODEL"), String::from("sim-turn")),
    ]
}

/// Runs one turn of `replies` against the stub gateway, the person answering
/// from `approvals`, in a fresh state directory over `work`.
fn drive(
    label: &str,
    replies: Vec<Reply>,
    approvals: Vec<Approval>,
    setup: impl FnOnce(&std::path::Path) -> TestResult<()>,
) -> TestResult<(super::Steps, Scripted, usize)> {
    let root = scratch(label)?;
    let state = root.join("state");
    let work = root.join("work");
    std::fs::create_dir_all(&work)?;
    setup(&work)?;
    let gateway = StubGateway::bind()?;
    let environment = table(
        state.to_str().ok_or("the state directory is not text")?,
        &gateway.url,
    );
    let settings = Settings::from_env({
        let environment = environment.clone();
        move |name: &str| {
            environment
                .iter()
                .find(|entry| entry.0.as_str() == name)
                .map(|entry| entry.1.clone())
        }
    })?;
    let client = ModelClient::new(
        ModelConfig::from_env(move |name: &str| {
            environment
                .iter()
                .find(|entry| entry.0.as_str() == name)
                .map(|entry| entry.1.clone())
        })?,
        larry_runtime::live_clock().map_err(|cause| format!("clock: {cause}"))?,
        |_| false,
    );
    let ledger = Ledger::open(&state)?;
    let mut observer = Scripted::new(approvals);
    let mut turn = Turn {
        client: &client,
        settings: &settings,
        ledger: &ledger,
        task_id: "sim-turn",
        place: Place::Open(&work),
        shown: String::from("."),
        system: String::from(super::SYSTEM),
        observer: &mut observer,
    };
    let (steps, served) = gateway.serve_while(replies, || Ok(turn.run("a simulated task")?))?;
    Ok((steps, observer, served))
}

/// A declined command lets the turn end instead of cancelling it: the model
/// is asked again, says done, and the stop is `EndTurn` with one decline and
/// no interruption.
#[test]
fn sim_a_declined_command_lets_the_turn_end() -> TestResult<()> {
    let replies = vec![
        Reply::from("{\"command\":\"echo hi\"}"),
        Reply::from("{\"done\":\"left it alone\"}"),
    ];
    let (steps, _, served) = drive("sim-turn-decline", replies, vec![Approval::Decline], |_| {
        Ok(())
    })?;
    assert_eq!(served, 2, "the model is asked again after the decline");
    assert!(
        matches!(steps.stop, Stop::EndTurn),
        "a decline ends the turn, never cancels it"
    );
    assert_eq!(steps.tally.declined, 1, "one decline is counted");
    assert_eq!(
        steps.tally.interrupted, 0,
        "a decline is never an interruption"
    );
    assert!(
        steps.tally.results.is_empty(),
        "a declined command never runs"
    );
    Ok(())
}

/// An approved command that exits zero passes its observation: the report
/// keeps the model's view of the run, and the pass flag says it succeeded.
#[test]
fn sim_an_approved_command_passes_its_observation_on_exit_zero() -> TestResult<()> {
    let replies = vec![
        Reply::from("{\"command\":\"echo hi\"}"),
        Reply::from("{\"done\":\"said hi\"}"),
    ];
    let (steps, observer, served) =
        drive("sim-turn-command", replies, vec![Approval::Run], |_| Ok(()))?;
    assert_eq!(served, 2, "the command, then the summary");
    assert!(
        matches!(steps.stop, Stop::EndTurn),
        "the turn ends at the model's done"
    );
    assert_eq!(steps.tally.results.len(), 1, "one command is recorded");
    assert_eq!(
        observer.passed,
        vec![true],
        "an exit-zero command passes its observation"
    );
    Ok(())
}

/// An approved read records the file's digest, which the model is shown; a read
/// of a path outside the tree is a typed refusal, and a declined read runs not
/// at all. A long file is cut for the model but digested whole.
#[test]
fn sim_an_approved_read_shows_the_digest() -> TestResult<()> {
    let before = "pub fn origin() {}\n";
    let replies = vec![
        Reply::from("{\"read\":\"lib.rs\"}"),
        Reply::from("{\"read\":\"../outside\"}"),
        Reply::from("{\"read\":\"lib.rs\"}"),
        Reply::from("{\"done\":\"read it\"}"),
    ];
    let approvals = vec![Approval::Run, Approval::Run, Approval::Decline];
    let (steps, observer, served) = drive("sim-turn-read", replies, approvals, |work| {
        std::fs::write(work.join("lib.rs"), before)?;
        Ok(())
    })?;
    assert_eq!(served, 4, "two reads, a decline, then the summary");
    assert_eq!(steps.tally.changes.len(), 2, "two reads ran");
    assert_eq!(steps.tally.declined, 1, "the third read was declined");
    assert_eq!(observer.passed, vec![true, false], "each read ends once");
    let digest = lgwks_std::hash::blake3(before.as_bytes()).to_hex();
    assert_eq!(
        observer.observations.first(),
        Some(&format!("read\ndigest {digest} (19 bytes)\n{before}")),
        "the model is shown the digest, the length and the file"
    );
    assert_eq!(
        steps
            .tally
            .changes
            .get(1)
            .and_then(|record| record.get("step")),
        Some(&lgwks_std::json::json!(1)),
        "each read takes the next step number"
    );
    let first = steps
        .tally
        .changes
        .first()
        .ok_or("the read earns a record")?;
    assert_eq!(
        first.get("digest").and_then(lgwks_std::json::Value::as_str),
        Some(lgwks_std::hash::blake3(before.as_bytes()).to_hex().as_str()),
        "the record carries the file's digest"
    );
    let second = steps
        .tally
        .changes
        .get(1)
        .and_then(|record| record.get("refusal"))
        .and_then(lgwks_std::json::Value::as_str)
        .ok_or("the second read is refused")?;
    assert_eq!(second, "edit path ../outside escapes the snapshot");
    Ok(())
}
