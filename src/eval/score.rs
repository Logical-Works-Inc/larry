//! Prices, costs and latency percentiles of `larry eval` (larry#87), in whole
//! numbers only. A price is micro-dollars per million tokens and a cost is
//! picodollars (tokens times micro-dollars per million tokens), so no float
//! rounds a cost and one run prints the same figures on every target.

use lgwks_macros::Error;
use lgwks_std::json::{Value, json};

/// Highest price accepted: 10,000 dollars per million tokens, in micro-dollars.
const MAX_PRICE_MICRO: u64 = 10_000_000_000;
/// Micro-dollars in one dollar, and the fraction digits a price may carry.
const MICRO_PER_USD: u64 = 1_000_000;
/// Fraction digits a price may carry: one micro-dollar per million tokens.
const PRICE_DIGITS: usize = 6;
/// Picodollars in one dollar.
const PICO_PER_USD: u128 = 1_000_000_000_000;

/// What one million tokens cost an arm, each direction, in micro-dollars.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prices {
    /// Price of a million prompt tokens.
    input: u64,
    /// Price of a million completion tokens.
    output: u64,
    /// Price of a million prompt tokens read from the provider's cache; the input
    /// price when the operator declared none (larry#230).
    cached: Option<u64>,
    /// Price of a million prompt tokens written to the provider's cache; the input
    /// price when the operator declared none (larry#230).
    cache_write: Option<u64>,
}

/// The tokens one task spent, as the gateway reported them (larry#230).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Spent {
    /// Prompt tokens, cached and written ones included.
    pub(crate) prompt: u64,
    /// Completion tokens.
    pub(crate) completion: u64,
    /// Prompt tokens read from the provider's cache, summed over the records that reported them.
    pub(crate) cached: u64,
    /// Prompt tokens written to the provider's cache, summed over the records that reported them.
    pub(crate) written: u64,
    /// Whether every completion of the task reported its cached tokens.
    pub(crate) cache_reported: bool,
}

/// Why a price setting was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PriceFault {
    /// Not a plain decimal: digits, then optionally `.` and digits.
    Malformed,
    /// More than six fraction digits: finer than a micro-dollar per million tokens.
    TooPrecise,
    /// Above 10,000 dollars per million tokens.
    TooLarge,
}

/// Why `LARRY_EVAL_PRICE_IN` or `LARRY_EVAL_PRICE_OUT` could not be used.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PriceError {
    /// The variable is unset or empty: a cost is never guessed.
    #[error("{variable} is unset: give the arm's dollars per million tokens, e.g. 0.30")]
    Missing {
        /// The variable.
        variable: &'static str,
    },
    /// The variable is set and refused.
    #[error("{variable} {}", fault.why())]
    Refused {
        /// The variable.
        variable: &'static str,
        /// Why.
        fault: PriceFault,
    },
}

impl PriceFault {
    /// What a refused setting is told, after its variable's name.
    const fn why(self) -> &'static str {
        match self {
            Self::Malformed => "is not a plain decimal like 0.30",
            Self::TooPrecise => "has more than 6 fraction digits",
            Self::TooLarge => "is over 10000 dollars per million tokens",
        }
    }
}

/// The value of an all-ASCII-digit string, `None` past `u64::MAX`.
fn decimal(digits: &str) -> Option<u64> {
    digits.bytes().try_fold(0_u64, |value, byte| {
        value
            .checked_mul(10)?
            .checked_add(u64::from(byte.checked_sub(b'0')?))
    })
}

/// Parses dollars per million tokens (`0.30`, `15`, `2.5`) into micro-dollars.
pub(crate) fn parse_price(text: &str) -> Result<u64, PriceFault> {
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (text, None),
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(whole) || fraction.is_some_and(|part| !digits(part)) {
        return Err(PriceFault::Malformed);
    }
    let fraction = match fraction {
        Some(part) if part.len() > PRICE_DIGITS => return Err(PriceFault::TooPrecise),
        Some(part) => format!("{part:0<6}"),
        None => String::from("000000"),
    };
    let micro = decimal(whole)
        .and_then(|units| units.checked_mul(MICRO_PER_USD))
        .and_then(|scaled| scaled.checked_add(decimal(&fraction)?))
        .ok_or(PriceFault::TooLarge)?;
    if micro > MAX_PRICE_MICRO {
        return Err(PriceFault::TooLarge);
    }
    Ok(micro)
}

impl Prices {
    /// Reads `LARRY_EVAL_PRICE_IN` and `LARRY_EVAL_PRICE_OUT` through `read`, and
    /// the optional `LARRY_EVAL_PRICE_CACHED` and `LARRY_EVAL_PRICE_CACHE_WRITE`
    /// (unset or empty: not declared, so the input price applies).
    pub fn from_env(read: impl Fn(&str) -> Option<String>) -> Result<Prices, PriceError> {
        let one = |variable: &'static str| -> Result<u64, PriceError> {
            let text = read(variable)
                .filter(|value| !value.trim().is_empty())
                .ok_or(PriceError::Missing { variable })?;
            parse_price(text.trim()).map_err(|fault| PriceError::Refused { variable, fault })
        };
        let optional = |variable: &'static str| -> Result<Option<u64>, PriceError> {
            match read(variable).filter(|value| !value.trim().is_empty()) {
                None => Ok(None),
                Some(text) => parse_price(text.trim())
                    .map(Some)
                    .map_err(|fault| PriceError::Refused { variable, fault }),
            }
        };
        Ok(Prices {
            input: one("LARRY_EVAL_PRICE_IN")?,
            output: one("LARRY_EVAL_PRICE_OUT")?,
            cached: optional("LARRY_EVAL_PRICE_CACHED")?,
            cache_write: optional("LARRY_EVAL_PRICE_CACHE_WRITE")?,
        })
    }

    /// The prices of a test, in micro-dollars per million tokens.
    #[cfg(test)]
    pub(crate) const fn micro(input: u64, output: u64) -> Prices {
        Prices {
            input,
            output,
            cached: None,
            cache_write: None,
        }
    }

    /// These prices with the cache rates a test declares.
    #[cfg(test)]
    pub(crate) const fn with_cache(self, cached: Option<u64>, cache_write: Option<u64>) -> Prices {
        Prices {
            cached,
            cache_write,
            ..self
        }
    }

    /// What `spent` costs, in picodollars: the prompt tokens neither cached nor
    /// written at the input price, the cached and written ones at their own
    /// price (the input price when none was declared), the completion at the
    /// output price. Cannot overflow: four products of a `u64` and a price under
    /// 2^34 sum below 2^100.
    pub(crate) fn cost(self, spent: Spent) -> u128 {
        let plain = spent
            .prompt
            .saturating_sub(spent.cached)
            .saturating_sub(spent.written);
        let rate = |declared: Option<u64>| match declared {
            Some(price) => price,
            None => self.input,
        };
        u128::from(plain)
            .saturating_mul(u128::from(self.input))
            .saturating_add(u128::from(spent.cached).saturating_mul(u128::from(rate(self.cached))))
            .saturating_add(
                u128::from(spent.written).saturating_mul(u128::from(rate(self.cache_write))),
            )
            .saturating_add(u128::from(spent.completion).saturating_mul(u128::from(self.output)))
    }
}

/// Picodollars as a dollar amount with all twelve fraction digits.
pub(crate) fn usd(pico: u128) -> String {
    format!(
        "{}.{:012}",
        pico.div_euclid(PICO_PER_USD),
        pico.rem_euclid(PICO_PER_USD)
    )
}

/// The nearest-rank `percent`ile of `sorted` (ascending): the smallest value at
/// least `percent` percent of the samples do not exceed. `None` for no samples.
pub(crate) fn percentile(sorted: &[u64], percent: usize) -> Option<u64> {
    let rank = percent.saturating_mul(sorted.len()).div_ceil(100).max(1);
    sorted.get(rank.saturating_sub(1)).copied()
}

/// How one task of an arm ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ending {
    /// Sealed and accepted.
    Accepted,
    /// Sealed and not accepted: a command failed or timed out.
    Rejected,
    /// The model step failed; nothing was sealed. The reason is the refusal.
    Refused(String),
}

/// One task's result: its id, how it ended, its wall time and its tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Outcome {
    /// The task id.
    pub(crate) id: String,
    /// How it ended.
    pub(crate) ending: Ending,
    /// Wall time, origin clone to seal, in microseconds.
    pub(crate) micros: u64,
    /// Prompt, completion and cache tokens, when every completion reported its counts.
    pub(crate) tokens: Option<Spent>,
}

/// The scorecard of one arm over one set.
#[derive(Debug, Clone)]
pub struct Scorecard {
    /// The card as printed.
    json: Value,
}

impl Scorecard {
    /// The card as printed: one JSON line.
    #[must_use]
    pub const fn json(&self) -> &Value {
        &self.json
    }

    /// The card with the id of the route the arm ran on (larry#201), so two
    /// cards of one model on two gateways are told apart.
    pub(crate) fn with_route(mut self, route: &str) -> Scorecard {
        if let Some(object) = self.json.as_object_mut() {
            object.insert(
                String::from("route"),
                Value::String(route.to_owned().into()),
            );
        }
        self
    }

    /// The card of an arm an infrastructure fault stopped at `task` (larry#399): the outcomes
    /// before it stand and were paid for, `complete` is false and `stopped` names the task and
    /// its typed cause.
    pub(crate) fn stopped(mut self, task: &str, cause: &str) -> Scorecard {
        if let Some(object) = self.json.as_object_mut() {
            object.insert(String::from("complete"), json!(false));
            object.insert(
                String::from("stopped"),
                json!({"task": task, "cause": cause}),
            );
        }
        self
    }
}

/// Scores `outcomes` of model `model` over set `set` at `prices`. Cost per accepted
/// task is every task's cost over the accepted count, rejected runs included, since
/// they were paid for; `cost_complete` is false when a refusal left a cost unknown.
pub(crate) fn card(set: &str, model: &str, prices: Prices, outcomes: &[Outcome]) -> Scorecard {
    let count =
        |wanted: fn(&Ending) -> bool| outcomes.iter().filter(|done| wanted(&done.ending)).count();
    let accepted = count(|ending| *ending == Ending::Accepted);
    let rejected = count(|ending| *ending == Ending::Rejected);
    let refused = count(|ending| matches!(*ending, Ending::Refused(_)));
    let (mut prompt, mut completion, mut pico) = (0_u64, 0_u64, 0_u128);
    let (mut cached, mut written) = (0_u64, 0_u64);
    let mut results = Vec::with_capacity(outcomes.len());
    for outcome in outcomes {
        let cost = outcome.tokens.map(|spent| {
            prompt = prompt.saturating_add(spent.prompt);
            completion = completion.saturating_add(spent.completion);
            cached = cached.saturating_add(spent.cached);
            written = written.saturating_add(spent.written);
            let task_cost = prices.cost(spent);
            pico = pico.saturating_add(task_cost);
            usd(task_cost)
        });
        let (ending, reason) = match outcome.ending {
            Ending::Accepted => ("accepted", None),
            Ending::Rejected => ("rejected", None),
            Ending::Refused(ref why) => ("refused", Some(why.clone())),
        };
        results.push(json!({
            "id": outcome.id,
            "outcome": ending,
            "reason": reason,
            "micros": outcome.micros,
            "prompt_tokens": outcome.tokens.map(|spent| spent.prompt),
            "completion_tokens": outcome.tokens.map(|spent| spent.completion),
            "cached_tokens": outcome.tokens.map(|spent| spent.cached),
            "cache_write_tokens": outcome.tokens.map(|spent| spent.written),
            "cost_usd": cost,
        }));
    }
    let mut micros: Vec<u64> = outcomes.iter().map(|done| done.micros).collect();
    micros.sort_unstable();
    let accepted_wide: u128 = outcomes
        .iter()
        .filter(|done| done.ending == Ending::Accepted)
        .map(|_| 1_u128)
        .sum();
    let cost_complete = outcomes.iter().all(|done| done.tokens.is_some());
    let per_accepted = if cost_complete {
        pico.checked_div(accepted_wide)
    } else {
        None
    };
    let per_accepted_usd = match per_accepted {
        Some(value) => Value::String(usd(value).into()),
        None if !cost_complete => Value::String(String::from("unknown").into()),
        None => Value::Null,
    };
    let per_accepted_pico = match per_accepted {
        Some(value) => Value::String(value.to_string().into()),
        None if !cost_complete => Value::String(String::from("unknown").into()),
        None => Value::Null,
    };
    Scorecard {
        json: json!({
            "set": set,
            "model": model,
            "tasks": outcomes.len(),
            "accepted": accepted,
            "rejected": rejected,
            "refused": refused,
            "acceptance_permille": accepted.saturating_mul(1000).checked_div(outcomes.len()),
            "prompt_tokens": prompt,
            "completion_tokens": completion,
            "cached_tokens": cached,
            "cache_write_tokens": written,
            "cache_hit_permille": cached.saturating_mul(1000).checked_div(prompt),
            "cache_complete": outcomes
                .iter()
                .all(|done| done.tokens.is_none_or(|spent| spent.cache_reported)),
            "price_cached_declared": prices.cached.is_some(),
            "price_cache_write_declared": prices.cache_write.is_some(),
            "cost_usd": usd(pico),
            "cost_picousd": pico.to_string(),
            "cost_per_accepted_task_usd": per_accepted_usd,
            "cost_per_accepted_task_picousd": per_accepted_pico,
            "cost_complete": cost_complete,
            "p50_micros": percentile(&micros, 50),
            "p95_micros": percentile(&micros, 95),
            "p99_micros": percentile(&micros, 99),
            "complete": true,
            "stopped": Value::Null,
            "results": results,
        }),
    }
}
