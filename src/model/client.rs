//! The blocking completion client: one exchange per attempt, retried within the
//! config's [`RetryPolicy`], with every bound checked before a byte is sent.

use std::time::{Duration, Instant};

use lgwks_std::http::{self, BodyPolicy, Error as HttpError, FailureKind, Options};

use super::config::ModelConfig;
use super::wire;
use super::{Completion, MAX_MESSAGES, MAX_REPLY_BYTES, MAX_REQUEST_BYTES, Message, ModelError};

/// A blocking OpenAI-compatible chat-completions client.
///
/// Built once per gateway and key. It holds no connection; each attempt opens
/// its own exchange through `lgwks_std::http`.
pub struct ModelClient {
    /// Everything one completion needs, fixed at construction.
    config: ModelConfig,
    /// Waits out one retry backoff and says whether to retry. The caller owns
    /// the clock and cancellation: the executor waits on its cancel token, the
    /// `rocco model` check refuses any wait, and a test records the wait. A
    /// `false` stops retrying and the last refusal is returned.
    pause: Box<dyn Fn(Duration) -> bool + Send + Sync>,
}

impl std::fmt::Debug for ModelClient {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.debug_struct("ModelClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl ModelClient {
    /// A client for `config` whose retries wait through `pause`, which returns
    /// `false` to stop retrying (cancelled, or the caller will not wait).
    #[must_use]
    pub fn new(
        config: ModelConfig,
        pause: impl Fn(Duration) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            config,
            pause: Box::new(pause),
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
        if messages.len() > MAX_MESSAGES {
            return Err(ModelError::BodyTooLarge);
        }
        let body = wire::request_body(self.config.model(), self.config.temperature(), messages);
        if body.len() > MAX_REQUEST_BYTES {
            return Err(ModelError::BodyTooLarge);
        }
        self.with_retry(&body)
    }

    /// Attempt the exchange until it succeeds, fails terminally, or the retry
    /// budget and deadline run out. `max_attempts` counts the first try, so a
    /// policy of three sends at most three requests.
    fn with_retry(&self, body: &[u8]) -> Result<Completion, ModelError> {
        let policy = self.config.retry();
        let started = Instant::now();
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
            let elapsed = started.elapsed();
            if !policy.should_retry(failures, elapsed) {
                return Err(refusal);
            }
            let remaining = policy.deadline.saturating_sub(elapsed);
            // No jitter: one seed replays one wait sequence. A fleet of clients
            // would draw the entropy from `lgwks_std::random` here.
            let backoff = policy.delay(retry_index, 0).min(remaining);
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
            .header("Authorization", format!("Bearer {}", self.config.api_key()));
        let url = format!("{}/chat/completions", self.config.base_url());
        let response =
            http::post_with(&url, "application/json", body, &options).map_err(map_transport)?;
        if !(200..300).contains(&response.status) {
            let retry_after = response
                .headers()
                .iter()
                .find(|header| header.0.eq_ignore_ascii_case("retry-after"))
                .and_then(|header| header.1.trim().parse::<u64>().ok());
            return Err(wire::status_error(
                response.status,
                retry_after,
                response.body(),
            ));
        }
        wire::parse_completion(response.body())
    }
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

/// Fold an `lgwks_std::http` failure into this module's typed error.
fn map_transport(error: HttpError) -> ModelError {
    match error {
        HttpError::BodyTooLarge { .. } => ModelError::BodyTooLarge,
        HttpError::Failure {
            kind: FailureKind::Timeout,
            ..
        } => ModelError::Timeout,
        HttpError::Failure { .. }
        | HttpError::InvalidUrl
        | HttpError::RedirectLimitTooLarge { .. } => ModelError::Transport,
        _ => ModelError::Transport,
    }
}
