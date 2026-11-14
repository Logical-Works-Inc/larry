//! larry#219 (ADR 0041): a command whose acceptance record failed to reach the
//! journal is journaled and unreduced, so it is pending, and a retry or a
//! restore reduces it once. The failure is a journal that has room for the
//! command and none for its `CommandAccepted` fact, with the ceiling set from a
//! seeded count of earlier commands, so no test seam is needed.

use std::collections::BTreeSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;

use larry_contracts::command::{Command, CommandAcceptance, CommandKind};
use larry_contracts::ids::{CommandId, OperationId, RunId, TenantId};
use larry_contracts::state::RunState;

use super::{
    Ack, Outcome, Rt, accepted, clock, command, durable, idle_body, ids, journal_file, memory,
    offer, pending, settings, work_body,
};
use crate::error::RuntimeError;
use crate::journal::Journal;
use crate::owner::fixture::ScriptedOwner;
use crate::projection::ProjectionBackend;
use crate::runtime::{Runtime, restore};

/// Tenant `who` (0 is A, 1 is B) and its `index`th submission for `seed`.
fn work(seed: u64, who: u8, index: u8) -> Result<(Command, RunId), Box<dyn std::error::Error>> {
    let tenant = TenantId::from_bytes(ids(who.saturating_add(2), seed));
    let step = who.saturating_mul(8).saturating_add(index);
    let run = RunId::from_bytes(ids(step.saturating_add(0x10), seed));
    let operation = OperationId::from_bytes(ids(step.saturating_add(0x40), seed));
    let body = work_body(run, tenant, operation);
    let id = CommandId::from_bytes(ids(step.saturating_add(0x70), seed));
    Ok((command(id, CommandKind::Submit, &body)?, run))
}

/// How many commands tenant B runs before tenant A's fails: 1 to 3.
fn b_count(seed: u64) -> Result<u8, std::num::TryFromIntError> {
    Ok(u8::try_from(seed % 3)?.saturating_add(1))
}

/// The journal ceiling that leaves room for A's command and none after it:
/// the length B's commands leave, plus one.
fn ceiling(seed: u64) -> Result<usize, Box<dyn std::error::Error>> {
    let mut probe = memory(ScriptedOwner::new())?;
    for index in 0..b_count(seed)? {
        probe.ack(work(seed, 1, index)?.0)?;
    }
    let reduced = probe.tick()?.admitted().len();
    assert_eq!(reduced, usize::from(b_count(seed)?), "seed {seed}: probe");
    Ok(probe.journal().len().saturating_add(1))
}

/// What a runtime holds after tenant A's reduce failed.
struct Case {
    runtime: Rt,
    first: Command,
    tenant_b: TenantId,
    run_a: RunId,
    b_ids: Vec<CommandId>,
    b_runs: Vec<RunId>,
    b_states: Vec<Option<RunState>>,
    capacity: usize,
}

/// Runs B's commands, then A's through the form `seed` picks (the ready queue
/// or durable admission), and returns the runtime after A's tick failed.
fn drive_to_failure(seed: u64, mut runtime: Rt) -> Result<Case, Box<dyn std::error::Error>> {
    let (mut b_ids, mut b_runs) = (Vec::new(), Vec::new());
    let mut tenant_b = None;
    for index in 0..b_count(seed)? {
        let (cmd, run) = work(seed, 1, index)?;
        tenant_b = Some(cmd.tenant());
        b_ids.push(cmd.id());
        b_runs.push(run);
        runtime.ack(cmd)?;
    }
    runtime.tick()?;
    let b_states = b_states_of(&runtime, &b_runs);
    let (first, run_a) = work(seed, 0, 0)?;
    if seed.is_multiple_of(2) {
        runtime.submit(first.clone())?;
    } else {
        runtime.ack(first.clone())?;
    }
    let capacity = runtime.journal().capacity();
    let failed = runtime.tick();
    assert!(
        matches!(failed, Err(RuntimeError::JournalFull { capacity: held }) if held == capacity),
        "seed {seed}: the acceptance record has no room: {failed:?}"
    );
    Ok(Case {
        tenant_b: tenant_b.ok_or("B ran nothing")?,
        runtime,
        first,
        run_a,
        b_ids,
        b_runs,
        b_states,
        capacity,
    })
}

/// The projection's state of each of B's runs.
fn b_states_of(runtime: &Rt, runs: &[RunId]) -> Vec<Option<RunState>> {
    runs.iter()
        .map(|run| runtime.projection().run_state(*run))
        .collect()
}

/// A hash of every journal entry in order, the trace one seed must repeat.
fn trace_hash(journal: &Journal) -> u64 {
    let mut hasher = DefaultHasher::new();
    for entry in journal.entries() {
        format!("{entry:?}").hash(&mut hasher);
    }
    hasher.finish()
}

/// The runtime over the file at `path`, bounded to `capacity` entries.
fn durable_with(path: &Path, capacity: usize) -> Result<Rt, Box<dyn std::error::Error>> {
    let mut settings = settings()?;
    settings.journal_capacity = capacity;
    let journal = Journal::open(path, capacity)?;
    Ok(restore(settings, ScriptedOwner::new(), clock(), journal)?)
}

/// A restored runtime holds A's command pending, reduces it in the next tick
/// and never again, and the journal accepted B's commands and A's once each.
fn reduces_a_once(seed: u64, restored: &mut Rt, a_id: CommandId, b_ids: Vec<CommandId>) -> Outcome {
    assert_eq!(
        pending(restored),
        [a_id],
        "seed {seed}: pending after restore"
    );
    assert_eq!(restored.tick()?.admitted(), &[a_id], "seed {seed}: reduced");
    assert!(restored.tick()?.admitted().is_empty(), "seed {seed}: once");
    let mut want = b_ids;
    want.push(a_id);
    assert_eq!(
        accepted(restored.journal())?,
        want,
        "seed {seed}: once each"
    );
    Ok(())
}

/// larry#219: for every seed, A's command fails to record its acceptance at
/// the point the ceiling picks. It stays pending in the journal's own order, a
/// retry of it through the queue, the protocol and a flood is never a silent
/// duplicate, a foreign tenant cannot take its run, B's runs do not move, and a
/// restore with room reduces it once. The reference is the fold of the journal:
/// the commands it holds without `CommandAccepted`.
#[test]
fn sim_a_reduce_failure_then_retry_is_never_a_silent_duplicate() -> Outcome {
    for seed in 0..32_u64 {
        let mut limited = settings()?;
        limited.journal_capacity = ceiling(seed)?;
        let runtime: Rt = Runtime::new(limited, ScriptedOwner::new(), clock())?;
        let mut case = drive_to_failure(seed, runtime)?;
        let (a_id, tenant_a) = (case.first.id(), case.first.tenant());

        let unreduced: Vec<CommandId> = super::super::recover(case.runtime.journal())?
            .iter()
            .map(|held| held.command.id())
            .collect();
        assert_eq!(
            unreduced,
            [a_id],
            "seed {seed}: the journal holds A unreduced"
        );
        assert_eq!(
            pending(&case.runtime),
            unreduced,
            "seed {seed}: what the journal holds unreduced, the pending set holds"
        );
        assert_eq!(
            accepted(case.runtime.journal())?,
            case.b_ids,
            "seed {seed}: only B's commands are accepted"
        );

        // A retry, once and then a seeded flood: each is refused with the same
        // typed error, none is answered as a duplicate, none is dropped, and the
        // journal does not grow past its ceiling.
        for attempt in 1..=usize::try_from(seed % 4)?.saturating_add(1) {
            case.runtime.submit(case.first.clone())?;
            let retry = case.runtime.tick();
            assert!(
                matches!(retry, Err(RuntimeError::JournalFull { .. })),
                "seed {seed}: retry {attempt} is refused typed, not a duplicate: {retry:?}"
            );
            assert_eq!(pending(&case.runtime), [a_id], "seed {seed}: still pending");
            assert_eq!(
                case.runtime.ready.len(),
                attempt,
                "seed {seed}: kept queued"
            );
            assert_eq!(case.runtime.journal().len(), case.capacity);
        }

        // The protocol retry is `DuplicateReplay` only while the pending set
        // holds the command for its one reduction.
        let (acceptance, _) =
            case.runtime
                .submit_protocol(&offer(tenant_a), case.first.clone(), None)?;
        assert_eq!(acceptance, CommandAcceptance::DuplicateReplay);
        assert_eq!(
            pending(&case.runtime),
            [a_id],
            "seed {seed}: held, not lost"
        );

        // Tenant B cannot adopt A's run, and its own runs have not moved.
        let steer = idle_body(
            case.run_a,
            case.tenant_b,
            OperationId::from_bytes(ids(0x20, seed)),
        );
        let foreign = case.runtime.ack(command(
            CommandId::from_bytes(ids(0x21, seed)),
            CommandKind::Steer,
            &steer,
        )?);
        assert!(
            matches!(foreign, Err(RuntimeError::RunTenantMismatch { got, .. }) if got == case.tenant_b),
            "seed {seed}: B is refused A's run: {foreign:?}"
        );
        assert_eq!(
            b_states_of(&case.runtime, &case.b_runs),
            case.b_states,
            "seed {seed}: A's failure and retries moved none of B's runs"
        );
        assert!(
            case.b_states.iter().all(Option::is_some),
            "seed {seed}: B ran"
        );

        let Case {
            runtime,
            first,
            b_ids,
            ..
        } = case;
        let mut restored: Rt = restore(
            settings()?,
            ScriptedOwner::new(),
            clock(),
            runtime.into_journal(),
        )?;
        reduces_a_once(seed, &mut restored, a_id, b_ids)?;
        restored.submit(first)?;
        assert_eq!(
            restored.tick()?.duplicates(),
            &[a_id],
            "seed {seed}: after its reduction the retry is a duplicate"
        );
    }
    Ok(())
}

/// One kill after the failed reduce: the file holds A's command unreduced, the
/// restore reduces it once with room for it, and B keeps running beside it.
fn killed_after_a_failed_reduce(seed: u64) -> Result<u64, Box<dyn std::error::Error>> {
    let path = journal_file("reducefail", seed)?;
    let case = drive_to_failure(seed, durable_with(&path, ceiling(seed)?)?)?;
    let Case {
        runtime,
        first,
        b_ids,
        ..
    } = case;
    drop(runtime);

    let mut restored = durable(ScriptedOwner::new(), &path)?;
    reduces_a_once(seed, &mut restored, first.id(), b_ids)?;

    // Tenant B's next command is reduced beside A's, unaffected by its failure.
    let (late, _) = work(seed, 1, 5)?;
    let late_id = late.id();
    restored.ack(late)?;
    assert_eq!(
        restored.tick()?.admitted(),
        &[late_id],
        "seed {seed}: B goes on"
    );
    let hash = trace_hash(restored.journal());
    drop(restored);

    let reopened = durable(ScriptedOwner::new(), &path)?;
    assert!(
        pending(&reopened).is_empty(),
        "seed {seed}: nothing left pending"
    );
    assert_eq!(trace_hash(reopened.journal()), hash, "seed {seed}: replay");
    drop(reopened);
    std::fs::remove_file(&path)?;
    Ok(hash)
}

/// larry#219: a kill after the failed reduce loses nothing and doubles
/// nothing. Each seed runs twice and must give one trace hash; the seeds
/// differ from each other.
#[test]
fn sim_a_reduce_failure_survives_restore_and_reduces_once() -> Outcome {
    let mut seen = BTreeSet::new();
    for seed in 0..16_u64 {
        let first = killed_after_a_failed_reduce(seed)?;
        assert_eq!(first, killed_after_a_failed_reduce(seed)?, "seed {seed}");
        seen.insert(first);
    }
    assert_eq!(seen.len(), 16, "every seed leaves its own trace");
    Ok(())
}
