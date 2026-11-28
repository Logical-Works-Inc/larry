//! A command taller than the terminal is not approved unread (larry#521), through the shipped
//! binary: `y` and `a` at once run nothing and say why, `PgDn` brings the rest on screen, and
//! only then does `y` run it. Source: CWE-451 (UI misrepresentation of critical information).

use super::journey::{ASK, ask, frame_in, scripted};
use super::pty::Journey;
use super::{APPLIED, REVIEW, assert_exited, tree_copies, wrote};

/// `PgDn` as the terminal sends it.
const PAGE_DOWN: &str = "\x1b[6~";

/// The journey: a 40-row command whose last row writes the file, on the 32-row terminal.
#[test]
fn tui_journey_a_tall_command_is_not_approved_unread() -> Journey {
    let mut rows: Vec<String> = (1..40).map(|row| format!("echo row-{row:02}")).collect();
    rows.push(String::from("printf 'hello\\n' > hello.txt"));
    let command = lgwks_std::json::json!({"command": rows.join("\n")}).to_string();
    let replies = [command.as_str(), r#"{"done":"read, then run"}"#];
    let (state, work) = super::origin::fixture()?;
    let workspaces = state.path().join("ws");
    let ran = frame_in(
        &state,
        work,
        scripted(61, &replies),
        Box::new(|pty, mock, _| {
            ask(pty)?;
            assert!(
                !pty.screen_text().contains("hello.txt"),
                "the last row is not on the first screen"
            );
            pty.send("y")?;
            pty.wait_for("PgDn first")?;
            pty.send("a")?;
            pty.send("\x1b[D\x1b[D\r")?;
            // The page that follows is painted after those keys were taken, so what they did
            // is on this screen.
            pty.send(PAGE_DOWN)?;
            pty.wait_for("hello.txt")?;
            let screen = pty.screen_text();
            assert!(screen.contains(ASK), "still asking: {screen}");
            assert_eq!(mock.served(), 1, "no command ran, so no second model call");
            assert!(
                tree_copies(&workspaces)
                    .iter()
                    .all(|tree| !tree.join("hello.txt").exists()),
                "the unread command ran"
            );
            pty.wait_for("all read")?;
            pty.send("y")?;
            pty.wait_for(REVIEW)?;
            pty.send("y")?;
            pty.wait_for(APPLIED)?;
            pty.wait_for("read, then run")
        }),
    )?;
    assert_exited(&ran.status, "Ctrl+C on an empty box exits 0");
    assert!(wrote(&ran), "the command read to its end ran");
    Ok(())
}
