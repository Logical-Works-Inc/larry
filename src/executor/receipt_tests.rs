//! Seeded simulations of the receipt seal (larry#86).
//!
//! Source: RFC 8032 (ed25519, strict verification), RFC 8785 (JSON
//! canonicalization) and forge#158 (a denied call reported as success).
//! Requirement proved: every sealed report verifies under its key, every
//! tampered byte, key, algorithm or verdict is refused with its exact error, the
//! canonical bytes round-trip for any text, and a seal over a success claim the
//! results contradict is refused.

use lgwks_std::json::{self, Value};
use lgwks_std::seeded::Seeded;
use lgwks_std::sign::{self, SecretKey};

use super::{
    ACCEPTED, DIFF_TRUNCATED, MAX_REPORT_BYTES, RECEIPT, ReceiptError, TAG, acceptance, seal,
    verify,
};
use crate::executor::workspace::{DIFF_LIMIT, Diff, DiffSink, report_members};

/// What every test here returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Seeds each simulation sweeps.
const SEEDS: u64 = 64;

/// Characters the generated text is drawn from: ASCII, every JSON escape,
/// control bytes, DEL, the line separator and non-BMP text.
const ALPHABET: [char; 16] = [
    'a', 'Z', '0', ' ', '"', '\\', '/', '\n', '\t', '\u{0}', '\u{1f}', '\u{7f}', 'é', '漢',
    '\u{2028}', '😀',
];

/// A seeded string of up to 24 characters from [`ALPHABET`].
fn text(stream: &mut Seeded) -> Result<String, Box<dyn std::error::Error>> {
    let length = stream.below(25)?;
    let mut drawn = String::new();
    for _ in 0..length {
        let index = stream.index(ALPHABET.len())?;
        drawn.push(*ALPHABET.get(index).ok_or("draw outside the alphabet")?);
    }
    Ok(drawn)
}

/// A seeded report body of the family `larry exec` prints.
fn body(stream: &mut Seeded) -> Result<Value, Box<dyn std::error::Error>> {
    let mut results = Vec::new();
    for _ in 0..stream.below(5)? {
        let exit_code = match stream.below(4)? {
            0 | 1 => json::json!(0),
            2 => json::json!(3),
            _ => Value::Null,
        };
        results.push(json::json!({
            "argv": [text(stream)?, text(stream)?],
            "pgid": stream.below(1 << 20)?,
            "exit_code": exit_code,
            "timed_out": stream.below(5)? == 0,
            "stdout": text(stream)?,
            "stderr": text(stream)?,
            "truncated": stream.below(2)? == 0,
            "duration_ms": stream.below(1 << 30)?,
        }));
    }
    let diff = if stream.below(6)? == 0 {
        Value::Null
    } else {
        json::json!(text(stream)?)
    };
    let denied = if stream.below(6)? == 0 {
        json::json!(["model_commands"])
    } else {
        json::json!([])
    };
    Ok(json::json!({
        "task_id": text(stream)?,
        "task_digest": text(stream)?,
        "diff": diff,
        "results": results,
        "denied": denied,
    }))
}

/// A signing key whose seed the stream draws.
fn key(stream: &mut Seeded) -> SecretKey {
    let mut seed = [0_u8; 32];
    for byte in &mut seed {
        *byte = stream.next_u64().to_le_bytes()[0];
    }
    SecretKey::from_seed(seed)
}

/// Sets the member `key` of the object `value` to `member`.
fn set(value: &mut Value, key: &str, member: Value) -> TestResult {
    value
        .as_object_mut()
        .ok_or("the edited value is an object")?
        .insert(key, member);
    Ok(())
}

/// The receipt object of a sealed report, mutable.
fn receipt_of(report: &mut Value) -> Result<&mut Value, Box<dyn std::error::Error>> {
    Ok(report
        .get_mut(RECEIPT)
        .ok_or("the sealed report has a receipt")?)
}

/// Verifies `report` under `trusted`, as printed text.
fn verify_value(
    report: &Value,
    trusted: &lgwks_std::sign::PublicKey,
) -> Result<super::Verdict, ReceiptError> {
    verify(report.to_string().as_bytes(), trusted)
}

/// Every sealed report verifies under its key, and every tamper is refused
/// with its exact error.
#[test]
fn sim_sealed_reports_verify_and_every_tamper_is_refused_for_every_seed() -> TestResult {
    for seed in 0..SEEDS {
        let mut stream = Seeded::from_seed(seed);
        let original = body(&mut stream)?;
        let expected = acceptance(&original);
        let signer = key(&mut stream);
        let trusted = signer.public_key();
        let (sealed, accepted) = seal(original, &signer);
        assert_eq!(
            accepted, expected,
            "seed {seed}: the seal keeps the derived verdict"
        );
        let verdict = verify_value(&sealed, &trusted)?;
        assert_eq!(
            verdict.accepted(),
            accepted,
            "seed {seed}: the verdict reads back"
        );

        let mut altered = sealed.clone();
        let task_id = format!("{}x", verdict.task_id());
        set(&mut altered, "task_id", json::json!(task_id))?;
        assert!(
            matches!(
                verify_value(&altered, &trusted),
                Err(ReceiptError::Mismatch(_))
            ),
            "seed {seed}: an altered task id is refused"
        );

        let mut flipped = sealed.clone();
        set(&mut flipped, ACCEPTED, Value::Bool(!accepted))?;
        assert!(
            matches!(
                verify_value(&flipped, &trusted),
                Err(ReceiptError::Mismatch(_))
            ),
            "seed {seed}: a flipped verdict is refused"
        );

        let mut extra = sealed.clone();
        set(&mut extra, "denied", json::json!([]))?;
        set(&mut extra, "results", json::json!([]))?;
        set(&mut extra, "diff", json::json!(""))?;
        if extra != sealed {
            assert!(
                matches!(
                    verify_value(&extra, &trusted),
                    Err(ReceiptError::Mismatch(_))
                ),
                "seed {seed}: a report rewritten to look accepted is refused"
            );
        }

        let mut forged_signature = sealed.clone();
        let signature = receipt_of(&mut forged_signature)?;
        let hex = signature["signature"].as_str().ok_or("signature is text")?;
        let first = if hex.starts_with('0') { "1" } else { "0" };
        let changed = format!("{first}{}", hex.get(1..).ok_or("signature has a tail")?);
        set(signature, "signature", json::json!(changed))?;
        assert!(
            matches!(
                verify_value(&forged_signature, &trusted),
                Err(ReceiptError::Mismatch(_))
            ),
            "seed {seed}: a changed signature byte is refused"
        );

        let stranger = key(&mut stream);
        assert!(
            matches!(
                verify_value(&sealed, &stranger.public_key()),
                Err(ReceiptError::UntrustedKey)
            ),
            "seed {seed}: a verifier holding another key refuses the receipt"
        );
        let (resealed, _) = seal(body(&mut Seeded::from_seed(seed))?, &stranger);
        assert!(
            matches!(
                verify_value(&resealed, &trusted),
                Err(ReceiptError::UntrustedKey)
            ),
            "seed {seed}: a report re-sealed under an attacker's key is refused"
        );

        let mut unsigned = sealed.clone();
        unsigned
            .as_object_mut()
            .ok_or("the report is an object")?
            .remove(RECEIPT);
        assert!(
            matches!(
                verify_value(&unsigned, &trusted),
                Err(ReceiptError::MissingReceipt)
            ),
            "seed {seed}: a report with no receipt is refused"
        );

        let mut algorithm = sealed.clone();
        set(receipt_of(&mut algorithm)?, "alg", json::json!("rsa"))?;
        assert!(
            matches!(
                verify_value(&algorithm, &trusted),
                Err(ReceiptError::BadAlgorithm)
            ),
            "seed {seed}: another algorithm is refused"
        );

        let mut short_key = sealed.clone();
        set(receipt_of(&mut short_key)?, "key", json::json!("abc"))?;
        assert!(
            matches!(
                verify_value(&short_key, &trusted),
                Err(ReceiptError::KeyNotHex(_))
            ),
            "seed {seed}: a key that is not 64 hex characters is refused"
        );

        let mut keyless = sealed.clone();
        set(receipt_of(&mut keyless)?, "key", Value::Null)?;
        assert!(
            matches!(
                verify_value(&keyless, &trusted),
                Err(ReceiptError::MissingKey)
            ),
            "seed {seed}: a receipt that names no key is refused"
        );

        let mut unsealed = sealed.clone();
        set(receipt_of(&mut unsealed)?, "signature", Value::Null)?;
        assert!(
            matches!(
                verify_value(&unsealed, &trusted),
                Err(ReceiptError::MissingSignature)
            ),
            "seed {seed}: a receipt that names no signature is refused"
        );

        let mut short_signature = sealed;
        set(
            receipt_of(&mut short_signature)?,
            "signature",
            json::json!("00"),
        )?;
        assert!(
            matches!(
                verify_value(&short_signature, &trusted),
                Err(ReceiptError::SignatureNotHex(_))
            ),
            "seed {seed}: a signature that is not 128 hex characters is refused"
        );
    }
    Ok(())
}

/// The bytes sealed are RFC 8785 for the report family: re-serializing a parsed
/// body gives the same bytes, and every object's keys are in sorted order.
#[test]
fn sim_canonical_bytes_roundtrip_for_every_seed() -> TestResult {
    for seed in 0..SEEDS {
        let generated = body(&mut Seeded::from_seed(seed))?;
        let printed = generated.to_string();
        let reparsed = json::from_str::<Value>(&printed)?;
        assert_eq!(
            reparsed.to_string(),
            printed,
            "seed {seed}: canonical bytes survive a parse"
        );
        let keys: Vec<&str> = reparsed
            .as_object()
            .ok_or("the body is an object")?
            .keys()
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "seed {seed}: keys serialize in sorted order");
        assert!(
            !printed.contains(": ") && !printed.contains(", "),
            "seed {seed}: canonical bytes carry no insignificant whitespace"
        );
    }
    Ok(())
}

/// A seal over a success claim that the sealed results contradict is refused:
/// a signer that lies about the verdict cannot make a failed run read as
/// accepted (forge#158).
#[test]
fn a_signed_success_claim_over_a_failed_run_is_inconsistent() -> TestResult {
    let signer = key(&mut Seeded::from_seed(7));
    let claimed = json::json!({
        "task_id": "t",
        "task_digest": "d",
        "diff": "",
        "results": [{"argv": ["false"], "exit_code": 1, "timed_out": false}],
        "denied": [],
        "accepted": true,
    });
    let signature = sign::sign(&signer, TAG, claimed.to_string().as_bytes());
    let mut report = claimed;
    set(
        &mut report,
        RECEIPT,
        json::json!({
            "alg": "ed25519",
            "key": lgwks_std::hex::encode(signer.public_key().to_bytes()),
            "signature": lgwks_std::hex::encode(signature.to_bytes()),
        }),
    )?;
    assert!(
        matches!(
            verify_value(&report, &signer.public_key()),
            Err(ReceiptError::Inconsistent)
        ),
        "a sealed accepted=true over an exit 1 is refused"
    );
    Ok(())
}

/// The ed25519 group order L, little-endian (RFC 8032 section 5.1).
const GROUP_ORDER: [u8; 32] = [
    0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
];

/// A genuine receipt whose scalar S is replaced by S + L, the malleability twin a
/// lax verifier accepts (GHSA-q67f-28xg-22rw, GHSA-x3ff-w252-2g7j), is refused:
/// the seal is checked with `verify_strict`.
#[test]
fn a_malleated_signature_is_refused() -> TestResult {
    let signer = key(&mut Seeded::from_seed(11));
    let (mut sealed, _) = seal(body(&mut Seeded::from_seed(11))?, &signer);
    let receipt = receipt_of(&mut sealed)?;
    let hex = receipt["signature"].as_str().ok_or("signature is text")?;
    let mut bytes = lgwks_std::hex::decode(hex)?;
    let mut carry = 0_u16;
    for (byte, order) in bytes.iter_mut().skip(32).zip(GROUP_ORDER) {
        let sum = u16::from(*byte)
            .saturating_add(u16::from(order))
            .saturating_add(carry);
        *byte = sum.to_le_bytes()[0];
        carry = sum >> 8;
    }
    assert_eq!(carry, 0, "S + L fits in 32 bytes");
    set(
        receipt,
        "signature",
        json::json!(lgwks_std::hex::encode(&bytes)),
    )?;
    assert!(
        matches!(
            verify_value(&sealed, &signer.public_key()),
            Err(ReceiptError::Mismatch(_))
        ),
        "a non-canonical scalar S + L is refused"
    );
    Ok(())
}

/// Seeds of the truncated-diff simulation; each takes one of the four diff sizes in turn.
const DIFF_SEEDS: u64 = 8;

/// Diff sizes the simulation cycles through: one byte under the limit, at it, one over, and twice it.
const DIFF_SIZES: [usize; 4] = [
    DIFF_LIMIT.saturating_sub(1),
    DIFF_LIMIT,
    DIFF_LIMIT.saturating_add(1),
    DIFF_LIMIT.saturating_mul(2),
];

/// The piece size a diff stream is fed to its sink in.
const CHUNK: usize = 8_192;

/// BLAKE3 of `bytes` through the estate hasher, as 64 hex characters.
fn blake3_hex(bytes: &[u8]) -> String {
    let mut hasher = lgwks_std::hash::Hasher::new();
    hasher.update(bytes);
    hasher.finalize().to_hex()
}

/// Streams `full` through a sink in [`CHUNK`]-byte pieces, as the diff reader does.
fn sunk(full: &[u8]) -> Result<Diff, Box<dyn std::error::Error>> {
    let mut sink = DiffSink::new();
    for piece in full.chunks(CHUNK) {
        sink.push(piece)?;
    }
    Ok(sink.finish())
}

/// A report body in the family `larry exec` prints, with a diff of `size` bytes of `letter`.
fn diffed_body(size: usize, letter: u8) -> Result<(Value, bool), Box<dyn std::error::Error>> {
    let diff = sunk(&vec![letter; size])?;
    let truncated = diff.truncated;
    let mut body = json::json!({
        "task_id": "t",
        "task_digest": "d",
        "results": [],
        "denied": [],
    });
    for (name, member) in report_members(Some(diff)) {
        set(&mut body, name, member)?;
    }
    Ok((body, truncated))
}

/// Seals `claimed` under `signer` exactly as written, so a signer that lies about its verdict can be simulated.
fn sign_claim(signer: &SecretKey, claimed: Value) -> Result<Value, Box<dyn std::error::Error>> {
    let signature = sign::sign(signer, TAG, claimed.to_string().as_bytes());
    let mut report = claimed;
    set(
        &mut report,
        RECEIPT,
        json::json!({
            "alg": "ed25519",
            "key": lgwks_std::hex::encode(signer.public_key().to_bytes()),
            "signature": lgwks_std::hex::encode(signature.to_bytes()),
        }),
    )?;
    Ok(report)
}

/// A diff over the limit is cut: it is never accepted, verify agrees, and every edit of the cut is refused.
#[test]
fn sim_a_truncated_diff_is_never_accepted_and_verify_agrees() -> TestResult {
    for (seed, &size) in (0..DIFF_SEEDS).zip(DIFF_SIZES.iter().cycle()) {
        let mut stream = Seeded::from_seed(seed);
        let letter = b'a'.saturating_add(u8::try_from(stream.index(26)?)?);
        let (original, truncated) = diffed_body(size, letter)?;
        assert_eq!(
            truncated,
            size > DIFF_LIMIT,
            "seed {seed}: a diff is cut only past the limit"
        );
        let signer = key(&mut stream);
        let trusted = signer.public_key();
        let (sealed, accepted) = seal(original, &signer);
        assert_eq!(
            accepted, !truncated,
            "seed {seed}: a cut diff is never accepted"
        );
        let verdict = verify_value(&sealed, &trusted)?;
        assert_eq!(
            verdict.accepted(),
            accepted,
            "seed {seed}: verify agrees with the seal"
        );

        let mut flipped = sealed.clone();
        set(&mut flipped, DIFF_TRUNCATED, Value::Bool(!truncated))?;
        assert!(
            matches!(
                verify_value(&flipped, &trusted),
                Err(ReceiptError::Mismatch(_))
            ),
            "seed {seed}: a flipped diff_truncated is refused"
        );

        let stranger = key(&mut stream);
        assert!(
            matches!(
                verify_value(&sealed, &stranger.public_key()),
                Err(ReceiptError::UntrustedKey)
            ),
            "seed {seed}: another tenant's key refuses the report"
        );

        if truncated {
            let mut claimed = sealed;
            claimed
                .as_object_mut()
                .ok_or("the report is an object")?
                .remove(RECEIPT);
            set(&mut claimed, ACCEPTED, Value::Bool(true))?;
            let forged = sign_claim(&signer, claimed)?;
            assert!(
                matches!(
                    verify_value(&forged, &trusted),
                    Err(ReceiptError::TruncatedDiffAccepted)
                ),
                "seed {seed}: a signed accepted claim over a cut diff is refused"
            );
        }
    }
    Ok(())
}

/// The digest is the estate's BLAKE3 of the whole diff, however the stream is cut into pieces.
#[test]
fn the_diff_digest_is_the_estate_blake3_of_the_full_diff() -> TestResult {
    for (seed, &size) in (0..DIFF_SEEDS).zip(DIFF_SIZES.iter().cycle()) {
        let letter = b'a'.saturating_add(u8::try_from(Seeded::from_seed(seed).index(26)?)?);
        let full = vec![letter; size];
        let diff = sunk(&full)?;
        assert_eq!(
            diff.digest,
            blake3_hex(&full),
            "seed {seed}: the digest covers the full diff"
        );
        assert_eq!(
            diff.bytes,
            u64::try_from(size)?,
            "seed {seed}: the byte count is the full length"
        );
        assert_eq!(
            diff.text.len(),
            size.min(DIFF_LIMIT),
            "seed {seed}: only the head is kept"
        );
        let mut whole = DiffSink::new();
        whole.push(&full)?;
        assert_eq!(
            whole.finish().digest,
            diff.digest,
            "seed {seed}: the digest does not depend on the piece size"
        );
    }
    Ok(())
}

/// A report sealed before the member existed keeps its verdict: accepted under the limit,
/// refused at the limit, and refused when the member is not a boolean.
#[test]
fn a_legacy_receipt_without_the_new_fields_keeps_its_verdict() -> TestResult {
    let signer = key(&mut Seeded::from_seed(21));
    let trusted = signer.public_key();
    let legacy = |diff: String| {
        json::json!({
            "task_id": "t",
            "task_digest": "d",
            "diff": diff,
            "results": [],
            "denied": [],
        })
    };
    let (under, accepted) = seal(legacy("x".to_owned()), &signer);
    assert!(
        accepted && verify_value(&under, &trusted)?.accepted(),
        "a legacy diff under the limit keeps its accepted verdict"
    );
    let (at_limit, accepted) = seal(legacy("x".repeat(DIFF_LIMIT)), &signer);
    assert!(
        !accepted && !verify_value(&at_limit, &trusted)?.accepted(),
        "a legacy diff at the limit may be cut, so it is refused"
    );
    let mut odd = legacy("x".to_owned());
    set(&mut odd, DIFF_TRUNCATED, json::json!("no"))?;
    let (_, accepted) = seal(odd, &signer);
    assert!(!accepted, "a non-boolean diff_truncated is refused as cut");
    Ok(())
}

/// A signer that seals `accepted: true` over a cut diff is refused with its own variant.
#[test]
fn a_signed_accepted_claim_over_a_truncated_diff_is_refused() -> TestResult {
    let signer = key(&mut Seeded::from_seed(23));
    let claimed = json::json!({
        "task_id": "t",
        "task_digest": "d",
        "diff": "x",
        "diff_truncated": true,
        "results": [],
        "denied": [],
        "accepted": true,
    });
    let forged = sign_claim(&signer, claimed)?;
    assert!(
        matches!(
            verify_value(&forged, &signer.public_key()),
            Err(ReceiptError::TruncatedDiffAccepted)
        ),
        "a signed accepted claim over a cut diff is refused"
    );
    Ok(())
}

/// Oversize, non-UTF-8, malformed and non-object reports each get their exact
/// error before any key is consulted.
#[test]
fn oversize_and_malformed_reports_get_their_exact_error() -> TestResult {
    let trusted = key(&mut Seeded::from_seed(1)).public_key();
    // Zeroed pages are not touched before the length is refused.
    let oversize = vec![0_u8; MAX_REPORT_BYTES.saturating_add(1)];
    assert!(
        matches!(
            verify(&oversize, &trusted),
            Err(ReceiptError::TooLarge { len }) if len == MAX_REPORT_BYTES.saturating_add(1)
        ),
        "a report past the bound is refused by length"
    );
    assert!(
        matches!(
            verify(&[0xff, 0xfe], &trusted),
            Err(ReceiptError::NotUtf8(_))
        ),
        "a report that is not UTF-8 is refused"
    );
    assert!(
        matches!(verify(b"{", &trusted), Err(ReceiptError::Malformed(_))),
        "a report that is not JSON is refused"
    );
    assert!(
        matches!(verify(b"[]", &trusted), Err(ReceiptError::NotAnObject)),
        "a report that is not an object is refused"
    );
    assert!(
        matches!(verify(b"{}", &trusted), Err(ReceiptError::MissingReceipt)),
        "an empty object has no receipt"
    );
    for refusal in [
        ReceiptError::TooLarge { len: 9 },
        ReceiptError::NotAnObject,
        ReceiptError::MissingReceipt,
        ReceiptError::BadAlgorithm,
        ReceiptError::MissingKey,
        ReceiptError::KeyNotPoint(sign::KeyError::InvalidPublicKey),
        ReceiptError::UntrustedKey,
        ReceiptError::MissingSignature,
        ReceiptError::Mismatch(sign::VerifyError::Mismatch),
        ReceiptError::Inconsistent,
        ReceiptError::TruncatedDiffAccepted,
    ] {
        let message = refusal.to_string();
        assert!(
            message.starts_with("receipt"),
            "every refusal names the receipt: {message}"
        );
    }
    Ok(())
}
