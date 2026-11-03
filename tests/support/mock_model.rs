//! Seeded OpenAI-compatible mock model endpoint (larry#82), shared by the
//! tests and benchmarks of the model client (#83), the MCP server (#84) and the
//! harness benchmark (#72), so none of them makes a paid model call.
//!
//! `POST /v1/chat/completions` (JSON, non-streaming: the forge client sends no
//! `stream` key, and `"stream": true` is refused with a 400). One seed fixes
//! every response: the reply for request number `n` is a pure function of the
//! seed, `n` and the request body, so the same seed and the same request
//! sequence give byte-identical responses and the same trace hash. The trace
//! hash is a BLAKE3 chain over every request body and every planned response.
//!
//! Faults, scripted per request index or drawn from the seed at a per-mille
//! rate: a delay that blocks on a gate the test releases (no wall-clock
//! sleep), HTTP 429 with `Retry-After`, an unparseable 200 body, a body
//! truncated against its declared length, and 5xx. Bounds: a request-body cap
//! (413) and a concurrent-connection cap (503). The server runs inside the
//! caller's `std::thread::scope` on `127.0.0.1:0`, so every thread is joined.
//!
//! Use from a test target with `#[path = "support/http_wire.rs"] mod http_wire;`
//! and `#[path = "support/mock_model.rs"] mod mock_model;` (no `mod.rs`, so
//! cargo never mistakes the support directory for a test target).

use std::io::{self, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::num::TryFromIntError;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, Scope};
use std::time::Duration;

use lgwks_std::hash::Hasher;
use lgwks_std::json::{self, Value, json};
use lgwks_std::seeded::{DrawError, Seeded};

use crate::http_wire::{Refusal, Request, read_request, render};

/// Socket read and write bound for every connection.
const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// Largest declared body the mock drains after refusing it with a 413, so the
/// refusal is not lost to a connection reset.
const DRAIN_LIMIT: usize = 65_536;
/// How many request bodies the mock keeps for `requests()`.
const MAX_RECORDED: usize = 256;
/// Words the mock composes replies from.
const VOCAB: [&str; 16] = [
    "plan", "patch", "verify", "stage", "commit", "receipt", "journal", "tenant", "fence", "gate",
    "seed", "replay", "intent", "grant", "owner", "tick",
];
/// Statuses a drawn 5xx fault picks from.
const SERVER_ERRORS: [u16; 4] = [500, 502, 503, 504];

/// One injected failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fault {
    /// Hold the reply until the test calls [`Handle::release_delayed`].
    Delay,
    /// Answer 429 with a `Retry-After` header.
    RateLimit {
        /// Seconds the `Retry-After` header carries.
        retry_after_secs: u32,
    },
    /// Answer 200 with a body that is not JSON.
    MalformedJson,
    /// Declare the full length, send half the body, close.
    TruncatedBody,
    /// Answer a 5xx status.
    ServerError {
        /// The 5xx status code.
        status: u16,
    },
}

/// What one mock server does.
#[derive(Debug, Clone)]
pub(crate) struct Config {
    /// The seed that fixes every response and the trace hash.
    pub(crate) seed: u64,
    /// Largest accepted request body, in bytes; more is refused with 413.
    pub(crate) max_body_bytes: usize,
    /// Most concurrently handled connections; more is refused with 503.
    pub(crate) max_connections: usize,
    /// Fault for request index `n` (`None` is a clean reply); indexes past the
    /// end draw from the seed at `fault_per_mille`.
    pub(crate) script: Vec<Option<Fault>>,
    /// Chance in 1000 that an unscripted request draws a fault.
    pub(crate) fault_per_mille: u32,
}

impl Config {
    /// Fault-free defaults: 64 KiB body cap, 8 connections.
    pub(crate) fn seeded(seed: u64) -> Self {
        Self {
            seed,
            max_body_bytes: 65_536,
            max_connections: 8,
            script: Vec::new(),
            fault_per_mille: 0,
        }
    }
}

/// An internal failure composing a reply; answered as a 500, never a panic.
#[derive(Debug)]
enum Failure {
    /// A bounded draw had no value.
    Draw(DrawError),
    /// A reply failed to encode.
    Encode(json::Error),
    /// A count did not fit its target type.
    Number(TryFromIntError),
    /// A fixed table lacked an entry.
    Table,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::Draw(ref cause) => write!(out, "draw failed: {cause}"),
            Self::Encode(ref cause) => write!(out, "encode failed: {cause}"),
            Self::Number(ref cause) => write!(out, "count out of range: {cause}"),
            Self::Table => out.write_str("fixed table entry missing"),
        }
    }
}

impl From<DrawError> for Failure {
    fn from(cause: DrawError) -> Self {
        Self::Draw(cause)
    }
}

impl From<json::Error> for Failure {
    fn from(cause: json::Error) -> Self {
        Self::Encode(cause)
    }
}

impl From<TryFromIntError> for Failure {
    fn from(cause: TryFromIntError) -> Self {
        Self::Number(cause)
    }
}

/// The response the mock committed to for one request.
struct Planned {
    /// HTTP status.
    status: u16,
    /// Extra headers.
    headers: Vec<(&'static str, String)>,
    /// Body bytes actually sent.
    body: Vec<u8>,
    /// Body length the head declares.
    declared_len: usize,
    /// Whether delivery waits on the gate.
    delayed: bool,
}

impl Planned {
    /// A reply that declares exactly the body it carries.
    fn whole(status: u16, headers: Vec<(&'static str, String)>, body: Vec<u8>) -> Self {
        Self {
            status,
            headers,
            declared_len: body.len(),
            body,
            delayed: false,
        }
    }
}

/// Arrival-ordered record of what the server saw and answered.
struct Ledger {
    /// Requests planned so far; also the next request's index.
    served: u64,
    /// Chain over every request and planned response.
    trace: Hasher,
    /// The first [`MAX_RECORDED`] request bodies.
    bodies: Vec<Vec<u8>>,
}

/// Shared server state.
struct State {
    /// The configuration.
    config: Config,
    /// Derived from the seed; the base every per-request stream hangs from.
    base: u64,
    /// Set by shutdown.
    stop: AtomicBool,
    /// Connections currently being handled.
    active: AtomicUsize,
    /// The request ledger.
    ledger: Mutex<Ledger>,
    /// Releases delayed replies; dropped on shutdown.
    gate_tx: Mutex<Option<SyncSender<()>>>,
    /// Delayed replies wait here.
    gate_rx: Mutex<Receiver<()>>,
    /// The bound address, used to wake the accept loop.
    addr: SocketAddr,
}

impl std::fmt::Debug for State {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.debug_struct("State")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

/// Lock, recovering the guard if a handler thread panicked.
fn locked<Item>(mutex: &Mutex<Item>) -> MutexGuard<'_, Item> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Drop the outcome of a best-effort socket operation.
fn settle<Done, Fail>(outcome: Result<Done, Fail>) {
    match outcome {
        Ok(_) | Err(_) => {}
    }
}

/// An OpenAI-shaped error body.
fn error_body(message: &str, kind: &str) -> Result<Vec<u8>, Failure> {
    let value = json!({ "error": { "message": message, "type": kind } });
    Ok(json::to_vec(&value)?)
}

/// A validation refusal.
fn reject(status: u16, message: &str) -> Result<Planned, Failure> {
    let body = error_body(message, "invalid_request_error")?;
    Ok(Planned::whole(status, Vec::new(), body))
}

/// The first eight bytes of the body's BLAKE3 digest, as a word.
fn fold(request: &Request) -> u64 {
    let mut hasher = Hasher::new();
    hasher.write_framed(&request.body);
    let [b0, b1, b2, b3, b4, b5, b6, b7, ..] = *hasher.finalize().as_bytes();
    u64::from_le_bytes([b0, b1, b2, b3, b4, b5, b6, b7])
}

/// A well-formed chat completion for `model`.
fn completion(model: &str, request: &Request, rng: &mut Seeded) -> Result<Vec<u8>, Failure> {
    let count = usize::try_from(rng.below(10)?.saturating_add(3))?;
    let mut words = Vec::with_capacity(count);
    for _ in 0..count {
        words.push(*VOCAB.get(rng.index(VOCAB.len())?).ok_or(Failure::Table)?);
    }
    let prompt = u64::try_from(request.body.len().div_ceil(4))?;
    let produced = u64::try_from(count)?;
    let value = json!({
        "id": format!("chatcmpl-{:016x}", rng.next_u64()),
        "object": "chat.completion",
        "created": 0,
        "model": model,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": words.join(" ") },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": prompt,
            "completion_tokens": produced,
            "total_tokens": prompt.saturating_add(produced)
        }
    });
    Ok(json::to_vec(&value)?)
}

impl State {
    /// Choose the fault for request `index`: the script first, then the seed.
    fn fault_for(&self, index: u64, rng: &mut Seeded) -> Result<Option<Fault>, Failure> {
        let roll = rng.below(1000)?;
        let kind = rng.below(4)?;
        let pick = rng.below(4)?;
        if let Some(&scripted) = self.config.script.get(usize::try_from(index)?) {
            return Ok(scripted);
        }
        if roll >= u64::from(self.config.fault_per_mille) {
            return Ok(None);
        }
        // A drawn fault never delays: a delay holds the client until the test
        // releases the gate, so it is scripted, never rolled.
        Ok(Some(match kind {
            0 => Fault::RateLimit {
                retry_after_secs: u32::try_from(pick)?.saturating_add(1),
            },
            1 => Fault::MalformedJson,
            2 => Fault::TruncatedBody,
            _ => Fault::ServerError {
                status: *SERVER_ERRORS
                    .get(usize::try_from(pick)?)
                    .ok_or(Failure::Table)?,
            },
        }))
    }

    /// Validate the request and compose the reply with its fault applied.
    fn compose(&self, index: u64, request: &Request) -> Result<Planned, Failure> {
        let mut rng = Seeded::from_seed(self.base.wrapping_add(index).wrapping_add(fold(request)));
        let fault = self.fault_for(index, &mut rng)?;
        if request.path != "/v1/chat/completions" {
            return reject(404, "no such route");
        }
        if !request.method.eq_ignore_ascii_case("POST") {
            return reject(405, "POST only");
        }
        let Ok(body) = json::from_slice::<Value>(&request.body) else {
            return reject(400, "request body is not JSON");
        };
        let Some(model) = body["model"].as_str() else {
            return reject(400, "model is required");
        };
        match body["messages"].as_array() {
            None => return reject(400, "messages is required"),
            Some(messages) if messages.is_empty() => return reject(400, "messages is empty"),
            Some(_) => {}
        }
        if body["stream"].as_bool() == Some(true) {
            return reject(400, "streaming is not supported");
        }
        let good = completion(model, request, &mut rng)?;
        apply(fault, good)
    }

    /// Take the next index, plan the reply, and chain both into the trace.
    fn record(&self, request: &Request) -> Result<Planned, Failure> {
        let mut ledger = locked(&self.ledger);
        let index = ledger.served;
        ledger.served = index.saturating_add(1);
        let planned = self.compose(index, request)?;
        ledger.trace.write_framed(&index.to_le_bytes());
        ledger.trace.write_framed(&request.body);
        ledger.trace.write_framed(&planned.status.to_le_bytes());
        ledger.trace.write_framed(&planned.body);
        ledger
            .trace
            .write_framed(&planned.declared_len.to_le_bytes());
        ledger.trace.write_framed(&[u8::from(planned.delayed)]);
        for header in &planned.headers {
            ledger.trace.write_framed(header.1.as_bytes());
        }
        if ledger.bodies.len() < MAX_RECORDED {
            ledger.bodies.push(request.body.clone());
        }
        Ok(planned)
    }

    /// Block until the test releases a delayed reply or the server stops.
    fn wait_gate(&self) {
        match locked(&self.gate_rx).recv() {
            Ok(()) | Err(_) => {}
        }
    }
}

/// Apply `fault` to a good reply body.
fn apply(fault: Option<Fault>, good: Vec<u8>) -> Result<Planned, Failure> {
    Ok(match fault {
        None => Planned::whole(200, Vec::new(), good),
        Some(Fault::Delay) => Planned {
            delayed: true,
            ..Planned::whole(200, Vec::new(), good)
        },
        Some(Fault::RateLimit { retry_after_secs }) => Planned::whole(
            429,
            vec![("Retry-After", retry_after_secs.to_string())],
            error_body("rate limit reached", "rate_limit_error")?,
        ),
        Some(Fault::MalformedJson) => Planned::whole(
            200,
            Vec::new(),
            String::from("{\"choices\":[{\"message\":<<not json>>").into_bytes(),
        ),
        Some(Fault::TruncatedBody) => {
            let declared_len = good.len();
            let cut = declared_len.checked_div(2).ok_or(Failure::Table)?;
            Planned {
                body: good.get(..cut).ok_or(Failure::Table)?.to_vec(),
                declared_len,
                ..Planned::whole(200, Vec::new(), Vec::new())
            }
        }
        Some(Fault::ServerError { status }) => Planned::whole(
            status,
            Vec::new(),
            error_body("upstream failure", "server_error")?,
        ),
    })
}

/// Write a transport-level refusal.
fn reply(stream: &mut TcpStream, status: u16, message: &str) {
    if let Ok(body) = error_body(message, "invalid_request_error") {
        settle(stream.write_all(&render(status, &[], &body, body.len())));
    }
}

/// Answer a connection past the cap: read its request (so closing does not
/// reset the reply away), then 503 with a retry hint. Never planned or traced.
fn turn_away(stream: &mut TcpStream) {
    if let Ok(body) = error_body("too many connections", "server_error") {
        let headers = [("Retry-After", String::from("1"))];
        settle(stream.write_all(&render(503, &headers, &body, body.len())));
    }
}

/// Serve one connection to completion; `admitted` is false past the cap.
fn handle(state: &State, mut stream: TcpStream, admitted: bool) {
    settle(stream.set_read_timeout(Some(IO_TIMEOUT)));
    settle(stream.set_write_timeout(Some(IO_TIMEOUT)));
    match read_request(&mut stream, state.config.max_body_bytes) {
        Ok(_) if !admitted => turn_away(&mut stream),
        Ok(request) => match state.record(&request) {
            Ok(planned) => {
                if planned.delayed {
                    state.wait_gate();
                }
                let bytes = render(
                    planned.status,
                    &planned.headers,
                    &planned.body,
                    planned.declared_len,
                );
                settle(stream.write_all(&bytes));
            }
            Err(cause) => reply(&mut stream, 500, &cause.to_string()),
        },
        Err(Refusal::TooLarge { declared }) => {
            reply(&mut stream, 413, "request body too large");
            settle(stream.shutdown(Shutdown::Write));
            if let (true, Ok(limit)) = (declared <= DRAIN_LIMIT, u64::try_from(declared)) {
                settle(io::copy(
                    &mut io::Read::take(&mut stream, limit),
                    &mut io::sink(),
                ));
            }
        }
        Err(Refusal::Head) => reply(&mut stream, 400, "malformed request head"),
        Err(Refusal::LengthRequired) => reply(&mut stream, 411, "Content-Length is required"),
        Err(Refusal::Closed | Refusal::Io) => {}
    }
    settle(stream.shutdown(Shutdown::Both));
}

/// Accept until shutdown, handling each admitted connection on its own scoped
/// thread and answering 503 past the connection cap.
fn accept_loop<'scope, 'env>(
    scope: &'scope Scope<'scope, 'env>,
    state: &Arc<State>,
    listener: &TcpListener,
) {
    let cap = state.config.max_connections;
    while let Ok((stream, _)) = listener.accept() {
        if state.stop.load(Ordering::SeqCst) {
            break;
        }
        let held = state.active.fetch_add(1, Ordering::SeqCst);
        // Past twice the cap a connection is dropped unanswered, so a flood
        // cannot grow the thread count without bound.
        if held >= cap.saturating_mul(2) {
            state.active.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let worker = Arc::clone(state);
        scope.spawn(move || {
            handle(&worker, stream, held < cap);
            worker.active.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

/// A running mock endpoint. Dropping it shuts the server down.
#[derive(Debug)]
pub(crate) struct Handle {
    /// Shared with the accept loop.
    state: Arc<State>,
}

impl Handle {
    /// The loopback address the server listens on.
    pub(crate) fn addr(&self) -> SocketAddr {
        self.state.addr
    }

    /// Hex BLAKE3 chain over every request and planned response so far.
    pub(crate) fn trace_hash(&self) -> String {
        locked(&self.state.ledger).trace.finalize().to_hex()
    }

    /// Requests planned so far.
    pub(crate) fn served(&self) -> u64 {
        locked(&self.state.ledger).served
    }

    /// The recorded request bodies, oldest first (at most 256).
    pub(crate) fn requests(&self) -> Vec<Vec<u8>> {
        locked(&self.state.ledger).bodies.clone()
    }

    /// Let one delayed reply through.
    pub(crate) fn release_delayed(&self) {
        if let Some(gate) = locked(&self.state.gate_tx).as_ref() {
            settle(gate.try_send(()));
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::SeqCst);
        *locked(&self.state.gate_tx) = None;
        settle(TcpStream::connect(self.state.addr));
    }
}

/// Bind `127.0.0.1:0`, serve inside `scope`, and return the handle.
fn start<'scope, 'env>(scope: &'scope Scope<'scope, 'env>, config: Config) -> io::Result<Handle> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let addr = listener.local_addr()?;
    let (gate_tx, gate_rx) = sync_channel(64);
    let base = Seeded::from_seed(config.seed).next_u64();
    let state = Arc::new(State {
        config,
        base,
        stop: AtomicBool::new(false),
        active: AtomicUsize::new(0),
        ledger: Mutex::new(Ledger {
            served: 0,
            trace: Hasher::new(),
            bodies: Vec::new(),
        }),
        gate_tx: Mutex::new(Some(gate_tx)),
        gate_rx: Mutex::new(gate_rx),
        addr,
    });
    let accepting = Arc::clone(&state);
    scope.spawn(move || accept_loop(scope, &accepting, &listener));
    Ok(Handle { state })
}

/// Run `body` against a fresh mock endpoint and join every server thread
/// before returning.
pub(crate) fn run<Out, Fail: From<io::Error>>(
    config: Config,
    body: impl FnOnce(&Handle) -> Result<Out, Fail>,
) -> Result<Out, Fail> {
    thread::scope(|scope| {
        let handle = start(scope, config)?;
        body(&handle)
    })
}
