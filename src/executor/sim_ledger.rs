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

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use lgwks_bot::kernel::clock::{Clock, ClockError};
use lgwks_macros::Error;
use lgwks_std::process::{ProcessIdentity, identify_process, process_group_exists};
use lgwks_std::seeded::Seeded;

use super::super::ExecError;
use super::super::run::await_gone;
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

/// The virtual step one group probe takes: never zero, so time always moves, and small
/// enough that a wait walks the 5 s deadline in a few dozen probes (larry#308).
const VIRTUAL_STEP: Duration = Duration::from_millis(250);

/// A fresh state directory: empty when it is made and removed when its guard drops, so a
/// process id the system reuses never lands a simulation in a stale tree (larry#267).
fn scratch_directory() -> Result<crate::executor::sim_tree::Scratch, crate::sim_error::SimError> {
    crate::executor::sim_tree::scratch("larry-ledger-sim")
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
    let directory = scratch_directory()?;
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
    Ok(())
}

/// The neighbour: for every seed many run ids keep their own report, no id ever
/// reads another's, and a second record under one id never moves the first.
#[test]
fn sim_two_runs_never_share_a_recorded_report_for_every_seed() -> TestResult {
    let directory = scratch_directory()?;
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
    Ok(())
}

/// The fat-finger client: a refusal past the bound names the length it found,
/// never a truncated or silent answer, for every seed's own report.
#[test]
fn sim_a_refusal_past_the_bound_names_the_recorded_length() -> TestResult {
    let directory = scratch_directory()?;
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
    Ok(())
}

/// The pulled plug: for every seed every row written before the store closed
/// reads back after it is opened again, so a keyed retry after a restart still
/// finds its run's evidence.
#[test]
fn sim_a_report_survives_a_reopen_for_every_seed() -> TestResult {
    for seed in SEEDS {
        let directory = scratch_directory()?;
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
#[derive(Debug, Error)]
enum GroupSimError {
    /// A ledger call failed.
    #[error("ledger call: {0}")]
    Exec(#[from] ExecError),
    /// A process, file or group probe failed.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// A statement the simulation runs by hand failed.
    #[error("statement: {0}")]
    Sql(#[from] lgwks_deps::storefront::rusqlite::Error),
    /// The seeded draw failed.
    #[error("draw: {0}")]
    Draw(#[from] lgwks_std::seeded::DrawError),
    /// A count did not fit its integer type.
    #[error("integer: {0}")]
    Int(#[from] std::num::TryFromIntError),
    /// A row's identity was refused.
    #[error("identity: {0}")]
    Identity(#[from] lgwks_std::process::ProcessIdentityError),
    /// The executor's own group could not be identified.
    #[error("the executor's own group could not be identified")]
    GroupDown,
    /// The virtual clock refused the step this simulation asked of it.
    #[error("the virtual clock refused a step: {0}")]
    Clock(#[from] ClockError),
    /// A racing start panicked.
    #[error("a racing start panicked")]
    Racer,
    /// Rows survived the racing starts.
    #[error("seed {seed:#x}: {count} rows survive the racing starts")]
    Survivors {
        /// The seed that drew the scenario.
        seed: u64,
        /// The rows still recorded.
        count: usize,
    },
    /// Fewer rows were settled than rows were written.
    #[error("seed {seed:#x}: {settled} settled for {rows} rows, one row went unsettled")]
    Unsettled {
        /// The seed that drew the scenario.
        seed: u64,
        /// Settlements the starts reported in all.
        settled: usize,
        /// Rows written.
        rows: usize,
    },
    /// A group was signalled by a row that does not name it.
    #[error("seed {seed:#x}: a stranger's group was signalled by a row it does not name")]
    Stranger {
        /// The seed that drew the scenario.
        seed: u64,
    },
    /// The damaged rows did not settle once each.
    #[error("seed {seed:#x}: {first} settled, expected the {expected} damaged rows")]
    Settled {
        /// The seed that drew the scenario.
        seed: u64,
        /// What the first reap settled.
        first: usize,
        /// The damaged rows written.
        expected: usize,
    },
    /// One state directory's reap settled or removed a row another directory recorded.
    #[error(
        "seed {seed:#x}: a foreign reap settled {foreign}, and {left} of {rows} rows stayed with their own directory"
    )]
    Crossed {
        /// The seed that drew the scenario.
        seed: u64,
        /// What the foreign reap settled.
        foreign: usize,
        /// The rows left in the directory that recorded them.
        left: usize,
        /// The rows that directory recorded.
        rows: usize,
    },
    /// The owned or stranger row settled other than once.
    #[error(
        "seed {seed:#x}: the owned and stranger rows settle once each, got {second} and {third}"
    )]
    Final {
        /// The seed that drew the scenario.
        seed: u64,
        /// What the second reap settled.
        second: usize,
        /// What the third reap settled.
        third: usize,
    },
    /// The shared scratch guard could not make its directory.
    #[error("scratch tree: {0}")]
    Tree(#[from] crate::sim_error::SimError),
}

/// Whether the group is gone within the deadline on a virtual clock this probe advances: one
/// step per probe, so the 5 s deadline passes with no real time and the seed replays the same
/// trace (larry#308, larry#268).
fn virtual_group_gone(clock: &Clock, pgid: i32) -> Result<bool, GroupSimError> {
    await_gone(clock, || -> Result<bool, GroupSimError> {
        clock.advance(VIRTUAL_STEP)?;
        Ok(process_group_exists(pgid)?)
    })
}

/// One start's reap, run on its own ledger handle: the rows it settled.
fn racing_start(directory: &Path) -> Result<usize, GroupSimError> {
    let ledger = Ledger::open(directory)?;
    let clock = Clock::virtual_at(Duration::ZERO);
    ledger.reap_with(&|_task| Ok(false), |pgid| virtual_group_gone(&clock, pgid))
}

/// The racing starts scenario for one seed: `rows` dead rows, then `starts` handles reap
/// at once. Returns the trace (the oracle's facts, no pids) after every row is judged.
fn racing_trace(seed: u64) -> Result<String, GroupSimError> {
    let directory = scratch_directory()?;
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
            let dir: &Path = &directory;
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

/// Two tenants, two state directories, one host: tenant A records seeded dead rows, then tenant
/// B's start reaps its own ledger, racing A's reopen. B settles nothing of A's and A keeps every
/// row until A's own start reaps them all. Returns the trace of what the oracle observed.
fn two_tenant_trace(seed: u64) -> Result<String, GroupSimError> {
    let tenant_a = scratch_directory()?;
    let tenant_b = scratch_directory()?;
    let mut seeded = Seeded::from_seed(seed);
    let rows = 1_usize.saturating_add(seeded.index(16)?);
    let b_rows = seeded.index(4)?;
    {
        let ledger = Ledger::open(&tenant_a)?;
        for index in 0..rows {
            let pid = DEAD_PID.saturating_sub(i32::try_from(index)?);
            ledger.insert(
                &format!("tenant-a-{index}"),
                &ProcessIdentity::new(pid, DEAD_START)?,
            )?;
        }
    }
    {
        let ledger = Ledger::open(&tenant_b)?;
        for index in 0..b_rows {
            let pid = DEAD_PID.saturating_sub(i32::try_from(index.saturating_add(rows))?);
            ledger.insert(
                &format!("tenant-b-{index}"),
                &ProcessIdentity::new(pid, DEAD_START)?,
            )?;
        }
    }
    let (foreign, left) = std::thread::scope(|scope| {
        let b_dir: &Path = &tenant_b;
        let a_dir: &Path = &tenant_a;
        let b_start = scope.spawn(move || racing_start(b_dir));
        let a_open =
            scope.spawn(move || Ledger::open(a_dir).and_then(|ledger| ledger.live_tasks()));
        let foreign = match b_start.join() {
            Ok(outcome) => outcome,
            Err(_panicked) => Err(GroupSimError::Racer),
        }?;
        let left = match a_open.join() {
            Ok(outcome) => outcome.map_err(GroupSimError::Exec),
            Err(_panicked) => Err(GroupSimError::Racer),
        }?;
        Ok::<(usize, usize), GroupSimError>((foreign, left.len()))
    })?;
    let own = racing_start(&tenant_a)?;
    let after = Ledger::open(&tenant_a)?.live_tasks()?.len();
    if foreign != b_rows || left != rows || own != rows || after != 0 {
        return Err(GroupSimError::Crossed {
            seed,
            foreign,
            left,
            rows,
        });
    }
    Ok(format!(
        "a_rows={rows} b_rows={b_rows} b_settled={foreign} a_left={left} a_settled={own} a_after=0"
    ))
}

/// Two tenants on one host: for every seed, one state directory's reap never settles a row
/// another directory recorded, the owner's own start settles each of its rows, and the seed
/// replays the same trace hash (larry#214).
#[test]
fn sim_two_tenants_reap_only_their_own_rows_for_every_seed() -> TestResult {
    for seed in SEEDS {
        let first = trace_hash(&two_tenant_trace(seed)?);
        let second = trace_hash(&two_tenant_trace(seed)?);
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
    let directory = scratch_directory()?;
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
    let clock = Clock::virtual_at(Duration::ZERO);
    let first = ledger.reap_with(&|_task| Ok(false), |pgid| virtual_group_gone(&clock, pgid))?;
    let stranger_up = process_group_exists(stranger_pid)?;
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
    let second = ledger.reap_with(&|_task| Ok(false), |pgid| virtual_group_gone(&clock, pgid))?;
    stranger.kill()?;
    stranger.wait()?;
    let third = ledger.reap_with(&|_task| Ok(false), |pgid| virtual_group_gone(&clock, pgid))?;
    let rows = ledger.live_tasks()?;
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

/// How long the busy-store simulation lets a write wait: the store's own 60 s bound, cut so
/// the simulation ends, on the same SQLite busy handler.
const BUSY_WAIT: Duration = Duration::from_millis(300);

/// This thread's own CPU clock: a sleeping wait does not advance it, a spin does, and host
/// load changes neither, so the bound below cannot pass or fail on how busy the machine is.
fn thread_cpu() -> Result<Duration, std::num::TryFromIntError> {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::ThreadCPUTime);
    Ok(Duration::new(
        u64::try_from(now.tv_sec)?,
        u32::try_from(now.tv_nsec)?,
    ))
}

/// A ledger write on a store another executor is writing waits on the store's busy handler,
/// spends under 5% of the wait on the CPU, is refused as a typed statement error once the
/// bound passes, and lands once the holder lets go (larry#398: the lock loop this replaced
/// spun a full core per waiter for 60 s and starved the holder on a 1 to 2 vCPU host).
#[test]
fn sim_a_ledger_write_on_a_busy_store_waits_without_spinning() -> TestResult {
    let directory = scratch_directory()?;
    let ledger = Ledger::open(&directory)?;
    let holder = lgwks_deps::sqlite::open(&directory.join("executor.db"))?;
    holder.execute_batch("BEGIN IMMEDIATE")?;
    ledger.conn.busy_timeout(BUSY_WAIT)?;
    let before = thread_cpu()?;
    let refused = ledger.keep_report("busy-run", "digest", b"report");
    let spent = thread_cpu()?.saturating_sub(before);
    holder.execute_batch("ROLLBACK")?;
    assert!(
        matches!(refused, Err(ExecError::Sql(_))),
        "a write past the busy bound is the typed statement error: {refused:?}"
    );
    assert!(
        spent.saturating_mul(20) < BUSY_WAIT,
        "the waiting write spent {spent:?} of CPU over a {BUSY_WAIT:?} wait"
    );
    ledger.keep_report("busy-run", "digest", b"report")?;
    assert_eq!(
        ledger.report("busy-run", 6)?,
        Some(b"report".to_vec()),
        "the write lands once the holder lets go"
    );
    Ok(())
}

/// A state directory that cannot be made is the typed state-directory error, naming the
/// cause, before any store file is touched.
#[test]
fn sim_a_state_directory_that_is_a_file_is_the_typed_state_dir_error() -> TestResult {
    let directory = scratch_directory()?;
    let state = directory.join("state");
    std::fs::write(&state, "a file where the state directory goes")?;
    match Ledger::open(&state.join("inner")) {
        Err(ExecError::StateDir(cause)) => {
            assert_eq!(cause.kind(), std::io::ErrorKind::NotADirectory, "{cause}");
            Ok(())
        }
        Err(other) => Err(format!("the state directory error, not: {other}").into()),
        Ok(_) => Err("a state directory under a file opened".into()),
    }
}
