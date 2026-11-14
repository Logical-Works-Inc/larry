//! Seeded adversarial simulations of the sealed-report rows a keyed retry is
//! answered from (larry#182 defect 3): the byte bound, the neighbours, the
//! fat-finger client and the pulled plug.
//!
//! The oracle is the report's own length and its bytes, never this code's
//! output: a row reads back exactly when the bound admits its length, and a row
//! written before a close reads back after a reopen. One seed drives every draw,
//! so a failing seed replays exactly.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use lgwks_std::seeded::Seeded;

use super::super::ExecError;
use super::Ledger;

/// What every simulation here returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The seeds the report models replay.
const SEEDS: [u64; 4] = [
    0x5EED_0000_0000_0021,
    0x5EED_0000_0000_0022,
    0x5EED_0000_FFFF_0021,
    0xDEAD_BEEF_CAFE_0021,
];

/// Per-process counter that gives each simulation its own state directory.
static DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory under the system temp dir.
fn scratch_directory() -> PathBuf {
    let index = DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("larry-ledger-sim-{}-{index}", std::process::id()))
}

/// A report of `length` bytes drawn from the alphabet a sealed report is made of.
fn drawn_report(
    seeded: &mut Seeded,
    length: usize,
) -> Result<Vec<u8>, lgwks_std::seeded::DrawError> {
    /// The bytes a report is drawn from: the JSON text a real report carries.
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz{}[]\":,";
    let mut report = Vec::with_capacity(length);
    while report.len() < length {
        let at = seeded.index(ALPHABET.len())?;
        let Some(byte) = ALPHABET.get(at) else {
            continue;
        };
        report.push(*byte);
    }
    Ok(report)
}

/// The giant-and-empty client: for every seed a recorded report of any length
/// from zero to 64 bytes reads back whole at exactly its own bound and is
/// refused one byte under it, and an id with no record reads as none.
#[test]
fn sim_a_report_roundtrips_at_every_seeded_bound() -> TestResult {
    let directory = scratch_directory();
    let ledger = Ledger::open(&directory)?;
    for seed in SEEDS {
        let mut seeded = Seeded::from_seed(seed);
        for round in 0..8_u32 {
            let length = seeded.index(65)?;
            let report = drawn_report(&mut seeded, length)?;
            let id = format!("run-{seed}-{round}");
            ledger.keep_report(&id, "digest", &report)?;
            assert_eq!(
                ledger.report(&id, length)?,
                Some(report),
                "seed {seed} round {round}: a report reads whole at its own bound"
            );
            if length > 0 {
                assert!(
                    matches!(
                        ledger.report(&id, length.saturating_sub(1)),
                        Err(ExecError::Receipt(_))
                    ),
                    "seed {seed} round {round}: one byte under the bound is refused"
                );
            }
            assert_eq!(
                ledger.report(&format!("absent-{id}"), length)?,
                None,
                "seed {seed} round {round}: an id with no record reads as none"
            );
        }
    }
    drop(std::fs::remove_dir_all(&directory));
    Ok(())
}

/// The neighbour: for every seed many run ids keep their own report, no id ever
/// reads another's, and a second record under one id never moves the first.
#[test]
fn sim_two_runs_never_share_a_recorded_report_for_every_seed() -> TestResult {
    let directory = scratch_directory();
    let ledger = Ledger::open(&directory)?;
    for seed in SEEDS {
        let mut seeded = Seeded::from_seed(seed);
        let count = 2_usize.saturating_add(seeded.index(4)?);
        let mut written: Vec<(String, Vec<u8>)> = Vec::with_capacity(count);
        for index in 0..count {
            let length = seeded.index(24)?;
            let report = drawn_report(&mut seeded, length)?;
            let id = format!("neighbour-{seed}-{index}");
            ledger.keep_report(&id, "digest", &report)?;
            written.push((id, report));
        }
        for entry in &written {
            let id = &entry.0;
            let report = &entry.1;
            assert_eq!(
                ledger.report(id, report.len())?,
                Some(report.clone()),
                "seed {seed}: {id} reads back its own report"
            );
        }
        let first = written.first().ok_or("a run was recorded")?;
        let first_id = &first.0;
        let first_report = &first.1;
        ledger.keep_report(first_id, "digest", b"a second claim")?;
        assert_eq!(
            ledger.report(first_id, first_report.len())?,
            Some(first_report.clone()),
            "seed {seed}: the first record under one id is the run"
        );
    }
    drop(std::fs::remove_dir_all(&directory));
    Ok(())
}

/// The fat-finger client: a refusal past the bound names the length it found,
/// never a truncated or silent answer, for every seed's own report.
#[test]
fn sim_a_refusal_past_the_bound_names_the_recorded_length() -> TestResult {
    let directory = scratch_directory();
    let ledger = Ledger::open(&directory)?;
    for seed in SEEDS {
        let mut seeded = Seeded::from_seed(seed);
        let length = 1_usize.saturating_add(seeded.index(48)?);
        let report = drawn_report(&mut seeded, length)?;
        let id = format!("bound-{seed}");
        ledger.keep_report(&id, "digest", &report)?;
        match ledger.report(&id, length.saturating_sub(1)) {
            Err(ExecError::Receipt(super::super::ReceiptError::TooLarge { len })) => {
                assert_eq!(
                    len, length,
                    "seed {seed}: the refusal names the recorded length"
                );
            }
            other => {
                return Err(format!(
                    "seed {seed}: one byte under the bound must refuse with the recorded length, got {other:?}"
                )
                .into());
            }
        }
    }
    drop(std::fs::remove_dir_all(&directory));
    Ok(())
}

/// The pulled plug: for every seed every row written before the store closed
/// reads back after it is opened again, so a keyed retry after a restart still
/// finds its run's evidence.
#[test]
fn sim_a_report_survives_a_reopen_for_every_seed() -> TestResult {
    for seed in SEEDS {
        let directory = scratch_directory();
        let mut written: Vec<(String, Vec<u8>)> = Vec::new();
        {
            let ledger = Ledger::open(&directory)?;
            let mut seeded = Seeded::from_seed(seed);
            let count = 1_usize.saturating_add(seeded.index(4)?);
            for index in 0..count {
                let length = seeded.index(32)?;
                let report = drawn_report(&mut seeded, length)?;
                let id = format!("restart-{seed}-{index}");
                ledger.keep_report(&id, "digest", &report)?;
                written.push((id, report));
            }
        }
        let reopened = Ledger::open(&directory)?;
        for entry in &written {
            let id = &entry.0;
            let report = &entry.1;
            assert_eq!(
                reopened.report(id, report.len())?,
                Some(report.clone()),
                "seed {seed}: {id} survives a reopen"
            );
        }
        drop(reopened);
        drop(std::fs::remove_dir_all(&directory));
    }
    Ok(())
}
