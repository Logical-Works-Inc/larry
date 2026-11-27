//! The spent-deadline verdict (larry#439): a report carrying `deadline_passed` is never
//! accepted, `verify` re-derives the rejection, and a forged `accepted: true` beside it is
//! refused as inconsistent.
//!
//! Source: forge#158 (a denied call reported as success). Requirement proved: the seal derives
//! false over any spent deadline, verify agrees, and a signer that lies about the verdict beside
//! the spent deadline is refused with its exact error.

use lgwks_std::json::{self, Value};
use lgwks_std::seeded::Seeded;
use lgwks_std::sign::{self, PublicKey, SecretKey};

use super::{ACCEPTED, DEADLINE_PASSED, RECEIPT, ReceiptError, Verdict, seal, seeded_key, verify};

/// What every test here returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Sets the member `key` of the JSON object `value` to `member`.
fn set(value: &mut Value, key: &str, member: Value) -> TestResult {
    value
        .as_object_mut()
        .ok_or("the edited value is an object")?
        .insert(key, member);
    Ok(())
}

/// Verifies `report` under `trusted`, as printed text.
fn verify_value(report: &Value, trusted: &PublicKey) -> Result<Verdict, ReceiptError> {
    verify(report.to_string().as_bytes(), trusted)
}

/// One command result that exited 0 inside its timeout.
fn passing() -> Value {
    json::json!({ "argv": ["true"], "exit_code": 0, "timed_out": false })
}

/// Seals `claimed` under `signer` exactly as written, so a signer that lies about its verdict can
/// be simulated.
fn sign_claim(signer: &SecretKey, claimed: Value) -> Result<Value, Box<dyn std::error::Error>> {
    let signature = sign::sign(signer, super::TAG, claimed.to_string().as_bytes());
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

/// A report whose whole-task deadline spent is never accepted, even when every result it kept
/// passed: the seal derives false, verify agrees, and a signer that claims accepted beside the
/// spent deadline is refused as inconsistent (larry#439).
#[test]
fn a_forged_accepted_claim_beside_a_spent_deadline_is_inconsistent() -> TestResult {
    let signer = seeded_key(&mut Seeded::from_seed(29));
    let trusted = signer.public_key();
    let mut spent = json::json!({
        "task_id": "t",
        "task_digest": "d",
        "diff": "d",
        "results": [passing()],
        "denied": [],
    });
    set(
        &mut spent,
        DEADLINE_PASSED,
        json::json!({"deadline_ms": 1_500}),
    )?;
    let (sealed, accepted) = seal(spent, &signer);
    assert!(
        !accepted,
        "a report whose deadline spent is never accepted, even over a passing command"
    );
    assert!(
        !verify_value(&sealed, &trusted)?.accepted(),
        "verify re-derives the rejection"
    );
    let mut claimed = sealed;
    claimed
        .as_object_mut()
        .ok_or("the report is an object")?
        .remove(RECEIPT);
    set(&mut claimed, ACCEPTED, Value::Bool(true))?;
    let forged = sign_claim(&signer, claimed)?;
    let refused = match verify_value(&forged, &trusted) {
        Err(refused) => refused,
        Ok(_) => {
            return Err(
                "a signed accepted claim beside a spent deadline verifies, but must not".into(),
            );
        }
    };
    assert!(
        matches!(refused, ReceiptError::Inconsistent),
        "a signed accepted claim beside a spent deadline is inconsistent, got {refused:?}"
    );
    assert_eq!(
        refused.to_string(),
        "receipt verdict contradicts the sealed results or names no task",
        "the refusal names the contradiction exactly"
    );
    Ok(())
}
