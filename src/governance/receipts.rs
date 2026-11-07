//! Reranker receipt checks under `tools/receipts/`, ported from `receipt_errors` in
//! `tools/check_governance.py`. A receipt is admitted only with the pinned model identity and
//! bounded, well-formed fields.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use lgwks_std::json::{self, Value};

use super::pystr;

/// The reranker model every receipt must name.
const RERANKER_MODEL: &str = "Qwen/Qwen3-Reranker-0.6B";
/// The pinned model revision every receipt must name.
const RERANKER_REVISION: &str = "e61197ed45024b0ed8a2d74b80b4d909f1255473";
/// Largest number of receipt files admitted.
const MAX_RECEIPTS: usize = 4096;
/// Largest admitted receipt, in bytes.
const MAX_RECEIPT_BYTES: u64 = 1_048_576;
/// Keys every receipt must carry, beside exactly one digest pair of [`DIGEST_PAIRS`].
const REQUIRED: [&str; 6] = [
    "phase",
    "model",
    "revision",
    "candidate_count",
    "top_k",
    "results",
];
/// The digest pairs a receipt may carry: SHA-256 from the retired Python tool, BLAKE3 from
/// `larry-rerank` (larry#150, ADR 0013).
const DIGEST_PAIRS: [[&str; 2]; 2] = [
    ["query_sha256", "candidates_sha256"],
    ["query_blake3", "candidates_blake3"],
];

/// Every receipt error under `root`, in the Python order. A receipt that is not a JSON object is
/// refused as `invalid identity or shape`, where CPython raised a `TypeError` and gave no report.
pub(crate) fn receipt_errors(root: &Path) -> Vec<String> {
    let directory = root.join("tools").join("receipts");
    let mut names: Vec<String> = match fs::read_dir(&directory) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".json"))
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    if names.is_empty() {
        return vec![String::from("reranker receipts: none retained")];
    }
    if names.len() > MAX_RECEIPTS {
        return vec![format!("reranker receipts: more than {MAX_RECEIPTS}")];
    }
    let mut errors = Vec::new();
    let mut phases = BTreeSet::new();
    for name in &names {
        let relative = format!("tools/receipts/{name}");
        match receipt_phase(&directory.join(name)) {
            Ok(phase) => {
                phases.insert(phase);
            }
            Err(fault) => errors.push(format!("reranker receipt {relative}: {fault}")),
        }
    }
    let both: BTreeSet<&str> = ["pre-exec", "post-exec"].into_iter().collect();
    let observed: BTreeSet<&str> = phases.iter().map(String::as_str).collect();
    if observed != both {
        errors.push(String::from(
            "reranker receipts: both pre-exec and post-exec phases are required",
        ));
    }
    errors
}

/// Why one receipt is not admitted; each prints as the tail of the Python checker's line for it.
#[derive(Debug)]
enum ReceiptFault {
    /// The file could not be read.
    Unreadable(io::Error),
    /// The file is past the 1 MiB bound.
    TooLarge,
    /// The file is not UTF-8; CPython's codec text.
    NotUtf8(String),
    /// The file is not JSON.
    Json(json::Error),
    /// Wrong keys, phase, model or revision, or not an object.
    Shape,
    /// Counts, bounds, digests or result shapes out of range.
    Bounds,
}

impl fmt::Display for ReceiptFault {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Unreadable(ref cause) => write!(out, "{cause}"),
            Self::TooLarge => write!(out, "exceeds {MAX_RECEIPT_BYTES} bytes"),
            Self::NotUtf8(ref codec) => out.write_str(codec),
            Self::Json(ref cause) => write!(out, "{cause}"),
            Self::Shape => out.write_str("invalid identity or shape"),
            Self::Bounds => out.write_str("invalid bounds or hashes"),
        }
    }
}

/// Reads one receipt and returns its phase, or why it is refused.
fn receipt_phase(path: &Path) -> Result<String, ReceiptFault> {
    let metadata = fs::metadata(path).map_err(ReceiptFault::Unreadable)?;
    if metadata.len() > MAX_RECEIPT_BYTES {
        return Err(ReceiptFault::TooLarge);
    }
    let bytes = fs::read(path).map_err(ReceiptFault::Unreadable)?;
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(error) => return Err(ReceiptFault::NotUtf8(pystr::utf8_error(&bytes, &error))),
    };
    let receipt: Value = json::from_str(text).map_err(ReceiptFault::Json)?;
    let Some(object) = receipt.as_object() else {
        return Err(ReceiptFault::Shape);
    };
    let keys: BTreeSet<&str> = object.iter().map(|(key, _)| key).collect();
    let Some(pair) = DIGEST_PAIRS
        .iter()
        .find(|pair| pair.iter().all(|key| keys.contains(key)))
    else {
        return Err(ReceiptFault::Shape);
    };
    let required: BTreeSet<&str> = REQUIRED.iter().chain(pair.iter()).copied().collect();
    let allowed: BTreeSet<&str> = required.iter().copied().chain(["max_seconds"]).collect();
    if !required.is_subset(&keys) || !keys.is_subset(&allowed) {
        return Err(ReceiptFault::Shape);
    }
    let phase = receipt_string(&receipt, "phase");
    let model = receipt_string(&receipt, "model");
    let revision = receipt_string(&receipt, "revision");
    let phase_ok = matches!(phase.as_deref(), Some("pre-exec" | "post-exec"));
    if !phase_ok
        || model.as_deref() != Some(RERANKER_MODEL)
        || revision.as_deref() != Some(RERANKER_REVISION)
    {
        return Err(ReceiptFault::Shape);
    }
    if !bounds_hold(&receipt, pair) {
        return Err(ReceiptFault::Bounds);
    }
    phase.ok_or(ReceiptFault::Shape)
}

/// The string at `key`, or `None` when absent or not a string.
fn receipt_string(receipt: &Value, key: &str) -> Option<String> {
    receipt.get(key).and_then(Value::as_str).map(String::from)
}

/// Whether the counts, bounds, `pair` digests and result shapes of a receipt all hold.
fn bounds_hold(receipt: &Value, pair: &[&str; 2]) -> bool {
    let (Some(candidates), Some(top_k)) = (
        integer(receipt, "candidate_count"),
        integer(receipt, "top_k"),
    ) else {
        return false;
    };
    if top_k < 1 || top_k > candidates || candidates > 256 {
        return false;
    }
    if !seconds_hold(receipt) || !digests_hold(receipt, pair) {
        return false;
    }
    let Some(results) = receipt.get("results").and_then(Value::as_array) else {
        return false;
    };
    let count = match u64::try_from(results.len()) {
        Ok(count) => count,
        Err(_) => return false,
    };
    if count != top_k {
        return false;
    }
    results.iter().all(result_shape_holds)
}

/// An integer field, rejecting booleans and non-integers.
fn integer(receipt: &Value, key: &str) -> Option<u64> {
    receipt.get(key).and_then(Value::as_u64)
}

/// `max_seconds`, when present, is an integer in `1..=3600`.
fn seconds_hold(receipt: &Value) -> bool {
    match receipt.get("max_seconds") {
        None => true,
        Some(value) => value
            .as_u64()
            .is_some_and(|seconds| (1..=3600).contains(&seconds)),
    }
}

/// Both digests of `pair` are 64 lowercase hex digits.
fn digests_hold(receipt: &Value, pair: &[&str; 2]) -> bool {
    pair.iter().all(|key| {
        receipt
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|digest| {
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
    })
}

/// A result is an object with exactly `id` (a non-empty string) and `score` (a number).
fn result_shape_holds(result: &Value) -> bool {
    let Some(object) = result.as_object() else {
        return false;
    };
    let keys: BTreeSet<&str> = object.iter().map(|(key, _)| key).collect();
    let expected: BTreeSet<&str> = ["id", "score"].into_iter().collect();
    if keys != expected {
        return false;
    }
    let id_ok = result
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty());
    let score_ok = result.get("score").and_then(Value::as_f64).is_some();
    id_ok && score_ok
}
