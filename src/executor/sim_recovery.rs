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

use larry_contracts::authority::Scope;
use larry_runtime::journal::Journal;
use lgwks_std::seeded::Seeded;

use super::super::ExecError;
use super::super::gate::{policy, task_settings};
use super::{JOURNALS_DIR, journal_live, journal_path, recover, recovered_evidence};

/// What every simulation here returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The seeds the recovery model replays.
const SEEDS: [u64; 4] = [
    0x5EED_0000_0000_0031,
    0x5EED_0000_0000_0032,
    0x5EED_0000_FFFF_0031,
    0xDEAD_BEEF_CAFE_0031,
];

/// A fresh state directory: empty when it is made and removed when its guard drops, so a
/// process id the system reuses never lands a simulation in a stale tree (larry#267).
fn scratch_directory() -> Result<crate::executor::sim_tree::Scratch, crate::sim_error::SimError> {
    crate::executor::sim_tree::scratch("larry-recovery")
}

/// The policy an unset `LARRY_POLICY` yields, and the journal capacity it sets.
fn settings() -> Result<(BTreeSet<Scope>, usize), ExecError> {
    let scopes = policy(&|_name: &str| None)?;
    let capacity = task_settings(&scopes).journal_capacity;
    Ok((scopes, capacity))
}

/// The clock-fool and the pulled plug: an absent journal is not live, a journal
/// whose lock this process holds is, and the same journal after the lock is
/// released is not again. The lock, never a process id or a name, is the proof.
#[test]
fn sim_a_journal_is_live_only_while_its_lock_is_held() -> TestResult {
    let (scopes, capacity) = settings()?;
    let state = scratch_directory()?;
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
    Ok(())
}

/// The neighbour: a journal a live executor holds is skipped without stopping
/// the sweep, a dead run's journal is set aside as recovered, one whose bytes are
/// not a journal is kept as refused, and a file that is not a journal is left
/// alone.
#[test]
fn sim_recover_skips_a_held_journal_and_sets_every_other_aside() -> TestResult {
    let (scopes, capacity) = settings()?;
    let state = scratch_directory()?;
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
        recover(&state, &scopes)?.set_aside,
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
    Ok(())
}

/// The pulled-plug and the monkey: for every seed a directory of journals a dead
/// executor left is set aside as recovered when its bytes are a journal and as
/// refused when they are not, and a file that is not a journal is left alone.
#[test]
fn sim_a_dead_journal_directory_is_set_aside_exactly_for_every_seed() -> TestResult {
    for seed in SEEDS {
        let (scopes, capacity) = settings()?;
        let state = scratch_directory()?;
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
            recover(&state, &scopes)?.set_aside,
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
        let left = scratch_directory()?;
        let right = scratch_directory()?;
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
            recover(&left, &scopes)?.set_aside,
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
            recover(&right, &scopes)?.set_aside,
            right_count,
            "seed {seed}: the right directory sets aside its own journals afterwards"
        );
    }
    Ok(())
}

/// The saboteur at every fault class, for every seed (larry#394): a journal the device refuses
/// to open stays a journal and is reported kept with its typed cause, and the next start
/// recovers it once the fault clears; a journal whose bytes are refused is set aside as refused
/// and is evidence a keyed retry answers from; a held journal is skipped; a sound one is
/// recovered. The oracle is the fault each file was given, never the code's own answer.
#[cfg(unix)]
#[test]
fn sim_each_recovery_fault_class_keeps_or_sets_aside_its_journal() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let (scopes, capacity) = settings()?;
    for seed in SEEDS {
        let state = scratch_directory()?;
        std::fs::create_dir_all(state.join(JOURNALS_DIR))?;
        let mut seeded = Seeded::from_seed(seed);
        let sound = format!("sound-{seed}");
        let damaged = format!("damaged-{seed}");
        let faulted = format!("faulted-{seed}");
        let held_id = format!("held-{seed}");
        drop(Journal::open(&journal_path(&state, &sound), capacity)?);
        let noise = format!("garbage-{}", seeded.index(100_000)?);
        std::fs::write(journal_path(&state, &damaged), noise.as_bytes())?;
        drop(Journal::open(&journal_path(&state, &faulted), capacity)?);
        let faulted_path = journal_path(&state, &faulted);
        std::fs::set_permissions(&faulted_path, std::fs::Permissions::from_mode(0o000))?;
        let held = Journal::open(&journal_path(&state, &held_id), capacity)?;

        let first = recover(&state, &scopes)?;
        std::fs::set_permissions(&faulted_path, std::fs::Permissions::from_mode(0o600))?;
        assert_eq!(first.set_aside, 2, "seed {seed}: the sound and the damaged");
        assert_eq!(
            first
                .kept
                .iter()
                .map(|kept| kept.path.clone())
                .collect::<Vec<_>>(),
            vec![faulted_path.clone()],
            "seed {seed}: only the faulted journal is kept"
        );
        assert!(
            first
                .kept
                .iter()
                .all(|kept| !kept.cause.refuses_the_record()),
            "seed {seed}: a kept journal's cause never refuses its bytes"
        );
        assert!(faulted_path.exists(), "seed {seed}: it stays a journal");
        assert!(
            recovered_evidence(&state, &damaged)?.is_some_and(|line| line.contains("refused")),
            "seed {seed}: a refused journal is evidence"
        );
        assert!(
            recovered_evidence(&state, &sound)?.is_some_and(|line| line.contains("recovered")),
            "seed {seed}: a recovered journal is evidence"
        );
        assert_eq!(
            recovered_evidence(&state, &faulted)?,
            None,
            "seed {seed}: a kept journal is no set-aside evidence yet"
        );

        let second = recover(&state, &scopes)?;
        assert_eq!(
            second.set_aside, 1,
            "seed {seed}: the cleared fault recovers"
        );
        assert!(second.kept.is_empty(), "seed {seed}: nothing is kept twice");
        assert!(
            recovered_evidence(&state, &faulted)?.is_some(),
            "seed {seed}: the cleared journal is recovered evidence"
        );
        assert!(
            journal_path(&state, &held_id).exists(),
            "seed {seed}: a held journal is never touched"
        );
        drop(held);
    }
    Ok(())
}

/// The hidden file: a journal whose lock cannot be probed is a typed journal error, never
/// "dead", and evidence whose directory cannot be searched is a typed journals error, never
/// "absent" (larry#334).
#[cfg(unix)]
#[test]
fn sim_a_probe_error_is_typed_and_never_dead_or_absent() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let (scopes, _capacity) = settings()?;
    let state = scratch_directory()?;
    let journals = state.join(JOURNALS_DIR);
    std::fs::create_dir_all(&journals)?;
    let id = "018f0000-0000-7000-8000-000000000005";
    let path = journal_path(&state, id);
    std::fs::write(&path, b"not a journal")?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000))?;
    match journal_live(&state, &scopes, id) {
        Err(ExecError::Journal(_)) => {}
        Ok(live) => {
            return Err(
                format!("a journal whose lock cannot be probed answered live={live}").into(),
            );
        }
        Err(other) => {
            return Err(format!("the probe error is not typed as a journal error: {other}").into());
        }
    }
    std::fs::set_permissions(&journals, std::fs::Permissions::from_mode(0o000))?;
    let hidden = recovered_evidence(&state, id);
    std::fs::set_permissions(&journals, std::fs::Permissions::from_mode(0o700))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    match hidden {
        Err(ExecError::Journals(_)) => {}
        Ok(found) => {
            return Err(format!("a stat error answered absent={}", found.is_none()).into());
        }
        Err(other) => {
            return Err(format!("the stat error is not typed as a journals error: {other}").into());
        }
    }
    Ok(())
}
