//! What the model client is pointed at: the gateway, the key, the pinned model,
//! the sampling temperature, the request timeout and the retry budget. Every
//! value comes from `LARRY_MODEL_*`, so an operator and a test configure the
//! client the same way.

use std::fmt;
use std::num::ParseIntError;
use std::str::FromStr;
use std::time::Duration;

use lgwks_std::retry::RetryPolicy;

use super::ModelError;

/// The gateway the client talks to when `LARRY_MODEL_URL` is unset.
const DEFAULT_BASE_URL: &str = "http://127.0.0.1:20128/v1";
/// The pinned model when `LARRY_MODEL` is unset: DeepSeek V4.1 Flash under the
/// id the local 9router gateway serves it as (larry#128).
const DEFAULT_MODEL: &str = "ocg/deepseek-v4.1-flash";
/// The sampling temperature when `LARRY_MODEL_TEMPERATURE` is unset.
const DEFAULT_TEMPERATURE: f64 = 0.0;
/// The per-request bound in milliseconds: one whole exchange, body included.
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
/// Attempts per completion, first try included.
const DEFAULT_ATTEMPTS: u32 = 3;
/// Backoff after the first failed attempt, in milliseconds.
const DEFAULT_BACKOFF_MS: u64 = 500;
/// Total budget across every attempt of one completion, in milliseconds.
const DEFAULT_DEADLINE_MS: u64 = 300_000;
/// Most attempts a configuration may ask for.
const MAX_ATTEMPTS: u32 = 16;

/// Everything one [`super::ModelClient`] needs, fixed at construction.
#[derive(Clone)]
pub struct ModelConfig {
    /// Base URL of the OpenAI-compatible gateway, without a trailing slash.
    base_url: String,
    /// Bearer credential for the gateway. Never logged, printed or shown by `Debug`.
    api_key: String,
    /// The pinned model name sent on every request.
    model: String,
    /// The sampling temperature sent on every request.
    temperature: f64,
    /// The bound on one whole request, body included.
    timeout: Duration,
    /// How failed attempts are spaced and when retrying stops.
    retry: RetryPolicy,
}

/// A whole number read from `variable`, or `default` when it is unset.
fn whole<Number: FromStr<Err = ParseIntError>>(
    read: &impl Fn(&str) -> Option<String>,
    variable: &'static str,
    default: Number,
) -> Result<Number, ModelError> {
    match read(variable) {
        Some(text) => match text.trim().parse::<Number>() {
            Ok(value) => Ok(value),
            Err(cause) => Err(ModelError::BadInteger { variable, cause }),
        },
        None => Ok(default),
    }
}

impl ModelConfig {
    /// Read the configuration through `read`, which maps a variable name to
    /// its value: `std::env::var` in the binary, a table in a test.
    ///
    /// `LARRY_MODEL_KEY` is required. `LARRY_MODEL_URL`, `LARRY_MODEL`,
    /// `LARRY_MODEL_TEMPERATURE`, `LARRY_MODEL_TIMEOUT_MS`,
    /// `LARRY_MODEL_ATTEMPTS` (1 to 16), `LARRY_MODEL_BACKOFF_MS` and
    /// `LARRY_MODEL_DEADLINE_MS` fall back to their defaults.
    ///
    /// # Errors
    ///
    /// [`ModelError::MissingKey`] when the key is unset or empty;
    /// [`ModelError::BadInteger`] or [`ModelError::BadFloat`] naming the
    /// variable that does not parse; [`ModelError::OutOfRange`] naming the one
    /// that parses but cannot be used.
    pub fn from_env(read: impl Fn(&str) -> Option<String>) -> Result<Self, ModelError> {
        let api_key = read("LARRY_MODEL_KEY")
            .filter(|key| !key.is_empty())
            .ok_or(ModelError::MissingKey)?;
        let base_url = match read("LARRY_MODEL_URL") {
            Some(url) => url,
            None => String::from(DEFAULT_BASE_URL),
        };
        let model = match read("LARRY_MODEL") {
            Some(model) => model,
            None => String::from(DEFAULT_MODEL),
        };
        let temperature = match read("LARRY_MODEL_TEMPERATURE") {
            Some(text) => match text.trim().parse::<f64>() {
                Ok(value) if value.is_finite() => value,
                Ok(_) => return Err(ModelError::OutOfRange("LARRY_MODEL_TEMPERATURE")),
                Err(cause) => {
                    return Err(ModelError::BadFloat {
                        variable: "LARRY_MODEL_TEMPERATURE",
                        cause,
                    });
                }
            },
            None => DEFAULT_TEMPERATURE,
        };
        let timeout = whole(&read, "LARRY_MODEL_TIMEOUT_MS", DEFAULT_TIMEOUT_MS)?;
        let attempts: u32 = whole(&read, "LARRY_MODEL_ATTEMPTS", DEFAULT_ATTEMPTS)?;
        if !(1..=MAX_ATTEMPTS).contains(&attempts) {
            return Err(ModelError::OutOfRange("LARRY_MODEL_ATTEMPTS"));
        }
        let backoff = whole(&read, "LARRY_MODEL_BACKOFF_MS", DEFAULT_BACKOFF_MS)?;
        let deadline = whole(&read, "LARRY_MODEL_DEADLINE_MS", DEFAULT_DEADLINE_MS)?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key,
            model,
            temperature,
            timeout: Duration::from_millis(timeout),
            retry: RetryPolicy::new(
                attempts,
                Duration::from_millis(backoff),
                Duration::from_millis(deadline),
            ),
        })
    }

    /// The gateway base URL.
    pub(super) fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The bearer credential.
    pub(super) fn api_key(&self) -> &str {
        &self.api_key
    }

    /// The pinned model name.
    pub(super) fn model(&self) -> &str {
        &self.model
    }

    /// The sampling temperature.
    pub(super) const fn temperature(&self) -> f64 {
        self.temperature
    }

    /// The per-request timeout.
    pub(super) const fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The retry budget.
    pub(super) const fn retry(&self) -> RetryPolicy {
        self.retry
    }
}

impl fmt::Debug for ModelConfig {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("ModelConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("temperature", &self.temperature)
            .field("timeout", &self.timeout)
            .field("retry", &self.retry)
            .finish()
    }
}
