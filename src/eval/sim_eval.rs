//! Seeded simulations of `larry eval`'s scoring and set checks (larry#87).
//! Source: the nearest-rank percentile (NIST/SEMATECH e-Handbook 7.2.6.2), the
//! Artificial Analysis coding-agent method (cost per solved task is total spend
//! over solved tasks) and RFC 8259 (JSON numbers). Requirement: a price, a cost
//! and a percentile equal an independent reference for every seed; every
//! malformed set gets its exact error before anything runs; a valid set reads
//! back exactly. One seed drives every string, count and bound.

use std::error::Error;

use lgwks_std::json::{Map, Value, json};
use lgwks_std::seeded::{DrawError, Seeded};

use super::score::{Ending, Outcome, PriceFault, Prices, card, parse_price, percentile, usd};
use super::set::{SetError, TaskSet};
use crate::executor::TaskError;

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
    match u64::try_from(micro) {
        Ok(value) if value <= 10_000_000_000 => Ok(value),
        Ok(_) | Err(_) => Err(PriceFault::TooLarge),
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
            text.push(PRICE_KEYS[rng.index(PRICE_KEYS.len())?]);
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
        let spread = [1, 3, 1_000, u64::MAX][rng.index(4)?];
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
        assert_eq!(prices.cost(prompt, completion), wide, "seed {seed}: cost");
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
                _ => Some((rng.below(1 << 24)?, rng.below(1 << 20)?)),
            };
            if let Some((spent_in, spent_out)) = tokens {
                let task_cost =
                    reference_cost(spent_in, spent_out, input, output).ok_or("overflow")?;
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

/// A seeded lowercase word of 1 to 8 letters.
fn word(rng: &mut Seeded) -> Result<String, DrawError> {
    let mut text = String::new();
    for _ in 0..=rng.index(8)? {
        text.push(char::from(b"abcdefghijklmnopqrstuvwxyz"[rng.index(26)?]));
    }
    Ok(text)
}

/// A seeded valid set of `count` tasks with unicode, right-to-left and empty origin files.
fn valid_set(rng: &mut Seeded, count: usize) -> Result<Value, DrawError> {
    let mut tasks = Vec::with_capacity(count);
    for index in 0..count {
        let mut files = Map::new();
        for slot in 0..=rng.index(4)? {
            let content = ["", "ünïcödé\n", "#!/bin/sh\necho 42\n", "\u{202e}rtl"][rng.index(4)?];
            files.insert(format!("{}/{slot}.txt", word(rng)?), json!(content));
        }
        let write: Vec<String> = (0..=rng.index(3)?)
            .map(|slot| format!("out/{slot}.sh"))
            .collect();
        let acceptance: Vec<String> = (0..=rng.index(4)?)
            .map(|slot| format!("test -s out/{slot}.sh"))
            .collect();
        tasks.push(json!({
            "id": format!("{}-{index}", word(rng)?),
            "files": files,
            "goal": format!("goal {index}"),
            "write": write,
            "acceptance": acceptance,
        }));
    }
    Ok(json!({ "name": "matched-sim", "tasks": tasks }))
}

/// Task `index` of set `value`, to rewrite in place.
fn task_mut(value: &mut Value, index: usize) -> Option<&mut Map> {
    value.get_mut("tasks")?.get_mut(index)?.as_object_mut()
}

/// The set `value` after `edit` on its first task's object; `None` when it has none.
fn with_task(value: &Value, edit: fn(&mut Map)) -> Option<Vec<u8>> {
    let mut changed = value.clone();
    edit(task_mut(&mut changed, 0)?);
    Some(changed.to_string().into_bytes())
}

/// The error a set is refused with; `None` when it was accepted.
fn refusal(bytes: &[u8]) -> Option<SetError> {
    TaskSet::parse(bytes).err()
}

/// A whole set's bytes and the exact refusal they must draw.
type WholeMistake = (&'static [u8], fn(&SetError) -> bool);

/// One mistake made to task 0, and the exact refusal it must draw.
type Mistake = (fn(&mut Map), fn(&SetError) -> bool);

/// Every single mistake a fat-fingered author makes in one task.
const MISTAKES: [Mistake; 13] = [
    (
        |task| drop(task.insert(String::from("cost"), json!(0))),
        |error| matches!(*error, SetError::UnknownKey { index: Some(0), .. }),
    ),
    (
        |task| drop(task.insert(String::from("id"), json!(""))),
        |error| matches!(*error, SetError::BadId { index: 0 }),
    ),
    (
        |task| drop(task.insert(String::from("files"), json!({}))),
        |error| matches!(*error, SetError::FileCount { index: 0, count: 0 }),
    ),
    (
        |task| drop(task.insert(String::from("files"), json!({"../escape": "x"}))),
        |error| matches!(*error, SetError::BadFile { index: 0, .. }),
    ),
    (
        |task| drop(task.insert(String::from("files"), json!({".git/config": "x"}))),
        |error| matches!(*error, SetError::BadFile { index: 0, .. }),
    ),
    (
        |task| drop(task.insert(String::from("files"), json!({"big": "x".repeat(262_145)}))),
        |error| matches!(*error, SetError::FileTooLarge { index: 0, .. }),
    ),
    (
        |task| drop(task.insert(String::from("files"), json!({"a": 7}))),
        |error| {
            matches!(
                *error,
                SetError::WrongType {
                    index: Some(0),
                    key: "files"
                }
            )
        },
    ),
    (
        |task| drop(task.insert(String::from("acceptance"), json!([]))),
        |error| matches!(*error, SetError::AcceptanceCount { index: 0, count: 0 }),
    ),
    (
        |task| drop(task.insert(String::from("acceptance"), json!(vec!["true"; 17]))),
        |error| {
            matches!(
                *error,
                SetError::AcceptanceCount {
                    index: 0,
                    count: 17
                }
            )
        },
    ),
    (
        |task| drop(task.insert(String::from("write"), json!([]))),
        |error| {
            matches!(
                *error,
                SetError::Task {
                    index: 0,
                    cause: TaskError::FileCount { count: 0 }
                }
            )
        },
    ),
    (
        |task| drop(task.insert(String::from("write"), json!(["a", "a"]))),
        |error| {
            matches!(
                *error,
                SetError::Task {
                    index: 0,
                    cause: TaskError::BadFile { index: 1 }
                }
            )
        },
    ),
    (
        |task| drop(task.insert(String::from("goal"), json!(" "))),
        |error| {
            matches!(
                *error,
                SetError::Task {
                    index: 0,
                    cause: TaskError::BadGoal
                }
            )
        },
    ),
    (
        |task| drop(task.insert(String::from("goal"), json!(7))),
        |error| {
            matches!(
                *error,
                SetError::WrongType {
                    index: Some(0),
                    key: "goal"
                }
            )
        },
    ),
];

/// fat-finger, giant-and-empty and wrong-order: every single mistake in a seeded
/// valid set is refused with its exact error before anything runs. Catches a set
/// that runs with no acceptance (accepted for nothing), an origin file outside the
/// clone, a repeated id that would merge two tasks' scores, and an oversize set
/// read whole.
#[test]
fn sim_eval_fat_finger_sets_get_their_exact_error() -> TestResult {
    for seed in 0..64 {
        let mut rng = Seeded::from_seed(seed);
        let count = rng.index(4)?.saturating_add(2);
        let set = valid_set(&mut rng, count)?;
        assert!(
            refusal(set.to_string().as_bytes()).is_none(),
            "seed {seed}: the base set is valid"
        );
        for (index, &(edit, expected)) in MISTAKES.iter().enumerate() {
            let error = refusal(&with_task(&set, edit).ok_or("task 0")?).ok_or("accepted")?;
            assert!(
                expected(&error),
                "seed {seed}: mistake {index} gave {error}"
            );
        }
        let mut twins = set.clone();
        let first = twins["tasks"][0]["id"].clone();
        task_mut(&mut twins, 1)
            .ok_or("task 1")?
            .insert(String::from("id"), first);
        let error = refusal(twins.to_string().as_bytes()).ok_or("accepted twins")?;
        assert!(
            matches!(error, SetError::DuplicateId { index: 1 }),
            "seed {seed}: {error}"
        );
        let mut extra = set;
        extra
            .as_object_mut()
            .ok_or("set object")?
            .insert(String::from("extra"), json!(1));
        let error = refusal(extra.to_string().as_bytes()).ok_or("accepted extra")?;
        assert!(
            matches!(error, SetError::UnknownKey { index: None, .. }),
            "{error}"
        );
    }
    let whole: [WholeMistake; 7] = [
        (br#"{"tasks":[]}"#, |error| {
            matches!(
                *error,
                SetError::WrongType {
                    index: None,
                    key: "name"
                }
            )
        }),
        (br#"{"name":"Bad Name","tasks":[]}"#, |error| {
            matches!(*error, SetError::BadName)
        }),
        (br#"{"name":"s","tasks":[]}"#, |error| {
            matches!(*error, SetError::TaskCount { count: 0 })
        }),
        (br#"{"name":"s","tasks":{}}"#, |error| {
            matches!(
                *error,
                SetError::WrongType {
                    index: None,
                    key: "tasks"
                }
            )
        }),
        (br#"{"name":"s","tasks":[7]}"#, |error| {
            matches!(*error, SetError::NotAnObject { index: Some(0) })
        }),
        (b"[1]", |error| {
            matches!(*error, SetError::NotAnObject { index: None })
        }),
        (b"{\"name\":", |error| {
            matches!(*error, SetError::NotJson(_))
        }),
    ];
    for (bytes, expected) in whole {
        let error = refusal(bytes).ok_or("accepted")?;
        assert!(expected(&error), "{error}");
    }
    let oversize = refusal(&vec![b' '; 4_194_305]).ok_or("accepted oversize")?;
    assert!(
        matches!(oversize, SetError::TooLarge { len: 4_194_305 }),
        "{oversize}"
    );
    Ok(())
}

/// monkey: seeded valid sets of 1 to 12 tasks with unicode, right-to-left and
/// empty origin files read back exactly, task by task, in order, and each task
/// builds the executor document naming its files. Catches a reordered or dropped
/// task, a mangled origin file, and a set check that disagrees with the executor.
#[test]
fn sim_eval_valid_sets_read_back_exactly() -> TestResult {
    for seed in 0..256 {
        let mut rng = Seeded::from_seed(seed);
        let count = rng.index(12)?.saturating_add(1);
        let value = valid_set(&mut rng, count)?;
        let set = TaskSet::parse(value.to_string().as_bytes())?;
        assert_eq!(set.name, "matched-sim", "seed {seed}: name");
        assert_eq!(set.tasks.len(), count, "seed {seed}: every task");
        for (index, task) in set.tasks.iter().enumerate() {
            let written = &value["tasks"][index];
            assert_eq!(json!(task.id), written["id"], "seed {seed}: id {index}");
            assert_eq!(
                json!(task.goal),
                written["goal"],
                "seed {seed}: goal {index}"
            );
            assert_eq!(
                json!(task.write),
                written["write"],
                "seed {seed}: write {index}"
            );
            assert_eq!(
                json!(task.acceptance),
                written["acceptance"],
                "seed {seed}: acceptance {index}"
            );
            let files: Map = task
                .files
                .iter()
                .map(|pair| (pair.0.clone(), json!(pair.1)))
                .collect();
            assert_eq!(
                Value::Object(files),
                written["files"],
                "seed {seed}: files {index}"
            );
            assert!(
                task.task("/origin").is_ok(),
                "seed {seed}: task {index} builds its document"
            );
        }
    }
    Ok(())
}
