//! Seeded adversarial simulations of the two sweeps (larry#87, larry#182 defect
//! 3, larry#214): a workspace survives exactly when the caller's live list names
//! it, an origin survives exactly when a live eval holds its lock, a file is
//! nobody's, and a second sweep removes nothing more.
//!
//! The oracle is a reference model, never this code's output: the live list for
//! the workspaces, and the lock each origin carries. One seed drives
//! every draw, so a failing seed replays exactly.

use lgwks_std::seeded::Seeded;

use super::super::ExecError;
#[cfg(unix)]
use super::mode::reset_with;
use super::{
    hold_origin, origin_lock_path, remove_workspace, strands, sweep, sweep_dir, sweep_origins,
};

/// What every simulation here returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A pid no live process holds: the largest positive `i32` less one.
const DEAD_PID: i32 = 2_147_483_646;

/// The seeds the sweep models replay.
const SEEDS: [u64; 4] = [
    0x5EED_0000_0000_0011,
    0x5EED_0000_0000_0012,
    0x5EED_0000_FFFF_0011,
    0xDEAD_BEEF_CAFE_0011,
];

/// A fresh state directory: empty when it is made and removed when its guard drops, so a
/// process id the system reuses never lands a simulation in a stale tree (larry#267).
fn scratch_directory() -> Result<crate::executor::sim_tree::Scratch, crate::sim_error::SimError> {
    crate::executor::sim_tree::scratch("larry-workspace-sim")
}

/// The names `seeded` marks live: the caller's own statement, which is the oracle
/// the sweep must match.
fn drawn_live(
    seeded: &mut Seeded,
    names: &[String],
) -> Result<Vec<String>, lgwks_std::seeded::DrawError> {
    let mut live: Vec<String> = Vec::with_capacity(names.len());
    for name in names {
        if seeded.below(2)? == 1 {
            live.push(name.clone());
        }
    }
    Ok(live)
}

/// The clock-fool: an origin whose eval holds its lock is kept, and an origin nobody
/// holds is removed, whatever pid its name carries (larry#214: the lock, not the pid).
#[test]
fn sim_sweep_keeps_this_eval_and_removes_a_dead_ones_origin() -> TestResult {
    let state = scratch_directory()?;
    let eval = state.join("eval");
    let mine_name = format!("origin-{}-0", std::process::id());
    let mine = eval.join(&mine_name);
    let dead = eval.join(format!("origin-{DEAD_PID}-0"));
    std::fs::create_dir_all(&mine)?;
    std::fs::create_dir_all(&dead)?;
    let _held = hold_origin(&eval, &mine_name)?;
    sweep_origins(&state)?;
    assert!(mine.is_dir(), "a running eval keeps its locked origin");
    assert!(!dead.exists(), "an origin nobody holds is removed");
    Ok(())
}

/// The neighbour and the fat-finger client: for every seed a workspace directory
/// survives exactly when the caller's live list names it, and a file beside them
/// is never removed.
#[test]
fn sim_the_sweep_keeps_exactly_the_named_live_runs_for_every_seed() -> TestResult {
    for seed in SEEDS {
        let state = scratch_directory()?;
        let ws = state.join("ws");
        std::fs::create_dir_all(&ws)?;
        let mut seeded = Seeded::from_seed(seed);
        let count = 1_usize.saturating_add(seeded.index(5)?);
        let mut names = Vec::with_capacity(count);
        let mut drawn = 0_usize;
        while drawn < count {
            // Names a directory may carry: digits, spaces and hyphens are all the
            // sweep will meet.
            let text = match seeded.index(3)? {
                0 => format!("{}", seeded.index(10_000)?),
                1 => format!("run {} {}", seeded.index(99)?, seeded.index(99)?),
                _ => format!("run-{}-{}", seeded.index(999)?, seeded.index(999)?),
            };
            if names.contains(&text) {
                continue;
            }
            names.push(text);
            drawn = drawn.saturating_add(1);
        }
        let live = drawn_live(&mut seeded, &names)?;
        for name in &names {
            std::fs::create_dir_all(ws.join(name))?;
        }
        let kept_file = ws.join("keep.txt");
        std::fs::write(&kept_file, b"a file is nobody's workspace")?;
        sweep(&state, &live, |_name| Ok(false))?;
        for name in &names {
            let wanted = live.contains(name);
            assert_eq!(
                ws.join(name).is_dir(),
                wanted,
                "seed {seed}: {name:?} survives exactly when the live list names it"
            );
        }
        assert!(kept_file.is_file(), "seed {seed}: a file is never removed");
    }
    Ok(())
}

/// The monkey: an origin nobody holds the lock of is removed whatever its name
/// carries, and the one a live eval holds is kept, for every seed's own drawn pid
/// (larry#214: a pid in a name proves nothing).
#[test]
fn sim_an_origin_without_a_held_lock_is_removed_for_every_seed() -> TestResult {
    let names: [&str; 7] = [
        "origin-",
        "origin-0-0",
        "origin--1-0",
        "origin-abc-0",
        "origin-2147483647-0",
        "origin-21474836470-0",
        "origin-99999999-0",
    ];
    for seed in SEEDS {
        let state = scratch_directory()?;
        let eval = state.join("eval");
        std::fs::create_dir_all(&eval)?;
        let mut seeded = Seeded::from_seed(seed);
        for name in names {
            std::fs::create_dir_all(eval.join(name))?;
        }
        let own = format!("origin-{}-{}", std::process::id(), seeded.index(9)?);
        std::fs::create_dir_all(eval.join(&own))?;
        let _held = hold_origin(&eval, &own)?;
        let pid = i32::try_from(seeded.below(2_147_483_646)?)?;
        let foreign = format!("origin-{pid}-0");
        std::fs::create_dir_all(eval.join(&foreign))?;
        sweep_origins(&state)?;
        for name in names {
            assert!(
                !eval.join(name).exists(),
                "seed {seed}: {name} is held by nobody and is removed"
            );
        }
        assert!(
            eval.join(&own).is_dir(),
            "seed {seed}: the origin under a held lock is kept"
        );
        assert!(
            !eval.join(&foreign).exists(),
            "seed {seed}: {foreign} with no lock is removed whatever its pid"
        );
    }
    Ok(())
}

/// The double-clicker: running the sweep twice over one tree removes nothing
/// more and keeps exactly the same set, for every seed's own names.
#[test]
fn sim_a_second_sweep_removes_nothing_more_for_every_seed() -> TestResult {
    for seed in SEEDS {
        let state = scratch_directory()?;
        let ws = state.join("ws");
        std::fs::create_dir_all(&ws)?;
        let mut seeded = Seeded::from_seed(seed);
        let count = 1_usize.saturating_add(seeded.index(6)?);
        let names: Vec<String> = (0..count)
            .map(|index| format!("ws-{seed}-{index}"))
            .collect();
        let live = drawn_live(&mut seeded, &names)?;
        for name in &names {
            std::fs::create_dir_all(ws.join(name))?;
        }
        sweep(&state, &live, |_name| Ok(false))?;
        let first: Vec<bool> = names.iter().map(|name| ws.join(name).is_dir()).collect();
        sweep(&state, &live, |_name| Ok(false))?;
        let second: Vec<bool> = names.iter().map(|name| ws.join(name).is_dir()).collect();
        assert_eq!(
            first, second,
            "seed {seed}: a second sweep keeps the same set"
        );
        for (name, kept) in names.iter().zip(first) {
            assert_eq!(
                kept,
                live.contains(name),
                "seed {seed}: {name} follows the live list"
            );
        }
    }
    Ok(())
}

/// The blocked remover: a workspace whose removal fails is kept and reported as one typed
/// fault, and every other dead workspace is still removed, so one entry never refuses the next
/// start (larry#334).
#[test]
fn sim_a_refused_workspace_is_one_typed_fault_and_the_sweep_goes_on() -> TestResult {
    let state = scratch_directory()?;
    let ws = state.join("ws");
    std::fs::create_dir_all(ws.join("refused"))?;
    std::fs::create_dir_all(ws.join("removed"))?;
    let faults = sweep_dir(
        &ws,
        &[],
        |_name| Ok(false),
        |path| {
            if path.ends_with("refused") {
                return Err(ExecError::Workspace(std::io::Error::other(
                    "the directory is held open",
                )));
            }
            remove_workspace(path)
        },
    )?;
    assert_eq!(faults.len(), 1, "the refused workspace is one typed fault");
    assert!(
        ws.join("refused").is_dir(),
        "an entry that still fails is kept, never removed as someone else's"
    );
    assert!(
        !ws.join("removed").exists(),
        "every other dead workspace is still removed"
    );
    Ok(())
}

/// The names the interleaved sims make: a kept entry beside every swept one, so a sweep that
/// stopped at the first fault would leave swept ones behind in any listing order.
const INTERLEAVED: usize = PAIRS.saturating_mul(2);

/// The kept entries among the interleaved ones: every odd index.
const PAIRS: usize = 4;

/// Interleaved probe faults: every odd workspace's probe errors and is kept as a fault, and
/// every even one is still removed, whatever order the directory lists them in (larry#334).
#[test]
fn sim_every_probe_fault_is_kept_and_every_dead_neighbour_is_removed() -> TestResult {
    let state = scratch_directory()?;
    let ws = state.join("ws");
    for index in 0..INTERLEAVED {
        std::fs::create_dir_all(ws.join(format!("run-{index}")))?;
    }
    let faults = sweep_dir(
        &ws,
        &[],
        |name| {
            let odd = name
                .strip_prefix("run-")
                .and_then(|digits| digits.parse::<usize>().ok())
                .is_some_and(|index| index % 2 == 1);
            if odd {
                return Err(ExecError::Workspace(std::io::Error::other("unprovable")));
            }
            Ok(false)
        },
        remove_workspace,
    )?;
    assert_eq!(faults.len(), PAIRS, "each probe fault is one typed fault");
    for index in 0..INTERLEAVED {
        assert_eq!(
            ws.join(format!("run-{index}")).is_dir(),
            index % 2 == 1,
            "run-{index}: a probe fault is kept and a dead workspace is removed"
        );
    }
    Ok(())
}

/// Interleaved lock faults: an origin lock that cannot be opened (a symlink loop, `ELOOP` even
/// for root) is kept as a fault, and every unheld lock beside it is still removed (larry#334).
#[cfg(unix)]
#[test]
fn sim_every_unreadable_origin_lock_is_kept_and_every_unheld_one_removed() -> TestResult {
    let state = scratch_directory()?;
    let eval = state.join("eval");
    std::fs::create_dir_all(&eval)?;
    let locks: Vec<_> = (0..INTERLEAVED)
        .map(|index| origin_lock_path(&eval, &format!("origin-{DEAD_PID}-{index}")))
        .collect();
    for (index, lock) in locks.iter().enumerate() {
        if index % 2 == 1 {
            std::os::unix::fs::symlink(lock, lock)?;
        } else {
            std::fs::write(lock, b"")?;
        }
    }
    let faults = sweep_origins(&state)?;
    assert_eq!(
        faults.len(),
        PAIRS,
        "each unreadable lock is one typed fault"
    );
    for (index, lock) in locks.iter().enumerate() {
        assert_eq!(
            lock.symlink_metadata().is_ok(),
            index % 2 == 1,
            "lock {index}: an unreadable lock is kept and an unheld one is removed"
        );
    }
    Ok(())
}

/// A workspace already gone is not stranded; every other refusal strands it, so the counter
/// the next start reads is exact (larry#334).
#[test]
fn only_a_removal_that_left_the_tree_behind_strands_it() {
    let gone = std::io::Error::from(std::io::ErrorKind::NotFound);
    let held = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
    assert!(!strands(&Ok(())), "a removed tree is not stranded");
    assert!(
        !strands(&Err(ExecError::Workspace(gone))),
        "a tree already gone is not stranded"
    );
    assert!(
        strands(&Err(ExecError::Workspace(held))),
        "a refused removal strands the tree"
    );
    assert!(
        strands(&Err(ExecError::Journals(std::io::Error::other("other")))),
        "a refusal that is not the file system's own strands it too"
    );
}

/// The unprovable owner: a liveness probe that errors keeps its workspace, because an unknown
/// owner may be live; a probed-dead neighbour is still removed (larry#334).
#[test]
fn sim_a_probe_error_keeps_its_workspace_and_the_sweep_goes_on() -> TestResult {
    let state = scratch_directory()?;
    let ws = state.join("ws");
    std::fs::create_dir_all(ws.join("unknown"))?;
    std::fs::create_dir_all(ws.join("dead"))?;
    let faults = sweep_dir(
        &ws,
        &[],
        |name| match name {
            "unknown" => Err(ExecError::Workspace(std::io::Error::other(
                "the lock could not be probed",
            ))),
            _ => Ok(false),
        },
        remove_workspace,
    )?;
    assert_eq!(
        faults.len(),
        1,
        "the unprovable workspace is one typed fault"
    );
    assert!(
        ws.join("unknown").is_dir(),
        "a probe error is never read as dead"
    );
    assert!(
        !ws.join("dead").exists(),
        "a workspace the probe says is dead is still removed"
    );
    Ok(())
}

/// The mode a sentinel outside the tree carries: owner read and write, group read.
#[cfg(unix)]
const SENTINEL_MODE: u32 = 0o640;

/// A directory judged a directory is swapped for a link to a file outside the tree before its
/// mode is reset, and that file's mode stays byte-identical. Source: CWE-367 (TOCTOU) and CWE-59
/// (link following); the child that left the command's process group is still alive while the
/// workspace drops, so the swap window is real (larry#334, larry#366, ADR 0081).
#[cfg(unix)]
#[test]
fn sim_a_link_swapped_in_between_the_check_and_the_reset_keeps_the_sentinels_mode() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    let state = scratch_directory()?;
    let workspace = state.join("ws").join("run");
    let sealed = workspace.join("tree").join("sealed");
    std::fs::create_dir_all(&sealed)?;
    std::fs::write(sealed.join("file"), b"x")?;
    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o000))?;
    let sentinel = state.join("sentinel");
    std::fs::write(&sentinel, b"outside the tree")?;
    std::fs::set_permissions(&sentinel, std::fs::Permissions::from_mode(SENTINEL_MODE))?;
    let moved = workspace.join("tree").join("sealed-before");
    let links = reset_with(&workspace, &mut |at| {
        if at == sealed.as_path() {
            std::fs::rename(&sealed, &moved)?;
            std::os::unix::fs::symlink(&sentinel, &sealed)?;
        }
        Ok(())
    })?;
    assert_eq!(
        std::fs::metadata(&sentinel)?.permissions().mode() & 0o777,
        SENTINEL_MODE,
        "the file a swapped-in link points at keeps its mode"
    );
    assert_eq!(links.len(), 1, "the swapped-in link is one typed event");
    assert_eq!(
        links.first().map(|kept| kept.entry()),
        Some(sealed.as_path()),
        "the reported entry is the swapped name"
    );
    Ok(())
}

/// A link found where a directory was is kept as it is and reported once, and the walk goes on: a
/// mode-000 directory beneath a mode-500 one is still reset and the sentinel is never followed.
/// Source: POSIX file modes and CWE-59 (larry#366).
#[cfg(unix)]
#[test]
fn sim_a_link_found_where_a_directory_was_is_one_typed_event_and_the_walk_goes_on() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    let state = scratch_directory()?;
    let workspace = state.join("ws").join("run");
    let sealed = workspace.join("sealed");
    let nested = sealed.join("nested");
    std::fs::create_dir_all(&nested)?;
    std::fs::write(nested.join("file"), b"x")?;
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o000))?;
    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o500))?;
    let sentinel = state.join("sentinel");
    std::fs::write(&sentinel, b"outside the tree")?;
    std::fs::set_permissions(&sentinel, std::fs::Permissions::from_mode(SENTINEL_MODE))?;
    let link = workspace.join("link");
    std::os::unix::fs::symlink(&sentinel, &link)?;
    let links = reset_with(&workspace, &mut |_| Ok(()))?;
    assert_eq!(
        std::fs::metadata(&sentinel)?.permissions().mode() & 0o777,
        SENTINEL_MODE,
        "a link to a file outside the tree is never followed"
    );
    assert_eq!(links.len(), 1, "the link is one typed event");
    assert_eq!(
        links.first().map(|kept| kept.entry()),
        Some(link.as_path()),
        "the reported entry is the link"
    );
    assert_eq!(
        std::fs::metadata(&nested)?.permissions().mode() & 0o777,
        0o700,
        "a mode-000 directory is reset through its held parent"
    );
    assert_eq!(
        std::fs::metadata(&sealed)?.permissions().mode() & 0o777,
        0o700,
        "a mode-500 directory is reset before its entries"
    );
    Ok(())
}

/// A name that vanishes before it is judged is skipped, a plain file is skipped, and the walk goes
/// on: the sibling directory is still reset and nothing outside the tree is touched. Source: the
/// POSIX `*at` family (a name can go away between a listing and a stat) and CWE-367 (larry#366).
#[cfg(unix)]
#[test]
fn sim_an_entry_that_vanishes_as_the_walk_reaches_it_is_skipped() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    let state = scratch_directory()?;
    let workspace = state.join("ws").join("run");
    let keep = workspace.join("keep");
    let gone = workspace.join("gone");
    std::fs::create_dir_all(&keep)?;
    std::fs::create_dir_all(&gone)?;
    std::fs::write(workspace.join("plain"), b"x")?;
    std::fs::set_permissions(&keep, std::fs::Permissions::from_mode(0o500))?;
    let links = reset_with(&workspace, &mut |at| {
        if at == gone.as_path() {
            std::fs::remove_dir(&gone)?;
        }
        Ok(())
    })?;
    assert!(links.is_empty(), "a vanished name and a file are no events");
    assert!(
        !gone.exists(),
        "the name the seam removed is skipped, never opened"
    );
    assert_eq!(
        std::fs::metadata(&keep)?.permissions().mode() & 0o777,
        0o700,
        "the walk goes on to the entries after a vanished one"
    );
    assert!(
        workspace.join("plain").is_file(),
        "a plain file is left where it is"
    );
    Ok(())
}
