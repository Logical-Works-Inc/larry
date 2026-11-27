//! The blocking completion client: one exchange per attempt, retried within the
//! config's [`RetryPolicy`], with every bound checked before a byte is sent.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use lgwks_bot::kernel::clock::Clock;
use lgwks_std::http::{self, BodyPolicy, Error as HttpError, FailureKind, Options, RedirectPolicy};
use lgwks_std::seeded::Seeded;

use super::config::ModelConfig;
use super::route::Kind;
use super::wire;
use super::{Completion, Limiter, MAX_MESSAGES, MAX_REPLY_BYTES, Message, ModelError};

/// The `User-Agent` every request names itself with (larry#230): a gateway that
/// meters or routes by client sees Larry and its version, not the HTTP library's.
const USER_AGENT: &str = concat!("larry/", env!("CARGO_PKG_VERSION"));
/// The Anthropic API version every Messages request names (larry#205): the API
/// refuses a request without one.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// The entropy words one retry draws, in the retry policy's units: the estate
/// seeded stream (`lgwks_std::seeded::Seeded`) draws them from the
/// tenant-route key, so one key replays one wait sequence on every host.
const JITTER_BOUND: u64 = 1_000;

/// Tokens a price's micro-dollar rate covers: prices are micro-dollars per
/// million tokens, so a tokens-times-price product is picodollars.
const TOKENS_PER_MILLION: u64 = 1_000_000;

/// One encoded completion request: its body and how many messages it carries.
/// Only [`ModelClient::encode`] builds one, and [`ModelClient::send`] checks its
/// bounds before any byte leaves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    /// The encoded body.
    body: Vec<u8>,
    /// The messages it carries.
    messages: usize,
}

impl Request {
    /// The encoded body's length in bytes: what the route's request bound reads.
    pub(crate) const fn bytes(&self) -> usize {
        self.body.len()
    }

    /// The encoded body, for a simulation's trace hash.
    #[cfg(test)]
    pub(crate) fn body(&self) -> &[u8] {
        &self.body
    }

    /// The messages it carries.
    #[cfg(test)]
    pub(crate) const fn messages(&self) -> usize {
        self.messages
    }
}

/// A blocking completion client for the route's provider kind.
///
/// Built per task from the route the task resolved to (larry#201). It holds no
/// connection; each attempt opens its own exchange through `lgwks_std::http`.
pub struct ModelClient {
    /// Everything one completion needs, fixed at construction.
    config: ModelConfig,
    /// The timeline the retry budget is measured on (larry#436, SD-4): the caller
    /// injects it, so a seeded run on a virtual clock drives the deadline and no
    /// retry decision reads the wall.
    clock: Clock,
    /// Waits out one retry backoff and says whether to retry. The caller owns
    /// the wait and cancellation: the executor waits on its cancel token, the
    /// `larry model` check refuses any wait, and a test records the wait. A
    /// `false` stops retrying and the last refusal is returned.
    pause: Box<dyn Fn(Duration) -> bool + Send + Sync>,
    /// Reported token usage for this tenant's route in this process.
    limiter: Arc<Limiter>,
    /// Authenticated tenant, part of the limiter key.
    tenant: String,
    /// Resolved route, part of the limiter key.
    route: String,
    /// Seeded per tenant-route so replay gives the same retry sequence.
    jitter: Mutex<Seeded>,
}

impl std::fmt::Debug for ModelClient {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.debug_struct("ModelClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl ModelClient {
    /// A client for `config` whose retry budget is measured on `clock` and whose
    /// retries wait through `pause`, which returns `false` to stop retrying
    /// (cancelled, or the caller will not wait).
    #[must_use]
    pub fn new(
        config: ModelConfig,
        clock: Clock,
        pause: impl Fn(Duration) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            config,
            clock,
            pause: Box::new(pause),
            limiter: Limiter::new(),
            tenant: String::new(),
            route: String::new(),
            jitter: Mutex::new(Seeded::from_seed(0)),
        }
    }

    /// A client sharing budget state with all calls for one executor process.
    pub(crate) fn with_limiter(
        config: ModelConfig,
        clock: Clock,
        limiter: Arc<Limiter>,
        tenant: impl Into<String>,
        route: impl Into<String>,
        pause: impl Fn(Duration) -> bool + Send + Sync + 'static,
    ) -> Self {
        let tenant = tenant.into();
        let route = route.into();
        let seed = jitter_seed(&tenant, &route);
        Self {
            config,
            clock,
            pause: Box::new(pause),
            limiter,
            tenant,
            route,
            jitter: Mutex::new(Seeded::from_seed(seed)),
        }
    }

    /// Run one completion, retrying 429 and 5xx within the retry budget.
    ///
    /// # Errors
    ///
    /// [`ModelError::BodyTooLarge`] before any send when the messages or the
    /// encoded request exceed their bound, and otherwise the typed failure of
    /// the last attempt. A config without a key cannot be built, so a client
    /// always has one.
    pub fn complete(&self, messages: &[Message]) -> Result<Completion, ModelError> {
        self.send(&self.encode(messages))
    }

    /// Whether `messages` pass the bounds [`ModelClient::complete`] checks before
    /// any send, asked without sending: the executor shares one prompt prefix
    /// across a brief only when every request of it fits the route (larry#230).
    pub(crate) fn fits(&self, messages: &[Message]) -> bool {
        self.within_bounds(&self.encode(messages))
    }

    /// The request for `messages`, encoded once: a terminal session measures the
    /// bytes it would send and then sends exactly those (larry#235).
    pub(crate) fn encode(&self, messages: &[Message]) -> Request {
        Request {
            body: wire::request_body(
                self.config.kind(),
                self.config.model(),
                self.config.temperature(),
                messages,
            ),
            messages: messages.len(),
        }
    }

    /// Send an encoded `request`, retrying 429 and 5xx within the retry budget.
    ///
    /// # Errors
    ///
    /// As [`ModelClient::complete`]: [`ModelError::BodyTooLarge`] before any send
    /// past the message count or the route's request bound.
    pub(crate) fn send(&self, request: &Request) -> Result<Completion, ModelError> {
        if !self.within_bounds(request) {
            return Err(ModelError::BodyTooLarge);
        }
        // The pending hold the budgets check: the request's own size as
        // tokens and its price at the dearer route rate, settled to the
        // reported usage once the gateway answers and released on any failure.
        let reserve_tokens = match u64::try_from(request.bytes()) {
            Ok(value) => value,
            Err(_) => return Err(ModelError::BodyTooLarge),
        };
        let dearer = self.config.price_input().max(self.config.price_output());
        let reserve_micros = match u64::try_from(
            u128::from(reserve_tokens)
                .saturating_mul(u128::from(dearer))
                .div_euclid(u128::from(TOKENS_PER_MILLION)),
        ) {
            Ok(value) => value,
            Err(_) => return Err(ModelError::MoneyCostOverflow),
        };
        self.limiter.before(
            &self.tenant,
            &self.route,
            self.config.token_budget(),
            self.config.micro_budget(),
            reserve_tokens,
            reserve_micros,
        )?;
        let completion = match self.with_retry(&request.body) {
            Ok(completion) => completion,
            Err(refusal) => {
                self.limiter.record(
                    &self.tenant,
                    &self.route,
                    reserve_tokens,
                    reserve_micros,
                    0,
                    0,
                )?;
                return Err(refusal);
            }
        };
        // Prices are micro-dollars per million tokens and costs are whole
        // micro-dollars, so the tokens-times-price product is picodollars
        // until it is scaled back down (larry#204): recording the product
        // itself would spend a million budgets on one call.
        let (tokens, micros) = match completion.usage() {
            Some(usage) => {
                let prompt = u128::from(usage.prompt_tokens);
                let produced = u128::from(usage.completion_tokens);
                let pico = prompt
                    .saturating_mul(u128::from(self.config.price_input()))
                    .saturating_add(
                        produced.saturating_mul(u128::from(self.config.price_output())),
                    );
                let micros = pico.div_euclid(u128::from(TOKENS_PER_MILLION));
                match u64::try_from(micros) {
                    Ok(value) => (
                        usage.prompt_tokens.saturating_add(usage.completion_tokens),
                        value,
                    ),
                    Err(_overflow) => {
                        self.limiter.record(
                            &self.tenant,
                            &self.route,
                            reserve_tokens,
                            reserve_micros,
                            0,
                            0,
                        )?;
                        return Err(ModelError::MoneyCostOverflow);
                    }
                }
            }
            None => {
                // A gateway that reports no usage spent the call anyway: many
                // OpenAI-compatible proxies omit `usage`, and one that omits
                // it on purpose would otherwise spend nothing (larry#204,
                // CWE-770). Settle at the reservation (the request size as
                // tokens, priced at the dearer route rate), never at zero.
                (reserve_tokens, reserve_micros)
            }
        };
        self.limiter.record(
            &self.tenant,
            &self.route,
            reserve_tokens,
            reserve_micros,
            tokens,
            micros,
        )?;
        Ok(completion)
    }

    /// The route's request body bound, in bytes.
    pub(crate) const fn request_bound(&self) -> usize {
        self.config.request_bytes()
    }

    /// Whether `request` is within the message count and the route's body bound.
    const fn within_bounds(&self, request: &Request) -> bool {
        request.messages <= MAX_MESSAGES && request.body.len() <= self.request_bound()
    }

    /// Attempt the exchange until it succeeds, fails terminally, or the retry
    /// budget and deadline run out. `max_attempts` counts the first try, so a
    /// policy of three sends at most three requests. The deadline is the time
    /// spent on the injected clock since the first try (larry#436).
    fn with_retry(&self, body: &[u8]) -> Result<Completion, ModelError> {
        let policy = self.config.retry();
        let started = self.clock.now();
        let mut failures: u32 = 0;
        loop {
            let refusal = match self.attempt(body) {
                Ok(completion) => return Ok(completion),
                Err(refusal) => refusal,
            };
            if !is_retryable(&refusal) {
                return Err(refusal);
            }
            let retry_index = failures;
            failures = failures.saturating_add(1);
            let elapsed = self.clock.now().saturating_sub(started);
            if !policy.should_retry(failures, elapsed) {
                return Err(refusal);
            }
            let remaining = policy.deadline.saturating_sub(elapsed);
            // No key, no jitter: the oracle of the injected-clock sims reads the
            // policy with zero entropy, so an unkeyed client draws the same waits.
            // A keyed client draws from the estate seeded stream its key names.
            let jitter = if self.tenant.is_empty() || self.route.is_empty() {
                0
            } else {
                self.jitter
                    .lock()
                    .map_err(|cause| ModelError::JitterUnavailable {
                        reason: cause.to_string(),
                    })?
                    .below(JITTER_BOUND)
                    .map_err(|cause| ModelError::JitterUnavailable {
                        reason: cause.to_string(),
                    })?
            };
            let backoff = policy.delay(retry_index, jitter).min(remaining);
            // RFC 9110 15.5.20 / 10.2.3: a Retry-After hint is a floor, never
            // shortened; a hint past the deadline cannot be honoured, so the
            // 429 is the answer.
            let wait = match retry_after_wait(&refusal) {
                Some(hinted) if hinted > remaining => return Err(refusal),
                Some(hinted) => backoff.max(hinted),
                None => backoff,
            };
            if !(self.pause)(wait) {
                return Err(refusal);
            }
        }
    }

    /// One exchange: post the body, read the reply, and map the outcome.
    fn attempt(&self, body: &[u8]) -> Result<Completion, ModelError> {
        // `timeout` bounds each phase; `deadline` bounds the whole exchange, so a
        // gateway trickling bytes cannot hold one attempt past the configured bound.
        let kind = self.config.kind();
        let mut options = Options::default()
            .timeout(self.config.timeout())
            .deadline(self.config.timeout())
            .max_body_bytes(MAX_REPLY_BYTES)
            .body_policy(BodyPolicy::Whole)
            // A gateway must never move the prompt to a host the route did not
            // name: the 3xx is the answer, not a step to follow (larry#327).
            .redirect_policy(RedirectPolicy::NoFollow)
            // `Options` already sends a `User-Agent` of its own; the setter replaces it,
            // where a `header` would add a second one.
            .user_agent(USER_AGENT);
        match kind {
            Kind::OpenAiChat | Kind::OpenAiResponses => {
                options =
                    options.header("Authorization", format!("Bearer {}", self.config.api_key()));
            }
            Kind::AnthropicMessages => {
                options = options.header("x-api-key", self.config.api_key().to_owned());
                options = options.header("anthropic-version", ANTHROPIC_VERSION);
            }
        }
        let url = format!("{}{}", self.config.base_url(), kind.path());
        let response =
            http::post_with(&url, "application/json", body, &options).map_err(map_transport)?;
        if !(200..300).contains(&response.status()) {
            let retry_after = response
                .headers()
                .iter()
                .find(|header| header.0.eq_ignore_ascii_case("retry-after"))
                .and_then(|header| header.1.trim().parse::<u64>().ok());
            return Err(wire::status_error(
                response.status(),
                retry_after,
                response.body(),
            ));
        }
        wire::parse_completion(kind, response.body())
    }
}

/// One deterministic seed for a tenant-route limiter key.
fn jitter_seed(tenant: &str, route: &str) -> u64 {
    let mut hasher = lgwks_std::hash::Hasher::new();
    hasher.update(tenant.as_bytes());
    hasher.update(route.as_bytes());
    let bytes = *hasher.finalize().as_bytes();
    u64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

/// Only 429 and 5xx retry: the gateway answered and said "not now". A
/// timeout or a cut connection may already have spent tokens upstream, so the
/// caller (the executor, #85) decides whether to pay again. Every other 4xx,
/// every malformed reply and every bound refusal is terminal.
fn is_retryable(refusal: &ModelError) -> bool {
    matches!(
        refusal,
        ModelError::RateLimited { .. } | ModelError::Server { .. }
    )
}

/// The wait the gateway asked for on a 429, when it asked for one.
fn retry_after_wait(refusal: &ModelError) -> Option<Duration> {
    match *refusal {
        ModelError::RateLimited {
            retry_after: Some(seconds),
        } => Some(Duration::from_secs(seconds)),
        _ => None,
    }
}

/// Fold an `lgwks_std::http` failure into this module's typed error. A timeout and a body past
/// its bound have their own variants; every other failure keeps the estate error whole, its
/// stage and kind included (larry#434).
fn map_transport(error: HttpError) -> ModelError {
    match error {
        HttpError::BodyTooLarge { .. } => ModelError::BodyTooLarge,
        HttpError::Failure {
            kind: FailureKind::Timeout,
            ..
        } => ModelError::Timeout,
        other => ModelError::Transport(other),
    }
}

/// A refused connection keeps its stage and kind, in the message and as the source, so an
/// operator tells a bad host from a dead network (larry#434). Source: `lgwks_std::http::Error`.
#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use lgwks_bot::kernel::clock::Clock;
    use lgwks_std::http::{Error as HttpError, FailureKind, FailureStage};

    use super::{
        JITTER_BOUND, Limiter, Message, ModelClient, ModelConfig, ModelError, Seeded, jitter_seed,
        map_transport,
    };
    use crate::model::{Draft, Kind, Role, Route};

    /// One keyed client driven through two 503s and a reply on a virtual
    /// clock: the waits its pause recorded and the requests the stub served.
    /// The script and the messages are fixed, so one tenant-route key replays
    /// one answer.
    fn drive_keyed_waits(
        tenant: &str,
        backoff_ms: u64,
    ) -> Result<(Vec<Duration>, u64), JitterSimError> {
        use std::io::Write;
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let base = format!("http://{}", listener.local_addr()?);
        let waits = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&waits);
        std::thread::scope(|scope| {
            let server = scope.spawn(move || {
                for status in [503_u16, 503, 200] {
                    let (mut stream, _) = listener.accept()?;
                    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
                    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
                    drain_request(&mut stream)?;
                    let body = if status == 200 {
                        String::from(
                            r#"{"id":"chatcmpl-keyed","object":"chat.completion","created":0,"model":"keyed-model","choices":[{"index":0,"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}"#,
                        )
                    } else {
                        String::from(r#"{"error":{"message":"busy","type":"server_error"}}"#)
                    };
                    let reason = if status == 200 { "OK" } else { "Service Unavailable" };
                    write!(
                        stream,
                        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    )?;
                    stream.flush()?;
                }
                Ok::<_, std::io::Error>(())
            });
            let client = ModelClient::with_limiter(
                keyed_config(&base, backoff_ms)?,
                Clock::virtual_at(Duration::ZERO),
                Limiter::new(),
                tenant,
                "route",
                move |wait| {
                    if let Ok(mut record) = seen.lock() {
                        record.push(wait);
                    }
                    true
                },
            );
            let outcome = client.complete(&[Message::new(Role::User, "wait-sequence")]);
            assert!(
                outcome.is_ok(),
                "tenant {tenant}: two faults then a reply succeed, got {outcome:?}"
            );
            match server.join() {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => return Err(JitterSimError::Server),
            }
            let waits = match waits.lock() {
                Ok(record) => record.clone(),
                Err(_) => return Err(JitterSimError::Poisoned),
            };
            Ok::<_, JitterSimError>((waits, 3))
        })
    }

    /// Read one length-delimited request (head plus its declared body) and drop
    /// it: the stub answers from its script, never from the bytes.
    fn drain_request(stream: &mut std::net::TcpStream) -> Result<(), std::io::Error> {
        use std::io::Read;
        let mut head = Vec::new();
        let mut byte = [0_u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if head.len() > 65_536 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "the request head is past 64 KiB",
                ));
            }
            match stream.read(&mut byte)? {
                0 => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "the peer closed before the head ended",
                    ));
                }
                _ => head.push(byte[0]),
            }
        }
        let text = String::from_utf8_lossy(&head);
        let length = text
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .find_map(|(name, value)| {
                name.trim()
                    .eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().to_owned())
            })
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "no content-length")
            })?;
        let count: usize = length
            .parse()
            .map_err(|cause| std::io::Error::new(std::io::ErrorKind::InvalidData, cause))?;
        let mut body = vec![0_u8; count.min(65_536)];
        stream.read_exact(&mut body)?;
        Ok(())
    }

    /// The config of a keyed route the wait simulation drives at `base`: the
    /// key is read from `LARRY_KEY_KEYED`, three attempts, the drawn backoff,
    /// a deadline no virtual run spends.
    fn keyed_config(base: &str, backoff_ms: u64) -> Result<ModelConfig, ModelError> {
        let route = Route::new(&Draft {
            id: "keyed",
            kind: Kind::OpenAiChat,
            base_url: base,
            model: "keyed-model",
            key_env: "LARRY_KEY_KEYED",
            temperature: None,
            request_bytes: None,
            residency: None,
            prompt: None,
            describe: "a keyed route the wait simulation drives",
        })
        .map_err(ModelError::BadRoute)?;
        ModelConfig::for_route(&route, &|name: &str| match name {
            "LARRY_KEY_KEYED" => Some(String::from("keyed-key")),
            "LARRY_MODEL_BACKOFF_MS" => Some(backoff_ms.to_string()),
            "LARRY_MODEL_ATTEMPTS" => Some(String::from("3")),
            "LARRY_MODEL_DEADLINE_MS" => Some(String::from("60000")),
            _ => None,
        })
    }

    /// Why one keyed wait simulation could not be driven to its assertions.
    #[derive(Debug, lgwks_macros::Error)]
    enum JitterSimError {
        /// The stub gateway failed.
        #[error("the stub gateway failed: {0}")]
        Mock(#[from] std::io::Error),
        /// The drawn config was refused.
        #[error("the keyed config was refused: {0}")]
        Config(#[from] ModelError),
        /// A drawn number does not fit the type its use needs.
        #[error("the drawn number is out of range: {0}")]
        Range(#[from] std::num::TryFromIntError),
        /// The seeded stream refused a draw.
        #[error("the seeded draw failed: {0}")]
        Draw(#[from] lgwks_std::seeded::DrawError),
        /// The stub server failed its script.
        #[error("the stub server failed its script")]
        Server,
        /// The wait record is poisoned.
        #[error("the wait record is poisoned")]
        Poisoned,
    }

    /// One tenant-route key replays one wait sequence: two clients built for
    /// one key and driven through one scripted 503 fault script on a virtual
    /// clock record the same waits, each inside the policy bound, and a second
    /// tenant records different waits.
    #[test]
    fn sim_keyed_clients_replay_one_wait_sequence() -> Result<(), JitterSimError> {
        use lgwks_std::retry::RetryPolicy;
        const BACKOFF_MS: u64 = 50;
        let policy = RetryPolicy::new(
            3,
            Duration::from_millis(BACKOFF_MS),
            Duration::from_millis(60_000),
        );
        let (left, left_served) = drive_keyed_waits("tenant-jitter-left", BACKOFF_MS)?;
        let (right, right_served) = drive_keyed_waits("tenant-jitter-left", BACKOFF_MS)?;
        assert_eq!(
            (left_served, right_served),
            (3, 3),
            "two faults then a reply: every run serves three requests"
        );
        assert_eq!(left, right, "one key replays one wait sequence");
        let mut stream = Seeded::from_seed(jitter_seed("tenant-jitter-left", "route"));
        for (index, wait) in left.iter().enumerate() {
            let attempt = u32::try_from(index)?;
            let draw = stream.below(JITTER_BOUND)?;
            assert_eq!(
                *wait,
                policy.delay(attempt, draw),
                "wait {index} is the policy at the keyed draw"
            );
            assert!(
                *wait <= policy.delay(attempt, 0),
                "wait {index} stays inside the policy bound"
            );
        }
        let (other, _) = drive_keyed_waits("tenant-jitter-other", BACKOFF_MS)?;
        assert_ne!(other, left, "a second tenant draws its own waits");
        Ok(())
    }

    #[test]
    fn a_transport_failure_keeps_its_stage_and_kind() {
        let resolve = HttpError::Failure {
            stage: FailureStage::Resolve,
            kind: FailureKind::Transport,
            cause: None,
        };
        let mapped = map_transport(resolve.clone());
        assert_eq!(mapped, ModelError::Transport(resolve));
        let text = mapped.to_string();
        assert!(
            text.contains("Resolve") && text.contains("Transport"),
            "the message names the stage and kind: {text}"
        );
        assert!(mapped.source().is_some(), "the estate error is the source");
        let timeout = HttpError::Failure {
            stage: FailureStage::Connect,
            kind: FailureKind::Timeout,
            cause: None,
        };
        assert_eq!(map_transport(timeout), ModelError::Timeout);
        assert_eq!(
            map_transport(HttpError::BodyTooLarge { limit: 1 }),
            ModelError::BodyTooLarge
        );
        assert!(matches!(
            map_transport(HttpError::InvalidUrl),
            ModelError::Transport(HttpError::InvalidUrl)
        ));
    }
}
