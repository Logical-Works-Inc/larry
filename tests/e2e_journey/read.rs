//! The session's read step (larry#586): the model asks to read a file, the person
//! approves it, and the model is shown the BLAKE3 digest an anchored edit will
//! name. A path outside the session's copy is a typed refusal the model retries
//! from, and neither read changes the folder.

use super::*;

/// The read's journey through the shipped binary: `y` approves each read, the
/// first shows the digest and the text, the second is refused by name, and the
/// folder is byte-identical afterwards.
#[test]
fn tui_journey_approved_read_shows_the_digest() -> Journey {
    let before = "pub fn origin() {}\n";
    let digest = lgwks_std::hash::blake3(before.as_bytes()).to_hex();
    let replies = [
        r#"{"read":"src/lib.rs"}"#,
        r#"{"read":"../outside"}"#,
        r#"{"write":"src/lib.rs"}"#,
        r#"{"done":"looked at it"}"#,
    ];
    let ran = frame(scripted(11, &replies), |pty, mock, work| {
        let unread = tree_hash(work)?;
        ask(pty)?;
        pty.send("y")?;
        pty.wait_for("read ../outside")?;
        pty.send("y")?;
        pty.wait_for("looked at it")?;
        let requests = mock.requests();
        let second = requests.get(1).ok_or("the model saw no first read")?;
        assert!(
            String::from_utf8_lossy(second).contains(digest.as_str()),
            "the model is shown the digest of the file it read"
        );
        let third = requests.get(2).ok_or("the model saw no second read")?;
        assert!(
            String::from_utf8_lossy(third).contains("path ../outside escapes the snapshot"),
            "the model is told why the outside read was refused"
        );
        let fourth = requests
            .get(3)
            .ok_or("the model was not told of the refusal")?;
        assert!(
            String::from_utf8_lossy(fourth).contains("exactly one key, command, done or read"),
            "a step the session does not offer is refused by name, never asked"
        );
        assert_eq!(tree_hash(work)?, unread, "a read changes nothing");
        Ok(())
    })?;
    assert_exited(&ran.status, "Ctrl+C on an empty box exits 0");
    Ok(())
}
