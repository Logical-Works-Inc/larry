//! The session's read step (larry#586): the model asks to read a file, the person
//! approves it, and the model is shown the BLAKE3 digest an anchored edit will
//! name. A path outside the session's copy is a typed refusal the model retries
//! from, and neither read changes the folder.

use super::*;

/// A terminal session's command runs in a copy beneath the state directory, so
/// `pwd` prints a host path: no request holds it, and the output still arrives
/// (larry#589, OWASP ASVS 4.0.3 §8.3.4).
#[test]
fn tui_journey_the_confined_copy_path_never_reaches_the_model() -> Journey {
    let (state, work) = fixture()?;
    let replies = [
        r#"{"command":"pwd; cat src/lib.rs"}"#,
        r#"{"done":"looked around"}"#,
    ];
    let ran = frame_in(
        &state,
        work,
        scripted(13, &replies),
        Box::new(|pty, mock, _| {
            ask(pty)?;
            pty.send("y")?;
            pty.wait_for("looked around")?;
            let requests = mock.requests();
            let hidden = state.path().display().to_string();
            for body in &requests {
                let sent = String::from_utf8_lossy(body);
                assert!(!sent.contains(&hidden), "the state path was sent: {sent}");
            }
            let second = requests.get(1).ok_or("the model saw no output")?;
            assert!(
                String::from_utf8_lossy(second).contains("pub fn origin"),
                "the command's output arrives"
            );
            Ok(())
        }),
    )?;
    assert_exited(&ran.status, "Ctrl+C on an empty box exits 0");
    Ok(())
}

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
            String::from_utf8_lossy(fourth)
                .contains("exactly one key, command, done, read, edit or patch"),
            "a step the session does not offer is refused by name, never asked"
        );
        assert_eq!(tree_hash(work)?, unread, "a read changes nothing");
        Ok(())
    })?;
    assert_exited(&ran.status, "Ctrl+C on an empty box exits 0");
    Ok(())
}
