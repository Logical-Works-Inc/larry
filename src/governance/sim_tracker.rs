//! Seeded simulations of the tracker drift checks, the reranker receipt rules, every bound the
//! checker declares, and two repositories checked at once. The tracker controls port the
//! Python `ProgressTrackerTests`; the receipt simulation ports
//! `test_malformed_reranker_receipt_is_rejected` against an independent predicate. Source:
//! CWE-400 (uncontrolled resource consumption) for the bounds, CWE-59 for the symlink refusal,
//! and the Python checker larry#53 replaced. Requirement: each drift, malformed receipt and
//! oversize input gets its exact error; concurrent roots never share a finding.

use std::fs;

use lgwks_std::json::{self, Map, Value, json};
use lgwks_std::seeded::Seeded;

use super::documents::{DOCUMENTS, MAX_DOCUMENT_BYTES, MAX_DOCUMENTS};
use super::patterns::Patterns;
use super::sim_support::{Scratch, SimError, SimResult, assert_refused, pick, root};
use super::{check, pyjson, receipts, tracker};

/// Seeds the scratch-tree simulations run.
const SEEDS: u64 = 12;
/// The reranker model a receipt must name.
const MODEL: &str = "Qwen/Qwen3-Reranker-0.6B";
/// The model revision a receipt must name.
const REVISION: &str = "e61197ed45024b0ed8a2d74b80b4d909f1255473";

/// A scratch tracker: the three tracker files and the specification, as the Python `setUp` made.
fn tracker_scratch() -> Result<Scratch, SimError> {
    let scratch = Scratch::new("tracker")?;
    for name in ["tracker.json", "template.html", "index.html"] {
        let relative = format!("tools/tracker/{name}");
        scratch.write(&relative, &fs::read(root().join(&relative))?)?;
    }
    scratch.write(
        "specification.md",
        &fs::read(root().join("specification.md"))?,
    )?;
    Ok(scratch)
}

/// The scratch tracker's data.
fn data(scratch: &Scratch) -> Result<Value, SimError> {
    json::from_str::<Value>(&scratch.read("tools/tracker/tracker.json")?).map_err(SimError::Json)
}

/// Saves `value` as the scratch tracker's data, as the Python `save` did.
fn save(scratch: &Scratch, value: &Value) -> Result<(), SimError> {
    scratch.write(
        "tools/tracker/tracker.json",
        pyjson::dumps(value, Some(2)).as_bytes(),
    )
}

/// The drift errors of the scratch tracker.
fn drift(scratch: &Scratch) -> Result<Vec<String>, SimError> {
    let patterns = Patterns::new()?;
    Ok(tracker::drift_errors(
        scratch.path(),
        &scratch.read("specification.md")?,
        &patterns,
    ))
}

/// The array at `key` of the tracker data, to rewrite in place.
fn items_mut<'value>(
    value: &'value mut Value,
    key: &str,
) -> Result<&'value mut Vec<Value>, SimError> {
    value
        .get_mut(key)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| SimError::Absent(format!("tracker {key}")))
}

/// Sets `field` of the item at a seeded index of the array at `key` to `value`.
fn set_seeded_item(
    rng: &mut Seeded,
    data: &mut Value,
    key: &str,
    field: &str,
    value: Value,
) -> Result<usize, SimError> {
    let items = items_mut(data, key)?;
    let index = rng.index(items.len())?;
    let item = items
        .get_mut(index)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| SimError::Absent(format!("a {key} item")))?;
    item.insert(field, value);
    Ok(index)
}

/// Ports `test_tracker_is_current` and `test_edited_data_without_regenerating_is_refused`: the
/// copy is current, and any edited percentage left unrendered is stale.
#[test]
fn sim_edited_data_without_regenerating_is_refused() -> SimResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let scratch = tracker_scratch()?;
        assert!(
            drift(&scratch)?.is_empty(),
            "seed {seed}: the copied tracker is current"
        );
        let mut value = data(&scratch)?;
        let pct = rng.below(1000)?.saturating_add(101);
        set_seeded_item(&mut rng, &mut value, "cells", "pct", Value::from(pct))?;
        save(&scratch, &value)?;
        assert_refused(&drift(&scratch)?, "tools/tracker/index.html is stale", seed);
    }
    Ok(())
}

/// Ports `test_cell_inventory_drift_is_refused`: a dropped, swapped, repeated or renamed cell.
#[test]
fn sim_cell_inventory_drift_is_refused() -> SimResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let scratch = tracker_scratch()?;
        let mut value = data(&scratch)?;
        let choice = rng.below(4)?;
        if choice == 3 {
            set_seeded_item(&mut rng, &mut value, "cells", "id", Value::from("C99"))?;
        } else {
            let cells = items_mut(&mut value, "cells")?;
            let index = rng.index(cells.len())?;
            let other = rng.index(cells.len().saturating_sub(1))?;
            let other = if other >= index {
                other.saturating_add(1)
            } else {
                other
            };
            match choice {
                0 => {
                    cells.swap_remove(index);
                }
                1 => cells.swap(index, other),
                _ => {
                    let copy = cells
                        .get(index)
                        .cloned()
                        .ok_or_else(|| SimError::Absent(String::from("a cell")))?;
                    cells.push(copy);
                }
            }
        }
        save(&scratch, &value)?;
        assert_refused(
            &drift(&scratch)?,
            "progress tracker: cell inventory does not match specification.md",
            seed,
        );
    }
    Ok(())
}

/// Ports `test_falsifier_not_in_specification_is_refused`: one undeclared id, named exactly.
#[test]
fn sim_falsifier_not_in_specification_is_named() -> SimResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let scratch = tracker_scratch()?;
        let mut value = data(&scratch)?;
        let bogus = format!("E{}", rng.below(9)?.saturating_add(81));
        set_seeded_item(
            &mut rng,
            &mut value,
            "falsifiers",
            "id",
            Value::from(bogus.as_str()),
        )?;
        save(&scratch, &value)?;
        let errors = drift(&scratch)?;
        let expected =
            format!("progress tracker: falsifier ids not in specification.md: ['{bogus}']");
        assert!(
            errors.contains(&expected),
            "seed {seed}: expected {expected:?} in {errors:?}"
        );
    }
    Ok(())
}

/// Ports `test_falsifier_total_drift_is_refused`: each wrong total is shown as Python's `str`
/// shows it, and an integral float equal to the count passes as `80.0 == 80` does.
#[test]
fn sim_falsifier_total_drift_is_shown_as_python_shows_it() -> SimResult {
    let cases: [(Option<Value>, Option<&str>); 7] = [
        (Some(json!(79)), Some("79")),
        (Some(json!(80.5)), Some("80.5")),
        (Some(json!("80")), Some("80")),
        (Some(json!(true)), Some("True")),
        (Some(Value::Null), Some("None")),
        (None, Some("None")),
        (Some(json!(80.0)), None),
    ];
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let (total, shown) = pick(&mut rng, &cases)?.clone();
        let scratch = tracker_scratch()?;
        let mut value = data(&scratch)?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| SimError::Absent(String::from("tracker object")))?;
        match total {
            Some(total) => {
                object.insert("falsifier_total", total);
            }
            None => {
                object.remove("falsifier_total");
            }
        }
        save(&scratch, &value)?;
        let errors = drift(&scratch)?;
        match shown {
            Some(text) => {
                let expected = format!(
                    "progress tracker: falsifier_total is {text}, specification.md declares 80"
                );
                assert!(
                    errors.contains(&expected),
                    "seed {seed}: expected {expected:?} in {errors:?}"
                );
            }
            None => assert!(
                !errors.iter().any(|error| error.contains("falsifier_total")),
                "seed {seed}: 80.0 equals 80: {errors:?}"
            ),
        }
    }
    Ok(())
}

/// Ports `test_missing_render_is_refused`: a removed page, or a directory in its place.
#[test]
fn sim_missing_render_is_refused() -> SimResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let scratch = tracker_scratch()?;
        let page = scratch.path().join("tools/tracker/index.html");
        fs::remove_file(&page)?;
        if rng.below(2)? == 0 {
            fs::create_dir(&page)?;
        }
        let errors = drift(&scratch)?;
        assert!(
            errors.contains(&String::from(
                "progress tracker: tools/tracker/index.html is missing"
            )),
            "seed {seed}: {errors:?}"
        );
        let current: bool = tracker::is_current(scratch.path())?;
        assert!(!current, "seed {seed}: a missing page is never current");
    }
    Ok(())
}

/// Writers racing on one tracker each leave the whole render, and no temporary file survives.
#[test]
fn sim_concurrent_tracker_writes_leave_one_whole_page() -> SimResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let scratch = tracker_scratch()?;
        let mut value = data(&scratch)?;
        let pct = rng.below(101)?;
        set_seeded_item(&mut rng, &mut value, "cells", "pct", Value::from(pct))?;
        save(&scratch, &value)?;
        let writers = rng.index(7)?.saturating_add(2);
        let results: Vec<Result<usize, tracker::TrackerError>> = std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(writers);
            for _ in 0..writers {
                handles.push(scope.spawn(|| tracker::write(scratch.path())));
            }
            handles
                .into_iter()
                .filter_map(|handle| handle.join().ok())
                .collect()
        });
        assert_eq!(results.len(), writers, "seed {seed}: every writer finished");
        let expected = tracker::render(scratch.path())?;
        for result in results {
            assert_eq!(
                result?,
                expected.len(),
                "seed {seed}: each writer wrote the whole page"
            );
        }
        assert_eq!(
            scratch.read("tools/tracker/index.html")?,
            expected,
            "seed {seed}: the page is the render"
        );
        let leftovers = fs::read_dir(scratch.path().join("tools/tracker"))?.count();
        assert_eq!(leftovers, 3, "seed {seed}: no temporary file survives");
        assert!(
            drift(&scratch)?.is_empty(),
            "seed {seed}: the written tracker is current"
        );
    }
    Ok(())
}

/// A receipt the Python rules accept, with `top_k` results.
fn valid_receipt(rng: &mut Seeded) -> Result<Map, SimError> {
    let candidates = rng.below(256)?.saturating_add(1);
    let top_k = rng.below(candidates)?.saturating_add(1);
    let results: Vec<Value> = (0..top_k)
        .map(|index| json!({"id": format!("doc-{index}"), "score": 0.5}))
        .collect();
    let mut receipt = Map::new();
    receipt.insert("phase", Value::from("pre-exec"));
    receipt.insert("model", Value::from(MODEL));
    receipt.insert("revision", Value::from(REVISION));
    receipt.insert("query_sha256", Value::from("a".repeat(64)));
    receipt.insert(
        "candidates_sha256",
        Value::from("0123456789abcdef".repeat(4)),
    );
    receipt.insert("candidate_count", Value::from(candidates));
    receipt.insert("top_k", Value::from(top_k));
    receipt.insert("results", Value::Array(results));
    if rng.below(2)? == 0 {
        receipt.insert(
            "max_seconds",
            Value::from(rng.below(3600)?.saturating_add(1)),
        );
    }
    Ok(receipt)
}

/// One seeded perturbation of a valid receipt: a field replaced, removed or added.
fn perturb(rng: &mut Seeded, receipt: &mut Map) -> Result<(), SimError> {
    let fields = [
        "phase",
        "model",
        "revision",
        "query_sha256",
        "candidates_sha256",
        "candidate_count",
        "top_k",
        "results",
        "max_seconds",
        "extra",
    ];
    let field = *pick(rng, &fields)?;
    let replacements = [
        Value::Null,
        json!(true),
        json!(0),
        json!(-1),
        json!(1),
        json!(257),
        json!(3601),
        json!(1.5),
        json!("post-exec"),
        json!("x"),
        json!("A".repeat(64)),
        json!("b".repeat(63)),
        json!([]),
        json!([{"id": "", "score": 1}]),
        json!([{"id": "a", "score": true}]),
        json!([{"id": "a", "score": 1, "rank": 1}]),
        json!(["forged"]),
    ];
    if rng.below(5)? == 0 {
        receipt.remove(field);
    } else {
        receipt.insert(field, pick(rng, &replacements)?.clone());
    }
    Ok(())
}

/// A non-negative integer field, as the reference reads it: a JSON integer, never a boolean.
fn reference_integer(receipt: &Map, key: &str) -> Option<i64> {
    receipt
        .get(key)
        .filter(|value| matches!(**value, Value::Number(_)))
        .and_then(Value::as_i64)
}

/// Whether `item` is an object of exactly a non-empty string `id` and a numeric `score`.
fn reference_result(item: &Value) -> bool {
    let Some(object) = item.as_object() else {
        return false;
    };
    object.iter().count() == 2
        && item
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty())
        && matches!(item.get("score"), Some(&Value::Number(_)))
}

/// The reference verdict of the Python rules (booleans are not integers, ADR 0010):
/// `None` when admitted, else the error's tail.
fn reference_verdict(receipt: &Map) -> Option<&'static str> {
    let text = |key: &str| receipt.get(key).and_then(Value::as_str);
    let required = [
        "phase",
        "model",
        "revision",
        "query_sha256",
        "candidates_sha256",
        "candidate_count",
        "top_k",
        "results",
    ];
    let shape = required.iter().all(|key| receipt.get(key).is_some())
        && receipt
            .iter()
            .all(|(key, _)| required.contains(&key) || key == "max_seconds")
        && matches!(text("phase"), Some("pre-exec" | "post-exec"))
        && text("model") == Some(MODEL)
        && text("revision") == Some(REVISION);
    if !shape {
        return Some("invalid identity or shape");
    }
    let hex = |key: &str| {
        text(key).is_some_and(|digest| {
            digest.len() == 64
                && digest
                    .chars()
                    .all(|character| matches!(character, '0'..='9' | 'a'..='f'))
        })
    };
    let top_k = reference_integer(receipt, "top_k");
    let counts = match (reference_integer(receipt, "candidate_count"), top_k) {
        (Some(candidates), Some(top)) => 1 <= top && top <= candidates && candidates <= 256,
        _ => false,
    };
    let seconds = receipt.get("max_seconds").is_none()
        || reference_integer(receipt, "max_seconds")
            .is_some_and(|seconds| (1..=3600).contains(&seconds));
    let results = receipt
        .get("results")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            i64::try_from(items.len()).ok() == top_k && items.iter().all(reference_result)
        });
    if counts && seconds && hex("query_sha256") && hex("candidates_sha256") && results {
        None
    } else {
        Some("invalid bounds or hashes")
    }
}

/// Ports `test_malformed_reranker_receipt_is_rejected`: every seeded perturbation gets exactly
/// the errors an independent reference of the Python rules gives.
#[test]
fn sim_receipts_match_the_python_rules() -> SimResult {
    for seed in 0..256 {
        let mut rng = Seeded::from_seed(seed);
        let scratch = Scratch::new("receipts")?;
        let mut receipt = valid_receipt(&mut rng)?;
        if rng.below(8)? != 0 {
            perturb(&mut rng, &mut receipt)?;
        }
        let mut post = valid_receipt(&mut rng)?;
        post.insert("phase", Value::from("post-exec"));
        scratch.write(
            "tools/receipts/post.json",
            pyjson::dumps(&Value::Object(post), None).as_bytes(),
        )?;
        scratch.write(
            "tools/receipts/pre.json",
            pyjson::dumps(&Value::Object(receipt.clone()), None).as_bytes(),
        )?;
        let verdict = reference_verdict(&receipt);
        let mut expected = Vec::new();
        if let Some(tail) = verdict {
            expected.push(format!("reranker receipt tools/receipts/pre.json: {tail}"));
        }
        let admitted_pre =
            verdict.is_none() && receipt.get("phase").and_then(Value::as_str) == Some("pre-exec");
        if !admitted_pre {
            expected.push(String::from(
                "reranker receipts: both pre-exec and post-exec phases are required",
            ));
        }
        assert_eq!(
            receipts::receipt_errors(scratch.path()),
            expected,
            "seed {seed}: receipt {receipt:?}"
        );
    }
    Ok(())
}

/// The errors of the whole check of `scratch`.
fn check_errors(scratch: &Scratch) -> Vec<String> {
    let report: super::Report = check(scratch.path());
    report.errors()
}

/// One load-time bound case of `variant`, built in `scratch` from `rng`.
fn bound_case(variant: u64, rng: &mut Seeded, scratch: &Scratch, seed: u64) -> SimResult {
    let over = rng.index(64)?.saturating_add(1);
    match variant {
        0 => {
            scratch.write(
                "index.md",
                &vec![b'x'; MAX_DOCUMENT_BYTES.saturating_add(over)],
            )?;
            assert_eq!(
                check_errors(scratch),
                ["document exceeds byte limit: index.md"],
                "seed {seed}"
            );
        }
        1 => {
            let text = scratch.read("index.md")?;
            let pad = MAX_DOCUMENT_BYTES.saturating_sub(text.len());
            scratch.write("index.md", format!("{text}{}", "\n".repeat(pad)).as_bytes())?;
            let errors = check_errors(scratch);
            assert!(
                !errors.iter().any(|error| error.contains("byte limit")),
                "seed {seed}: exactly 2 MiB is admitted: {errors:?}"
            );
        }
        2 => {
            scratch.write(
                "tools/receipts/zz-large.json",
                &vec![b' '; 1_048_576_usize.saturating_add(over)],
            )?;
            assert_refused(
                &check_errors(scratch),
                "reranker receipt tools/receipts/zz-large.json: exceeds 1048576 bytes",
                seed,
            );
        }
        3 => {
            scratch.write(
                "tools/tracker/tracker.json",
                &vec![b' '; 4_194_304_usize.saturating_add(over)],
            )?;
            assert_refused(&check_errors(scratch), "exceeds 4194304 bytes", seed);
        }
        4 => {
            let name = *pick(rng, &["README.md", "log.md", "GOVERNANCE.md"])?;
            let target = scratch.path().join(name);
            fs::rename(&target, scratch.path().join("moved.txt"))?;
            std::os::unix::fs::symlink(scratch.path().join("moved.txt"), &target)?;
            assert_eq!(
                check_errors(scratch),
                [format!("symbolic-link document is not admitted: {name}")],
                "seed {seed}"
            );
        }
        _ => {
            let text = scratch.read("AGENTS.md")?;
            let boundaries: Vec<usize> = text.char_indices().map(|(offset, _)| offset).collect();
            let at = *pick(rng, &boundaries)?;
            let mut bytes = text.into_bytes();
            bytes.insert(at, 0xff);
            scratch.write("AGENTS.md", &bytes)?;
            let expected = format!(
                "'utf-8' codec can't decode byte 0xff in position {at}: invalid start byte"
            );
            assert_eq!(check_errors(scratch), [expected], "seed {seed}");
        }
    }
    Ok(())
}

/// Every declared bound: a document past 2 MiB, a receipt past 1 MiB and tracker data past
/// 4 MiB are refused; a document of exactly 2 MiB is admitted; a symlinked or non-UTF-8
/// document stops the load with its exact error.
#[test]
fn sim_bounds_refuse_past_every_limit() -> SimResult {
    for variant in 0..6_u64 {
        for round in 0..2_u64 {
            let seed = variant.saturating_mul(2).saturating_add(round);
            let mut rng = Seeded::from_seed(seed);
            let scratch = Scratch::governed("bounds")?;
            bound_case(variant, &mut rng, &scratch, seed)?;
        }
    }
    Ok(())
}

/// More than 4096 documents is refused before any is read; 4096 is admitted.
#[test]
fn sim_document_count_bound_is_exact() -> SimResult {
    for seed in 0..2 {
        let mut rng = Seeded::from_seed(seed);
        let scratch = Scratch::new("count")?;
        let room = MAX_DOCUMENTS.saturating_sub(DOCUMENTS.len());
        let nested: String = (0..rng.index(3)?)
            .map(|level| format!("d{level}/"))
            .collect();
        for index in 0..room {
            scratch.write(&format!("docs/{nested}page-{index}.md"), b"x")?;
        }
        let errors = check_errors(&scratch);
        assert!(
            !errors.iter().any(|error| error.contains("document count")),
            "seed {seed}: 4096 is admitted"
        );
        scratch.write(&format!("docs/{nested}page-extra.md"), b"x")?;
        assert_eq!(
            check_errors(&scratch),
            ["document count exceeds 4096"],
            "seed {seed}: one more is refused"
        );
    }
    Ok(())
}

/// Both reports of two repositories, as `larry governance --report` prints them.
fn both_reports(first: &Scratch, second: &Scratch) -> (String, String) {
    let (left, right): (super::Report, super::Report) = (check(first.path()), check(second.path()));
    (left.pretty(), right.pretty())
}

/// Two repositories checked on several threads at once each get exactly their solo report.
#[test]
fn sim_two_roots_checked_at_once_never_share_findings() -> SimResult {
    for seed in 0..6 {
        let mut rng = Seeded::from_seed(seed);
        let first = Scratch::governed("tenant-a")?;
        let second = Scratch::governed("tenant-b")?;
        let stray = format!(
            "\nStray reference R{} in tenant A.\n",
            rng.below(800)?.saturating_add(200)
        );
        let readme = first.read("README.md")?;
        first.write("README.md", format!("{readme}{stray}").as_bytes())?;
        if rng.below(2)? == 0 {
            fs::remove_file(second.path().join("log.md"))?;
        }
        let solo = both_reports(&first, &second);
        assert_ne!(solo.0, solo.1, "seed {seed}: the tenants differ");
        let threads = rng.index(3)?.saturating_add(2);
        let together: Vec<(String, String)> = std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(threads);
            for _ in 0..threads {
                handles.push(scope.spawn(|| both_reports(&first, &second)));
            }
            handles
                .into_iter()
                .filter_map(|handle| handle.join().ok())
                .collect()
        });
        assert_eq!(
            together.len(),
            threads,
            "seed {seed}: every thread finished"
        );
        for pair in together {
            assert_eq!(
                pair, solo,
                "seed {seed}: concurrent checks equal the solo reports"
            );
        }
    }
    Ok(())
}
