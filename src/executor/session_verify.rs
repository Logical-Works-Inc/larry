//! Unit and seeded tests for the typed verification gate (larry#271, ADR 0130):
//! the legacy `done` token is refused, a finish needs the last fresh passing
//! check receipt and present artifacts, the first finish returns the checklist,
//! the repeated identical finish ends the session, and any later command voids
//! the handshake. The seeded sim draws each request from its seed, so the same
//! seed replays the same handshake. One shared binding carries each boolean, so
//! a flip of any of them breaks the discriminating assert that names it.

use lgwks_std::seeded::Seeded;

use crate::sim_error::SimError;

use super::super::session_gate::invalidate_finish;
use super::super::session_reply::{ReplyFault, parse_reply};
use super::{FinishRequest, command_passed, has_fresh_check, same_finish, verification_receipt};

/// One lowercase word drawn from the seed; every request field stays bounded text.
fn word(rng: &mut Seeded) -> Result<String, SimError> {
    let len = rng.index(12)?.saturating_add(1);
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        out.push(char::from(
            b'a'.saturating_add(u8::try_from(rng.index(26)?)?),
        ));
    }
    Ok(out)
}

/// Separates a colliding edit from the check it must differ from.
fn separate(check: &str, mut edit: String) -> String {
    if edit.as_str() == check {
        edit.push('x');
    }
    edit
}

#[test]
fn finish_refuses_without_fresh_passing_check() {
    let passed = true;
    let failed = false;
    assert_eq!(
        parse_reply(r#"{"done":"the task is complete"}"#),
        Err(ReplyFault::Keys),
        "the legacy model-written done token must not bypass verification"
    );
    assert!(
        !has_fresh_check(Some("cargo test"), failed, "cargo test"),
        "a failed check has no receipt"
    );
    assert!(
        !has_fresh_check(None, passed, "cargo test"),
        "a finish without a receipt is refused"
    );
    assert!(
        !has_fresh_check(Some("cargo fmt"), passed, "cargo test"),
        "a different passing command is not the named check"
    );
    assert!(
        has_fresh_check(Some("cargo test"), passed, "cargo test"),
        "a passing check is the receipt"
    );
    assert!(
        !has_fresh_check(Some("cargo test"), failed, "cargo test"),
        "the same check failed is no receipt"
    );
}

fn finish_request(summary: &str) -> FinishRequest {
    FinishRequest {
        summary: summary.to_owned(),
        check: String::from("true"),
        artifacts: vec![String::from("out")],
        state: vec![String::from("clean")],
        servers: vec![String::from("none")],
    }
}

#[test]
fn repeated_finish_requires_the_identical_request() {
    let first = finish_request("ready");
    assert!(same_finish(Some(&first), &first));
    assert!(!same_finish(None, &first));
    assert!(!same_finish(Some(&finish_request("different")), &first));
}

#[test]
fn sim_gate_handshake_refuses_an_edit_after_the_check_for_every_seed() -> Result<(), SimError> {
    let passed = true;
    let failed = false;
    for seed in [1_u64, 2, 0x5EED_0089] {
        let mut rng = Seeded::from_seed(seed);
        for _ in 0..64 {
            let check = word(&mut rng)?;
            let edit = separate(&check, word(&mut rng)?);
            let artifact = word(&mut rng)?;
            let request = FinishRequest {
                summary: word(&mut rng)?,
                check: check.clone(),
                artifacts: vec![artifact.clone()],
                state: vec![word(&mut rng)?],
                servers: vec![word(&mut rng)?],
            };
            assert!(
                !has_fresh_check(None, failed, &request.check),
                "seed {seed:#x}: a finish with no receipt is refused"
            );
            assert!(
                !has_fresh_check(Some(check.as_str()), failed, &check),
                "seed {seed:#x}: a failed check has no receipt"
            );
            assert!(
                !has_fresh_check(Some(edit.as_str()), passed, &check),
                "seed {seed:#x}: a different passing command is not the named check"
            );
            assert!(
                has_fresh_check(Some(check.as_str()), passed, &check),
                "seed {seed:#x}: the passing check is the receipt"
            );
            let collided = separate(&check, check.clone());
            assert_ne!(
                collided, check,
                "seed {seed:#x}: a colliding edit is separated"
            );
            let mut first: Option<FinishRequest> = None;
            assert!(
                !same_finish(first.as_ref(), &request),
                "seed {seed:#x}: the first finish returns the checklist"
            );
            first = Some(request.clone());
            assert!(
                same_finish(first.as_ref(), &request),
                "seed {seed:#x}: the repeated finish ends the session"
            );
            invalidate_finish(&mut first);
            assert!(
                first.is_none(),
                "seed {seed:#x}: a later command voids the handshake"
            );
            assert!(
                !has_fresh_check(Some(edit.as_str()), passed, &check),
                "seed {seed:#x}: an edit after the check is refused"
            );
            let receipt =
                verification_receipt(&check, &[artifact], &request.state, &request.servers);
            assert_eq!(
                receipt
                    .get("check")
                    .and_then(lgwks_std::json::Value::as_str),
                Some(check.as_str()),
                "seed {seed:#x}: the receipt names the check"
            );
            assert_eq!(
                receipt
                    .get("fresh_shell")
                    .and_then(lgwks_std::json::Value::as_bool),
                Some(true),
                "seed {seed:#x}: the receipt names its fresh shell"
            );
        }
    }
    Ok(())
}

#[test]
fn receipt_marks_the_check_as_fresh_shell_evidence() {
    let receipt = verification_receipt(
        "true",
        &[String::from("out")],
        &[String::from("clean")],
        &[String::from("none")],
    );
    assert_eq!(
        receipt
            .get("fresh_shell")
            .and_then(lgwks_std::json::Value::as_bool),
        Some(true)
    );
}

#[test]
fn command_passes_only_for_a_clean_zero_exit() {
    let off = false;
    let on = true;
    assert!(command_passed(Some(0), off, off));
    assert!(!command_passed(Some(1), off, off));
    assert!(!command_passed(Some(0), on, off));
    assert!(!command_passed(Some(0), off, on));
    assert!(!command_passed(Some(1), on, on));
}
