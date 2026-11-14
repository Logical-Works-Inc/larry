//! What the model client is pointed at: the gateway, the key, the model, the
//! sampling temperature, the request bound, the request timeout and the retry
//! budget. A route (`super::route`) says where the call goes and which variable
//! holds its key; the operator's `LARRY_MODEL_*` knobs say how patient the
//! client is. Nothing here is compiled in: no gateway, no model id.

use std::fmt;
use std::num::ParseIntError;
use std::str::FromStr;
use std::time::Duration;

use lgwks_std::retry::RetryPolicy;

use super::ModelError;
use super::route::{Draft, ENV_ROUTE, Kind, Route};

/// The variable the environment route reads its gateway URL from.
const URL_VARIABLE: &str = "LARRY_MODEL_URL";
/// The variable the environment route reads its model id from.
const MODEL_VARIABLE: &str = "LARRY_MODEL";
/// The variable the environment route reads its key from: the route's `key_env`.
const KEY_VARIABLE: &str = "LARRY_MODEL_KEY";
/// The variable the environment route reads its sampling temperature from.
const TEMPERATURE_VARIABLE: &str = "LARRY_MODEL_TEMPERATURE";
/// The variable the environment route reads its residency from; unset means undeclared.
const RESIDENCY_VARIABLE: &str = "LARRY_MODEL_RESIDENCY";
/// What the environment route says about itself in a listing.
const ENV_DESCRIBE: &str = "declared by the LARRY_MODEL_* environment";
/// The sampling temperature of a route that pins none.
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
    /// The model name sent on every request.
    model: String,
    /// The sampling temperature sent on every request.
    temperature: f64,
    /// The bound on one request body, in bytes.
    request_bytes: usize,
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

/// `variable`'s value through `read`, unset when absent or empty.
fn present(read: &impl Fn(&str) -> Option<String>, variable: &str) -> Option<String> {
    read(variable).filter(|value| !value.is_empty())
}

/// The temperature `LARRY_MODEL_TEMPERATURE` sets, `None` when it is unset.
fn temperature_from(read: &impl Fn(&str) -> Option<String>) -> Result<Option<f64>, ModelError> {
    match read(TEMPERATURE_VARIABLE) {
        Some(text) => match text.trim().parse::<f64>() {
            Ok(value) if value.is_finite() => Ok(Some(value)),
            Ok(_) => Err(ModelError::OutOfRange(TEMPERATURE_VARIABLE)),
            Err(cause) => Err(ModelError::BadFloat {
                variable: TEMPERATURE_VARIABLE,
                cause,
            }),
        },
        None => Ok(None),
    }
}

/// The route the operator's environment declares: `LARRY_MODEL_URL`,
/// `LARRY_MODEL` and `LARRY_MODEL_KEY` all set and not empty. Any of the three
/// missing means the environment declares no route, and `None` is the answer.
///
/// # Errors
///
/// [`ModelError::BadFloat`] or [`ModelError::OutOfRange`] naming a temperature
/// that does not parse, and [`ModelError::BadRoute`] for a value its field
/// check refuses.
pub(crate) fn env_route(
    read: &impl Fn(&str) -> Option<String>,
) -> Result<Option<Route>, ModelError> {
    let (Some(url), Some(model), Some(_key)) = (
        present(read, URL_VARIABLE),
        present(read, MODEL_VARIABLE),
        present(read, KEY_VARIABLE),
    ) else {
        return Ok(None);
    };
    let residency = present(read, RESIDENCY_VARIABLE);
    let draft = Draft {
        id: ENV_ROUTE,
        kind: Kind::OpenAiChat,
        base_url: &url,
        model: &model,
        key_env: KEY_VARIABLE,
        temperature: temperature_from(read)?,
        request_bytes: None,
        residency: residency.as_deref(),
        describe: ENV_DESCRIBE,
    };
    Route::environment(&draft)
        .map(Some)
        .map_err(ModelError::BadRoute)
}

impl ModelConfig {
    /// Read the environment route's configuration through `read`, which maps a
    /// variable name to its value: `std::env::var` in the binary, a table in a
    /// test.
    ///
    /// `LARRY_MODEL_KEY`, `LARRY_MODEL_URL` and `LARRY_MODEL` are required.
    /// `LARRY_MODEL_TEMPERATURE`, `LARRY_MODEL_TIMEOUT_MS`,
    /// `LARRY_MODEL_ATTEMPTS` (1 to 16), `LARRY_MODEL_BACKOFF_MS` and
    /// `LARRY_MODEL_DEADLINE_MS` fall back to their defaults.
    ///
    /// # Errors
    ///
    /// [`ModelError::MissingKey`] when the key is unset or empty;
    /// [`ModelError::Unset`] naming the URL or model variable that is missing;
    /// [`ModelError::BadInteger`] or [`ModelError::BadFloat`] naming the
    /// variable that does not parse; [`ModelError::OutOfRange`] naming the one
    /// that parses but cannot be used.
    pub fn from_env(read: impl Fn(&str) -> Option<String>) -> Result<Self, ModelError> {
        present(&read, KEY_VARIABLE).ok_or(ModelError::MissingKey)?;
        let Some(route) = env_route(&read)? else {
            let missing = if present(&read, URL_VARIABLE).is_some() {
                MODEL_VARIABLE
            } else {
                URL_VARIABLE
            };
            return Err(ModelError::Unset(missing));
        };
        Self::for_route(&route, &read)
    }

    /// The configuration for one task's model calls over `route`: the key is
    /// read from the variable the route names, now, and nowhere else.
    ///
    /// # Errors
    ///
    /// [`ModelError::KeyUnset`] naming the route's key variable when it is unset
    /// or empty, and the integer and range errors of the `LARRY_MODEL_*` knobs.
    pub(crate) fn for_route(
        route: &Route,
        read: &impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ModelError> {
        let api_key = present(read, route.key_env()).ok_or_else(|| ModelError::KeyUnset {
            variable: route.key_env().to_owned(),
        })?;
        let temperature = match route.temperature() {
            Some(pinned) => pinned,
            None => DEFAULT_TEMPERATURE,
        };
        let timeout = whole(read, "LARRY_MODEL_TIMEOUT_MS", DEFAULT_TIMEOUT_MS)?;
        let attempts: u32 = whole(read, "LARRY_MODEL_ATTEMPTS", DEFAULT_ATTEMPTS)?;
        if !(1..=MAX_ATTEMPTS).contains(&attempts) {
            return Err(ModelError::OutOfRange("LARRY_MODEL_ATTEMPTS"));
        }
        let backoff = whole(read, "LARRY_MODEL_BACKOFF_MS", DEFAULT_BACKOFF_MS)?;
        let deadline = whole(read, "LARRY_MODEL_DEADLINE_MS", DEFAULT_DEADLINE_MS)?;
        Ok(Self {
            base_url: route.base_url().to_owned(),
            api_key,
            model: route.model().to_owned(),
            temperature,
            request_bytes: route.request_bytes(),
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

    /// The model name.
    pub(crate) fn model(&self) -> &str {
        &self.model
    }

    /// The sampling temperature.
    pub(super) const fn temperature(&self) -> f64 {
        self.temperature
    }

    /// The bound on one request body, in bytes.
    pub(super) const fn request_bytes(&self) -> usize {
        self.request_bytes
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
            .field("request_bytes", &self.request_bytes)
            .field("timeout", &self.timeout)
            .field("retry", &self.retry)
            .finish()
    }
}
