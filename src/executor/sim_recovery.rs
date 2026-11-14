//! Seeded adversarial simulations of the task-journal recovery (larry#75,
//! larry#182 defect 3): the lock as the only liveness proof, the sweep that must
//! not stop at the first journal it may not touch, and the neighbours whose
//! evidence never crosses.
//!
//! The oracle is a reference model, never this code's output: a journal is live
//! exactly while a process holds its lock, and a directory of dead journals is
//! set aside as recovered when the bytes are a journal and as refused when they
//! are not. One seed drives every draw, so a failing seed replays exactly.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use larry_contracts::authority::Scope;
use larry_runtime::journal::Journal;
use lgwks_std::seeded::Seeded;

use super::super::ExecError;
use super::super::gate::{lease_end, policy, task_settings};
use super::{JOURNALS_DIR, journal_live, journal_path, recover};

/// What every simulation here returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The seeds the recovery model replays.
const SEEDS: [u64; 4] = [
    0x5EED_0000_0000_0031,
    0x5EED_0000_0000_0032,
    0x5EED_0000_FFFF_0031,
    0xDEAD_BEEF_CAFE_0031,
];

/// Per-process counter that gives each simulation its own state directory.
static DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory under the system temp dir.
fn scratch_directory() -> PathBuf {
    let index = DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("larry-recovery-{}-{index}", std::process::id()))
}

/// The policy an unset `LARRY_POLICY` yields, and the journal capacity it sets.
fn settings() -> Result<(BTreeSet<Scope>, usize), ExecError> {
    let scopes = policy(&|_name: &str| None)?;
    let expiry = lease_end(SystemTime::now()).map_err(ExecError::Gate)?;
    let capacity = task_settings(&scopes, expiry).journal_capacity;
    Ok((scopes, capacity))
}

/// The clock-fool and the pulled plug: an absent journal is not live, a journal
/// whose lock this process holds is, and the same journal after the lock is
/// released is not again. The lock, never a process id or a name, is the proof.
#[test]
fn sim_a_journal_is_live_only_while_its_lock_is_held() -> TestResult {
    let (scopes, capacity) = settings()?;
    let state = scratch_directory();
    std::fs::create_dir_all(state.join(JOURNALS_DIR))?;
    let id = "018f0000-0000-7000-8000-000000000001";
    assert!(
        !journal_live(&state, &scopes, id)?,
        "an absent journal is not live"
    );
    let path = journal_path(&state, id);
    let held = Journal::open(&path, capacity)?;
    assert!(
        journal_live(&state, &scopes, id)?,
        "a journal whose lock is held is live"
    );
    drop(held);
    assert!(
        !journal_live(&state, &scopes, id)?,
        "a journal whose lock was released is not live"
    );
    drop(std::fs::remove_dir_all(&state));
    Ok(())
}

/// The neighbour: a journal a live executor holds is skipped without stopping
/// the sweep, a dead run's journal is set aside as recovered, one whose bytes are
/// not a journal is kept as refused, and a file that is not a journal is left
/// alone.
#[test]
fn sim_recover_skips_a_held_journal_and_sets_every_other_aside() -> TestResult {
    let (scopes, capacity) = settings()?;
    let state = scratch_directory();
    let journals = state.join(JOURNALS_DIR);
    std::fs::create_dir_all(&journals)?;
    let held_id = "018f0000-0000-7000-8000-000000000002";
    let dead_id = "018f0000-0000-7000-8000-000000000003";
    let broken_id = "018f0000-0000-7000-8000-000000000004";
    let note = journals.join("0000.note");
    std::fs::write(&note, b"not a journal")?;
    let held = Journal::open(&journal_path(&state, held_id), capacity)?;
    drop(Journal::open(&journal_path(&state, dead_id), capacity)?);
    let broken = journal_path(&state, broken_id);
    std::fs::write(&broken, b"not a journal")?;

    assert_eq!(
        recover(&state, &scopes)?,
        2,
        "the dead journal and the damaged one are set aside, the held one is not"
    );
    assert!(
        journal_path(&state, held_id).exists(),
        "a live journal stays where it is"
    );
    assert!(
        journal_path(&state, dead_id)
            .with_extension("recovered")
            .exists(),
        "a dead run's journal is kept as recovered"
    );
    assert!(
        broken.with_extension("refused").exists(),
        "a journal whose bytes are refused is kept as refused"
    );
    assert!(note.exists(), "a file that is not a journal is left alone");
    drop(held);
    drop(std::fs::remove_dir_all(&state));
    Ok(())
}

/// The pulled-plug and the monkey: for every seed a directory of journals a dead
/// executor left is set aside as recovered when its bytes are a journal and as
/// refused when they are not, and a file that is not a journal is left alone.
#[test]
fn sim_a_dead_journal_directory_is_set_aside_exactly_for_every_seed() -> TestResult {
    for seed in SEEDS {
        let (scopes, capacity) = settings()?;
        let state = scratch_directory();
        let journals = state.join(JOURNALS_DIR);
        std::fs::create_dir_all(&journals)?;
        let mut seeded = Seeded::from_seed(seed);
        let count = 1_usize.saturating_add(seeded.index(4)?);
        let note = journals.join("0000.note");
        std::fs::write(&note, b"not a journal")?;
        let mut expected: Vec<(String, bool)> = Vec::with_capacity(count);
        for index in 0..count {
            let id = format!("run-{seed}-{index}");
            let valid = seeded.below(2)? == 1;
            let path = journal_path(&state, &id);
            if valid {
                drop(Journal::open(&path, capacity)?);
            } else {
                let text = format!(
                    "garbage-{}-{}",
                    seeded.index(100_000)?,
                    seeded.index(100_000)?
                );
                std::fs::write(&path, text.as_bytes())?;
            }
            expected.push((id, valid));
        }
        assert_eq!(
            recover(&state, &scopes)?,
            count,
            "seed {seed}: every journal is set aside, the note is not"
        );
        for entry in &expected {
            let id = &entry.0;
            let kept_as = if entry.1 { "recovered" } else { "refused" };
            assert!(
                journal_path(&state, id).with_extension(kept_as).exists(),
                "seed {seed}: {id} is kept as {kept_as}"
            );
            assert!(
                !journal_path(&state, id).exists(),
                "seed {seed}: {id} is no longer a running journal"
            );
        }
        assert!(note.exists(), "seed {seed}: a file is left alone");
        drop(std::fs::remove_dir_all(&state));
    }
    Ok(())
}

/// The fat-finger client: for every seed two state directories are two tenants,
/// and recovering one directory neither touches nor counts the journals of the
/// other, however many either holds.
#[test]
fn sim_two_state_directories_never_set_aside_each_others_journals() -> TestResult {
    let (scopes, capacity) = settings()?;
    for seed in SEEDS {
        let left = scratch_directory();
        let right = scratch_directory();
        std::fs::create_dir_all(left.join(JOURNALS_DIR))?;
        std::fs::create_dir_all(right.join(JOURNALS_DIR))?;
        let mut seeded = Seeded::from_seed(seed);
        let left_count = 1_usize.saturating_add(seeded.index(3)?);
        let right_count = 1_usize.saturating_add(seeded.index(3)?);
        let mut right_ids = Vec::with_capacity(right_count);
        for index in 0..left_count {
            drop(Journal::open(
                &journal_path(&left, &format!("left-{seed}-{index}")),
                capacity,
            )?);
        }
        for index in 0..right_count {
            let id = format!("right-{seed}-{index}");
            drop(Journal::open(&journal_path(&right, &id), capacity)?);
            right_ids.push(id);
        }
        assert_eq!(
            recover(&left, &scopes)?,
            left_count,
            "seed {seed}: the left directory sets aside only its own journals"
        );
        for id in &right_ids {
            assert!(
                journal_path(&right, id).exists(),
                "seed {seed}: the right directory's journal {id} is untouched"
            );
        }
        assert_eq!(
            recover(&right, &scopes)?,
            right_count,
            "seed {seed}: the right directory sets aside its own journals afterwards"
        );
        drop(std::fs::remove_dir_all(&left));
        drop(std::fs::remove_dir_all(&right));
    }
    Ok(())
}
