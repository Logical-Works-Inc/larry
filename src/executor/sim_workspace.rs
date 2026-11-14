//! Seeded adversarial simulations of the two sweeps (larry#87, larry#182 defect
//! 3, larry#214): a workspace survives exactly when the caller's live list names
//! it, an origin survives exactly when a live eval holds its lock, a file is
//! nobody's, and a second sweep removes nothing more.
//!
//! The oracle is a reference model, never this code's output: the live list for
//! the workspaces, and the lock each origin carries. One seed drives
//! every draw, so a failing seed replays exactly.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use lgwks_std::seeded::Seeded;

use super::{hold_origin, sweep, sweep_origins};

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

/// Per-process counter that gives each simulation its own state directory.
static DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory under the system temp dir.
fn scratch_directory() -> PathBuf {
    let index = DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "larry-workspace-sim-{}-{index}",
        std::process::id()
    ))
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
    let state = scratch_directory();
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
    drop(std::fs::remove_dir_all(&state));
    Ok(())
}

/// The neighbour and the fat-finger client: for every seed a workspace directory
/// survives exactly when the caller's live list names it, and a file beside them
/// is never removed.
#[test]
fn sim_the_sweep_keeps_exactly_the_named_live_runs_for_every_seed() -> TestResult {
    for seed in SEEDS {
        let state = scratch_directory();
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
        drop(std::fs::remove_dir_all(&state));
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
        let state = scratch_directory();
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
        drop(std::fs::remove_dir_all(&state));
    }
    Ok(())
}

/// The double-clicker: running the sweep twice over one tree removes nothing
/// more and keeps exactly the same set, for every seed's own names.
#[test]
fn sim_a_second_sweep_removes_nothing_more_for_every_seed() -> TestResult {
    for seed in SEEDS {
        let state = scratch_directory();
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
        drop(std::fs::remove_dir_all(&state));
    }
    Ok(())
}
