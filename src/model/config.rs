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
/// The variable the environment route reads its provider kind from; unset means OpenAI chat.
const KIND_VARIABLE: &str = "LARRY_MODEL_KIND";
/// The variable the environment route reads its sampling temperature from.
const TEMPERATURE_VARIABLE: &str = "LARRY_MODEL_TEMPERATURE";
/// The variable the environment route reads its residency from; unset means undeclared.
const RESIDENCY_VARIABLE: &str = "LARRY_MODEL_RESIDENCY";
/// What the environment route says about itself in a listing.
const ENV_DESCRIBE: &str = "declared by the LARRY_MODEL_* environment";
/// The per-request bound in milliseconds: one whole exchange, body included.
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
/// Attempts per completion, first try included.
const DEFAULT_ATTEMPTS: u32 = 3;
/// Backoff after the first failed attempt, in milliseconds. Zero by default,
/// like the executor, which never waits for a retry: a wait it is asked for
/// stops the retry at once, so the default asks for none.
const DEFAULT_BACKOFF_MS: u64 = 0;
/// Total budget across every attempt of one completion, in milliseconds.
const DEFAULT_DEADLINE_MS: u64 = 300_000;
/// An unset model token budget keeps the legacy unlimited route behaviour.
const DEFAULT_TOKEN_BUDGET: u64 = u64::MAX;
/// An unset model money budget keeps the legacy unlimited route behaviour.
const DEFAULT_MICRO_BUDGET: u64 = u64::MAX;
/// Most attempts a configuration may ask for.
const MAX_ATTEMPTS: u32 = 16;

/// Everything one [`super::ModelClient`] needs, fixed at construction.
#[derive(Clone)]
pub struct ModelConfig {
    /// Which provider wire the client speaks.
    kind: Kind,
    /// Base URL of the gateway, without a trailing slash.
    base_url: String,
    /// Bearer credential for the gateway. Never logged, printed or shown by `Debug`.
    api_key: String,
    /// The model name sent on every request.
    model: String,
    /// The sampling temperature sent on a request, when the route pins one.
    temperature: Option<f64>,
    /// The bound on one request body, in bytes.
    request_bytes: usize,
    /// The bound on one whole request, body included.
    timeout: Duration,
    /// How failed attempts are spaced and when retrying stops.
    retry: RetryPolicy,
    /// The maximum reported tokens this client may spend for its tenant.
    token_budget: u64,
    /// The maximum micro-dollar spend this client may report.
    micro_budget: u64,
    /// Route input price, in micro-dollars per million tokens.
    price_input: u64,
    /// Route output price, in micro-dollars per million tokens.
    price_output: u64,
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

/// The kind `LARRY_MODEL_KIND` names, OpenAI chat when it is unset.
///
/// # Errors
///
/// [`ModelError::BadRoute`] naming an unknown kind.
fn kind_from(read: &impl Fn(&str) -> Option<String>) -> Result<Kind, ModelError> {
    match read(KIND_VARIABLE) {
        Some(name) => {
            Kind::parse(name.trim()).ok_or(ModelError::BadRoute(crate::model::RouteError::BadKind))
        }
        None => Ok(Kind::OpenAiChat),
    }
}
/// The route the operator's environment declares: `LARRY_MODEL_URL`,
/// `LARRY_MODEL` and `LARRY_MODEL_KEY` all set and not empty. Any of the three
/// missing means the environment declares no route, and `None` is the answer.
/// `LARRY_MODEL_KIND` names the provider kind, OpenAI chat when unset.
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
        kind: kind_from(read)?,
        base_url: &url,
        model: &model,
        key_env: KEY_VARIABLE,
        temperature: temperature_from(read)?,
        request_bytes: None,
        residency: residency.as_deref(),
        prompt: None,
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
    /// `LARRY_MODEL_KIND` names the provider kind (OpenAI chat when unset),
    /// `LARRY_MODEL_TEMPERATURE`, `LARRY_MODEL_TIMEOUT_MS`,
    /// `LARRY_MODEL_ATTEMPTS` (1 to 16), `LARRY_MODEL_BACKOFF_MS` (zero when
    /// unset, like the executor, which never waits) and
    /// `LARRY_MODEL_DEADLINE_MS` fall back to their defaults, as do the
    /// token and micro-dollar budgets (unlimited when unset) and the price
    /// overrides (the route table's rates when it declares any).
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
    /// read from the variable the route names, now, and nowhere else. That
    /// variable is `LARRY_MODEL_KEY` for the environment route and a
    /// `LARRY_KEY_*` name for a stored one (`Route::new` refuses any other), so
    /// a tenant's route can never make the executor send another secret.
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
        let timeout = whole(read, "LARRY_MODEL_TIMEOUT_MS", DEFAULT_TIMEOUT_MS)?;
        let attempts: u32 = whole(read, "LARRY_MODEL_ATTEMPTS", DEFAULT_ATTEMPTS)?;
        if !(1..=MAX_ATTEMPTS).contains(&attempts) {
            return Err(ModelError::OutOfRange("LARRY_MODEL_ATTEMPTS"));
        }
        let backoff = whole(read, "LARRY_MODEL_BACKOFF_MS", DEFAULT_BACKOFF_MS)?;
        let deadline = whole(read, "LARRY_MODEL_DEADLINE_MS", DEFAULT_DEADLINE_MS)?;
        let token_budget = whole(read, "LARRY_MODEL_TOKEN_BUDGET", DEFAULT_TOKEN_BUDGET)?;
        let micro_budget = whole(
            read,
            "LARRY_MODEL_MICRO_DOLLAR_BUDGET",
            DEFAULT_MICRO_BUDGET,
        )?;
        // A declared route table prices the calls; the environment rates only
        // fill a route that carries none (the environment route always does).
        let (price_input, price_output) = if route.price_input() != 0 || route.price_output() != 0 {
            (route.price_input(), route.price_output())
        } else {
            (
                whole(read, "LARRY_MODEL_PRICE_IN_MICROS", 0)?,
                whole(read, "LARRY_MODEL_PRICE_OUT_MICROS", 0)?,
            )
        };
        Ok(Self {
            kind: route.kind(),
            base_url: route.base_url().to_owned(),
            api_key,
            model: route.model().to_owned(),
            temperature: route.temperature(),
            request_bytes: route.request_bytes(),
            timeout: Duration::from_millis(timeout),
            retry: RetryPolicy::new(
                attempts,
                Duration::from_millis(backoff),
                Duration::from_millis(deadline),
            ),
            token_budget,
            micro_budget,
            price_input,
            price_output,
        })
    }

    /// Which provider wire the client speaks.
    pub(super) const fn kind(&self) -> Kind {
        self.kind
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

    /// The sampling temperature, when the route pins one. A route that pins
    /// none sends no temperature member: a model that rejects the field would
    /// answer with a 4xx, which is terminal (larry#205).
    pub(super) const fn temperature(&self) -> Option<f64> {
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

    /// The tenant's maximum reported token usage.
    pub(super) const fn token_budget(&self) -> u64 {
        self.token_budget
    }

    /// The tenant's maximum micro-dollar spend.
    pub(super) const fn micro_budget(&self) -> u64 {
        self.micro_budget
    }

    /// The route's input price in micro-dollars per million tokens.
    pub(super) const fn price_input(&self) -> u64 {
        self.price_input
    }

    /// The route's output price in micro-dollars per million tokens.
    pub(super) const fn price_output(&self) -> u64 {
        self.price_output
    }
}

impl fmt::Debug for ModelConfig {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("ModelConfig")
            .field("base_url", &self.base_url)
            .field("kind", &self.kind.name())
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("temperature", &self.temperature)
            .field("request_bytes", &self.request_bytes)
            .field("timeout", &self.timeout)
            .field("retry", &self.retry)
            .field("token_budget", &self.token_budget)
            .field("micro_budget", &self.micro_budget)
            .field("price_input", &self.price_input)
            .field("price_output", &self.price_output)
            .finish()
    }
}

/// The config of a route the simulations never send on: its request bound is
/// `bound` bytes (the default when `None`) and its key is read from `LARRY_KEY_SIM`.
#[cfg(test)]
pub(crate) fn sim_config(bound: Option<usize>) -> Result<ModelConfig, ModelError> {
    let route = Route::new(&Draft {
        id: "sim",
        kind: Kind::OpenAiChat,
        base_url: "http://127.0.0.1:9",
        model: "sim-model",
        key_env: "LARRY_KEY_SIM",
        temperature: None,
        request_bytes: bound,
        residency: None,
        prompt: None,
        describe: "a route the simulation never sends on",
    })
    .map_err(ModelError::BadRoute)?;
    ModelConfig::for_route(&route, &|name: &str| {
        (name == "LARRY_KEY_SIM").then(|| String::from("sim-key"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The unset backoff matches the executor, which never waits for a retry:
    /// a wait it is asked for stops the retry at once, so the default asks for
    /// none. A default that waited would promise retries the executor refuses.
    #[test]
    fn the_unset_backoff_matches_the_executor_that_never_waits() -> Result<(), ModelError> {
        let config = ModelConfig::from_env(|name: &str| match name {
            "LARRY_MODEL_URL" => Some(String::from("http://127.0.0.1:9/v1")),
            "LARRY_MODEL" => Some(String::from("sim-model")),
            "LARRY_MODEL_KEY" => Some(String::from("sim-key")),
            _ => None,
        })?;
        assert_eq!(
            config.retry().base_delay,
            Duration::ZERO,
            "the unset backoff is zero, like the executor's"
        );
        Ok(())
    }
}
