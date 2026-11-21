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
use super::read::{self, ReadFault};

/// The pinned reranker identity every receipt must name, as data: the model id on the first line and
/// its 40-hex revision on the second, the one home `larry-rerank` also reads (ADR 0013). A checker
/// compares the identity a receipt records; ADR 0039's "no model is compiled in" governs execution.
const MODEL_PIN: &str = include_str!("../rerank/model.pin");
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
    let mut names: Vec<String> = Vec::new();
    match fs::read_dir(&directory) {
        Ok(entries) => {
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(cause) => {
                        return vec![format!(
                            "reranker receipts: cannot list the receipts directory: {cause}"
                        )];
                    }
                };
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".json") {
                    names.push(name);
                }
            }
        }
        Err(cause) if cause.kind() == io::ErrorKind::NotFound => {}
        Err(cause) => {
            return vec![format!(
                "reranker receipts: cannot read the receipts directory: {cause}"
            )];
        }
    }
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
        match receipt_phase(root, &relative) {
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
    /// The file, or a directory on its path, is a symbolic link (larry#336).
    Symlink,
    /// The file is not a regular file: a FIFO or a device (larry#336).
    NotRegular,
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
            Self::Symlink => out.write_str("is a symbolic link"),
            Self::NotRegular => out.write_str("is not a regular file"),
            Self::TooLarge => write!(out, "exceeds {MAX_RECEIPT_BYTES} bytes"),
            Self::NotUtf8(ref codec) => out.write_str(codec),
            Self::Json(ref cause) => write!(out, "{cause}"),
            Self::Shape => out.write_str("invalid identity or shape"),
            Self::Bounds => out.write_str("invalid bounds or hashes"),
        }
    }
}

/// Reads one receipt and returns its phase, or why it is refused.
fn receipt_phase(root: &Path, relative: &str) -> Result<String, ReceiptFault> {
    let bytes = match read::read_bounded(root, Path::new(relative), MAX_RECEIPT_BYTES) {
        Ok(bytes) => bytes,
        Err(ReadFault::Missing(cause) | ReadFault::Unreadable(cause)) => {
            return Err(ReceiptFault::Unreadable(cause));
        }
        Err(ReadFault::Symlink) => return Err(ReceiptFault::Symlink),
        Err(ReadFault::NotRegular { .. }) => return Err(ReceiptFault::NotRegular),
        Err(ReadFault::TooLarge { .. }) => return Err(ReceiptFault::TooLarge),
    };
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(error) => return Err(ReceiptFault::NotUtf8(pystr::utf8_error(&bytes, &error))),
    };
    let receipt: Value = json::from_str(text).map_err(ReceiptFault::Json)?;
    let Some(object) = receipt.as_object() else {
        return Err(ReceiptFault::Shape);
    };
    let Some((pinned_model, pinned_revision)) = pinned_identity() else {
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
        || model.as_deref() != Some(pinned_model)
        || revision.as_deref() != Some(pinned_revision)
    {
        return Err(ReceiptFault::Shape);
    }
    if !bounds_hold(&receipt, pair) {
        return Err(ReceiptFault::Bounds);
    }
    phase.ok_or(ReceiptFault::Shape)
}

/// The pinned identity [`MODEL_PIN`] names: the model id on its first line and the revision on its
/// second. `None` when the data is not exactly two non-empty lines.
fn pinned_identity() -> Option<(&'static str, &'static str)> {
    let mut lines = MODEL_PIN.lines();
    match (lines.next(), lines.next(), lines.next()) {
        (Some(model), Some(revision), None) if !model.is_empty() && !revision.is_empty() => {
            Some((model, revision))
        }
        _ => None,
    }
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

/// `max_seconds`, when present, is an integer in `1..=3600`. The receipt schema makes it optional:
/// an absent field is a receipt that declares no limit, and every other absent key is refused by
/// the shape check before this one runs.
fn seconds_hold(receipt: &Value) -> bool {
    if let Some(value) = receipt.get("max_seconds") {
        return value
            .as_u64()
            .is_some_and(|seconds| (1..=3600).contains(&seconds));
    }
    true
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
