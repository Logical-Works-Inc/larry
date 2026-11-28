//! Journeys of what `larry eval` reads from a gateway's reply (larry#87, larry#230,
//! larry#576). Source: the OpenAI chat completions `usage.prompt_tokens_details` object
//! and RFC 9110 refusal statuses. Requirement: the card counts the cached and written
//! tokens the gateway reported, and a session whose model call is refused is scored as
//! the arm's refusal with a clean reason, never as a rejection or a stopped arm.

use crate::eval_arm::{FLASH, TestResult, number, operator, origins_left, score_arm, task};
use crate::json_at::At;
use crate::mock_model::{Config, Fault, Handle, hostile_message};
use crate::temp_dir::TempDir;
use lgwks_std::json::json;
use std::ffi::OsString;

/// liar-upstream: two completions each reporting 7 cached and 3 written prompt tokens
/// give a card and a result row that say 14 and 6 and call the cache report complete.
/// Catches a scorer that drops the cache counts or sums only the last completion.
#[test]
fn the_card_sums_the_cached_and_written_tokens_the_gateway_reported() -> TestResult {
    let set = json!({"name": "cached", "tasks": [task("two", 2, &[])]}).to_string();
    let state = TempDir::new()?;
    let config = Config {
        cache_usage: Some((7, 3)),
        ..Config::seeded(53)
    };
    let ran = crate::mock_model::run(config, |handle: &Handle| {
        let ran = score_arm(
            &operator(&state, handle.addr(), &FLASH, "1"),
            set.as_bytes(),
        )?;
        assert_eq!(handle.served(), 2, "one completion per file");
        Ok::<_, Box<dyn std::error::Error>>(ran)
    })?;
    let card = ran.card()?;
    assert_eq!(number(card, "cached_tokens")?, 14, "{card}");
    assert_eq!(number(card, "cache_write_tokens")?, 6, "{card}");
    let complete = card.at("/cache_complete")?;
    assert_eq!(complete, &json!(true), "every completion reported");
    assert_eq!(card.at("/results/0/cached_tokens")?, &json!(14), "{card}");
    assert_eq!(
        card.at("/results/0/cache_write_tokens")?,
        &json!(6),
        "{card}"
    );
    assert_eq!(origins_left(&state)?, 0, "every origin removed");
    Ok(())
}

/// liar-upstream: a session whose gateway answers 401 with a hostile message is the arm's
/// refusal: the card is printed, the task is `refused`, the reason is bounded and holds no
/// escape bytes, and the run exits 0. Catches a refused session scored as rejected, or an
/// upstream message that reaches the operator's terminal (CWE-150).
#[test]
fn a_session_the_gateway_refuses_is_scored_as_a_clean_refusal() -> TestResult {
    let set = json!({"name": "refused", "tasks": [{
        "id": "session-path",
        "files": {"README.md": "# refused\n"},
        "goal": "write session.txt",
        "path": "session",
        "write": ["session.txt"],
        "acceptance": ["test -s session.txt"],
    }]})
    .to_string();
    let state = TempDir::new()?;
    let config = Config {
        script: vec![Some(Fault::HostileRefusal { status: 401 })],
        ..Config::seeded(59)
    };
    let ran = crate::mock_model::run(config, |handle: &Handle| {
        let mut env = operator(&state, handle.addr(), &FLASH, "1");
        env.push((
            "LARRY_POLICY",
            OsString::from(
                "file.read,file.write,process.spawn,model.invoke,model.command,net.egress",
            ),
        ));
        let ran = score_arm(&env, set.as_bytes())?;
        assert_eq!(handle.served(), 1, "one refused call, never retried");
        Ok::<_, Box<dyn std::error::Error>>(ran)
    })?;
    assert_eq!(
        ran.code,
        Some(0),
        "a refusal is scored, not fatal: {}",
        ran.stderr
    );
    let card = ran.card()?;
    let outcome = card.at("/results/0/outcome")?;
    assert_eq!(outcome, &json!("refused"), "{card}");
    let reason = card
        .at("/results/0/reason")?
        .as_str()
        .ok_or_else(|| format!("a refusal says why: {card}"))?;
    assert!(
        !reason.contains('\u{1b}') && !reason.contains('\u{7}'),
        "no escape bytes: {reason:?}"
    );
    assert!(reason.len() < hostile_message().len(), "bounded: {reason}");
    assert_eq!(number(card, "refused")?, 1, "{card}");
    assert_eq!(origins_left(&state)?, 0, "every origin removed");
    Ok(())
}
