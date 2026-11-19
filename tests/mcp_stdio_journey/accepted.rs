//! What an accepted submitted task must leave, read from its tool result and from the mock
//! gateway it ran against (larry#123).

use lgwks_std::json::{self, Value};

use crate::json_at::At;
use crate::mcp_support::TestResult;
use crate::mock_model::Handle;

/// Checks that `result` is an accepted success whose sealed report records both files and
/// both commands, and returns the sealed diff.
pub(crate) fn check_accepted(seed: u64, result: &Value) -> TestResult<String> {
    assert_eq!(
        *result.at("/isError")?,
        false,
        "seed {seed}: accepted is a success: {result}"
    );
    let structured = result.at("/structuredContent")?;
    assert_eq!(
        *structured.at("/accepted")?,
        true,
        "seed {seed}: the verdict"
    );
    let text = result
        .at("/content/0/text")?
        .as_str()
        .ok_or("a text block")?;
    assert_eq!(
        &json::from_str::<Value>(text)?,
        structured,
        "the text block is the structured content"
    );
    let report = structured.at("/report")?;
    let diff = report
        .at("/diff")?
        .as_str()
        .ok_or("a sealed diff")?
        .to_owned();
    assert!(
        diff.contains("-origin readme"),
        "seed {seed}: README was rewritten: {diff}"
    );
    assert!(
        diff.contains("+++ b/src/new.rs"),
        "seed {seed}: src/new.rs was written"
    );
    assert_eq!(
        *report.at("/authored/0/path")?,
        "README",
        "seed {seed}: README is recorded"
    );
    assert_eq!(
        *report.at("/authored/0/existed")?,
        true,
        "seed {seed}: README existed"
    );
    assert_eq!(
        *report.at("/authored/1/existed")?,
        false,
        "seed {seed}: src/new.rs is new"
    );
    for index in 0..2 {
        assert_eq!(
            *report.at(&format!("/results/{index}/exit_code"))?,
            0,
            "seed {seed}: command {index} saw the files"
        );
    }
    Ok(diff)
}

/// Checks what the mock gateway saw of an accepted run: one completion per file, the goal and
/// the files in the first prompt, and only the operator's key.
pub(crate) fn check_model_side(seed: u64, handle: &Handle) -> TestResult<()> {
    assert_eq!(handle.served(), 2, "seed {seed}: one completion per file");
    let first: Value = json::from_slice(handle.requests().first().ok_or("a model request")?)?;
    let turn = first
        .at("/messages/1/content")?
        .as_str()
        .ok_or("a user turn")?;
    assert!(
        turn.contains("replace the readme")
            && turn.contains("=== README ===")
            && turn.contains("origin readme")
            && turn.ends_with("Write this file now: README"),
        "seed {seed}: the goal, the files and the file to write reached the model: {turn}"
    );
    assert!(
        handle
            .authorizations()
            .iter()
            .all(|sent| sent.as_deref() == Some("Bearer mcp-key")),
        "seed {seed}: the operator's key, and only it, reached the gateway"
    );
    Ok(())
}
