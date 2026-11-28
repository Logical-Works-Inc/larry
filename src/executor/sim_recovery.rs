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
use super::super::gate::{finish, policy, task_settings};
use super::{
    JOURNALS_DIR, KeptCause, RunLock, journal_live, journal_path, live_between, recover,
    recovered_evidence, run_held,
};

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

/// The starting run (larry#456): a run holds its `<task>.lock` before its journal is locked,
/// so between the two its journal is unlocked. A run lock held is live for the reap
/// (`run_held`), for the workspace sweep (`journal_live`) and for recovery, which skips that
/// journal without stopping and still sets the next, dead one aside; released, it is not.
#[test]
fn sim_a_held_run_lock_is_live_before_its_journal_is_locked() -> TestResult {
    let (scopes, capacity) = settings()?;
    let state = scratch_directory()?;
    std::fs::create_dir_all(state.join(JOURNALS_DIR))?;
    let mut seeded = Seeded::from_seed(SEEDS[0]);
    let starting = format!("018f0000-0000-7000-8000-{:012x}", seeded.below(1 << 40)?);
    // Sorted after the starting run, so a sweep that stopped at the skip never reaches it.
    let dead = "ffff0000-0000-7000-8000-000000000002";
    assert!(!run_held(&state, &starting), "no lock file is no live run");
    let lock = RunLock::take(&state, &starting)?.ok_or("the first take holds the lock")?;
    // The journal exists and is unlocked, as it is between a run's create and its lock.
    drop(Journal::open(&journal_path(&state, &starting), capacity)?);
    drop(Journal::open(&journal_path(&state, dead), capacity)?);
    assert!(run_held(&state, &starting), "a held run lock is a live run");
    assert!(
        journal_live(&state, &scopes, &starting)?,
        "the workspace sweep reads a held run lock as live"
    );
    let swept = recover(&state, &scopes)?;
    assert_eq!(swept.set_aside, 1, "only the dead run is set aside");
    assert!(
        journal_path(&state, &starting).exists(),
        "the starting run keeps its journal"
    );
    assert!(
        recovered_evidence(&state, dead)?.is_some(),
        "the sweep goes on past the held run to the dead one"
    );
    lock.release()?;
    assert!(
        !run_held(&state, &starting),
        "a released run lock is not live"
    );
    Ok(())
}

/// The saboteur at the run lock (larry#591): for every seed one dead run's lock is a directory,
/// another's has its mode cleared, another's is a FIFO, and one is free. Each unopenable lock
/// reads as held and keeps its journal with a typed lock cause, a retry of that run is refused,
/// the free one is recovered, and clearing the faults recovers the rest. The oracle is the
/// fault each lock was given, never the code's own answer.
#[cfg(unix)]
#[test]
fn sim_an_unopenable_run_lock_reads_as_held_and_keeps_its_journal() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let (scopes, capacity) = settings()?;
    for seed in SEEDS {
        let state = scratch_directory()?;
        std::fs::create_dir_all(state.join(JOURNALS_DIR))?;
        let mut seeded = Seeded::from_seed(seed);
        let tag = seeded.below(1 << 40)?;
        let ids = ["directory", "closed", "fifo", "free"].map(|kind| format!("{kind}-{tag}"));
        let [ref directory, ref closed, ref fifo, ref free] = ids;
        let lock_of = |id: &str| journal_path(&state, id).with_extension("lock");
        for id in &ids {
            drop(Journal::open(&journal_path(&state, id), capacity)?);
        }
        std::fs::create_dir(lock_of(directory))?;
        std::fs::write(lock_of(closed), b"")?;
        std::fs::set_permissions(lock_of(closed), std::fs::Permissions::from_mode(0o000))?;
        let made = std::process::Command::new("mkfifo")
            .arg(lock_of(fifo))
            .status()?;
        assert!(made.success(), "seed {seed:#x}: the fifo is made");

        for id in [directory, closed, fifo] {
            assert!(run_held(&state, id), "seed {seed:#x}: {id} reads as held");
            assert!(
                matches!(RunLock::take(&state, id), Err(ExecError::Journals(_))),
                "seed {seed:#x}: a retry of {id} is refused typed, never run beside it"
            );
        }
        assert!(
            !run_held(&state, free),
            "seed {seed:#x}: no lock is not held"
        );
        let first = recover(&state, &scopes)?;
        assert_eq!(first.set_aside, 1, "seed {seed:#x}: only the free run");
        let mut kept: Vec<_> = first
            .kept
            .iter()
            .filter(|kept| matches!(kept.cause, KeptCause::RunLock(_)))
            .map(|kept| kept.path.clone())
            .collect();
        kept.sort();
        let mut spoiled: Vec<_> = [closed, directory, fifo]
            .map(|id| journal_path(&state, id))
            .to_vec();
        spoiled.sort();
        assert_eq!(kept, spoiled, "seed {seed:#x}: each is kept for its lock");

        std::fs::remove_dir(lock_of(directory))?;
        std::fs::set_permissions(lock_of(closed), std::fs::Permissions::from_mode(0o600))?;
        std::fs::remove_file(lock_of(fifo))?;
        let second = recover(&state, &scopes)?;
        assert_eq!(second.set_aside, 3, "seed {seed:#x}: the cleared locks");
        assert!(second.kept.is_empty(), "seed {seed:#x}: nothing stays");
    }
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
            first.kept.iter().all(|kept| matches!(
                kept.cause,
                KeptCause::Journal(ref cause) if !cause.refuses_the_record()
            )),
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

/// larry#460, the run that finishes as the sweep reads it: a run removes its journal, then its
/// lock, and a probe that saw the journal before the run finished then found no lock and opened
/// the journal, which made an empty one in the finished run's place, so the next start set it
/// aside as `.recovered` evidence of a run that never died. For every seed the run finishes
/// either before the probe or right after the probe's first read; the probe never makes a
/// journal, and a recovery after it sets nothing aside.
#[test]
fn sim_a_run_finishing_as_the_sweep_probes_it_leaves_no_journal() -> TestResult {
    let (scopes, capacity) = settings()?;
    let state = scratch_directory()?;
    std::fs::create_dir_all(state.join(JOURNALS_DIR))?;
    for seed in SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let id = format!("018f0000-0000-7000-8000-{:012x}", rng.below(1 << 40)?);
        let path = journal_path(&state, &id);
        let lock = RunLock::take(&state, &id)?.ok_or("a fresh run takes its lock")?;
        let journal = Journal::open(&path, capacity)?;
        // The run's own end, in the gate's order: the journal goes, then the lock.
        let end = move || -> Result<(), ExecError> {
            drop(journal);
            std::fs::remove_file(&path).map_err(ExecError::Journals)?;
            lock.release()
        };
        let early = rng.below(2)? == 0;
        let live = if early {
            end()?;
            live_between(&state, &scopes, &id, || Ok(()))?
        } else {
            live_between(&state, &scopes, &id, end)?
        };
        assert_eq!(
            live, !early,
            "seed {seed:#x}: a run is live exactly while it held its lock at the probe"
        );
        assert!(
            !journal_path(&state, &id).try_exists()?,
            "seed {seed:#x}: the probe made no journal in a finished run's place"
        );
        assert_eq!(
            recover(&state, &scopes)?.set_aside,
            0,
            "seed {seed:#x}: no run died, so nothing is set aside"
        );
    }
    Ok(())
}

/// larry#394: a journal already there when a retry opened it is a dead run's evidence, so the
/// gate keeps it on any failure of that retry; a run's own journal goes with a failure the store
/// did not cause. The injected failure is one the store-refusal list does not name.
#[test]
fn the_gate_keeps_a_leftover_journal_on_any_failed_retry() -> TestResult {
    let state = scratch_directory()?;
    std::fs::create_dir_all(state.join(JOURNALS_DIR))?;
    let path = journal_path(&state, "018f0000-0000-7000-8000-000000000394");
    let injected = || ExecError::Journals(std::io::Error::other("injected"));
    std::fs::write(&path, b"evidence")?;
    assert!(finish(&path, Err(injected()), true).is_err());
    assert!(path.exists(), "a failed retry keeps the leftover journal");
    assert!(finish(&path, Err(injected()), false).is_err());
    assert!(
        !path.exists(),
        "a run's own journal goes with a failure the store did not cause"
    );
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
