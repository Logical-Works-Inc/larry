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

use std::num::{ParseFloatError, ParseIntError};
use std::str::Utf8Error;

use lgwks_macros::Error;

pub use client::ModelClient;
pub(crate) use client::Request;
pub use config::ModelConfig;
pub(crate) use config::env_route;
#[cfg(test)]
pub(crate) use config::sim_config;
pub use route::RouteError;
#[cfg(test)]
pub(crate) use route::tenant_namespace;
pub(crate) use route::{Draft, Kind, MAX_ROUTES, Route, Table, is_route_id, resolve};

/// Most messages one completion request carries.
pub(crate) const MAX_MESSAGES: usize = 256;
/// `error.code` stems a gateway's structured refusal uses when a request is past
/// the model's context window (larry#236): OpenAI's `context_length_exceeded`
/// and a gateway's bare `context_length` both begin with this one, which no
/// other refusal code does (larry#295).
const CONTEXT_CODES: [&str; 1] = ["context_length"];
/// Phrases a gateway's 400 uses, lowercased, when a request is past the model's
/// context window (larry#235): OpenAI and vLLM ("maximum context length"),
/// Anthropic ("prompt is too long") and Gemini ("exceeds the maximum number of
/// tokens"). Read together with the codes: either one is enough, so a gateway
/// that translates its words but keeps its code, or sends no code, is still read
/// (larry#236, larry#295).
const CONTEXT_OVERFLOW: [&str; 4] = [
    "context length",
    "context window",
    "prompt is too long",
    "maximum number of tokens",
];
/// Words a refusal puts the refused request's own token count after (larry#236):
/// Anthropic, OpenAI, vLLM and Gemini.
const COUNTED_MARKERS: [&str; 4] = [
    "prompt is too long: ",
    "resulted in ",
    "you requested ",
    "input token count (",
];
/// Words a refusal puts the model's own limit after (larry#236): OpenAI and
/// vLLM, then Gemini's parenthesis. Anthropic's "N tokens > M maximum" is read
/// by [`number_before`].
const LIMIT_MARKERS: [&str; 2] = [
    "maximum context length is ",
    "maximum number of tokens allowed (",
];
/// Largest reply body the client reads, in bytes.
const MAX_REPLY_BYTES: usize = 4_194_304;

/// Why one completion failed. Every refusal is a typed variant; no catch-all exists.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ModelError {
    /// The gateway did not answer within the configured timeout.
    #[error("the model gateway timed out")]
    Timeout,
    /// The gateway answered 429. `retry_after` is its hint in seconds, when it sent one.
    #[error("the model gateway rate limited the request{}", retry_hint(*retry_after))]
    RateLimited {
        /// The `Retry-After` value in seconds, if the header was a whole number.
        retry_after: Option<u64>,
    },
    /// The gateway answered a 5xx status.
    #[error("the model gateway failed with status {status}{}", with_detail(detail))]
    Server {
        /// The status code.
        status: u16,
        /// The gateway's own reason, at most 512 characters, control characters replaced.
        detail: String,
    },
    /// The gateway answered with a status that is never retried: a 4xx other than
    /// 429, or a 3xx, which the client does not follow.
    #[error(
        "the model gateway refused with status {status}{}",
        with_detail(detail)
    )]
    Client {
        /// The status code.
        status: u16,
        /// The gateway's own reason, at most 512 characters, control characters replaced.
        detail: String,
    },
    /// The reply is not a completion document. The reason names what is missing.
    #[error("malformed model reply: {0}")]
    Malformed(&'static str),
    /// A `data:` chunk of a streamed reply is not JSON, so the content it carried is lost and
    /// the assembled reply is not the model's (larry#396).
    #[error("malformed model reply: the stream chunk on line {line} is not JSON (column {column})")]
    StreamChunk {
        /// The reply line the chunk is on, from 1.
        line: usize,
        /// The column of the chunk's payload where the JSON reader refused it.
        column: usize,
    },
    /// The model did not finish its reply: the gateway's `finish_reason` names a cut
    /// (`length`, `max_tokens`), a filter (`content_filter`, `SAFETY`) or another reason the
    /// client does not read as finished, so the content is never used (larry#396).
    #[error("the model did not finish its reply (finish_reason {reason})")]
    Unfinished {
        /// The gateway's reason, at most 64 characters, control characters replaced.
        reason: String,
    },
    /// The reply body is not UTF-8.
    #[error("the model reply is not UTF-8: {0}")]
    NotUtf8(#[source] Utf8Error),
    /// The connection failed, or the reply could not be read to its end. The estate client's
    /// error is kept whole: its stage (resolve, connect, TLS, send, body) and kind tell a bad
    /// host from a dead network, and its sanitized cause never carries the URL (larry#434).
    #[error("the model gateway could not be reached: {0}")]
    Transport(#[source] lgwks_std::http::Error),
    /// No API key is configured, so nothing was sent.
    #[error("no model API key is configured (LARRY_MODEL_KEY)")]
    MissingKey,
    /// The variable a route names for its key is unset or empty, so nothing was
    /// sent. The name is the variable's, never the key's value.
    #[error("the route's key variable {variable} is unset or empty")]
    KeyUnset {
        /// The variable the route reads its key from.
        variable: String,
    },
    /// A `LARRY_MODEL_*` variable the environment route needs is unset.
    #[error("{0} is unset")]
    Unset(&'static str),
    /// The environment route fails its field checks.
    #[error("the environment route: {0}")]
    BadRoute(#[source] RouteError),
    /// A `LARRY_MODEL_*` whole-number setting does not parse.
    #[error("{variable} is not a whole number: {cause}")]
    BadInteger {
        /// The variable that was set.
        variable: &'static str,
        /// Why it does not parse.
        #[source]
        cause: ParseIntError,
    },
    /// A `LARRY_MODEL_*` decimal setting does not parse.
    #[error("{variable} is not a number: {cause}")]
    BadFloat {
        /// The variable that was set.
        variable: &'static str,
        /// Why it does not parse.
        #[source]
        cause: ParseFloatError,
    },
    /// A `LARRY_MODEL_*` setting parses but is outside what the client accepts.
    #[error("{0} is out of range")]
    OutOfRange(&'static str),
    /// A request or reply exceeded its size bound. Nothing past the bound was read.
    #[error("a model request or reply exceeded its size bound")]
    BodyTooLarge,
}

/// Whether a member of a refusal's code names the context window: the stem itself, or the
/// stem followed by an underscore, so `context_length_exceeded` reads and
/// `max_context_length` does not (larry#295, larry#343).
fn names_context_code(member: &str) -> bool {
    CONTEXT_CODES.iter().any(|stem| {
        member == *stem
            || member
                .strip_prefix(*stem)
                .is_some_and(|rest| rest.starts_with('_'))
    })
}

impl ModelError {
    /// Whether the gateway refused the request as too large for the model: a 413,
    /// or a 400 whose `error.code` or reason names the context window (larry#235,
    /// larry#236). A terminal session compacts, shrinks its window to what the
    /// refusal names and sends again; nothing else retries on it.
    pub(crate) fn is_context_overflow(&self) -> bool {
        let Self::Client { status, ref detail } = *self else {
            return false;
        };
        match status {
            413 => true,
            400 => {
                // The wire puts `error.code` first (`refusal_detail`), so a code is
                // read only there: a message that merely mentions a parameter such
                // as `max_context_length` is no overflow, and a namespaced code is read
                // as one of its members, not as a prefix of the whole reason (larry#295,
                // larry#343).
                let reason = detail.to_ascii_lowercase();
                let mut code = reason.as_str();
                if let Some((head, _)) = reason.split_once(':') {
                    code = head;
                }
                code.split(|character: char| {
                    !(character.is_ascii_alphanumeric() || character == '_')
                })
                .any(names_context_code)
                    || CONTEXT_OVERFLOW
                        .iter()
                        .any(|phrase| reason.contains(phrase))
            }
            _ => false,
        }
    }

    /// The model's window as this refusal names it, in tokens; read it only after
    /// [`ModelError::is_context_overflow`] says the refusal is one (larry#236).
    /// The reason it reads is bounded to the 512 characters a refusal carries,
    /// so no hostile body reaches this parse whole.
    pub(crate) fn window(&self) -> WindowRefusal {
        let Self::Client { ref detail, .. } = *self else {
            return WindowRefusal::default();
        };
        WindowRefusal::parse(&detail.to_ascii_lowercase())
    }
}

/// The model's context window as a gateway's refusal names it, in tokens
/// (larry#236). Either number may be absent: vLLM names only the limit and a
/// gateway that gives no number says neither.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct WindowRefusal {
    /// Prompt tokens the gateway counted for the refused request, when it said.
    pub(crate) counted: Option<u64>,
    /// The model's limit, when the gateway said it.
    pub(crate) limit: Option<u64>,
}

impl WindowRefusal {
    /// The numbers a lowercased refusal `reason` names.
    fn parse(reason: &str) -> Self {
        let limit = match LIMIT_MARKERS
            .iter()
            .find_map(|marker| number_after(reason, marker))
        {
            Some(limit) => Some(limit),
            None => number_before(reason, " maximum"),
        };
        Self {
            counted: COUNTED_MARKERS
                .iter()
                .find_map(|marker| number_after(reason, marker)),
            limit,
        }
    }
}

/// The whole number that begins just after the spaces at byte `from`, checked:
/// digits that do not fit a `u64` are no number, never a wrapped one, and a
/// word where a number should be is no number either.
fn number_at(text: &str, from: usize) -> Option<u64> {
    let digits = text
        .get(from..)?
        .trim_start_matches(' ')
        .split(|character: char| !character.is_ascii_digit())
        .next()?;
    digits.parse::<u64>().ok()
}

/// The first whole number in `text` after `marker`'s first occurrence.
fn number_after(text: &str, marker: &str) -> Option<u64> {
    number_at(text, text.find(marker)?.checked_add(marker.len())?)
}

/// The whole number that ends just before `marker`'s first occurrence, with
/// spaces between them allowed.
fn number_before(text: &str, marker: &str) -> Option<u64> {
    let head = text.get(..text.find(marker)?)?.trim_end();
    let digits = head
        .rsplit(|character: char| !character.is_ascii_digit())
        .next()?;
    digits.parse::<u64>().ok()
}

/// The gateway's reason after a status, when it gave one.
fn with_detail(detail: &str) -> String {
    if detail.is_empty() {
        return String::new();
    }
    format!(": {detail}")
}

/// The wait a rate-limited gateway asked for, when it said.
fn retry_hint(retry_after: Option<u64>) -> String {
    match retry_after {
        Some(seconds) => format!("; retry after {seconds}s"),
        None => String::new(),
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
    /// Prompt tokens the provider read from its prefix cache, when it reported them (larry#230).
    pub(crate) cached_tokens: Option<u64>,
    /// Prompt tokens the provider wrote to its prefix cache, when it reported them (larry#230).
    pub(crate) cache_write_tokens: Option<u64>,
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
