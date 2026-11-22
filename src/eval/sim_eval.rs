//! Seeded simulations of `larry eval`'s scoring and set checks (larry#87).
//! Source: the nearest-rank percentile (NIST/SEMATECH e-Handbook 7.2.6.2), the
//! Artificial Analysis coding-agent method (cost per solved task is total spend
//! over solved tasks) and RFC 8259 (JSON numbers). Requirement: a price, a cost
//! and a percentile equal an independent reference for every seed; every
//! malformed set gets its exact error before anything runs; a valid set reads
//! back exactly. One seed drives every string, count and bound.

use std::error::Error;

use lgwks_std::json::json;
use lgwks_std::seeded::{DrawError, Seeded};

use super::score::{
    Ending, Outcome, PriceFault, Prices, Scorecard, Spent, card, parse_price, percentile, usd,
};
use super::{EvalTask, Origin, finish_origin};
use crate::sim_error::SimError;

/// What every test returns.
type TestResult = Result<(), Box<dyn Error>>;

/// Seeds the cheap simulations run.
const SEEDS: u64 = 2048;

/// Characters a fat-fingered price is typed from: digits, the separators people
/// reach for, signs, an exponent, an Arabic-Indic digit, a space and NUL.
const PRICE_KEYS: [char; 14] = [
    '0', '1', '5', '9', '.', '.', ',', '-', '+', 'e', '\u{663}', ' ', '\0', '7',
];

/// Digits folded into a wide integer, `None` on a non-digit or past `u128::MAX`.
fn wide_digits(digits: &str) -> Option<u128> {
    digits.chars().try_fold(0_u128, |value, digit| {
        value
            .checked_mul(10)?
            .checked_add(u128::from(digit.to_digit(10)?))
    })
}

/// The reference price parser: a second implementation over wide integers.
fn reference_price(text: &str) -> Result<u64, PriceFault> {
    let dots = text.matches('.').count();
    let (whole, fraction) = match text.split_once('.') {
        Some(parts) => parts,
        None => (text, ""),
    };
    let plain = |part: &str| !part.is_empty() && part.chars().all(|digit| digit.is_ascii_digit());
    if dots > 1 || !plain(whole) || (dots == 1 && !plain(fraction)) {
        return Err(PriceFault::Malformed);
    }
    if fraction.len() > 6 {
        return Err(PriceFault::TooPrecise);
    }
    let padded = format!("{whole}{fraction:0<6}");
    let micro = wide_digits(&padded).ok_or(PriceFault::TooLarge)?;
    let Ok(value) = u64::try_from(micro) else {
        return Err(PriceFault::TooLarge);
    };
    if value <= 10_000_000_000 {
        Ok(value)
    } else {
        Err(PriceFault::TooLarge)
    }
}

/// weird-bytes and fat-finger: seeded strings of digits, separators, signs,
/// exponents, non-ASCII digits, spaces and NUL, plus the named edges, parse to
/// exactly what the reference parser says. Catches a parser that accepts `1e3`,
/// `,` or a non-ASCII digit, truncates a seventh fraction digit, or overflows.
#[test]
fn sim_eval_weird_bytes_prices_match_a_reference_parser() -> TestResult {
    let edges = [
        "",
        ".",
        "1.",
        ".5",
        "0",
        "0.000001",
        "0.0000001",
        "10000",
        "10000.000001",
        "18446744073709551616",
        "00000000000000000000000000000.5",
        "0.30",
        "15",
    ];
    for edge in edges {
        assert_eq!(parse_price(edge), reference_price(edge), "edge {edge:?}");
    }
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let mut text = String::new();
        for _ in 0..rng.below(15)? {
            text.push(
                *PRICE_KEYS
                    .get(rng.index(PRICE_KEYS.len())?)
                    .ok_or("price keys")?,
            );
        }
        assert_eq!(
            parse_price(&text),
            reference_price(&text),
            "seed {seed}: {text:?}"
        );
    }
    Ok(())
}

/// The reference percentile: the smallest sample such that at least `percent`
/// percent of the samples do not exceed it, found by counting.
fn reference_percentile(samples: &[u64], percent: usize) -> Option<u64> {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let needed = percent.saturating_mul(sorted.len());
    sorted.iter().copied().find(|&value| {
        let within = sorted.iter().filter(|&&other| other <= value).count();
        within.saturating_mul(100) >= needed
    })
}

/// giant-and-empty: zero, one and hundreds of samples, heavy duplicates and
/// `u64::MAX`, at the 0th to 100th percentile, give the counting reference's
/// answer. Catches an off-by-one rank, a floor where a ceiling belongs, and a
/// panic on the empty set.
#[test]
fn sim_eval_giant_and_empty_percentiles_match_a_counting_reference() -> TestResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let count = rng.index(301)?;
        let spread = *[1, 3, 1_000, u64::MAX]
            .get(rng.index(4)?)
            .ok_or("spreads")?;
        let mut samples = Vec::with_capacity(count);
        for _ in 0..count {
            samples.push(if spread == u64::MAX {
                rng.next_u64()
            } else {
                rng.below(spread)?
            });
        }
        let mut sorted = samples.clone();
        sorted.sort_unstable();
        for percent in [0, 1, 50, 95, 99, 100] {
            assert_eq!(
                percentile(&sorted, percent),
                reference_percentile(&samples, percent),
                "seed {seed}: p{percent} of {count}"
            );
        }
    }
    Ok(())
}

/// Picodollars read back from a printed dollar amount; the reference for `usd`.
fn read_usd(text: &str) -> Option<u128> {
    let (whole, fraction) = text.split_once('.')?;
    if fraction.len() != 12 {
        return None;
    }
    wide_digits(&format!("{whole}{fraction}"))
}

/// What `prompt` and `completion` tokens cost at `input` and `output`, by checked
/// wide arithmetic: the reference for `Prices::cost`.
fn reference_cost(prompt: u64, completion: u64, input: u64, output: u64) -> Option<u128> {
    u128::from(prompt)
        .checked_mul(u128::from(input))?
        .checked_add(u128::from(completion).checked_mul(u128::from(output))?)
}

/// A spend with no cache report: every prompt token at the input price.
const fn plain(prompt: u64, completion: u64) -> Spent {
    Spent {
        prompt,
        completion,
        cached: 0,
        written: 0,
        cache_reported: true,
    }
}

/// giant-and-empty and saboteur: seeded prices to the bound and token counts to
/// `u64::MAX` cost what wide arithmetic says, print back to the same picodollars,
/// and a scorecard over seeded outcomes keeps every invariant: the counts add up,
/// the total is the sum of known costs, cost per accepted task is its floor share,
/// an unknown cost marks the card incomplete, and p50 <= p95 <= p99. Catches an
/// overflowing cost, a float rounding a cent away, and a card that hides refusals.
#[test]
fn sim_eval_costs_match_wide_arithmetic_and_cards_keep_their_invariants() -> TestResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let (input, output) = (rng.below(10_000_000_001)?, rng.below(10_000_000_001)?);
        let prices = Prices::micro(input, output);
        let (prompt, completion) = (rng.next_u64(), rng.next_u64());
        let wide = reference_cost(prompt, completion, input, output).ok_or("reference overflow")?;
        assert_eq!(
            prices.cost(plain(prompt, completion)),
            wide,
            "seed {seed}: cost"
        );
        assert_eq!(
            read_usd(&usd(wide)),
            Some(wide),
            "seed {seed}: printed cost reads back"
        );
        let mut outcomes = Vec::new();
        let mut known: u128 = 0;
        for index in 0..rng.index(40)? {
            let ending = match rng.below(3)? {
                0 => Ending::Accepted,
                1 => Ending::Rejected,
                _ => Ending::Refused(format!("refusal {index}")),
            };
            let tokens = match ending {
                Ending::Refused(_) => None,
                _ if rng.below(10)? == 0 => None,
                _ => Some(plain(rng.below(1 << 24)?, rng.below(1 << 20)?)),
            };
            if let Some(spent) = tokens {
                let task_cost = reference_cost(spent.prompt, spent.completion, input, output)
                    .ok_or("overflow")?;
                known = known.checked_add(task_cost).ok_or("overflow")?;
            }
            outcomes.push(Outcome {
                id: format!("t{index}"),
                ending,
                micros: rng.below(1 << 40)?,
                tokens,
            });
        }
        let scored = card("set", "model", prices, &outcomes);
        let sheet = scored.json();
        let field = |key: &str| sheet[key].as_u64();
        let tasks = u64::try_from(outcomes.len())?;
        assert_eq!(field("tasks"), Some(tasks), "seed {seed}: tasks");
        let ends: u64 = [field("accepted"), field("rejected"), field("refused")]
            .iter()
            .flatten()
            .sum();
        assert_eq!(ends, tasks, "seed {seed}: every task ends once");
        assert_eq!(
            sheet["cost_picousd"],
            json!(known.to_string()),
            "seed {seed}: total is the known sum"
        );
        let accepted = u128::from(field("accepted").ok_or("accepted")?);
        match sheet["cost_per_accepted_task_picousd"].as_str() {
            Some(text) => {
                let share = wide_digits(text).ok_or("share digits")?;
                let floor = share.checked_mul(accepted).ok_or("overflow")?;
                let ceiling = share.saturating_add(1).saturating_mul(accepted);
                assert!(
                    floor <= known && known < ceiling,
                    "seed {seed}: floor share"
                );
            }
            None => assert_eq!(
                accepted, 0,
                "seed {seed}: no share only without an accepted task"
            ),
        }
        let complete = outcomes.iter().all(|done| done.tokens.is_some());
        assert_eq!(
            sheet["cost_complete"],
            json!(complete),
            "seed {seed}: unknown cost is visible"
        );
        if !outcomes.is_empty() {
            let (p50, p95, p99) = (
                field("p50_micros"),
                field("p95_micros"),
                field("p99_micros"),
            );
            assert!(p50 <= p95 && p95 <= p99, "seed {seed}: percentiles ordered");
        }
        for result in sheet["results"].as_array().ok_or("results")? {
            let refused = result["outcome"] == "refused";
            assert_eq!(
                refused,
                result["reason"].is_string(),
                "seed {seed}: a refusal says why"
            );
        }
    }
    Ok(())
}

/// A seeded spend of `prompt` tokens whose cached and written counts fit inside it.
fn spend_within(rng: &mut Seeded, prompt: u64, reported: bool) -> Result<Spent, DrawError> {
    let cached = rng.below(prompt.saturating_add(1))?;
    let written = rng.below(prompt.saturating_sub(cached).saturating_add(1))?;
    Ok(Spent {
        prompt,
        completion: rng.below(1 << 20)?,
        cached,
        written,
        cache_reported: reported,
    })
}

/// The reference cost of `spent` by checked wide arithmetic: plain prompt tokens at
/// `input`, cached and written ones at their declared rate or `input`, completion at `output`.
fn reference_cache_cost(
    spent: Spent,
    rates: (u64, u64),
    declared: (Option<u64>, Option<u64>),
) -> Option<u128> {
    let (input, output) = rates;
    let price = |own: Option<u64>| match own {
        Some(value) => value,
        None => input,
    };
    let plain_tokens = u128::from(spent.prompt)
        .checked_sub(u128::from(spent.cached))?
        .checked_sub(u128::from(spent.written))?;
    plain_tokens
        .checked_mul(u128::from(input))?
        .checked_add(u128::from(spent.cached).checked_mul(u128::from(price(declared.0)))?)?
        .checked_add(u128::from(spent.written).checked_mul(u128::from(price(declared.1)))?)?
        .checked_add(u128::from(spent.completion).checked_mul(u128::from(output))?)
}

/// What a drawn set of outcomes adds up to, in wide integers.
struct Tally {
    /// Prompt tokens of the outcomes that counted tokens.
    prompt: u128,
    /// Cached tokens of those outcomes.
    cached: u128,
    /// Written tokens of those outcomes.
    written: u128,
    /// The reference cost of those outcomes.
    known: u128,
}

/// Up to eleven seeded outcomes, some without a token count, and their reference sums.
fn draw_outcomes(
    rng: &mut Seeded,
    rates: (u64, u64),
    declared: (Option<u64>, Option<u64>),
) -> Result<(Vec<Outcome>, Tally), SimError> {
    let mut outcomes = Vec::new();
    let mut tally = Tally {
        prompt: 0,
        cached: 0,
        written: 0,
        known: 0,
    };
    for index in 0..rng.index(12)? {
        let reported = rng.below(5)? != 0;
        let size = rng.below(1 << 40)?;
        let one = spend_within(rng, size, reported)?;
        let tokens = if rng.below(6)? == 0 { None } else { Some(one) };
        if let Some(counted) = tokens {
            tally.prompt = tally.prompt.saturating_add(u128::from(counted.prompt));
            tally.cached = tally.cached.saturating_add(u128::from(counted.cached));
            tally.written = tally.written.saturating_add(u128::from(counted.written));
            let task = reference_cache_cost(counted, rates, declared)
                .ok_or(SimError::Missing("reference cost"))?;
            tally.known = tally.known.saturating_add(task);
        }
        outcomes.push(Outcome {
            id: format!("t{index}"),
            ending: Ending::Accepted,
            micros: 1,
            tokens,
        });
    }
    Ok((outcomes, tally))
}

/// giant-and-empty, liar-upstream and fat-finger: for 256 seeds a spend whose cached and
/// written tokens fit the prompt (up to `u64::MAX`), priced with and without the two
/// optional cache rates, costs what wide checked arithmetic says; an undeclared rate is
/// the input rate; the card's cached tokens, written tokens and cache-hit permille equal
/// the reference sums; the card names which rates were declared; and one outcome that did
/// not report its cached tokens makes `cache_complete` false. Catches a cached token
/// billed at the full input price, a written one billed twice, a cache-hit rate computed
/// over the wrong total, and a card that calls a partial report complete.
#[test]
fn sim_cached_tokens_are_priced_at_their_own_rate() -> TestResult {
    for seed in 0..256 {
        let mut rng = Seeded::from_seed(seed);
        let (input, output) = (rng.below(10_000_000_001)?, rng.below(10_000_000_001)?);
        let cached_rate = match rng.below(2)? {
            0 => None,
            _ => Some(rng.below(10_000_000_001)?),
        };
        let write_rate = match rng.below(2)? {
            0 => None,
            _ => Some(rng.below(10_000_000_001)?),
        };
        let prices = Prices::micro(input, output).with_cache(cached_rate, write_rate);
        let ceiling = if rng.below(8)? == 0 {
            u64::MAX
        } else {
            rng.below(1 << 40)?
        };
        let spent = spend_within(&mut rng, ceiling, true)?;
        let want = reference_cache_cost(spent, (input, output), (cached_rate, write_rate))
            .ok_or("reference overflow")?;
        assert_eq!(prices.cost(spent), want, "seed {seed}: cost of {spent:?}");
        let at_input = Prices::micro(input, output)
            .with_cache(cached_rate.or(Some(input)), write_rate.or(Some(input)));
        assert_eq!(
            prices.cost(spent),
            at_input.cost(spent),
            "seed {seed}: an undeclared rate is the input rate"
        );
        let (mut outcomes, tally) =
            draw_outcomes(&mut rng, (input, output), (cached_rate, write_rate))?;
        let (prompt_sum, cached_sum, written_sum, known) =
            (tally.prompt, tally.cached, tally.written, tally.known);
        let all_reported = outcomes
            .iter()
            .all(|done| done.tokens.is_none_or(|counted| counted.cache_reported));
        let scored = card("set", "model", prices, &outcomes);
        let sheet = scored.json();
        assert_eq!(
            sheet["cached_tokens"].as_u64(),
            Some(u64::try_from(cached_sum)?),
            "seed {seed}: cached tokens"
        );
        assert_eq!(
            sheet["cache_write_tokens"].as_u64(),
            Some(u64::try_from(written_sum)?),
            "seed {seed}: written tokens"
        );
        let permille = match cached_sum.saturating_mul(1000).checked_div(prompt_sum) {
            Some(share) => Some(u64::try_from(share)?),
            None => None,
        };
        assert_eq!(
            sheet["cache_hit_permille"].as_u64(),
            permille,
            "seed {seed}: cache hit permille"
        );
        assert_eq!(
            sheet["cost_picousd"],
            json!(known.to_string()),
            "seed {seed}: the total prices each spend at its own rates"
        );
        assert_eq!(
            sheet["cache_complete"],
            json!(all_reported),
            "seed {seed}: complete only when every outcome reported"
        );
        assert_eq!(
            sheet["price_cached_declared"],
            json!(cached_rate.is_some()),
            "seed {seed}: the cached rate's declaration is on the card"
        );
        assert_eq!(
            sheet["price_cache_write_declared"],
            json!(write_rate.is_some()),
            "seed {seed}: the write rate's declaration is on the card"
        );
        check_rows(seed, &scored, &outcomes)?;
        check_silenced(seed, prices, &mut outcomes);
    }
    Ok(())
}

/// Each result row of `scored` names its outcome's cached and written tokens.
fn check_rows(seed: u64, scored: &Scorecard, outcomes: &[Outcome]) -> Result<(), SimError> {
    let sheet = scored.json();
    for (result, outcome) in sheet["results"]
        .as_array()
        .ok_or(SimError::Missing("results"))?
        .iter()
        .zip(outcomes)
    {
        assert_eq!(
            result["cached_tokens"].as_u64(),
            outcome.tokens.map(|counted| counted.cached),
            "seed {seed}: each result names its cached tokens"
        );
        assert_eq!(
            result["cache_write_tokens"].as_u64(),
            outcome.tokens.map(|counted| counted.written),
            "seed {seed}: each result names its written tokens"
        );
    }
    Ok(())
}

/// Settings over `state` with the test process's PATH.
fn settings_for(state: &std::path::Path) -> Result<crate::executor::Settings, SimError> {
    let state = state.to_string_lossy().into_owned();
    let path = std::env::var("PATH")?;
    let settings = crate::executor::Settings::from_env(move |name| match name {
        "LARRY_STATE_DIR" => Some(state.clone()),
        "PATH" => Some(path.clone()),
        _ => None,
    })?;
    Ok(settings)
}

/// A write that fails inside a new origin takes its partial tree and lock with it, so
/// nothing waits for the next sweep (larry#343).
#[test]
fn a_partial_origin_removes_itself_when_creation_fails() -> TestResult {
    let state = crate::executor::sim_tree::scratch("larry-eval-partial")?;
    let settings = settings_for(&state)?;
    let task = EvalTask {
        id: String::from("partial"),
        files: vec![
            (String::from("a"), String::from("x")),
            (String::from("a/b"), String::from("y")),
        ],
        goal: String::from("g"),
        write: Vec::new(),
        acceptance: Vec::new(),
    };
    let refused = Origin::create(&settings, 0, &task);
    assert!(refused.is_err(), "a file cannot be written under a file");
    let eval = state.join("eval");
    assert!(
        std::fs::read_dir(&eval)?.next().is_none(),
        "the partial origin and its lock are gone"
    );
    Ok(())
}

/// A cleanup that cannot remove its origin never fails the arm: the entry is left for the
/// next run's sweep and the task's outcome stands (larry#343).
#[test]
fn a_refused_origin_cleanup_keeps_the_arms_outcome() -> TestResult {
    let state = crate::executor::sim_tree::scratch("larry-eval-kept")?;
    let eval = state.join("eval");
    std::fs::create_dir_all(&eval)?;
    let dir = eval.join("origin-test-0");
    std::fs::write(&dir, b"not a directory")?;
    let lock = eval.join("origin-test-0.lock");
    let held = std::fs::File::create(&lock)?;
    let origin = Origin {
        dir: dir.clone(),
        lock,
        _held: held,
        cleaned: false,
    };
    let outcome = Outcome {
        id: String::from("kept"),
        ending: Ending::Accepted,
        micros: 7,
        tokens: None,
    };
    let finished = finish_origin(origin, Ok(outcome.clone()));
    assert!(
        matches!(finished, Ok(ref kept) if *kept == outcome),
        "the arm's outcome stands"
    );
    assert!(
        dir.exists(),
        "the tree the cleanup could not remove is left for the sweep"
    );
    Ok(())
}

/// One outcome that did not report its cached tokens makes the card incomplete.
fn check_silenced(seed: u64, prices: Prices, outcomes: &mut [Outcome]) {
    let silenced = match outcomes.iter_mut().find_map(|done| done.tokens.as_mut()) {
        Some(counted) => {
            counted.cache_reported = false;
            true
        }
        None => false,
    };
    let rescored = card("set", "model", prices, outcomes);
    assert_eq!(
        rescored.json()["cache_complete"],
        json!(!silenced),
        "seed {seed}: one outcome that did not report makes the card incomplete"
    );
}
