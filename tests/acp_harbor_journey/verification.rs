//! The typed verification gate through `larry acp` (larry#271), split from
//! `tests/acp_harbor_journey.rs` so each file stays one responsibility: a check
//! that writes the tree is refused, an edit after the passing check voids the
//! finish, and a model that never checks ends in refusal.

use crate::TestResult;
use crate::acp_agent::{Agent, prompt};
use crate::acp_session::{SESSION_POLICY, agent, scripted};
use crate::json_at::At;
use crate::mock_model;
use crate::mock_model::Handle;
use crate::origin::{TempDir, fixture};
use lgwks_std::json::Value;

/// Opens a session in `work`, sends `text` through the mock gateway, and
/// refuses: without a fresh read-only check no finish ends `end_turn`, so the
/// session always stops `refusal` here.
fn refused_prompt(
    mock: &Handle,
    state: &TempDir,
    work: &TempDir,
    text: &str,
) -> TestResult<(Agent, Vec<Value>, Value)> {
    let mut acp = agent(state.path(), mock, Some(SESSION_POLICY))?;
    let session = acp.open(work.path())?;
    let (notes, reply) = acp.call(&prompt(2, &session, text))?;
    assert_eq!(*reply.at("/result/stopReason")?, "refusal", "{reply}");
    Ok((acp, notes, reply))
}

/// Runs `replies` from `seed` through the mock gateway, sends `text`, and hands
/// the refused session to `check`: every finish here is refused, so each test
/// only asserts what its run left behind.
fn refused_session(
    seed: u64,
    replies: &[&str],
    text: &str,
    state: &TempDir,
    work: &TempDir,
    check: impl FnOnce(Agent, Vec<Value>, Value, u64) -> TestResult,
) -> TestResult {
    mock_model::run(scripted(seed, replies), |mock| -> TestResult {
        let (acp, notes, reply) = refused_prompt(mock, state, work, text)?;
        let served = mock.served();
        check(acp, notes, reply, served)
    })
}

/// A check that writes the tree is refused (larry#271): `touch b.txt` ran and
/// passed, but it changed the session tree, so the finish pair never ends
/// `end_turn` and the sealed report carries no verification receipt.
#[test]
fn a_finish_whose_check_writes_the_tree_is_refused() -> TestResult {
    let (state, work) = fixture()?;
    let finish = r#"{"finish":{"summary":"touched b","check":"touch b.txt","artifacts":["b.txt"],"state":["ready"],"servers":["none"]}}"#;
    let replies = [
        r#"{"command":"touch a.txt"}"#,
        r#"{"command":"touch b.txt"}"#,
        finish,
        finish,
    ];
    refused_session(
        271,
        &replies,
        "touch two files",
        &state,
        &work,
        |acp, _, reply, served| -> TestResult {
            assert!(work.path().join("b.txt").exists(), "the mutating check ran");
            let report = reply.at("/result/_meta/larry/report")?;
            assert!(
                report.member("/verification").is_none(),
                "a mutating check seals no receipt: {report}"
            );
            assert_eq!(served, 5, "two commands, two finishes, one drawn reply");
            assert_eq!(acp.finish()?, Some(0), "end of input is a clean exit");
            Ok(())
        },
    )
}

/// An edit after the passing check voids the finish (larry#271): write, check,
/// finish, write again, then the old check is refused and the session ends
/// `refusal` after the third refusal in a row.
#[test]
fn an_edit_after_the_check_voids_the_finish() -> TestResult {
    let (state, work) = fixture()?;
    let finish = r#"{"finish":{"summary":"checked hello","check":"test -f hello.txt","artifacts":["hello.txt"],"state":["ready"],"servers":["none"]}}"#;
    let replies = [
        r#"{"command":"printf hello > hello.txt"}"#,
        r#"{"command":"test -f hello.txt"}"#,
        finish,
        r#"{"command":"touch after.txt"}"#,
        finish,
        finish,
    ];
    refused_session(
        272,
        &replies,
        "check then edit",
        &state,
        &work,
        |acp, notes, reply, served| -> TestResult {
            let mut chunks = notes.iter().filter_map(|note| {
                let chunk = note.member("/params/update/sessionUpdate")?.as_str()
                    == Some("agent_message_chunk");
                if chunk {
                    note.member("/params/update/content/text")?.as_str()
                } else {
                    None
                }
            });
            assert!(
                chunks.any(|text| text.contains("3 replies in a row")),
                "the third refusal ends the session: {notes:?}"
            );
            assert!(work.path().join("hello.txt").exists(), "the write ran");
            assert!(
                work.path().join("after.txt").exists(),
                "the voiding edit ran"
            );
            let report = reply.at("/result/_meta/larry/report")?;
            assert!(
                report.member("/verification").is_none(),
                "a voided check seals no receipt: {report}"
            );
            assert_eq!(served, 7, "four commands and three finishes");
            assert_eq!(acp.finish()?, Some(0), "end of input is a clean exit");
            Ok(())
        },
    )
}

/// A model that never checks cannot finish (larry#271): one write, then the same
/// finish twice naming a check that never ran; the session ends `refusal` on the
/// third refusal and seals no receipt.
#[test]
fn a_model_that_never_checks_cannot_finish() -> TestResult {
    let (state, work) = fixture()?;
    let finish = r#"{"finish":{"summary":"no check ran","check":"test -f hello.txt","artifacts":["hello.txt"],"state":["ready"],"servers":["none"]}}"#;
    let replies = [r#"{"command":"printf hello > hello.txt"}"#, finish, finish];
    refused_session(
        273,
        &replies,
        "write without checking",
        &state,
        &work,
        |acp, _, reply, served| -> TestResult {
            assert!(work.path().join("hello.txt").exists(), "the write ran");
            let report = reply.at("/result/_meta/larry/report")?;
            assert!(
                report.member("/verification").is_none(),
                "no check seals no receipt: {report}"
            );
            assert_eq!(served, 4, "one command, two finishes, one drawn reply");
            assert_eq!(acp.finish()?, Some(0), "end of input is a clean exit");
            Ok(())
        },
    )
}
