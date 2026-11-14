//! Shared drivers for the seeded simulation suites.
//!
//! Every `sim_*` suite drives the real runtime through its public surface
//! twice per seed and compares what it observed, so a divergence surfaces as
//! a failing seed rather than a silent pass (runtime invariant 3: replay is
//! bit-identical). The helpers here keep that harness in one place.

use std::fmt::Debug;
use std::ops::Range;
use std::path::{Path, PathBuf};

use larry_contracts::authority::Scope;
use larry_contracts::effect::ReconciliationStrategy;
use larry_contracts::ids::ContentDigest;
use larry_runtime::journal::{Journal, JournalEntry};
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::payload::WorkProposal;
use larry_runtime::runtime::{TickReport, restore};

use crate::support::{
    TestRuntime, clock, command_id, operation, proposal, run, runtime_with, seeded, settings,
    submit, tenant,
};

/// A fallible simulation step; setup failures propagate instead of panicking.
pub type Sim<T> = Result<T, Box<dyn std::error::Error>>;

/// Runs `drive` twice for every seed in `seeds` and refuses any divergence.
///
/// Each drive runs inside `support::seeded`, so the seed decides every
/// identity the drive builds and the clock it starts from: two seeds are two
/// different worlds, and one seed must replay its world exactly.
///
/// # Errors
///
/// Propagates the first error `drive` returns, or names the seed whose two
/// runs disagreed.
pub fn replay<T: PartialEq + Debug>(
    seeds: Range<u64>,
    mut drive: impl FnMut(u64) -> Sim<T>,
) -> Sim<()> {
    for seed in seeds {
        let first = seeded(seed, || drive(seed))?;
        let second = seeded(seed, || drive(seed))?;
        if first != second {
            return Err(format!("seed {seed}: replay diverged: {first:?} != {second:?}").into());
        }
    }
    Ok(())
}

/// The trace hash of a journal: every command envelope and event payload, in
/// journal order.
pub fn trace(journal: &Journal) -> ContentDigest {
    journal.entries().iter().fold(
        ContentDigest::over(&[b"sim-trace"]),
        |digest, entry| match *entry {
            JournalEntry::Command(ref command) => {
                ContentDigest::over(&[digest.as_bytes(), command.envelope_digest().as_bytes()])
            }
            JournalEntry::Event(ref event) => {
                ContentDigest::over(&[digest.as_bytes(), event.payload_digest().as_bytes()])
            }
            _ => digest,
        },
    )
}

/// A journal file for `name` and `seed` in the OS temporary directory, absent
/// when the drive starts so both replays of a seed start from nothing.
///
/// # Errors
///
/// Propagates a refusal to remove a file the previous run left.
pub fn fresh_journal(name: &str, seed: u64) -> Sim<PathBuf> {
    let path = std::env::temp_dir().join(format!(
        "larry-sim-{}-{name}-{seed}.journal",
        std::process::id()
    ));
    remove(&path)?;
    Ok(path)
}

/// Removes `path` if it exists.
///
/// # Errors
///
/// Propagates any removal refusal other than an absent file.
pub fn remove(path: &Path) -> Sim<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// A runtime whose policy and worker both allow `FileRead` only.
///
/// # Errors
///
/// As `support::runtime_with`.
pub fn reader(owner: ScriptedOwner, budget: u64) -> Sim<TestRuntime> {
    runtime_with(owner, &[Scope::FileRead], &[Scope::FileRead], budget)
}

/// The `FileRead` runtime restored from `journal` after a crash, with a fresh
/// owner that has nothing scripted.
///
/// # Errors
///
/// Propagates the restore refusal for a journal the runtime cannot fold.
pub fn reopen(journal: Journal, budget: u64) -> Sim<TestRuntime> {
    Ok(restore(
        settings(&[Scope::FileRead], &[Scope::FileRead], budget)?,
        ScriptedOwner::new(),
        clock()?,
        journal,
    )?)
}

/// One `FileRead` unit of work for run `run_n`, tenant `tenant_n` and
/// operation `op`.
pub fn work(
    run_n: u8,
    tenant_n: u8,
    op: u8,
    cost: u64,
    strategy: ReconciliationStrategy,
) -> WorkProposal {
    proposal(
        run(run_n),
        tenant(tenant_n),
        operation(op),
        &[Scope::FileRead],
        cost,
        strategy,
    )
}

/// One `FileRead` unit of work for run and operation `n` under tenant 1.
pub fn read_work(n: u8, cost: u64, strategy: ReconciliationStrategy) -> WorkProposal {
    work(n, 1, n, cost, strategy)
}

/// Submits one `FileRead` unit per `(run, operation)` pair under tenant 1,
/// each as the command numbered like its run.
///
/// # Errors
///
/// Propagates the first admission refusal.
pub fn offer_runs(
    runtime: &mut TestRuntime,
    runs: &[(u8, u8)],
    cost: u64,
    strategy: ReconciliationStrategy,
) -> Sim<()> {
    for &(run_n, op) in runs {
        runtime.offer(run_n, &work(run_n, 1, op, cost, strategy))?;
    }
    Ok(())
}

/// Submits run `run_op` under tenant 1 as command `command` and ticks once.
///
/// # Errors
///
/// Propagates the admission refusal and the tick's error.
pub fn dispatch(
    runtime: &mut TestRuntime,
    command: u8,
    run_op: u8,
    scopes: &[Scope],
    cost: u64,
    strategy: ReconciliationStrategy,
) -> Sim<TickReport> {
    let body = proposal(
        run(run_op),
        tenant(1),
        operation(run_op),
        scopes,
        cost,
        strategy,
    );
    runtime.offer(command, &body)?;
    Ok(runtime.tick()?)
}

/// Submits a work proposal as a `Submit` command with a one-byte identity.
pub trait Offer {
    /// Submits `body` as command `id`.
    ///
    /// # Errors
    ///
    /// Propagates the command encoding error and the admission refusal.
    fn offer(&mut self, id: u8, body: &WorkProposal) -> Sim<()>;
}

impl Offer for TestRuntime {
    fn offer(&mut self, id: u8, body: &WorkProposal) -> Sim<()> {
        self.submit(submit(command_id(id), body)?)?;
        Ok(())
    }
}

/// The kit's own contract: a replayed journey traces identically and survives
/// a restore, a fresh runtime traces as an empty journal, and a diverging
/// drive is refused.
#[test]
fn sim_kit_replay_and_trace_contract() -> Sim<()> {
    let empty = trace(reader(ScriptedOwner::new(), 10)?.journal());
    replay(0..4, |seed| {
        let mut runtime = reader(ScriptedOwner::new(), 10)?;
        let number = u8::try_from(seed % 8)?.saturating_add(1);
        runtime.offer(
            number,
            &read_work(number, 1, ReconciliationStrategy::Queryable),
        )?;
        runtime.tick()?;
        dispatch(
            &mut runtime,
            9,
            9,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::Queryable,
        )?;
        offer_runs(
            &mut runtime,
            &[(10, 10)],
            1,
            ReconciliationStrategy::Queryable,
        )?;
        let after = trace(runtime.journal());
        assert_ne!(
            after, empty,
            "seed {seed}: submitted commands enter the trace"
        );
        let restored = reopen(runtime.into_journal(), 10)?;
        assert_eq!(
            trace(restored.journal()),
            after,
            "seed {seed}: restore keeps the trace"
        );
        Ok((
            after,
            work(number, 2, number, 1, ReconciliationStrategy::Queryable).tenant,
        ))
    })?;
    let world = |seed| {
        seeded(seed, || -> Sim<ContentDigest> {
            let mut runtime = reader(ScriptedOwner::new(), 10)?;
            runtime.offer(1, &read_work(1, 1, ReconciliationStrategy::Queryable))?;
            runtime.tick()?;
            Ok(trace(runtime.journal()))
        })
    };
    assert_ne!(world(1)?, world(2)?, "two seeds drive two different worlds");
    assert_eq!(world(1)?, world(1)?, "one seed drives one world");
    let mut calls = 0_u64;
    let diverged = replay(0..1, |_| {
        calls = calls.saturating_add(1);
        Ok(calls)
    });
    assert!(
        diverged.is_err(),
        "a drive that differs between runs is refused"
    );
    // A fresh file starts absent, and removing one that is already gone is not
    // a refusal: the durable suites depend on both halves of this.
    let path = fresh_journal("kit", 0)?;
    assert!(!path.exists(), "a fresh journal path names no file yet");
    remove(&path)?;
    remove(&path)?;
    Ok(())
}
