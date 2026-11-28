//! The session's anchored edit (larry#606) and patch (larry#624), parts two and
//! three of larry#277: the model reads a file for its digest and changes it
//! under that precondition. A change naming another digest is refused and writes
//! nothing; the approved one that names the right digest lands in the copy, the
//! review applies it to the folder, and the sealed report still verifies
//! accepted.

use super::*;

/// The `patch` reply that rewrites `origin` to `new` against `digest` and adds
/// `src/more.rs`.
fn patch_reply(new: &str, digest: &str) -> String {
    let text = format!(
        "*** Begin Patch\\n*** Update File: src/lib.rs\\n@@\\n-pub fn origin() {{}}\\n+{new}\\n*** Add File: src/more.rs\\n+pub fn more() {{}}\\n*** End Patch\\n"
    );
    format!(
        "{{\"patch\":{{\"text\":\"{text}\",\"expect_digests\":{{\"src/lib.rs\":\"{digest}\"}}}}}}"
    )
}

/// The patch's journey through the shipped binary: a stale patch is refused by
/// name and writes neither file, the right one lands both, the review applies
/// them to the folder, and the receipt says accepted.
#[test]
fn tui_journey_approved_patch_lands_every_file_with_its_receipt() -> Journey {
    let before = "pub fn origin() {}\n";
    let after = "pub fn origin() -> u64 { 7 }\n";
    let digest = lgwks_std::hash::blake3(before.as_bytes()).to_hex();
    let other = lgwks_std::hash::blake3(b"another file").to_hex();
    let stale = patch_reply("pub fn origin() -> u8 { 1 }", &other);
    let good = patch_reply("pub fn origin() -> u64 { 7 }", &digest);
    let replies = [
        r#"{"read":"src/lib.rs"}"#,
        stale.as_str(),
        good.as_str(),
        r#"{"command":"test -f src/more.rs"}"#,
        r#"{"done":"patched lib.rs and more.rs"}"#,
    ];
    let seen = ["-> u8 { 1 }", "-> u64 { 7 }", "test -f src/more.rs"];
    let ran = frame(scripted(12, &replies), |pty, mock, work| {
        approve_to_the_report(pty, seen)?;
        told_why_then_what(&mock.requests(), after)?;
        let file = |name: &str| std::fs::read_to_string(work.join("src").join(name));
        assert_eq!(file("lib.rs")?, after, "only the right patch is applied");
        assert_eq!(file("more.rs")?, "pub fn more() {}\n", "and its new file");
        Ok(())
    })?;
    assert_exited(&ran.status, "Ctrl+C on an empty box exits 0");
    Ok(())
}

/// The `edit` reply that replaces the body of `origin` with `new` against `digest`.
fn edit_reply(new: &str, digest: &str) -> String {
    format!(
        "{{\"edit\":{{\"path\":\"src/lib.rs\",\"old\":\"pub fn origin() {{}}\",\"new\":\"{new}\",\"expect_digest\":\"{digest}\"}}}}"
    )
}

/// The person's side of both journeys: `y` to the read, the stale change, the
/// right one and the command, each seen as its line of `seen`, then `y` to the
/// review, until the report says accepted over the one command.
fn approve_to_the_report(pty: &mut Pty<'_>, seen: [&str; 3]) -> Journey {
    ask(pty)?;
    for line in seen {
        pty.send("y")?;
        pty.wait_for(line)?;
    }
    pty.send("y")?;
    pty.wait_for(REVIEW)?;
    pty.send("y")?;
    pty.wait_for(APPLIED)?;
    pty.wait_for("verdict   accepted")?;
    pty.wait_for("commands  1 run, 0 declined, 0 interrupted")?;
    Ok(())
}

/// The model is told why the stale change was refused, and is shown the digest
/// of `after`, which the right one left.
fn told_why_then_what(requests: &[Vec<u8>], after: &str) -> Journey {
    let shown = |index: usize| -> Journey<String> {
        let request = requests.get(index).ok_or("the model was asked again")?;
        Ok(String::from_utf8_lossy(request).into_owned())
    };
    assert!(
        shown(2)?.contains("changed since it was read"),
        "the model is told why the stale change was refused"
    );
    let wanted = lgwks_std::hash::blake3(after.as_bytes()).to_hex();
    assert!(
        shown(3)?.contains(wanted.as_str()),
        "the model is shown the digest the change left"
    );
    Ok(())
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
    let seen = ["-> u8 { 1 }", "-> u64 { 7 }", "test -f src/lib.rs"];
    let ran = frame(scripted(11, &replies), |pty, mock, work| {
        approve_to_the_report(pty, seen)?;
        told_why_then_what(&mock.requests(), after)?;
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
