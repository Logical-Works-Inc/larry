//! The blocking completion client: one exchange per attempt, retried within the
//! config's [`RetryPolicy`], with every bound checked before a byte is sent.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lgwks_bot::kernel::clock::Clock;
use lgwks_std::http::{self, BodyPolicy, Error as HttpError, FailureKind, Options, RedirectPolicy};
use lgwks_std::seeded::Seeded;

use super::config::ModelConfig;
use super::wire;
use super::{Completion, MAX_MESSAGES, MAX_REPLY_BYTES, Message, ModelError};

/// The `User-Agent` every request names itself with (larry#230): a gateway that
/// meters or routes by client sees Larry and its version, not the HTTP library's.
const USER_AGENT: &str = concat!("larry/", env!("CARGO_PKG_VERSION"));

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

/// The bounded state shared by model clients in one executor process.
pub(crate) struct Limiter {
    /// Tenant and route usage, bounded before a new key is inserted.
    entries: Mutex<BTreeMap<(String, String), (u64, u64)>>,
}

impl Limiter {
    /// Maximum distinct tenant-route keys held by one process.
    const MAX_KEYS: usize = 256;

    /// A fresh process-local limiter.
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(BTreeMap::new()),
        })
    }

    /// Refuse a call already over either configured budget, before any HTTP work.
    fn before(
        &self,
        tenant: &str,
        route: &str,
        token_limit: u64,
        micro_limit: u64,
    ) -> Result<(), ModelError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|cause| ModelError::BudgetUnavailable {
                reason: cause.to_string(),
            })?;
        let key = (tenant.to_owned(), route.to_owned());
        let usage = match entries.get(&key) {
            Some(usage) => *usage,
            None if entries.len() < Self::MAX_KEYS => (0, 0),
            None => {
                return Err(ModelError::LimiterFull {
                    limit: Self::MAX_KEYS,
                });
            }
        };
        if usage.0 >= token_limit {
            return Err(ModelError::BudgetExceeded {
                tokens: usage.0,
                limit: token_limit,
            });
        }
        if usage.1 >= micro_limit {
            return Err(ModelError::MoneyBudgetExceeded {
                micros: usage.1,
                limit: micro_limit,
            });
        }
        entries.entry(key).or_insert(usage);
        Ok(())
    }

    /// Add the usage reported by a successful route call. A key the limiter
    /// never admitted is a new key, so past the bound it is refused rather
    /// than grown: the table never holds more than its key budget.
    fn record(
        &self,
        tenant: &str,
        route: &str,
        tokens: u64,
        micros: u64,
    ) -> Result<(), ModelError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|cause| ModelError::BudgetUnavailable {
                reason: cause.to_string(),
            })?;
        let key = (tenant.to_owned(), route.to_owned());
        if !entries.contains_key(&key) && entries.len() >= Self::MAX_KEYS {
            return Err(ModelError::LimiterFull {
                limit: Self::MAX_KEYS,
            });
        }
        let usage = entries.entry(key).or_insert((0, 0));
        usage.0 = usage.0.saturating_add(tokens);
        usage.1 = usage.1.saturating_add(micros);
        Ok(())
    }
}

/// A blocking OpenAI-compatible chat-completions client.
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
            body: wire::request_body(self.config.model(), self.config.temperature(), messages),
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
        self.limiter.before(
            &self.tenant,
            &self.route,
            self.config.token_budget(),
            self.config.micro_budget(),
        )?;
        let completion = self.with_retry(&request.body)?;
        if let Some(usage) = completion.usage() {
            // Prices are micro-dollars per million tokens and costs are whole
            // micro-dollars, so the tokens-times-price product is picodollars
            // until it is scaled back down (larry#204): recording the product
            // itself would spend a million budgets on one call.
            let prompt = u128::from(usage.prompt_tokens);
            let produced = u128::from(usage.completion_tokens);
            let pico = prompt
                .saturating_mul(u128::from(self.config.price_input()))
                .saturating_add(produced.saturating_mul(u128::from(self.config.price_output())));
            let micros = pico.div_euclid(u128::from(TOKENS_PER_MILLION));
            let bounded = match u64::try_from(micros) {
                Ok(value) => value,
                Err(_overflow) => return Err(ModelError::MoneyCostOverflow),
            };
            let tokens = usage.prompt_tokens.saturating_add(usage.completion_tokens);
            self.limiter
                .record(&self.tenant, &self.route, tokens, bounded)?;
        }
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
        let options = Options::default()
            .timeout(self.config.timeout())
            .deadline(self.config.timeout())
            .max_body_bytes(MAX_REPLY_BYTES)
            .body_policy(BodyPolicy::Whole)
            // A gateway must never move the prompt to a host the route did not
            // name: the 3xx is the answer, not a step to follow (larry#327).
            .redirect_policy(RedirectPolicy::NoFollow)
            // `Options` already sends a `User-Agent` of its own; the setter replaces it,
            // where a `header` would add a second one.
            .user_agent(USER_AGENT)
            .header("Authorization", format!("Bearer {}", self.config.api_key()));
        let url = format!("{}/chat/completions", self.config.base_url());
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
        wire::parse_completion(response.body())
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
    use std::sync::Arc;

    use lgwks_bot::kernel::clock::Clock;
    use lgwks_std::http::{Error as HttpError, FailureKind, FailureStage};

    use super::{
        JITTER_BOUND, Limiter, ModelClient, ModelError, Seeded, jitter_seed, map_transport,
    };
    use crate::model::config::sim_config;

    /// The jitter draws one tenant-route key names: one seed, one sequence.
    fn draws(tenant: &str) -> Result<Vec<u64>, ModelError> {
        let mut stream = Seeded::from_seed(jitter_seed(tenant, "route"));
        let mut out = Vec::with_capacity(8);
        for _ in 0..8 {
            out.push(stream.below(JITTER_BOUND).map_err(|cause| {
                ModelError::JitterUnavailable {
                    reason: cause.to_string(),
                }
            })?);
        }
        Ok(out)
    }

    /// Two tenants sharing one limiter keep their own budgets: one tenant's
    /// burst to its token budget refuses only its own next call, and the other
    /// tenant still admits and records under the same route.
    #[test]
    fn sim_two_tenants_sharing_one_limiter_keep_their_own_budgets() -> Result<(), ModelError> {
        for seed in 0..16_u64 {
            let limiter = Limiter::new();
            let spent = format!("tenant-{seed}-spent");
            let fresh = format!("tenant-{seed}-fresh");
            limiter.record(&spent, "route", 1_000, 50)?;
            assert!(
                matches!(
                    limiter.before(&spent, "route", 1_000, u64::MAX),
                    Err(ModelError::BudgetExceeded {
                        tokens: 1_000,
                        limit: 1_000
                    })
                ),
                "seed {seed}: the spent tenant is refused at its budget"
            );
            limiter.before(&fresh, "route", 1_000, u64::MAX)?;
            limiter.record(&fresh, "route", 1, 0)?;
            limiter.before(&fresh, "route", 1_000, u64::MAX)?;
        }
        Ok(())
    }

    /// A full limiter refuses a new tenant-route key on both paths: `before`
    /// admits nothing past 256 keys, and `record` refuses rather than growing
    /// past the bound, while a key already held still admits.
    #[test]
    fn sim_a_full_limiter_refuses_a_new_key_on_record() -> Result<(), ModelError> {
        let limiter = Limiter::new();
        for index in 0..256_u32 {
            limiter.record(&format!("tenant-{index}"), "route", 0, 0)?;
        }
        assert!(
            matches!(
                limiter.record("tenant-new", "route", 0, 0),
                Err(ModelError::LimiterFull { limit: 256 })
            ),
            "a record past 256 keys is refused, never grown"
        );
        assert!(
            matches!(
                limiter.before("tenant-new", "route", u64::MAX, u64::MAX),
                Err(ModelError::LimiterFull { limit: 256 })
            ),
            "a call past 256 keys is refused before any gateway byte"
        );
        limiter.before("tenant-0", "route", u64::MAX, u64::MAX)?;
        Ok(())
    }

    /// One tenant-route key replays one jitter sequence on every process, and
    /// every draw fits the retry policy's bound. Two clients built for one key
    /// draw the same waits, so a seeded run replays them.
    #[test]
    fn sim_one_seed_replays_one_jitter_sequence() -> Result<(), ModelError> {
        for seed in 0..16_u64 {
            let tenant = format!("tenant-{seed}");
            let first = draws(&tenant)?;
            let second = draws(&tenant)?;
            assert_eq!(first, second, "seed {seed}: one key draws one sequence");
            assert!(
                first.iter().all(|draw| *draw < JITTER_BOUND),
                "seed {seed}: every draw fits the bound"
            );
            let limiter = Limiter::new();
            let config = sim_config(None)?;
            let clock = Clock::virtual_at(std::time::Duration::ZERO);
            let left = ModelClient::with_limiter(
                config.clone(),
                clock.clone(),
                Arc::clone(&limiter),
                tenant.clone(),
                "route",
                |_wait| true,
            );
            let right = ModelClient::with_limiter(
                config,
                clock,
                Arc::clone(&limiter),
                tenant,
                "route",
                |_wait| true,
            );
            assert_eq!(
                format!("{left:?}"),
                format!("{right:?}"),
                "seed {seed}: one key builds one client shape"
            );
        }
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
