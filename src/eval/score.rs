//! Prices, costs and latency percentiles of `larry eval` (larry#87), in whole
//! numbers only. A price is micro-dollars per million tokens and a cost is
//! picodollars (tokens times micro-dollars per million tokens), so no float
//! rounds a cost and one run prints the same figures on every target.

use std::fmt;

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
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PriceError {
    /// The variable is unset or empty: a cost is never guessed.
    Missing {
        /// The variable.
        variable: &'static str,
    },
    /// The variable is set and refused.
    Refused {
        /// The variable.
        variable: &'static str,
        /// Why.
        fault: PriceFault,
    },
}

impl fmt::Display for PriceError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Missing { variable } => write!(
                out,
                "{variable} is unset: give the arm's dollars per million tokens, e.g. 0.30"
            ),
            Self::Refused { variable, fault } => {
                let why = match fault {
                    PriceFault::Malformed => "is not a plain decimal like 0.30",
                    PriceFault::TooPrecise => "has more than 6 fraction digits",
                    PriceFault::TooLarge => "is over 10000 dollars per million tokens",
                };
                write!(out, "{variable} {why}")
            }
        }
    }
}

impl std::error::Error for PriceError {}

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
    /// Reads `LARRY_EVAL_PRICE_IN` and `LARRY_EVAL_PRICE_OUT` through `read`.
    pub fn from_env(read: impl Fn(&str) -> Option<String>) -> Result<Prices, PriceError> {
        let one = |variable: &'static str| -> Result<u64, PriceError> {
            let text = read(variable)
                .filter(|value| !value.trim().is_empty())
                .ok_or(PriceError::Missing { variable })?;
            parse_price(text.trim()).map_err(|fault| PriceError::Refused { variable, fault })
        };
        Ok(Prices {
            input: one("LARRY_EVAL_PRICE_IN")?,
            output: one("LARRY_EVAL_PRICE_OUT")?,
        })
    }

    /// The prices of a test, in micro-dollars per million tokens.
    #[cfg(test)]
    pub(crate) const fn micro(input: u64, output: u64) -> Prices {
        Prices { input, output }
    }

    /// What `prompt` and `completion` tokens cost, in picodollars. Cannot overflow:
    /// two products of a `u64` and a price under 2^34 sum below 2^99.
    pub(crate) fn cost(self, prompt: u64, completion: u64) -> u128 {
        u128::from(prompt)
            .saturating_mul(u128::from(self.input))
            .saturating_add(u128::from(completion).saturating_mul(u128::from(self.output)))
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
    /// Prompt and completion tokens, when every completion reported them.
    pub(crate) tokens: Option<(u64, u64)>,
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
    let mut results = Vec::with_capacity(outcomes.len());
    for outcome in outcomes {
        let cost = outcome.tokens.map(|(spent_in, spent_out)| {
            prompt = prompt.saturating_add(spent_in);
            completion = completion.saturating_add(spent_out);
            let task_cost = prices.cost(spent_in, spent_out);
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
            "prompt_tokens": outcome.tokens.map(|pair| pair.0),
            "completion_tokens": outcome.tokens.map(|pair| pair.1),
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
    let per_accepted = pico.checked_div(accepted_wide);
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
            "cost_usd": usd(pico),
            "cost_picousd": pico.to_string(),
            "cost_per_accepted_task_usd": per_accepted.map(usd),
            "cost_per_accepted_task_picousd": per_accepted.map(|value| value.to_string()),
            "cost_complete": outcomes.iter().all(|done| done.tokens.is_some()),
            "p50_micros": percentile(&micros, 50),
            "p95_micros": percentile(&micros, 95),
            "p99_micros": percentile(&micros, 99),
            "results": results,
        }),
    }
}
