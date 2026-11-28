//! The Linux session of a confined command (larry#386, ADR 0091). Landlock leaves `/dev`
//! readable, because it is a sibling on the walk to the state directory, so a command that
//! kept the person's controlling terminal could open `/dev/tty` and read their keys. A command
//! in a session of its own has no controlling terminal: setsid(2), setsid(1) and the POSIX
//! definition of a controlling terminal (IEEE Std 1003.1, 11.1.3).
//!
//! A wrapper also answers for the program behind it, so the program is looked up before the
//! wrapper (larry#598): execvp(3) and the PATH rules of IEEE Std 1003.1, 8.3.

use std::ffi::OsStr;
use std::io;
// The executor refuses to build on a target that is not Unix, and so do its simulations.
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use lgwks_std::seeded::Seeded;

use super::{
    ConfineError, Platform, SANDBOX_EXEC, SETSID, confined, layout, shell, supported_with,
};
use crate::executor::ExecError;
use crate::executor::run::{Place, command_for};

/// What every simulation returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Seeds the program-lookup simulation sweeps.
const LOOKUP_SEEDS: u64 = 256;

/// The directories one lookup seed draws its PATH from.
const LOOKUP_DIRS: [&str; 4] = ["first", "second", "third", "fourth"];

/// What one directory holds under the looked-up name.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Held {
    /// Nothing of that name.
    Nothing,
    /// A regular file with an execute bit.
    Program,
    /// A regular file with no execute bit.
    Plain,
    /// A directory of that name.
    Directory,
}

/// Puts `held` at `path`.
fn place_held(path: &Path, held: Held) -> io::Result<()> {
    let mode = match held {
        Held::Nothing => return Ok(()),
        Held::Directory => return std::fs::create_dir(path),
        Held::Program => 0o755,
        Held::Plain => 0o644,
    };
    std::fs::write(path, "#!/bin/sh\n")?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

/// The reference answer, execvp(3)'s: the first executable file on the way wins; else a name that
/// is there but cannot run is refused permission, and a name that is nowhere is not found.
fn reference_lookup(way: &[Held]) -> Option<io::ErrorKind> {
    if way.contains(&Held::Program) {
        None
    } else if way.iter().any(|held| *held != Held::Nothing) {
        Some(io::ErrorKind::PermissionDenied)
    } else {
        Some(io::ErrorKind::NotFound)
    }
}

/// The refusal `command_for` gave for `program`, or `None` when it built the command.
fn lookup_answer(
    program: &str,
    workspace: &Path,
    path_env: &str,
) -> Result<Option<io::ErrorKind>, ExecError> {
    let argv = [String::from(program)];
    match command_for(&argv, Place::Open(workspace), path_env) {
        Ok(_) => Ok(None),
        Err(ExecError::Spawn { argv0, source }) => {
            assert_eq!(
                argv0, program,
                "the refusal names the command's own program"
            );
            Ok(Some(source.kind()))
        }
        Err(other) => Err(other),
    }
}

/// larry#598: a program the command's PATH does not hold is a typed spawn refusal on every host,
/// whether or not `setsid` wraps it, so exit 127 never stands for a command that was not run.
/// For every seed a drawn PATH of absolute, relative, empty and missing entries gets the
/// reference's answer for a bare name, and a relative name with a slash is judged where it points.
#[test]
fn sim_a_program_missing_from_path_is_refused_typed() -> TestResult {
    for seed in 0..LOOKUP_SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let root = crate::executor::sim_tree::scratch("larry-lookup")?;
        let mut held = Vec::new();
        for name in LOOKUP_DIRS {
            std::fs::create_dir(root.join(name))?;
            let drawn = match rng.below(6)? {
                0 => Held::Program,
                1 => Held::Plain,
                2 => Held::Directory,
                _ => Held::Nothing,
            };
            place_held(&root.join(name).join("tool"), drawn)?;
            held.push(drawn);
        }
        let mut entries = Vec::new();
        let mut way = Vec::new();
        for _ in 0..rng.index(5)? {
            let at = rng.index(LOOKUP_DIRS.len())?;
            let name = LOOKUP_DIRS.get(at).ok_or("a directory")?;
            way.push(*held.get(at).ok_or("what it holds")?);
            entries.push(match rng.below(2)? {
                0 => root.join(name).to_string_lossy().into_owned(),
                _ => (*name).to_owned(),
            });
        }
        // A missing entry, or one that is a file, is walked past wherever it sits.
        std::fs::write(root.join("a-file"), "")?;
        for passed_by in ["/larry-no-such-directory", "a-file"] {
            if rng.below(2)? == 0 {
                let at = rng.index(entries.len().saturating_add(1))?;
                entries.insert(at, String::from(passed_by));
            }
        }
        // An empty PATH, and an empty entry, name the working directory, which holds no tool.
        let path_env = entries.join(":");
        assert_eq!(
            lookup_answer("tool", &root, &path_env)?,
            reference_lookup(&way),
            "seed {seed}: a bare name on {path_env:?}"
        );
        // POSIX exec: an empty file name names nothing, whatever the PATH holds.
        assert_eq!(
            lookup_answer("", &root, &path_env)?,
            Some(io::ErrorKind::NotFound),
            "seed {seed}: an empty name on {path_env:?}"
        );
        let at = rng.index(LOOKUP_DIRS.len())?;
        let named = format!("{}/tool", LOOKUP_DIRS.get(at).ok_or("a directory")?);
        let there = [*held.get(at).ok_or("what it holds")?];
        assert_eq!(
            lookup_answer(&named, &root, &path_env)?,
            reference_lookup(&there),
            "seed {seed}: {named} is a path, never looked up on PATH"
        );
    }
    Ok(())
}

/// larry#598: the wrapper would report a program the host lacks as an exit code of its own (127
/// from `setpriv`, 71 from `sandbox-exec`), which reads as the command's judgement. A bare or
/// absolute name the host does not hold is refused typed before the wrapper; a relative path names
/// a file of the confined tree, which the model writes, so the wrapper's exit judges it.
#[test]
fn sim_a_confined_program_the_host_lacks_is_refused_typed() -> TestResult {
    let (_root, state, tree, tmp) = layout("lacks", "w")?;
    for platform in [Platform::Linux, Platform::Macos] {
        let confine = confined(platform, &state, &tree, &tmp)?;
        let build = |program: &str| {
            command_for(
                &[String::from(program)],
                Place::Confined(&confine),
                "/usr/bin:/bin",
            )
        };
        for missing in ["larry-no-such-tool", "/larry-no-such-directory/tool"] {
            match build(missing) {
                Err(ExecError::Spawn { argv0, source }) => {
                    assert_eq!(argv0, missing, "{platform:?}: the program is named");
                    assert_eq!(source.kind(), io::ErrorKind::NotFound, "{platform:?}");
                }
                other => {
                    return Err(
                        format!("{platform:?}: {missing} was not refused: {other:?}").into(),
                    );
                }
            }
        }
        for judged_inside in ["./built-by-the-model", "out/tool", "sh", "/bin/sh"] {
            assert!(
                build(judged_inside).is_ok(),
                "{platform:?}: {judged_inside} starts behind the wrapper"
            );
        }
    }
    Ok(())
}

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
