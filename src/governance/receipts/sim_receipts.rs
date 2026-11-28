//! Adversarial controls of the reranker receipt check (ADR 0013, larry#554): every field of a
//! receipt mutated to a hostile value and every kind of file where a receipt belongs.
//! Source: ADR 0013, CWE-20, CWE-59 and CWE-400. Requirement: only the pinned identity with
//! bounded, well-formed fields is admitted, and each refusal names the file and the reason.

use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};

use lgwks_std::json::{self, Value};

use super::super::sim_support::{Scratch, SimError, SimResult};
use super::{
    MAX_RECEIPT_BYTES, MAX_RECEIPTS, MODEL_PIN, ReceiptFault, pinned_identity, receipt_errors,
    receipt_phase,
};

/// The one receipt a case rewrites.
const CASE: &str = "tools/receipts/case.json";
/// The refusal for a directory that holds no admitted receipt of one phase.
const PHASES: &str = "reranker receipts: both pre-exec and post-exec phases are required";
/// The refusal of wrong keys or identity.
const SHAPE: &str = "invalid identity or shape";
/// The refusal of a value out of range.
const BOUNDS: &str = "invalid bounds or hashes";

/// One well-formed BLAKE3 receipt of `phase`.
fn valid(phase: &str) -> Result<Value, SimError> {
    let (model, revision) =
        pinned_identity().ok_or_else(|| SimError::Absent(String::from("the pinned identity")))?;
    let digest = "a".repeat(64);
    Ok(json::json!({
        "phase": phase,
        "model": model,
        "revision": revision,
        "candidate_count": 1,
        "top_k": 1,
        "query_blake3": digest,
        "candidates_blake3": digest,
        "results": [{"id": "a", "score": 1.0}],
    }))
}

/// The valid receipt with each `(key, value)` set, a `null` removing the key.
fn changed(changes: &[(&str, Value)]) -> Result<Value, SimError> {
    let mut receipt = valid("pre-exec")?;
    let object = receipt
        .as_object_mut()
        .ok_or_else(|| SimError::Absent(String::from("a receipt object")))?;
    for &(key, ref value) in changes {
        if value.is_null() {
            object.remove(key);
        } else {
            object.insert(String::from(key), value.clone());
        }
    }
    Ok(receipt)
}

/// The verdict on `bytes` as a receipt: its phase, or the refusal's text.
fn verdict(tree: &Scratch, bytes: &[u8]) -> Result<String, SimError> {
    tree.write(CASE, bytes)?;
    Ok(match receipt_phase(tree.path(), CASE) {
        Ok(phase) => phase,
        Err(fault) => fault.to_string(),
    })
}

/// The identity is the two lines of the pin, and a receipt of any other model, revision or phase,
/// with a missing or an extra key, or that is no object, is refused for its shape.
#[test]
fn sim_a_receipt_of_the_wrong_shape_or_identity_is_refused() -> SimResult {
    let pin: Vec<&str> = MODEL_PIN.lines().collect();
    assert_eq!(
        pinned_identity().map(|(model, revision)| vec![model, revision]),
        Some(pin)
    );
    let tree = Scratch::new("receipt-shape")?;
    let judged =
        |value: &Value| verdict(&tree, super::super::pyjson::dumps(value, None).as_bytes());
    assert_eq!(judged(&valid("pre-exec")?)?, "pre-exec");
    assert_eq!(judged(&valid("post-exec")?)?, "post-exec");
    let digest = Value::from("a".repeat(64));
    let retained = changed(&[
        ("query_blake3", Value::Null),
        ("candidates_blake3", Value::Null),
        ("query_sha256", digest.clone()),
        ("candidates_sha256", digest.clone()),
    ])?;
    assert_eq!(judged(&retained)?, "pre-exec", "the retained SHA-256 pair");

    let both_pairs = changed(&[
        ("query_sha256", digest.clone()),
        ("candidates_sha256", digest),
    ])?;
    for (case, receipt) in [
        ("an array", json::json!([1])),
        ("both pairs", both_pairs),
        ("half a pair", changed(&[("query_blake3", Value::Null)])?),
        ("an extra key", changed(&[("extra", Value::from(1))])?),
        ("no results", changed(&[("results", Value::Null)])?),
        (
            "a third phase",
            changed(&[("phase", Value::from("mid-exec"))])?,
        ),
        ("a numeric phase", changed(&[("phase", Value::from(7))])?),
        (
            "another model",
            changed(&[("model", Value::from("other"))])?,
        ),
        (
            "another revision",
            changed(&[("revision", Value::from("0"))])?,
        ),
    ] {
        assert_eq!(judged(&receipt)?, SHAPE, "{case}");
    }
    Ok(())
}

/// Counts, digests, the optional time limit and every result are held to their exact range: the
/// edge is admitted and one past it refused.
#[test]
fn sim_a_receipt_value_one_past_its_range_is_refused() -> SimResult {
    let tree = Scratch::new("receipt-bounds")?;
    let judged =
        |value: &Value| verdict(&tree, super::super::pyjson::dumps(value, None).as_bytes());
    let one = |key: &str, value: Value| changed(&[(key, value)]);
    for admitted in [
        one("candidate_count", Value::from(256))?,
        one("max_seconds", Value::from(1))?,
        one("max_seconds", Value::from(3600))?,
        one("results", json::json!([{"id": "a", "score": -2}]))?,
    ] {
        assert_eq!(judged(&admitted)?, "pre-exec", "{admitted:?}");
    }
    let zero = changed(&[
        ("candidate_count", Value::from(0)),
        ("top_k", Value::from(0)),
        ("results", json::json!([])),
    ])?;
    let past_candidates = changed(&[
        ("top_k", Value::from(2)),
        (
            "results",
            json::json!([{"id": "a", "score": 1}, {"id": "b", "score": 1}]),
        ),
    ])?;
    for refused in [
        zero,
        past_candidates,
        one("candidate_count", Value::from(257))?,
        one("candidate_count", Value::from(true))?,
        one("candidate_count", Value::from("1"))?,
        one("top_k", Value::from(1.5))?,
        one("max_seconds", Value::from(0))?,
        one("max_seconds", Value::from(3601))?,
        one("max_seconds", Value::from(-1))?,
        one("max_seconds", Value::from("soon"))?,
        one("query_blake3", Value::from("a".repeat(63)))?,
        one("query_blake3", Value::from("A".repeat(64)))?,
        one("candidates_blake3", Value::from("g".repeat(64)))?,
        one("candidates_blake3", Value::from(1))?,
        one("results", json::json!({"id": "a", "score": 1}))?,
        one("results", json::json!([]))?,
        one("results", json::json!([[1]]))?,
        one("results", json::json!([{"id": "a"}]))?,
        one("results", json::json!([{"id": "a", "score": 1, "x": 1}]))?,
        one("results", json::json!([{"id": "", "score": 1}]))?,
        one("results", json::json!([{"id": 1, "score": 1}]))?,
        one("results", json::json!([{"id": "a", "score": "1"}]))?,
    ] {
        assert_eq!(judged(&refused)?, BOUNDS, "{refused:?}");
    }
    Ok(())
}

/// Bytes that are no JSON or no UTF-8, a file past the bound, a missing one, a directory and a
/// link are each their own refusal (CWE-59, CWE-400).
#[test]
fn sim_a_hostile_receipt_file_keeps_its_own_refusal() -> SimResult {
    let tree = Scratch::new("receipt-files")?;
    tree.write(CASE, b"not json")?;
    let refused = receipt_phase(tree.path(), CASE);
    assert!(matches!(refused, Err(ReceiptFault::Json(_))), "{refused:?}");
    let codec = verdict(&tree, &[0xff])?;
    assert!(
        codec.starts_with("'utf-8' codec can't decode byte 0xff in position 0"),
        "{codec}"
    );

    let bound = usize::try_from(MAX_RECEIPT_BYTES)?;
    assert_eq!(
        verdict(&tree, &vec![b' '; bound.saturating_add(1)])?,
        format!("exceeds {MAX_RECEIPT_BYTES} bytes")
    );
    let refused = receipt_phase(tree.path(), CASE);
    assert!(
        matches!(refused, Err(ReceiptFault::TooLarge)),
        "{refused:?}"
    );

    let refused = receipt_phase(tree.path(), "tools/receipts/absent.json");
    assert!(
        matches!(&refused, Err(ReceiptFault::Unreadable(cause))
            if cause.kind() == std::io::ErrorKind::NotFound),
        "{refused:?}"
    );
    let receipts = tree.path().join("tools/receipts");
    fs::create_dir(receipts.join("dir.json"))?;
    let refused =
        receipt_phase(tree.path(), "tools/receipts/dir.json").map_err(|fault| fault.to_string());
    assert_eq!(refused, Err(String::from("is not a regular file")));
    symlink(receipts.join("case.json"), receipts.join("link.json"))?;
    let refused =
        receipt_phase(tree.path(), "tools/receipts/link.json").map_err(|fault| fault.to_string());
    assert_eq!(refused, Err(String::from("is a symbolic link")));
    Ok(())
}

/// The directory check: none retained, one phase alone, a bad receipt named beside the pair, a
/// pair admitted past a file of another kind, one receipt past the count bound and a directory
/// that cannot be listed.
#[test]
fn sim_the_receipts_directory_needs_both_phases_within_its_bound() -> SimResult {
    let tree = Scratch::new("receipt-directory")?;
    let none = "reranker receipts: none retained";
    assert_eq!(receipt_errors(tree.path()), [none]);
    tree.write("tools/receipts/note.txt", b"not a receipt")?;
    assert_eq!(receipt_errors(tree.path()), [none]);

    let pre = super::super::pyjson::dumps(&valid("pre-exec")?, None);
    let post = super::super::pyjson::dumps(&valid("post-exec")?, None);
    tree.write("tools/receipts/pre.json", pre.as_bytes())?;
    assert_eq!(receipt_errors(tree.path()), [PHASES]);
    tree.write("tools/receipts/post.json", post.as_bytes())?;
    assert_eq!(receipt_errors(tree.path()), Vec::<String>::new());
    tree.write("tools/receipts/bad.json", b"[1]")?;
    assert_eq!(
        receipt_errors(tree.path()),
        [format!("reranker receipt tools/receipts/bad.json: {SHAPE}")]
    );

    let directory = tree.path().join("tools/receipts");
    for number in 3..MAX_RECEIPTS {
        fs::hard_link(
            directory.join("pre.json"),
            directory.join(format!("n{number}.json")),
        )?;
    }
    assert_eq!(receipt_errors(tree.path()).len(), 1, "the bound itself");
    fs::hard_link(directory.join("pre.json"), directory.join("past.json"))?;
    assert_eq!(
        receipt_errors(tree.path()),
        [format!("reranker receipts: more than {MAX_RECEIPTS}")]
    );

    fs::set_permissions(&directory, fs::Permissions::from_mode(0o000))?;
    let locked = receipt_errors(tree.path());
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755))?;
    assert!(
        matches!(locked.as_slice(), [one]
            if one.starts_with("reranker receipts: cannot read the receipts directory: ")),
        "{locked:?}"
    );
    Ok(())
}
