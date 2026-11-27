//! Signed receipts over a run's report (larry#86, ADR 0006).
//!
//! The body `larry exec` prints (task id, task digest, every command's result,
//! the diff, the denials and the verdict) is the run's journal. It is sealed
//! once, at the end of the run, with `lgwks_std::sign`: ed25519 over [`TAG`]
//! and the BLAKE3 of the body's RFC 8785 bytes. Compact JSON over this body is
//! RFC 8785: every key is ASCII and sorted (no `preserve_order`), every number
//! is an integer, and strings carry only the minimal escapes. A verifier
//! rebuilds the body without its `receipt` member, checks the seal under the
//! key it trusts, then re-derives the verdict, so a seal over a success claim
//! the results contradict is refused too.

use lgwks_macros::Error;
use lgwks_std::json::{self, Value};
use lgwks_std::sign::{self, PublicKey, SecretKey, Signature};

use super::workspace::DIFF_LIMIT;

/// Domain tag: a seal over a Larry receipt never verifies as any other record.
const TAG: &[u8] = b"larry.receipt.v1";

/// The one signature algorithm a receipt names.
const ALGORITHM: &str = "ed25519";

/// The report member that carries the receipt.
pub(crate) const RECEIPT: &str = "receipt";

/// The report member that carries the verdict.
const ACCEPTED: &str = "accepted";

/// The report member that says the diff is not the whole change (larry#206).
const DIFF_TRUNCATED: &str = "diff_truncated";

/// The report member that says which key id sealed the report (larry#211): a
/// rotated key verifies old receipts by id and seals new ones under the new id.
pub(crate) const KEY_ID: &str = "kid";

/// Largest report `verify` reads, in bytes (256 MiB). A report holds at most 16
/// commands with 1 MiB of stdout and 1 MiB of stderr each, plus a 1 MiB diff:
/// 33 MiB of text. JSON spends at most six bytes on one control byte, so the
/// largest honest report is under 199 MiB.
pub const MAX_REPORT_BYTES: usize = 268_435_456;

/// Why a receipt was refused. A refused receipt is never a success.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ReceiptError {
    /// The report exceeded [`MAX_REPORT_BYTES`].
    #[error("receipt report is too large ({len} bytes)")]
    TooLarge {
        /// Report length in bytes.
        len: usize,
    },
    /// The report was not UTF-8.
    #[error("receipt report is not UTF-8: {0}")]
    NotUtf8(#[source] std::str::Utf8Error),
    /// The report was not JSON.
    #[error("receipt report is not valid JSON: {0}")]
    Malformed(#[source] json::Error),
    /// The report was not a JSON object.
    #[error("receipt report is not a JSON object")]
    NotAnObject,
    /// The report carried no receipt.
    #[error("receipt is missing from the report")]
    MissingReceipt,
    /// The receipt named an algorithm other than ed25519.
    #[error("receipt algorithm is not ed25519")]
    BadAlgorithm,
    /// The receipt named no key.
    #[error("receipt names no key")]
    MissingKey,
    /// The key was not 64 hex characters.
    #[error("receipt key is not a 64-hex ed25519 public key: {0}")]
    KeyNotHex(#[source] lgwks_std::hex::DecodeError),
    /// The key's 32 bytes do not encode an ed25519 point.
    #[error("receipt key is not a 64-hex ed25519 public key: {0}")]
    KeyNotPoint(#[source] sign::KeyError),
    /// The receipt was sealed under a key the verifier does not trust.
    #[error("receipt is sealed under an untrusted key")]
    UntrustedKey,
    /// The receipt named no signature.
    #[error("receipt names no signature")]
    MissingSignature,
    /// The receipt's key id is not a positive integer.
    #[error("receipt key id is not a positive integer")]
    BadKeyId,
    /// The signature was not 128 hex characters.
    #[error("receipt signature is not 128 hex characters: {0}")]
    SignatureNotHex(#[source] lgwks_std::hex::DecodeError),
    /// The seal does not match the report: a byte of it changed.
    #[error("receipt does not match the report: it was altered: {0}")]
    Mismatch(#[source] sign::VerifyError),
    /// The sealed verdict contradicts the sealed results, or names no task.
    #[error("receipt verdict contradicts the sealed results or names no task")]
    Inconsistent,
    /// The receipt claims accepted over a diff it says it cut (larry#206).
    #[error("receipt claims accepted over a truncated diff")]
    TruncatedDiffAccepted,
}

/// What a verified receipt says about its run.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Verdict {
    /// Whether the run was accepted: no denial, a diff, every command passed.
    accepted: bool,
    /// The run's task id.
    task_id: String,
}

impl Verdict {
    /// Whether the run was accepted.
    #[must_use]
    pub const fn accepted(&self) -> bool {
        self.accepted
    }

    /// The run's task id.
    #[must_use]
    pub fn task_id(&self) -> &str {
        self.task_id.as_str()
    }
}

/// Whether a report body is an accepted run: no denial, a diff that is the whole change, at least
/// one command, and every command exited 0 inside its timeout. Anything missing or mistyped is
/// not, and a report that ran no command checked nothing (larry#395).
#[must_use]
fn acceptance(body: &Value) -> bool {
    let none_denied = body
        .get("denied")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty);
    let has_diff = body.get("diff").is_some_and(Value::is_string);
    let whole_diff = !diff_cut(body);
    let all_passed = body
        .get("results")
        .and_then(Value::as_array)
        .is_some_and(|results| !results.is_empty() && results.iter().all(passed));
    none_denied && has_diff && whole_diff && all_passed
}

/// Whether the sealed diff is not the whole change (larry#206). A report that carries
/// `diff_truncated` says so. One sealed before the member existed is cut only when its diff
/// reaches the limit, because its length cannot show whether anything was dropped. A member
/// that is not a boolean is cut: the refusal fails closed.
#[must_use]
fn diff_cut(body: &Value) -> bool {
    match body.get(DIFF_TRUNCATED) {
        None => body
            .get("diff")
            .and_then(Value::as_str)
            .is_none_or(|diff| diff.len() >= DIFF_LIMIT),
        Some(member) => member.as_bool() != Some(false),
    }
}

/// Whether one command result exited 0 inside its timeout.
fn passed(result: &Value) -> bool {
    result.get("exit_code").and_then(Value::as_i64) == Some(0)
        && result.get("timed_out").and_then(Value::as_bool) == Some(false)
}

/// Seals a report body: inserts its derived verdict, then the receipt over both.
/// Returns the sealed report and the verdict. A body that is not an object
/// gets no receipt and is never accepted.
#[must_use]
pub(crate) fn seal(mut body: Value, key: &SecretKey, key_id: i64) -> (Value, bool) {
    let accepted = acceptance(&body);
    let Some(object) = body.as_object_mut() else {
        return (body, false);
    };
    object.insert(ACCEPTED.to_owned(), Value::Bool(accepted));
    let signature = sign::sign(key, TAG, body.to_string().as_bytes());
    let receipt = json::json!({
        "alg": ALGORITHM,
        "key": lgwks_std::hex::encode(key.public_key().to_bytes()),
        "kid": key_id,
        "signature": lgwks_std::hex::encode(signature.to_bytes()),
    });
    if let Some(sealed) = body.as_object_mut() {
        sealed.insert(RECEIPT.to_owned(), receipt);
    }
    (body, accepted)
}

/// The key id a sealed report names: what `seal` wrote as `kid`, or 1 for a
/// report sealed before key ids existed. Refuses a mistyped member rather than
/// guessing which key sealed it.
pub(crate) fn key_id_of(body: &Value) -> Result<i64, ReceiptError> {
    let Some(receipt) = body.get(RECEIPT) else {
        return Err(ReceiptError::MissingReceipt);
    };
    match receipt.get(KEY_ID) {
        None => Ok(1),
        Some(member) => member
            .as_i64()
            .filter(|id| *id >= 1)
            .ok_or(ReceiptError::BadKeyId),
    }
}

/// Parses a public key from its 64 hex characters.
pub(crate) fn parse_public_key(text: &str) -> Result<PublicKey, ReceiptError> {
    let mut bytes = [0_u8; 32];
    lgwks_std::hex::decode_into(text, &mut bytes).map_err(ReceiptError::KeyNotHex)?;
    PublicKey::from_bytes(bytes).map_err(ReceiptError::KeyNotPoint)
}

/// Parses a signature from its 128 hex characters.
fn parse_signature(text: &str) -> Result<Signature, ReceiptError> {
    let mut bytes = [0_u8; 64];
    lgwks_std::hex::decode_into(text, &mut bytes).map_err(ReceiptError::SignatureNotHex)?;
    Ok(Signature::from_bytes(bytes))
}

/// Verifies a printed report against the key the caller trusts.
pub fn verify(bytes: &[u8], trusted: &PublicKey) -> Result<Verdict, ReceiptError> {
    if bytes.len() > MAX_REPORT_BYTES {
        return Err(ReceiptError::TooLarge { len: bytes.len() });
    }
    let text = std::str::from_utf8(bytes).map_err(ReceiptError::NotUtf8)?;
    let mut body = json::from_str::<Value>(text).map_err(ReceiptError::Malformed)?;
    // The body is every member but the receipt. Its map is sorted (no
    // `preserve_order`; `sim_canonical_bytes_roundtrip_for_every_seed` holds
    // that), so removing a member leaves exactly the bytes that were sealed.
    let receipt = body
        .as_object_mut()
        .ok_or(ReceiptError::NotAnObject)?
        .remove(RECEIPT)
        .ok_or(ReceiptError::MissingReceipt)?;
    if receipt.get("alg").and_then(Value::as_str) != Some(ALGORITHM) {
        return Err(ReceiptError::BadAlgorithm);
    }
    let key = parse_public_key(
        receipt
            .get("key")
            .and_then(Value::as_str)
            .ok_or(ReceiptError::MissingKey)?,
    )?;
    if key != *trusted {
        return Err(ReceiptError::UntrustedKey);
    }
    let signature = parse_signature(
        receipt
            .get("signature")
            .and_then(Value::as_str)
            .ok_or(ReceiptError::MissingSignature)?,
    )?;
    sign::verify_strict(trusted, TAG, body.to_string().as_bytes(), &signature)
        .map_err(ReceiptError::Mismatch)?;
    let accepted = acceptance(&body);
    let claimed = body.get(ACCEPTED).and_then(Value::as_bool);
    if claimed == Some(true) && diff_cut(&body) {
        return Err(ReceiptError::TruncatedDiffAccepted);
    }
    if claimed != Some(accepted) {
        return Err(ReceiptError::Inconsistent);
    }
    let task_id = body
        .get("task_id")
        .and_then(Value::as_str)
        .ok_or(ReceiptError::Inconsistent)?
        .to_owned();
    Ok(Verdict { accepted, task_id })
}

/// What a run seals under: the tenant's active receipt key id and secret.
#[derive(Clone, Copy)]
pub(crate) struct Seal<'seal> {
    /// The key id the receipt names.
    pub(crate) id: i64,
    /// The secret that seals.
    pub(crate) key: &'seal SecretKey,
}

/// One sealed run report and the verdict its receipt carries.
#[derive(Debug)]
#[non_exhaustive]
pub struct Report {
    /// The report with its `accepted` verdict and `receipt`.
    json: Value,
    /// Whether the run was accepted.
    accepted: bool,
    /// The sealed report exactly as it was printed, so a retry answers with the
    /// bytes the ledger recorded rather than a re-rendering of them.
    bytes: Vec<u8>,
}

impl Report {
    /// The sealed report, as printed.
    #[must_use]
    pub const fn json(&self) -> &Value {
        &self.json
    }

    /// The sealed report's exact bytes, the value the ledger keeps.
    #[must_use]
    pub(crate) fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    /// Whether the run was accepted: no denial, a diff, every command passed.
    #[must_use]
    pub const fn accepted(&self) -> bool {
        self.accepted
    }

    /// Whether this report carries a seal at all: one the gate could not make
    /// durable is never a record a retry may answer from (larry#182).
    #[must_use]
    pub(crate) fn carries_a_seal(&self) -> bool {
        self.json.get(RECEIPT).is_some()
    }

    /// Seals a report body under `key` as key id `key_id`.
    pub(crate) fn sealed(body: Value, key: &SecretKey, key_id: i64) -> Report {
        let (json, accepted) = seal(body, key, key_id);
        let bytes = json.to_string().into_bytes();
        Report {
            json,
            accepted,
            bytes,
        }
    }

    /// The report a ledger recorded, re-derived rather than trusted: its seal is
    /// checked under this state directory's key and its verdict re-derived, so a
    /// retry never answers with a record this state directory did not seal
    /// (larry#182 defect 3).
    ///
    /// # Errors
    ///
    /// `ExecError::Receipt` when the recorded bytes are not a report this
    /// directory's key seals.
    pub(crate) fn parse(bytes: &[u8], key: &SecretKey) -> Result<Report, super::ExecError> {
        let verdict = verify(bytes, &key.public_key()).map_err(super::ExecError::Receipt)?;
        let json = json::from_slice(bytes)
            .map_err(|cause| super::ExecError::Receipt(ReceiptError::Malformed(cause)))?;
        Ok(Report {
            json,
            accepted: verdict.accepted(),
            bytes: bytes.to_vec(),
        })
    }

    /// The report the ledger recorded for `tenant`, verified under the exact
    /// key id its receipt names, so a retry answers from the row its own run
    /// sealed and never from another tenant's (larry#211).
    ///
    /// # Errors
    ///
    /// `ExecError::Receipt` when the bytes are not a report,
    /// `ExecError::NoReceiptKey` when the named key id is gone, and
    /// `ExecError::Sql` when the lookup fails.
    pub(crate) fn parse_for(
        ledger: &super::Ledger,
        tenant: &str,
        bytes: &[u8],
    ) -> Result<Report, super::ExecError> {
        let body = json::from_slice::<Value>(bytes)
            .map_err(|cause| super::ExecError::Receipt(ReceiptError::Malformed(cause)))?;
        let key_id = key_id_of(&body).map_err(super::ExecError::Receipt)?;
        let key = ledger
            .receipt_secret(tenant, key_id)?
            .ok_or(super::ExecError::NoReceiptKey)?;
        Self::parse(bytes, &key)
    }
}

#[cfg(test)]
#[path = "receipt_tests.rs"]
mod tests;
