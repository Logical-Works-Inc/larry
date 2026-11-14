//! The model client (#83): one blocking OpenAI-compatible chat-completions
//! client that the executor (#85) calls to run a task on a cheap model through
//! the route the task resolves to (#201): a gateway and a model the tenant owns
//! as data, never one compiled into the binary.
//!
//! Ported from forge-harness `sdk/forge-providers` (`openai_compatible.rs`,
//! `completion_body.rs`) onto `lgwks_std::http`, `retry` and `json`. Every
//! request is non-streaming and carries the route's model and temperature. Every
//! bound refuses with a typed error: 256 messages, the route's request body
//! bound (1 MiB unless the route names another, 16 MiB at most) and a 4 MiB reply.

mod client;
mod config;
mod route;
mod wire;

use std::fmt;
use std::num::{ParseFloatError, ParseIntError};
use std::str::Utf8Error;

pub use client::ModelClient;
pub use config::ModelConfig;
pub(crate) use config::env_route;
pub use route::RouteError;
pub(crate) use route::{Draft, Kind, MAX_ROUTES, Route, Table, is_route_id, resolve};

/// Most messages one completion request carries.
const MAX_MESSAGES: usize = 256;
/// Largest reply body the client reads, in bytes.
const MAX_REPLY_BYTES: usize = 4_194_304;

/// Why one completion failed. Every refusal is a typed variant; no catch-all exists.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModelError {
    /// The gateway did not answer within the configured timeout.
    Timeout,
    /// The gateway answered 429. `retry_after` is its hint in seconds, when it sent one.
    RateLimited {
        /// The `Retry-After` value in seconds, if the header was a whole number.
        retry_after: Option<u64>,
    },
    /// The gateway answered a 5xx status.
    Server {
        /// The status code.
        status: u16,
        /// The gateway's own reason, at most 512 characters, control characters replaced.
        detail: String,
    },
    /// The gateway answered with a status that is never retried: a 4xx other than
    /// 429, or a 3xx, which the client does not follow.
    Client {
        /// The status code.
        status: u16,
        /// The gateway's own reason, at most 512 characters, control characters replaced.
        detail: String,
    },
    /// The reply is not a completion document. The reason names what is missing.
    Malformed(&'static str),
    /// The reply body is not UTF-8.
    NotUtf8(Utf8Error),
    /// The connection failed or the reply could not be read to its end.
    Transport,
    /// No API key is configured, so nothing was sent.
    MissingKey,
    /// The variable a route names for its key is unset or empty, so nothing was
    /// sent. The name is the variable's, never the key's value.
    KeyUnset {
        /// The variable the route reads its key from.
        variable: String,
    },
    /// A `LARRY_MODEL_*` variable the environment route needs is unset.
    Unset(&'static str),
    /// The environment route fails its field checks.
    BadRoute(RouteError),
    /// A `LARRY_MODEL_*` whole-number setting does not parse.
    BadInteger {
        /// The variable that was set.
        variable: &'static str,
        /// Why it does not parse.
        cause: ParseIntError,
    },
    /// A `LARRY_MODEL_*` decimal setting does not parse.
    BadFloat {
        /// The variable that was set.
        variable: &'static str,
        /// Why it does not parse.
        cause: ParseFloatError,
    },
    /// A `LARRY_MODEL_*` setting parses but is outside what the client accepts.
    OutOfRange(&'static str),
    /// A request or reply exceeded its size bound. Nothing past the bound was read.
    BodyTooLarge,
}

impl fmt::Display for ModelError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Timeout => out.write_str("the model gateway timed out"),
            Self::RateLimited {
                retry_after: Some(seconds),
            } => write!(
                out,
                "the model gateway rate limited the request; retry after {seconds}s"
            ),
            Self::RateLimited { retry_after: None } => {
                out.write_str("the model gateway rate limited the request")
            }
            Self::Server { status, ref detail } => {
                write!(out, "the model gateway failed with status {status}")?;
                with_detail(out, detail)
            }
            Self::Client { status, ref detail } => {
                write!(out, "the model gateway refused with status {status}")?;
                with_detail(out, detail)
            }
            Self::Malformed(reason) => write!(out, "malformed model reply: {reason}"),
            Self::NotUtf8(ref cause) => write!(out, "the model reply is not UTF-8: {cause}"),
            Self::Transport => out.write_str("the model gateway could not be reached"),
            Self::MissingKey => out.write_str("no model API key is configured (LARRY_MODEL_KEY)"),
            Self::KeyUnset { ref variable } => {
                write!(out, "the route's key variable {variable} is unset or empty")
            }
            Self::Unset(variable) => write!(out, "{variable} is unset"),
            Self::BadRoute(ref cause) => write!(out, "the environment route: {cause}"),
            Self::BadInteger {
                variable,
                ref cause,
            } => write!(out, "{variable} is not a whole number: {cause}"),
            Self::BadFloat {
                variable,
                ref cause,
            } => write!(out, "{variable} is not a number: {cause}"),
            Self::OutOfRange(variable) => write!(out, "{variable} is out of range"),
            Self::BodyTooLarge => out.write_str("a model request or reply exceeded its size bound"),
        }
    }
}

/// Append the gateway's reason after a status, when it gave one.
fn with_detail(out: &mut fmt::Formatter<'_>, detail: &str) -> fmt::Result {
    if detail.is_empty() {
        return Ok(());
    }
    write!(out, ": {detail}")
}

impl std::error::Error for ModelError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::NotUtf8(ref cause) => Some(cause),
            Self::BadInteger { ref cause, .. } => Some(cause),
            Self::BadFloat { ref cause, .. } => Some(cause),
            Self::BadRoute(ref cause) => Some(cause),
            _ => None,
        }
    }
}

/// Who wrote one message in the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Role {
    /// Standing instructions.
    System,
    /// The caller's input.
    User,
    /// A prior model reply.
    Assistant,
}

impl Role {
    /// The role's name on the wire.
    const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

/// One conversation turn sent to the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Who wrote the turn.
    role: Role,
    /// The turn's text.
    content: String,
}

impl Message {
    /// A turn by `role` with `content`.
    #[must_use]
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }
}

/// The tokens one completion spent, as the gateway reported them (larry#87).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Usage {
    /// Tokens the request carried.
    pub(crate) prompt_tokens: u64,
    /// Tokens the reply carried.
    pub(crate) completion_tokens: u64,
}

/// The assistant's reply to one completion request.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Completion {
    /// The reply's text, from the first choice.
    content: String,
    /// The tokens spent, when the gateway reported both counts; never invented.
    usage: Option<Usage>,
}

impl Completion {
    /// The reply text.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    /// The tokens the gateway reported for this exchange, `None` when it did not.
    pub(crate) const fn usage(&self) -> Option<Usage> {
        self.usage
    }
}
