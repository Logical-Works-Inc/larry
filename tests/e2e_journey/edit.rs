//! The session's anchored edit (larry#606, part two of larry#277): the model
//! reads a file for its digest and edits it under that precondition. An edit
//! naming another digest is refused and writes nothing; the approved edit that
//! names the right one lands in the copy, the review applies it to the folder,
//! and the sealed report still verifies accepted.

use super::*;

/// The `edit` reply that replaces the body of `origin` with `new` against `digest`.
fn edit_reply(new: &str, digest: &str) -> String {
    format!(
        "{{\"edit\":{{\"path\":\"src/lib.rs\",\"old\":\"pub fn origin() {{}}\",\"new\":\"{new}\",\"expect_digest\":\"{digest}\"}}}}"
    )
}

/// The edit's journey through the shipped binary: `y` approves the read, a
/// stale edit, the right edit and the check. The stale one is refused by name
/// and changes nothing; the model is shown the digest the right one left; the
/// folder holds only that change; and the receipt counts the one command and
/// says accepted, because an edit's record is never a command.
#[test]
fn tui_journey_approved_edit_lands_with_its_receipt() -> Journey {
    let before = "pub fn origin() {}\n";
    let after = "pub fn origin() -> u64 { 7 }\n";
    let digest = lgwks_std::hash::blake3(before.as_bytes()).to_hex();
    let other = lgwks_std::hash::blake3(b"another file").to_hex();
    let stale = edit_reply("pub fn origin() -> u8 { 1 }", &other);
    let good = edit_reply("pub fn origin() -> u64 { 7 }", &digest);
    let replies = [
        r#"{"read":"src/lib.rs"}"#,
        stale.as_str(),
        good.as_str(),
        r#"{"command":"test -f src/lib.rs"}"#,
        r#"{"done":"edited lib.rs"}"#,
    ];
    let ran = frame(scripted(11, &replies), |pty, mock, work| {
        ask(pty)?;
        pty.send("y")?;
        pty.wait_for("-> u8 { 1 }")?;
        pty.send("y")?;
        pty.wait_for("-> u64 { 7 }")?;
        pty.send("y")?;
        pty.wait_for("test -f src/lib.rs")?;
        pty.send("y")?;
        pty.wait_for(REVIEW)?;
        pty.send("y")?;
        pty.wait_for(APPLIED)?;
        pty.wait_for("verdict   accepted")?;
        pty.wait_for("commands  1 run, 0 declined, 0 interrupted")?;
        let requests = mock.requests();
        let shown = |index: usize| -> Journey<String> {
            let request = requests.get(index).ok_or("the model was asked again")?;
            Ok(String::from_utf8_lossy(request).into_owned())
        };
        assert!(
            shown(2)?.contains("changed since it was read"),
            "the model is told why the stale edit was refused"
        );
        let wanted = lgwks_std::hash::blake3(after.as_bytes()).to_hex();
        assert!(
            shown(3)?.contains(wanted.as_str()),
            "the model is shown the digest the edit left"
        );
        assert_eq!(
            std::fs::read_to_string(work.join("src").join("lib.rs"))?,
            after,
            "only the approved edit with the right digest is in the folder"
        );
        Ok(())
    })?;
    assert_exited(&ran.status, "Ctrl+C on an empty box exits 0");
    Ok(())
}
