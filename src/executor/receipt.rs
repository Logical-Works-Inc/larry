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

use std::error::Error;
use std::fmt;

use lgwks_std::json::{self, Value};
use lgwks_std::sign::{self, PublicKey, SecretKey, Signature};

/// Domain tag: a seal over a Larry receipt never verifies as any other record.
const TAG: &[u8] = b"larry.receipt.v1";

/// The one signature algorithm a receipt names.
const ALGORITHM: &str = "ed25519";

/// The report member that carries the receipt.
const RECEIPT: &str = "receipt";

/// The report member that carries the verdict.
const ACCEPTED: &str = "accepted";

/// Largest report `verify` reads, in bytes (256 MiB). A report holds at most 16
/// commands with 1 MiB of stdout and 1 MiB of stderr each, plus a 1 MiB diff:
/// 33 MiB of text. JSON spends at most six bytes on one control byte, so the
/// largest honest report is under 199 MiB.
pub const MAX_REPORT_BYTES: usize = 268_435_456;

/// Why a receipt was refused. A refused receipt is never a success.
#[derive(Debug)]
#[non_exhaustive]
pub enum ReceiptError {
    /// The report exceeded [`MAX_REPORT_BYTES`].
    TooLarge {
        /// Report length in bytes.
        len: usize,
    },
    /// The report was not UTF-8.
    NotUtf8(std::str::Utf8Error),
    /// The report was not JSON.
    Malformed(json::Error),
    /// The report was not a JSON object.
    NotAnObject,
    /// The report carried no receipt.
    MissingReceipt,
    /// The receipt named an algorithm other than ed25519.
    BadAlgorithm,
    /// The receipt named no key.
    MissingKey,
    /// The key was not 64 hex characters.
    KeyNotHex(lgwks_std::hex::DecodeError),
    /// The key's 32 bytes do not encode an ed25519 point.
    KeyNotPoint(sign::KeyError),
    /// The receipt was sealed under a key the verifier does not trust.
    UntrustedKey,
    /// The receipt named no signature.
    MissingSignature,
    /// The signature was not 128 hex characters.
    SignatureNotHex(lgwks_std::hex::DecodeError),
    /// The seal does not match the report: a byte of it changed.
    Mismatch(sign::VerifyError),
    /// The sealed verdict contradicts the sealed results, or names no task.
    Inconsistent,
}

impl ReceiptError {
    /// The refusal text; a cause, when there is one, follows it.
    const fn fixed(&self) -> &'static str {
        match *self {
            Self::TooLarge { .. } => "receipt report is too large",
            Self::NotUtf8(_) => "receipt report is not UTF-8",
            Self::Malformed(_) => "receipt report is not valid JSON",
            Self::NotAnObject => "receipt report is not a JSON object",
            Self::MissingReceipt => "receipt is missing from the report",
            Self::BadAlgorithm => "receipt algorithm is not ed25519",
            Self::MissingKey => "receipt names no key",
            Self::KeyNotHex(_) | Self::KeyNotPoint(_) => {
                "receipt key is not a 64-hex ed25519 public key"
            }
            Self::UntrustedKey => "receipt is sealed under an untrusted key",
            Self::MissingSignature => "receipt names no signature",
            Self::SignatureNotHex(_) => "receipt signature is not 128 hex characters",
            Self::Mismatch(_) => "receipt does not match the report: it was altered",
            Self::Inconsistent => "receipt verdict contradicts the sealed results or names no task",
        }
    }
}

impl fmt::Display for ReceiptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.fixed())?;
        if let Self::TooLarge { len } = *self {
            return write!(formatter, " ({len} bytes)");
        }
        match self.source() {
            Some(cause) => write!(formatter, ": {cause}"),
            None => Ok(()),
        }
    }
}

impl Error for ReceiptError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match *self {
            Self::NotUtf8(ref utf8) => Some(utf8),
            Self::Malformed(ref json) => Some(json),
            Self::KeyNotHex(ref hex) | Self::SignatureNotHex(ref hex) => Some(hex),
            Self::KeyNotPoint(ref point) => Some(point),
            Self::Mismatch(ref seal) => Some(seal),
            Self::TooLarge { .. }
            | Self::NotAnObject
            | Self::MissingReceipt
            | Self::BadAlgorithm
            | Self::MissingKey
            | Self::UntrustedKey
            | Self::MissingSignature
            | Self::Inconsistent => None,
        }
    }
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

/// Whether a report body is an accepted run: no denial, a diff, and every
/// command exited 0 inside its timeout. Anything missing or mistyped is not.
#[must_use]
fn acceptance(body: &Value) -> bool {
    let none_denied = body
        .get("denied")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty);
    let has_diff = body.get("diff").is_some_and(Value::is_string);
    let all_passed = body
        .get("results")
        .and_then(Value::as_array)
        .is_some_and(|results| results.iter().all(passed));
    none_denied && has_diff && all_passed
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
pub(crate) fn seal(mut body: Value, key: &SecretKey) -> (Value, bool) {
    let accepted = acceptance(&body);
    let Some(object) = body.as_object_mut() else {
        return (body, false);
    };
    object.insert(ACCEPTED.to_owned(), Value::Bool(accepted));
    let signature = sign::sign(key, TAG, body.to_string().as_bytes());
    let receipt = json::json!({
        "alg": ALGORITHM,
        "key": lgwks_std::hex::encode(key.public_key().to_bytes()),
        "signature": lgwks_std::hex::encode(signature.to_bytes()),
    });
    if let Some(sealed) = body.as_object_mut() {
        sealed.insert(RECEIPT.to_owned(), receipt);
    }
    (body, accepted)
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
    if body.get(ACCEPTED).and_then(Value::as_bool) != Some(accepted) {
        return Err(ReceiptError::Inconsistent);
    }
    let task_id = body
        .get("task_id")
        .and_then(Value::as_str)
        .ok_or(ReceiptError::Inconsistent)?
        .to_owned();
    Ok(Verdict { accepted, task_id })
}

#[cfg(test)]
#[path = "receipt_tests.rs"]
mod tests;
