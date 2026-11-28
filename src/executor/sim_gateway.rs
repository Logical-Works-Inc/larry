//! The two doubles every terminal session simulation shares (larry#586,
//! larry#566): the one loopback OpenAI-compatible gateway (completions, HTTP
//! refusals and dropped connections, served in order, every request body kept so
//! a test reads exactly what the model was sent) and the scripted person who
//! approves, declines or cancels.

use std::cell::Cell;
use std::collections::VecDeque;
use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};

use lgwks_std::json::{self, Value, json};

use super::session::{Approval, Observer};

/// What every simulation over this gateway returns.
pub(in crate::executor) type TestResult<Outcome = ()> =
    Result<Outcome, Box<dyn std::error::Error + Send + Sync>>;

/// One scripted answer of the gateway.
#[derive(Debug, Clone)]
pub(in crate::executor) enum Reply {
    /// A finished completion whose content is the text.
    Said(String),
    /// A finished completion that also reports what it spent.
    Spent {
        /// The completion's content.
        content: String,
        /// Prompt tokens.
        prompt: u64,
        /// Completion tokens.
        completion: u64,
    },
    /// The gateway refuses with this status and body.
    Refused {
        /// The HTTP status.
        status: u16,
        /// The response body.
        body: String,
    },
    /// The gateway reads the request and hangs up without a byte.
    Dropped,
}

impl From<&str> for Reply {
    fn from(content: &str) -> Self {
        Self::Said(content.to_owned())
    }
}

impl From<String> for Reply {
    fn from(content: String) -> Self {
        Self::Said(content)
    }
}

/// A loopback gateway: the test binds it, serves the scripted replies on a
/// scoped thread, and drops it once the run returned. The server thread always
/// exits: after the run the test sets `done` and opens one more connection,
/// which wakes the one accept the server holds, so no thread outlives the test.
pub(in crate::executor) struct StubGateway {
    /// Where the client posts its completions.
    url: String,
    /// Set once the run returned, so the server stops after its wake-up.
    done: AtomicBool,
    /// The listener, dropped with the gateway.
    listener: TcpListener,
}

impl StubGateway {
    /// Binds a loopback socket for the scripted replies.
    pub(in crate::executor) fn bind() -> TestResult<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}", listener.local_addr()?);
        Ok(Self {
            url,
            done: AtomicBool::new(false),
            listener,
        })
    }

    /// Where this gateway listens.
    pub(in crate::executor) fn url(&self) -> &str {
        &self.url
    }

    /// Serves `replies` in order on a scoped thread while `run` drives the code
    /// under test, and returns what it made with the body of every request that
    /// was answered. A request past the script is refused with a 400, so a run
    /// that asks too often fails at once instead of waiting out a timeout.
    pub(in crate::executor) fn serve_while<R>(
        &self,
        replies: Vec<Reply>,
        run: impl FnOnce() -> TestResult<R>,
    ) -> TestResult<(R, Vec<Value>)> {
        std::thread::scope(|owner| {
            let server = owner.spawn(|| {
                let exhausted = std::iter::repeat_with(|| Reply::Refused {
                    status: 400,
                    body: String::from("{\"error\":{\"message\":\"script exhausted\"}}"),
                });
                let mut seen = Vec::new();
                for reply in replies.into_iter().chain(exhausted) {
                    let Ok((stream, _)) = self.listener.accept() else {
                        break;
                    };
                    if self.done.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(request) = respond(&stream, &reply) else {
                        break;
                    };
                    seen.push(request);
                }
                seen
            });
            let outcome = run();
            self.done.store(true, Ordering::SeqCst);
            let wake = TcpStream::connect(self.listener.local_addr()?)?;
            wake.shutdown(Shutdown::Both)?;
            match server.join() {
                Ok(seen) => Ok((outcome?, seen)),
                Err(panic) => Err(format!("the gateway thread panicked: {panic:?}").into()),
            }
        })
    }
}

/// Reads one HTTP request off `stream`, answers it as `reply` says and returns
/// the request's JSON body.
fn respond(stream: &TcpStream, reply: &Reply) -> io::Result<Value> {
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
    reader.read_exact(&mut body)?;
    drop(reader);
    let request = json::from_slice::<Value>(&body)
        .map_err(|cause| io::Error::new(io::ErrorKind::InvalidData, cause.to_string()))?;
    let (status, document) = match *reply {
        Reply::Said(ref content) => (
            200,
            json!({"choices": [{"message": {"content": content}, "finish_reason": "stop"}]})
                .to_string(),
        ),
        Reply::Spent {
            ref content,
            prompt,
            completion,
        } => (
            200,
            json!({
                "choices": [{"message": {"content": content}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": prompt, "completion_tokens": completion},
            })
            .to_string(),
        ),
        Reply::Refused { status, ref body } => (status, body.clone()),
        Reply::Dropped => return Ok(request),
    };
    let mut stream = stream.try_clone()?;
    write!(
        stream,
        "HTTP/1.1 {status} Scripted\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{document}",
        document.len(),
    )?;
    stream.flush()?;
    Ok(request)
}

/// When a scripted person cancels the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::executor) enum Cancel {
    /// Never.
    Never,
    /// Once the session has asked more than this many times whether it is cancelled.
    AfterAsks(usize),
    /// While deciding on a step: the yes arrives with the cancel.
    OnApprove,
    /// While a command is starting, so the cancel lands mid-command.
    OnStart,
}

/// The person's answers, scripted: each approval is spent in order, every
/// other observation is recorded for the assertions.
#[derive(Debug)]
pub(in crate::executor) struct Scripted {
    /// The approvals the run spends, in order.
    approvals: VecDeque<Approval>,
    /// Whether each finished step passed, in order.
    pub(in crate::executor) passed: Vec<bool>,
    /// What the model is shown for each finished step, in order.
    pub(in crate::executor) observations: Vec<String>,
    /// What the session said to the person, in order.
    pub(in crate::executor) messages: Vec<String>,
    /// Every step that started, as it was described.
    pub(in crate::executor) started: Vec<String>,
    /// When this person cancels.
    cancel: Cancel,
    /// How often the session has asked whether the run is cancelled.
    asks: Cell<usize>,
    /// Set once the person has cancelled.
    cancelled: bool,
}

impl Scripted {
    /// Answers approvals from `approvals`, in order, and never cancels.
    pub(in crate::executor) fn new(approvals: Vec<Approval>) -> Self {
        Self::cancelling(approvals, Cancel::Never)
    }

    /// Answers approvals from `approvals` and cancels as `cancel` says.
    pub(in crate::executor) fn cancelling(approvals: Vec<Approval>, cancel: Cancel) -> Self {
        Self {
            approvals: approvals.into(),
            passed: Vec::new(),
            observations: Vec::new(),
            messages: Vec::new(),
            started: Vec::new(),
            cancel,
            asks: Cell::new(0),
            cancelled: false,
        }
    }
}

impl Observer for Scripted {
    fn approve(&mut self, _step: usize, _command: &str) -> Approval {
        if self.cancel == Cancel::OnApprove {
            self.cancelled = true;
        }
        match self.approvals.pop_front() {
            Some(approval) => approval,
            // An exhausted script declines: nothing runs, and the stop
            // exposes the script bug instead of running unscripted work.
            None => Approval::Decline,
        }
    }

    fn started(&mut self, _step: usize, command: &str) {
        self.started.push(command.to_owned());
        if self.cancel == Cancel::OnStart {
            self.cancelled = true;
        }
    }

    fn finished(&mut self, _step: usize, observation: &str, passed: bool) {
        self.observations.push(observation.to_owned());
        self.passed.push(passed);
    }

    fn message(&mut self, text: &str) {
        self.messages.push(text.to_owned());
    }

    fn cancelled(&self) -> bool {
        let asks = self.asks.get().saturating_add(1);
        self.asks.set(asks);
        self.cancelled || matches!(self.cancel, Cancel::AfterAsks(after) if asks > after)
    }
}
