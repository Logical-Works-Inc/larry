//! Seeded adversarial simulations of the sealed-report rows a keyed retry is
//! answered from (larry#182 defect 3): the byte bound, the neighbours, the
//! fat-finger client and the pulled plug. Also the live-group rows the reap settles
//! (larry#214): racing starts and a saboteur who damages the rows between starts.
//!
//! The oracle is the report's own length and its bytes, never this code's
//! output: a row reads back exactly when the bound admits its length, and a row
//! written before a close reads back after a reopen. The group rows are judged by
//! what must hold: no row survives its settled group, and no group dies that its
//! row does not name. One seed drives every draw, so a failing seed replays exactly.

use std::fmt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use lgwks_std::process::{ProcessIdentity, identify_process};
use lgwks_std::seeded::Seeded;

use super::super::ExecError;
use super::Ledger;

/// A pid no live process holds: the largest positive `i32` less one.
const DEAD_PID: i32 = 2_147_483_646;

/// The start token a dead pid carries in these rows: the reading this platform makes.
const DEAD_START: &str = if cfg!(target_os = "linux") {
    "proc:x:1"
} else {
    "kinfo:1.000000"
};

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

/// BLAKE3 of a trace, as 64 hex characters: one seed must give one trace hash.
fn trace_hash(trace: &str) -> String {
    let mut hasher = lgwks_std::hash::Hasher::new();
    hasher.update(trace.as_bytes());
    hasher.finalize().to_hex()
}

/// Why a group-row simulation refused: a call under it, or one of its own oracles (larry#214).
#[derive(Debug)]
enum GroupSimError {
    /// A ledger call failed.
    Exec(ExecError),
    /// A process, file or group probe failed.
    Io(std::io::Error),
    /// A statement the simulation runs by hand failed.
    Sql(lgwks_deps::storefront::rusqlite::Error),
    /// The seeded draw failed.
    Draw(lgwks_std::seeded::DrawError),
    /// A count did not fit its integer type.
    Int(std::num::TryFromIntError),
    /// A row's identity was refused.
    Identity(lgwks_std::process::ProcessIdentityError),
    /// The executor's own group could not be identified.
    GroupDown,
    /// A racing start panicked.
    Racer,
    /// Rows survived the racing starts.
    Survivors {
        /// The seed that drew the scenario.
        seed: u64,
        /// The rows still recorded.
        count: usize,
    },
    /// Fewer rows were settled than rows were written.
    Unsettled {
        /// The seed that drew the scenario.
        seed: u64,
        /// Settlements the starts reported in all.
        settled: usize,
        /// Rows written.
        rows: usize,
    },
    /// A group was signalled by a row that does not name it.
    Stranger {
        /// The seed that drew the scenario.
        seed: u64,
    },
    /// The damaged rows did not settle once each.
    Settled {
        /// The seed that drew the scenario.
        seed: u64,
        /// What the first reap settled.
        first: usize,
        /// The damaged rows written.
        expected: usize,
    },
    /// The owned or stranger row settled other than once.
    Final {
        /// The seed that drew the scenario.
        seed: u64,
        /// What the second reap settled.
        second: usize,
        /// What the third reap settled.
        third: usize,
    },
}

impl fmt::Display for GroupSimError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Exec(ref cause) => write!(out, "ledger call: {cause}"),
            Self::Io(ref cause) => write!(out, "io: {cause}"),
            Self::Sql(ref cause) => write!(out, "statement: {cause}"),
            Self::Draw(ref cause) => write!(out, "draw: {cause}"),
            Self::Int(ref cause) => write!(out, "integer: {cause}"),
            Self::Identity(ref cause) => write!(out, "identity: {cause}"),
            Self::GroupDown => out.write_str("the executor's own group could not be identified"),
            Self::Racer => out.write_str("a racing start panicked"),
            Self::Survivors { seed, count } => {
                write!(
                    out,
                    "seed {seed:#x}: {count} rows survive the racing starts"
                )
            }
            Self::Unsettled {
                seed,
                settled,
                rows,
            } => write!(
                out,
                "seed {seed:#x}: {settled} settled for {rows} rows, one row went unsettled"
            ),
            Self::Stranger { seed } => write!(
                out,
                "seed {seed:#x}: a stranger's group was signalled by a row it does not name"
            ),
            Self::Settled {
                seed,
                first,
                expected,
            } => write!(
                out,
                "seed {seed:#x}: {first} settled, expected the {expected} damaged rows"
            ),
            Self::Final {
                seed,
                second,
                third,
            } => write!(
                out,
                "seed {seed:#x}: the owned and stranger rows settle once each, got {second} and {third}"
            ),
        }
    }
}

impl std::error::Error for GroupSimError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Exec(ref cause) => Some(cause),
            Self::Io(ref cause) => Some(cause),
            Self::Sql(ref cause) => Some(cause),
            Self::Draw(ref cause) => Some(cause),
            Self::Int(ref cause) => Some(cause),
            Self::Identity(ref cause) => Some(cause),
            Self::GroupDown
            | Self::Racer
            | Self::Survivors { .. }
            | Self::Unsettled { .. }
            | Self::Stranger { .. }
            | Self::Settled { .. }
            | Self::Final { .. } => None,
        }
    }
}

impl From<ExecError> for GroupSimError {
    fn from(cause: ExecError) -> Self {
        Self::Exec(cause)
    }
}

impl From<std::io::Error> for GroupSimError {
    fn from(cause: std::io::Error) -> Self {
        Self::Io(cause)
    }
}

impl From<lgwks_deps::storefront::rusqlite::Error> for GroupSimError {
    fn from(cause: lgwks_deps::storefront::rusqlite::Error) -> Self {
        Self::Sql(cause)
    }
}

impl From<lgwks_std::seeded::DrawError> for GroupSimError {
    fn from(cause: lgwks_std::seeded::DrawError) -> Self {
        Self::Draw(cause)
    }
}

impl From<std::num::TryFromIntError> for GroupSimError {
    fn from(cause: std::num::TryFromIntError) -> Self {
        Self::Int(cause)
    }
}

impl From<lgwks_std::process::ProcessIdentityError> for GroupSimError {
    fn from(cause: lgwks_std::process::ProcessIdentityError) -> Self {
        Self::Identity(cause)
    }
}

/// One start's reap, run on its own ledger handle: the rows it settled.
fn racing_start(directory: &Path) -> Result<usize, GroupSimError> {
    let ledger = Ledger::open(directory)?;
    Ok(ledger.reap()?)
}

/// The racing starts scenario for one seed: `rows` dead rows, then `starts` handles reap
/// at once. Returns the trace (the oracle's facts, no pids) after every row is judged.
fn racing_trace(seed: u64) -> Result<String, GroupSimError> {
    let directory = scratch_directory();
    let mut seeded = Seeded::from_seed(seed);
    let rows = 1_usize.saturating_add(seeded.index(24)?);
    let starts = 2_usize.saturating_add(seeded.index(14)?);
    {
        let ledger = Ledger::open(&directory)?;
        for index in 0..rows {
            let pid = DEAD_PID.saturating_sub(i32::try_from(index)?);
            ledger.insert(
                &format!("racer-{index}"),
                &ProcessIdentity::new(pid, DEAD_START)?,
            )?;
        }
    }
    let settled: Vec<usize> = std::thread::scope(|scope| {
        // Every racer is spawned before the first is joined, so the starts overlap.
        let mut racers = Vec::with_capacity(starts);
        for _ in 0..starts {
            let dir = directory.as_path();
            racers.push(scope.spawn(move || racing_start(dir)));
        }
        let mut outcomes = Vec::with_capacity(starts);
        for racer in racers {
            outcomes.push(match racer.join() {
                Ok(outcome) => outcome,
                Err(_panicked) => Err(GroupSimError::Racer),
            }?);
        }
        Ok::<Vec<usize>, GroupSimError>(outcomes)
    })?;
    let total: usize = settled.iter().sum();
    let left = Ledger::open(&directory)?.live_tasks()?;
    drop(std::fs::remove_dir_all(&directory));
    if !left.is_empty() {
        return Err(GroupSimError::Survivors {
            seed,
            count: left.len(),
        });
    }
    if total < rows {
        return Err(GroupSimError::Unsettled {
            seed,
            settled: total,
            rows,
        });
    }
    Ok(format!(
        "rows={rows} starts={starts} unsettled=0 settled_at_least_rows=true"
    ))
}

/// Racing starts on one ledger: for every seed, between 2 and 16 starts reap the same dead
/// rows at once; every row is gone afterwards, no row is left unsettled, and the seed replays
/// the same trace hash (larry#214).
#[test]
fn sim_racing_starts_settle_every_dead_row_and_leave_none_for_every_seed() -> TestResult {
    for seed in SEEDS {
        let first = trace_hash(&racing_trace(seed)?);
        let second = trace_hash(&racing_trace(seed)?);
        assert_eq!(
            first, second,
            "seed {seed:#x}: one seed gives one trace hash"
        );
    }
    Ok(())
}

/// Damage between starts: a saboteur who writes damaged rows and a stranger's row beside one
/// the executor really recorded. A group dies only for the row that names its identity, a
/// stranger's live group is never signalled, and a damaged row is dropped unsignalled.
/// Returns the trace of what the oracle observed.
fn saboteur_trace(seed: u64) -> Result<String, GroupSimError> {
    let directory = scratch_directory();
    let mut seeded = Seeded::from_seed(seed);
    let damage = 1_usize.saturating_add(seeded.index(4)?);
    let mut owned = Command::new("sleep").arg("30").process_group(0).spawn()?;
    let mut stranger = Command::new("sleep").arg("30").process_group(0).spawn()?;
    let owned_pid = i32::try_from(owned.id())?;
    let stranger_pid = i32::try_from(stranger.id())?;
    let ledger = Ledger::open(&directory)?;
    let owned_identity = identify_process(owned_pid)?.ok_or(GroupSimError::GroupDown)?;
    ledger.insert("owned", &owned_identity)?;
    ledger.insert("stranger", &ProcessIdentity::new(stranger_pid, DEAD_START)?)?;
    // Damaged rows, one of each kind: a damaged start token, pid zero, a pid past i32, and a
    // start token past the length bound. Each has its own pid, so no two collide on the key.
    let long = "x".repeat(200);
    let dead = i64::from(DEAD_PID);
    let damaged_rows: [(i64, &str); 4] = [
        (dead.saturating_sub(1), "zzz"),
        (0, DEAD_START),
        (4_294_967_296, DEAD_START),
        (dead.saturating_sub(2), long.as_str()),
    ];
    for &(pid, started) in damaged_rows.iter().take(damage) {
        ledger.conn.execute(
            "INSERT INTO live_groups (task_id, pid, started) VALUES (?1, ?2, ?3)",
            lgwks_deps::storefront::rusqlite::params!["damaged", pid, started],
        )?;
    }
    let first = ledger.reap()?;
    let stranger_up = lgwks_std::process::process_group_exists(stranger_pid)?;
    let trace = format!("damaged={damage} reap1={first} stranger_up_after_reap1={stranger_up}");
    if !stranger_up {
        return Err(GroupSimError::Stranger { seed });
    }
    if first != damage {
        return Err(GroupSimError::Settled {
            seed,
            first,
            expected: damage,
        });
    }
    owned.wait()?;
    let second = ledger.reap()?;
    stranger.kill()?;
    stranger.wait()?;
    let third = ledger.reap()?;
    let rows = ledger.live_tasks()?;
    drop(std::fs::remove_dir_all(&directory));
    if second != 1 || third != 1 || !rows.is_empty() {
        return Err(GroupSimError::Final {
            seed,
            second,
            third,
        });
    }
    Ok(format!("{trace} reap2={second} reap3={third} rows_left=0"))
}

/// The saboteur: for every seed, damaged rows and a stranger's live group never get a signal
/// they did not earn, the executor's own row settles only once its group is gone, and the seed
/// replays the same trace hash (larry#214).
#[test]
fn sim_a_saboteur_damages_rows_and_no_group_outside_its_identity_dies_for_every_seed() -> TestResult
{
    for seed in SEEDS {
        let first = trace_hash(&saboteur_trace(seed)?);
        let second = trace_hash(&saboteur_trace(seed)?);
        assert_eq!(
            first, second,
            "seed {seed:#x}: one seed gives one trace hash"
        );
    }
    Ok(())
}
