//! Seeded adversarial fault injection through the workspace's own seams (larry#575): a git that
//! cannot start or refuses a step, a diff stream that lies, a tree that cannot be removed and
//! origin locks that cannot be taken or removed. Each answer is asserted as its exact typed
//! error, and one seed drives every draw.

use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use lgwks_std::seeded::Seeded;

use super::super::super::ExecError;
use super::super::super::sim_tree::{Scratch, path_env, scratch};
#[cfg(unix)]
use super::super::mode::reset_with;
use super::super::{
    DiffSink, STRANDED, STREAM_CHUNK, Workspace, hold_origin, origin_lock_path, pump, seed_origin,
    sweep, sweep_origins,
};

/// What every simulation here returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The seeds the fault models replay.
const SEEDS: [u64; 3] = [
    0x5EED_0000_0000_0021,
    0x5EED_0000_0000_0022,
    0xDEAD_BEEF_CAFE_0021,
];

/// A fresh state directory, removed when its guard drops (larry#267).
fn state_directory() -> Result<Scratch, crate::sim_error::SimError> {
    scratch("larry-workspace-fault")
}

/// A repository at `<state>/<name>` with one seeded file and one commit.
fn committed(state: &Path, name: &str, seed: u64) -> Result<PathBuf, ExecError> {
    let dir = state.join(name);
    std::fs::create_dir_all(&dir).map_err(ExecError::Workspace)?;
    std::fs::write(dir.join("seed.txt"), format!("{seed:x}\n")).map_err(ExecError::Workspace)?;
    let path = path_env().map_err(|cause| ExecError::Workspace(std::io::Error::other(cause)))?;
    seed_origin(state, &path, &dir)?;
    Ok(dir)
}

/// A PATH holding no git is one typed spawn failure naming git, never a silent success or a
/// git found some other way: the executor runs git only from the PATH it was given (CWE-426,
/// larry#85).
#[test]
fn sim_a_git_the_path_cannot_start_is_one_typed_spawn_failure() -> TestResult {
    let state = state_directory()?;
    for seed in SEEDS {
        let absent = state.join(format!("no-bin-{seed:x}"));
        let refused = seed_origin(&state, &absent.display().to_string(), &state);
        let Err(ExecError::Spawn { argv0, source }) = refused else {
            return Err(format!("seed {seed:#x}: not a spawn failure: {refused:?}").into());
        };
        assert_eq!(argv0, "git", "seed {seed:#x}: the failure names git");
        assert_eq!(
            source.kind(),
            ErrorKind::NotFound,
            "seed {seed:#x}: the cause is a program that is not there"
        );
    }
    Ok(())
}

/// A git step git itself refuses is one typed failure that names the step and carries git's own
/// words, so an operator sees why (CWE-20 hostile `.git` file, larry#87).
#[test]
fn sim_a_refused_git_step_names_the_step_and_carries_gits_words() -> TestResult {
    for seed in SEEDS {
        let state = state_directory()?;
        let dir = state.join("hostile");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(".git"), format!("not a gitfile {seed:x}\n"))?;
        let refused = seed_origin(&state, &path_env()?, &dir);
        let Err(ExecError::Git { step, detail }) = refused else {
            return Err(format!("seed {seed:#x}: not a git failure: {refused:?}").into());
        };
        assert_eq!(step, "init", "seed {seed:#x}: the failed step is named");
        assert!(
            detail.contains("gitfile"),
            "seed {seed:#x}: git's own words are carried: {detail:?}"
        );
    }
    Ok(())
}

/// A folder that is no repository is `NotARepo` naming it, a base the clone does not hold is
/// `BadBase` naming it, and neither leaves a workspace behind (larry#85).
#[test]
fn sim_a_refused_clone_or_base_is_typed_and_leaves_no_workspace() -> TestResult {
    for seed in SEEDS {
        let state = state_directory()?;
        let plain = state.join("plain");
        std::fs::create_dir_all(&plain)?;
        let path = path_env()?;
        match Workspace::create(&state, &plain, "HEAD", "run-plain", &path) {
            Err(ExecError::NotARepo { repo }) => {
                assert_eq!(repo, plain, "seed {seed:#x}: the refused folder is named");
            }
            other => {
                return Err(format!("seed {seed:#x}: not NotARepo: {:?}", other.err()).into());
            }
        }
        let origin = committed(&state, "origin", seed)?;
        let missing = format!("{seed:040x}");
        match Workspace::create(&state, &origin, &missing, "run-base", &path) {
            Err(ExecError::BadBase { base }) => {
                assert_eq!(base, missing, "seed {seed:#x}: the missing base is named");
            }
            other => {
                return Err(format!("seed {seed:#x}: not BadBase: {:?}", other.err()).into());
            }
        }
        for run in ["run-plain", "run-base"] {
            assert!(
                !state.join("ws").join(run).exists(),
                "seed {seed:#x}: the refused {run} workspace is removed"
            );
        }
    }
    Ok(())
}

/// A command that destroys its own work tree leaves no diff, and a base git cannot diff
/// against is a typed diff failure carrying git's words, never an empty diff that would read as
/// "nothing changed" (CWE-754, larry#206).
#[test]
fn sim_a_destroyed_tree_has_no_diff_and_an_unknown_base_is_a_failed_diff() -> TestResult {
    for seed in SEEDS {
        let state = state_directory()?;
        let origin = committed(&state, "origin", seed)?;
        let workspace = Workspace::create(&state, &origin, "HEAD", "run", &path_env()?)?;
        let unknown = format!("{seed:040x}");
        let refused = workspace
            .diff(&unknown)
            .map(|diff| diff.map(|diff| diff.text));
        let Err(ExecError::Git { step, detail }) = refused else {
            return Err(format!("seed {seed:#x}: not a git failure: {refused:?}").into());
        };
        assert_eq!(step, "diff", "seed {seed:#x}: the failed step is named");
        assert!(
            detail.contains(&unknown),
            "seed {seed:#x}: git names the base it could not read: {detail:?}"
        );
        std::fs::remove_dir_all(workspace.path())?;
        assert!(
            workspace.diff("HEAD")?.is_none(),
            "seed {seed:#x}: a destroyed work tree has no diff"
        );
    }
    Ok(())
}

/// A workspace its drop cannot remove is counted as stranded and left for the next sweep, never
/// silently forgotten. The parent is swapped for a link to itself, which refuses even root
/// (CWE-59, larry#343).
#[cfg(unix)]
#[test]
fn sim_a_workspace_that_cannot_be_removed_is_counted_stranded() -> TestResult {
    let state = state_directory()?;
    let origin = committed(&state, "origin", SEEDS[0])?;
    let workspace = Workspace::create(&state, &origin, "HEAD", "run", &path_env()?)?;
    let ws = state.join("ws");
    let moved = state.join("ws-moved");
    std::fs::rename(&ws, &moved)?;
    std::os::unix::fs::symlink(&ws, &ws)?;
    let before = STRANDED.load(Ordering::Relaxed);
    drop(workspace);
    assert!(
        STRANDED.load(Ordering::Relaxed) > before,
        "the refused removal is counted"
    );
    assert!(
        moved.join("run").join("tree").is_dir(),
        "the stranded tree is kept"
    );
    Ok(())
}

/// A reader that answers from a script, last step first: each step is one `read` answer.
struct Scripted(Vec<std::io::Result<Vec<u8>>>);

impl Read for Scripted {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self.0.pop() {
            None => Ok(0),
            Some(Err(error)) => Err(error),
            Some(Ok(bytes)) => {
                for (slot, byte) in buffer.iter_mut().zip(&bytes) {
                    *slot = *byte;
                }
                Ok(bytes.len())
            }
        }
    }
}

/// Git's diff stream interrupted by a signal is read on and keeps every byte; a stream that
/// fails, or a read that claims more bytes than its buffer holds, is a typed workspace error and
/// never a short diff (CWE-252, CWE-130, larry#206).
#[test]
fn sim_the_diff_stream_survives_interrupts_and_refuses_faults_and_lies() -> TestResult {
    for seed in SEEDS {
        let mut seeded = Seeded::from_seed(seed);
        let mut script = Vec::new();
        let mut expected = Vec::new();
        for _ in 0..8 {
            let length = 1_usize.saturating_add(seeded.index(64)?);
            let piece = vec![u8::try_from(seeded.below(256)?)?; length];
            expected.extend_from_slice(&piece);
            script.push(Ok(piece));
            if seeded.below(2)? == 1 {
                script.push(Err(std::io::Error::from(ErrorKind::Interrupted)));
            }
        }
        script.reverse();
        let diff = pump(Scripted(script))?.finish();
        assert_eq!(
            diff.bytes,
            u64::try_from(expected.len())?,
            "seed {seed:#x}: every byte is counted"
        );
        let mut hasher = lgwks_std::hash::Hasher::new();
        hasher.update(&expected);
        assert_eq!(
            diff.digest,
            hasher.finalize().to_hex(),
            "seed {seed:#x}: every byte is hashed in order"
        );

        let broken = Scripted(vec![Err(std::io::Error::from(ErrorKind::BrokenPipe))]);
        match pump(broken).map(|_sink| ()) {
            Err(ExecError::Workspace(error)) => assert_eq!(
                error.kind(),
                ErrorKind::BrokenPipe,
                "seed {seed:#x}: the cause is kept"
            ),
            other => return Err(format!("seed {seed:#x}: not the fault: {other:?}").into()),
        }
        let lying = Scripted(vec![Ok(vec![0; STREAM_CHUNK.saturating_add(1)])]);
        match pump(lying).map(|_sink| ()) {
            Err(ExecError::Workspace(error)) => assert_eq!(
                error.to_string(),
                "a read reported more bytes than its buffer holds",
                "seed {seed:#x}: a lying read is refused"
            ),
            other => return Err(format!("seed {seed:#x}: a lie was accepted: {other:?}").into()),
        }
    }
    Ok(())
}

/// A diff whose byte count would pass `u64` is refused, never wrapped into a short length
/// (CWE-190, larry#206).
#[test]
fn a_diff_byte_count_that_would_wrap_is_refused() {
    let mut sink = DiffSink::new();
    sink.bytes = u64::MAX;
    let refused = sink.push(b"x").map_err(|error| error.to_string());
    assert_eq!(refused, Err("the diff is longer than u64 bytes".to_owned()));
}

/// A second holder of an origin's lock is refused with `WouldBlock`, so two evals never share
/// one origin, and a lock under a missing folder is a typed fault (CWE-667, larry#214).
#[test]
fn sim_a_second_holder_of_an_origin_is_refused_while_the_first_lives() -> TestResult {
    for seed in SEEDS {
        let state = state_directory()?;
        let eval = state.join("eval");
        std::fs::create_dir_all(&eval)?;
        let name = format!("origin-{seed:x}-0");
        let first = hold_origin(&eval, &name)?;
        match hold_origin(&eval, &name) {
            Err(ExecError::Workspace(error)) => {
                assert_eq!(error.kind(), ErrorKind::WouldBlock, "seed {seed:#x}");
                assert_eq!(
                    error.to_string(),
                    "the origin is already held by a live eval",
                    "seed {seed:#x}"
                );
            }
            other => return Err(format!("seed {seed:#x}: a second holder: {other:?}").into()),
        }
        drop(first);
        hold_origin(&eval, &name)?;
        match hold_origin(&state.join("absent"), &name) {
            Err(ExecError::Workspace(error)) => {
                assert_eq!(error.kind(), ErrorKind::NotFound, "seed {seed:#x}");
            }
            other => return Err(format!("seed {seed:#x}: no folder, no lock: {other:?}").into()),
        }
    }
    Ok(())
}

/// A sweep folder that is a file is a typed fault, never an empty folder read as "nothing to
/// sweep" (CWE-754, larry#334).
#[test]
fn sim_a_sweep_folder_that_is_a_file_is_a_typed_fault() -> TestResult {
    let state = state_directory()?;
    std::fs::write(state.join("ws"), b"a file")?;
    std::fs::write(state.join("eval"), b"a file")?;
    for refused in [sweep(&state, &[], |_name| Ok(false)), sweep_origins(&state)] {
        match refused {
            Err(ExecError::Workspace(error)) => {
                assert_eq!(error.kind(), ErrorKind::NotADirectory);
            }
            other => return Err(format!("a file was swept as a folder: {other:?}").into()),
        }
    }
    Ok(())
}

/// A stale lock that cannot be removed (a directory by the lock's name, which refuses even
/// root) is kept as one typed fault naming it, and the sweep still removes the rest
/// (larry#334).
#[test]
fn sim_a_lock_that_cannot_be_removed_is_one_typed_fault() -> TestResult {
    let state = state_directory()?;
    let eval = state.join("eval");
    let stuck = eval.join("origin-stuck.lock");
    std::fs::create_dir_all(&stuck)?;
    let _kept = hold_origin(&eval, "origin-stuck.lock")?;
    let stale = origin_lock_path(&eval, "origin-stale");
    std::fs::write(&stale, b"")?;
    let faults = sweep_origins(&state)?;
    let named: Vec<&Path> = faults.iter().map(|fault| fault.entry.as_path()).collect();
    assert_eq!(named, [stuck.as_path()], "only the stuck lock is a fault");
    assert!(stuck.is_dir(), "the stuck entry is kept");
    assert!(!stale.exists(), "the stale lock beside it is removed");
    Ok(())
}

/// A reset root with no parent or no name is refused by name, a file or a link root is left
/// as it is, a missing root is nothing to do, and a name the file system cannot stat is that
/// refusal, never a silent skip (CWE-59, CWE-754, larry#366).
#[cfg(unix)]
#[test]
fn sim_a_root_the_reset_cannot_name_or_stat_is_typed() -> TestResult {
    let state = state_directory()?;
    let mut nothing = |_: &Path| Ok(());
    for (root, why) in [
        (PathBuf::from("/"), "/ has no parent directory".to_owned()),
        (
            state.join("x").join(".."),
            format!(
                "{} does not end in a UTF-8 name",
                state.join("x").join("..").display()
            ),
        ),
    ] {
        match reset_with(&root, &mut nothing) {
            Err(cause) => assert_eq!(
                (cause.kind(), cause.to_string()),
                (ErrorKind::InvalidInput, why)
            ),
            Ok(links) => return Err(format!("{} was reset: {links:?}", root.display()).into()),
        }
    }
    let file = state.join("file");
    std::fs::write(&file, b"x")?;
    assert!(reset_with(&file, &mut nothing)?.is_empty(), "a file root");
    assert!(
        reset_with(&state.join("gone"), &mut nothing)?.is_empty(),
        "a missing root"
    );
    let link = state.join("link");
    std::os::unix::fs::symlink(&file, &link)?;
    let kept = reset_with(&link, &mut nothing)?;
    let entries: Vec<&Path> = kept.iter().map(|link| link.entry()).collect();
    assert_eq!(entries, [link.as_path()], "a link root is kept and named");
    let long = state.join("n".repeat(300));
    match reset_with(&long, &mut nothing) {
        Err(cause) => assert_eq!(cause.kind(), ErrorKind::InvalidFilename),
        Ok(links) => return Err(format!("an unstatable root: {links:?}").into()),
    }
    Ok(())
}

/// A directory swapped for a file between its check and its reset is a typed refusal, never a
/// mode change reported as done; a name that is not UTF-8 is left alone (CWE-367, larry#366).
#[cfg(unix)]
#[test]
fn sim_a_directory_swapped_for_a_file_before_its_reset_is_refused() -> TestResult {
    for seed in SEEDS {
        let state = state_directory()?;
        let workspace = state.join("run");
        let sealed = workspace.join(format!("sealed-{seed:x}"));
        std::fs::create_dir_all(&sealed)?;
        let swapped = reset_with(&workspace, &mut |at| {
            if at == sealed.as_path() {
                std::fs::remove_dir(&sealed)?;
                std::fs::write(&sealed, b"now a file")?;
            }
            Ok(())
        });
        match swapped {
            Err(cause) => assert_eq!(cause.kind(), ErrorKind::NotADirectory, "seed {seed:#x}"),
            Ok(links) => return Err(format!("seed {seed:#x}: reset a file: {links:?}").into()),
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        let state = state_directory()?;
        let workspace = state.join("run");
        let odd = workspace.join(std::ffi::OsStr::from_bytes(b"d-\xff"));
        std::fs::create_dir_all(&odd)?;
        let links = reset_with(&workspace, &mut |_| Ok(()))?;
        assert!(links.is_empty(), "a foreign name is no link");
        assert!(odd.is_dir(), "a foreign name is left where it is");
    }
    Ok(())
}

/// A directory whose name is not UTF-8 is not a run the executor made (it names runs by UUID
/// text), so the sweep leaves it and reports nothing (CWE-176, larry#182). Only Linux file
/// systems hold such a name; elsewhere the arm does not exist.
#[test]
fn sim_a_directory_with_a_non_utf8_name_is_never_swept() -> TestResult {
    let state = state_directory()?;
    std::fs::create_dir_all(state.join("ws"))?;
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        let odd = state
            .join("ws")
            .join(std::ffi::OsStr::from_bytes(b"run-\xff"));
        std::fs::create_dir(&odd)?;
        let faults = sweep(&state, &[], |_name| Ok(false))?;
        assert!(faults.is_empty(), "a foreign name is no fault");
        assert!(odd.is_dir(), "a foreign name is never removed");
    }
    Ok(())
}
