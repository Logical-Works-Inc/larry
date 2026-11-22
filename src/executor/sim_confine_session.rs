//! The Linux session of a confined command (larry#386, ADR 0091). Landlock leaves `/dev`
//! readable, because it is a sibling on the walk to the state directory, so a command that
//! kept the person's controlling terminal could open `/dev/tty` and read their keys. A command
//! in a session of its own has no controlling terminal: setsid(2), setsid(1) and the POSIX
//! definition of a controlling terminal (IEEE Std 1003.1, 11.1.3).

use std::ffi::OsStr;

use super::{
    ConfineError, Platform, SANDBOX_EXEC, SETSID, confined, layout, shell, supported_with,
};
use crate::executor::run::{Place, command_for};

/// What every simulation returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The session comes from the wrapper itself, never from what the host happens to have: a
/// host with `setpriv` but no `setsid` is refused typed, and the command is launched through
/// exactly one `setsid`. A second one would find a session leader, fork, and leave the
/// executor waiting on a parent that already exited while the command runs on outside it.
#[test]
fn sim_confine_linux_needs_setsid_and_starts_its_own_session() -> TestResult {
    let hidden = |tool: &str| tool != SETSID;
    assert!(
        matches!(
            supported_with(Platform::Linux, &hidden),
            Err(ConfineError::MissingWrapper { tool: SETSID })
        ),
        "a Linux host with no setsid cannot confine a command"
    );
    assert!(
        supported_with(Platform::Macos, &hidden).is_ok(),
        "macOS denies /dev/tty in its profile and needs no session tool"
    );
    let (_root, state, tree, tmp) = layout("session", "w")?;
    let command = shell("true");
    let linux = confined(Platform::Linux, &state, &tree, &tmp)?;
    assert_eq!(
        linux.wrap(&command).first().map(String::as_str),
        Some(SETSID),
        "the Linux wrapper starts the session before Landlock"
    );
    let (built, program) = command_for(&command, Place::Confined(&linux), "/usr/bin:/bin")?;
    assert_eq!(program, SETSID, "the session tool is the program");
    assert_eq!(built.get_program(), OsStr::new(SETSID));
    assert!(
        built.get_args().all(|word| word != SETSID),
        "the command runs through exactly one setsid"
    );
    let mac = confined(Platform::Macos, &state, &tree, &tmp)?;
    let (_, mac_program) = command_for(&command, Place::Confined(&mac), "/usr/bin:/bin")?;
    assert_eq!(
        mac_program, SANDBOX_EXEC,
        "macOS runs its own wrapper first"
    );
    Ok(())
}
