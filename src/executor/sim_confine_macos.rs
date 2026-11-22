//! The macOS channel simulations of the kernel confinement (larry#326): the exec, the Mach
//! and the signal channels a `(allow default)` profile leaves open. Each runs the shipped
//! wrapper on this host and reads what the kernel did. Source: Apple's sandbox profile
//! language (`sandbox-exec`), SBPL operations `process-exec`, `mach-lookup` and `signal`.

use std::path::Path;

use super::{Confine, Platform, layout, run_confined, shell};
use crate::executor::sim_tree::{path_env, tree_digest};
use crate::sim_error::SimError;

/// What every simulation returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The launch service a Mach lookup asks for: `notifyutil -p` resolves
/// `com.apple.system.notification_center` through `launchd`, the service SBPL calls a
/// `global-name`. A command that could look it up could ask another process to act.
const LAUNCH_SERVICE: &str = "com.example.larry326";

/// The launchers the profile refuses to `process-exec`: each starts a process outside the
/// profile or asks a system service to act for the command. The first names an application
/// that does not exist, so a host that runs it starts nothing.
const LAUNCHERS: [&[&str]; 3] = [
    &["/usr/bin/open", "-a", "larry-326-no-such-application"],
    &["/bin/launchctl", "list"],
    &["/usr/bin/osascript", "-e", "return 1"],
];

/// A folder beside the tree holding one file, and that folder's digest: the origin each
/// channel simulation must leave byte-identical.
fn beside_origin(root: &Path) -> Result<String, SimError> {
    let beside = root.join("beside");
    std::fs::create_dir_all(&beside)?;
    std::fs::write(beside.join("victim.txt"), b"original")?;
    tree_digest(&beside, false)
}

/// The macOS profile refuses a Mach lookup of a launch service: Apple's sandbox profile
/// language names the operation `mach-lookup` and the service a `global-name`.
/// `notifyutil -p` posts through `notifyd`; a host that allows the lookup prints nothing,
/// and a refused one says the post failed. The folder beside the tree stays byte-identical.
#[test]
fn sim_confine_macos_refuses_a_launch_service_lookup() -> TestResult {
    let path = path_env()?;
    let (root, state, tree, tmp) = layout("mach", "w")?;
    let before = beside_origin(&root)?;
    let confine = Confine::new(Platform::Macos, &state, &tree, &tmp)?;
    let argv = [
        String::from("/usr/bin/notifyutil"),
        String::from("-p"),
        String::from(LAUNCH_SERVICE),
    ];
    let refused = run_confined(&confine, &argv, &path, &tmp)?;
    // `notifyutil` reports through its standard output, so both streams are read.
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(
        said.contains("Failed"),
        "a launch service lookup was allowed: exit {:?}: {said:?}",
        refused.status.code()
    );
    assert_eq!(
        tree_digest(&root.join("beside"), false)?,
        before,
        "the folder beside the tree stays byte-identical"
    );
    Ok(())
}

/// The macOS profile refuses `process-exec` of the three launchers: `open`, `launchctl`
/// and `osascript` each start or ask a process that is outside the profile. Apple's sandbox
/// profile language names the operation `process-exec` and its filter `literal`. Each
/// refusal is the kernel's, not the launcher's own failure, and the folder beside the tree
/// stays byte-identical.
#[test]
fn sim_confine_macos_refuses_the_launchers() -> TestResult {
    let path = path_env()?;
    let (root, state, tree, tmp) = layout("launch", "w")?;
    let before = beside_origin(&root)?;
    let confine = Confine::new(Platform::Macos, &state, &tree, &tmp)?;
    for launcher in LAUNCHERS {
        let argv: Vec<String> = launcher.iter().map(|word| String::from(*word)).collect();
        let refused = run_confined(&confine, &argv, &path, &tmp)?;
        let said = String::from_utf8_lossy(&refused.stderr).into_owned();
        let named = launcher.join(" ");
        assert!(!refused.status.success(), "`{named}` ran: {said:?}");
        assert!(
            said.contains("Operation not permitted"),
            "`{named}` was refused for another reason: {said:?}"
        );
        assert_eq!(
            tree_digest(&root.join("beside"), false)?,
            before,
            "`{named}` changed the folder beside the tree"
        );
    }
    Ok(())
}

/// The macOS profile refuses a `signal` to a process outside the command's own process
/// group: Apple's sandbox profile language names the operation `signal` and the filters
/// `target self` and `target pgrp`. The command may signal itself and its own group, and a
/// process in another group is untouched. The folder beside the tree stays byte-identical.
#[test]
fn sim_confine_macos_refuses_a_signal_outside_the_group() -> TestResult {
    use std::os::unix::process::CommandExt as _;
    use std::process::{Command, Stdio};
    let path = path_env()?;
    let (root, state, tree, tmp) = layout("signal", "w")?;
    let before = beside_origin(&root)?;
    // A process in a group of its own, so the confined command's group is another.
    let mut victim = Command::new("/usr/bin/yes")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    let pid = victim.id();
    let confine = Confine::new(Platform::Macos, &state, &tree, &tmp)?;
    let attempted = run_confined(&confine, &shell(&format!("kill -TERM {pid}")), &path, &tmp)?;
    let said = String::from_utf8_lossy(&attempted.stderr).into_owned();
    // The victim is judged before it is cleaned up: a killed group would report the
    // refusal for the wrong reason.
    let survived = Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()?
        .success();
    victim.kill()?;
    victim.wait()?;
    assert!(
        !attempted.status.success(),
        "a signal outside the group was allowed: {said:?}"
    );
    assert!(
        survived,
        "the process outside the group was signalled: {said:?}"
    );
    // The command still signals its own group: a shell may end its own child.
    let own = run_confined(
        &confine,
        &shell("/bin/yes > /dev/null & stop=$!; kill -TERM $stop && echo group-ok"),
        &path,
        &tmp,
    )?;
    assert!(
        String::from_utf8_lossy(&own.stdout).contains("group-ok"),
        "the command cannot signal its own group: {}",
        String::from_utf8_lossy(&own.stderr)
    );
    assert_eq!(
        tree_digest(&root.join("beside"), false)?,
        before,
        "the folder beside the tree stays byte-identical"
    );
    Ok(())
}
