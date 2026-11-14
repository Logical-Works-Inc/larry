//! larry#219 (ADR 0041): a command whose acceptance record failed to reach the
//! journal is journaled and unreduced, so it is pending, and a retry or a
//! restore reduces it once. The failure is a journal that has room for the
//! command and none for its `CommandAccepted` fact, with the ceiling set from a
//! seeded count of earlier commands, so no test seam is needed.

use std::collections::BTreeSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::iter;
use std::path::Path;

use larry_contracts::command::{Command, CommandAcceptance, CommandKind};
use larry_contracts::event::EventKind;
use larry_contracts::ids::{CommandId, OperationId, RunId, TenantId};
use larry_contracts::state::RunState;

use super::{
    Ack, Outcome, Rt, accepted, clock, command, durable, idle_body, ids, journal_file, memory,
    offer, pending, settings, work_body,
};
use crate::error::RuntimeError;
use crate::journal::{Journal, JournalEntry};
use crate::owner::fixture::ScriptedOwner;
use crate::payload::CommandAcceptedPayload;
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

/// Where `journal` records the acceptance of `id`.
fn acceptance_index(journal: &Journal, id: CommandId) -> Result<usize, Box<dyn std::error::Error>> {
    for (index, entry) in journal.entries().iter().enumerate() {
        if let JournalEntry::Event(ref event) = *entry
            && event.kind() == EventKind::CommandAccepted
        {
            let payload: CommandAcceptedPayload = lgwks_std::json::from_slice(event.payload())?;
            if payload.command == id {
                return Ok(index);
            }
        }
    }
    Err("the command was never accepted".into())
}

/// The pending line: B's commands ahead of A's, A's, and B's behind it.
struct Line {
    before: Vec<Command>,
    before_runs: Vec<RunId>,
    first: Command,
    first_run: RunId,
    after: Vec<Command>,
}

impl Line {
    /// One to three of B's commands ahead of A's and none to two behind it.
    fn of(seed: u64) -> Result<Self, Box<dyn std::error::Error>> {
        let ahead = u8::try_from(seed % 3)?.saturating_add(1);
        let behind = u8::try_from((seed >> 2) % 3)?;
        let (mut before, mut before_runs, mut after) = (Vec::new(), Vec::new(), Vec::new());
        for index in 0..ahead {
            let (item, run) = work(seed, 1, index)?;
            before.push(item);
            before_runs.push(run);
        }
        for index in 0..behind {
            after.push(work(seed, 1, ahead.saturating_add(index))?.0);
        }
        let (first, first_run) = work(seed, 0, 0)?;
        Ok(Self {
            before,
            before_runs,
            first,
            first_run,
            after,
        })
    }

    /// Every command in the order it is admitted.
    fn in_order(&self) -> Vec<Command> {
        let ahead = self.before.iter().cloned();
        ahead
            .chain(iter::once(self.first.clone()))
            .chain(self.after.iter().cloned())
            .collect()
    }
}

/// The ids of `items`, in order.
fn ids_of(items: &[Command]) -> Vec<CommandId> {
    items.iter().map(Command::id).collect()
}

/// One seed of the interleaving: A's acceptance record fails with B's commands
/// ahead of and behind it, and a seeded monkey then retries any of them, floods
/// A's retry and hijacks A's command id as B, before a restore with room.
fn interleaved(seed: u64) -> Result<u64, Box<dyn std::error::Error>> {
    let line = Line::of(seed)?;
    let order = line.in_order();
    let first_id = line.first.id();
    let before_ids = ids_of(&line.before);
    let mut want_pending = vec![first_id];
    want_pending.extend(ids_of(&line.after));

    // The reference run, with room for everything, says where A's acceptance
    // lands; a journal that ends exactly there has no room for it.
    let mut probe = memory(ScriptedOwner::new())?;
    for item in &order {
        probe.ack(item.clone())?;
    }
    probe.tick()?;
    let capacity = acceptance_index(probe.journal(), first_id)?;
    let mut limited = settings()?;
    limited.journal_capacity = capacity;
    let mut runtime: Rt = Runtime::new(limited, ScriptedOwner::new(), clock())?;
    for item in &order {
        runtime.ack(item.clone())?;
    }
    let failed = runtime.tick();
    assert!(
        matches!(failed, Err(RuntimeError::JournalFull { capacity: held }) if held == capacity),
        "seed {seed}: A's acceptance has no room: {failed:?}"
    );
    let unreduced: Vec<CommandId> = super::super::recover(runtime.journal())?
        .iter()
        .map(|held| held.command.id())
        .collect();
    assert_eq!(unreduced, want_pending, "seed {seed}: the journal's fold");

    let candidates: Vec<&Command> = line
        .before
        .iter()
        .chain(iter::once(&line.first))
        .chain(line.after.iter())
        .collect();
    let mut state = seed ^ 0x9e37_79b9_7f4a_7c15;
    for step in 0..12_u32 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let slot = usize::try_from(state >> 40)?.checked_rem(candidates.len());
        let chosen = slot
            .and_then(|slot| candidates.get(slot))
            .ok_or("no command to retry")?;
        match (state >> 33) % 3 {
            0 => {
                let (acceptance, _) =
                    runtime.submit_protocol(&offer(chosen.tenant()), (*chosen).clone(), None)?;
                assert_eq!(
                    acceptance,
                    CommandAcceptance::DuplicateReplay,
                    "seed {seed} step {step}: reduced or held, so the retry is a duplicate"
                );
            }
            1 => {
                runtime.submit(line.first.clone())?;
                let retry = runtime.tick();
                assert!(
                    matches!(retry, Err(RuntimeError::JournalFull { .. })),
                    "seed {seed} step {step}: A's retry is refused typed: {retry:?}"
                );
            }
            _ => {
                let body = work_body(
                    line.first_run,
                    chosen.tenant(),
                    OperationId::from_bytes(ids(0x30, seed)),
                );
                let hijack = command(first_id, CommandKind::Submit, &body)?;
                let refused = runtime.submit_protocol(&offer(chosen.tenant()), hijack, None);
                let own = chosen.tenant() == line.first.tenant();
                assert!(
                    own || matches!(refused, Err(RuntimeError::Command(_))),
                    "seed {seed} step {step}: another tenant cannot take A's id: {refused:?}"
                );
            }
        }
        assert_eq!(pending(&runtime), want_pending, "seed {seed} step {step}");
        assert_eq!(runtime.journal().len(), capacity, "seed {seed}: no growth");
        assert_eq!(accepted(runtime.journal())?, before_ids, "seed {seed}");
    }

    let mut restored: Rt = restore(
        settings()?,
        ScriptedOwner::new(),
        clock(),
        runtime.into_journal(),
    )?;
    assert_eq!(pending(&restored), want_pending, "seed {seed}: restored");
    assert_eq!(
        restored.tick()?.admitted(),
        want_pending.as_slice(),
        "seed {seed}: A, then B's behind it, each reduced"
    );
    assert!(restored.tick()?.admitted().is_empty(), "seed {seed}: once");
    let mut want_accepted = before_ids;
    want_accepted.extend(&want_pending);
    assert_eq!(accepted(restored.journal())?, want_accepted, "seed {seed}");
    let tenant_b = line.before.first().ok_or("B ran nothing")?.tenant();
    for run in &line.before_runs {
        let owner = restored.projection().run_tenant(*run);
        assert_eq!(owner, Some(tenant_b), "seed {seed}: B's run stays B's");
    }
    let owner = restored.projection().run_tenant(line.first_run);
    assert_eq!(owner, Some(line.first.tenant()), "seed {seed}: A's run");
    for item in &order {
        restored.submit(item.clone())?;
    }
    assert_eq!(
        restored.tick()?.duplicates(),
        ids_of(&order).as_slice(),
        "seed {seed}: every retry is now a duplicate"
    );
    Ok(trace_hash(restored.journal()))
}

/// larry#219, the neighbour and the double-clicker: tenant A's reduce fails
/// with B's commands pending ahead of and behind it; B and A retry in a seeded
/// order, a second tenant hijacks A's command id, and A's queued retry is
/// refused typed. Nothing is dropped, doubled or crossed, the pending order is
/// the journal's, and one seed gives one trace hash.
#[test]
fn sim_a_reduce_failure_interleaved_with_a_second_tenants_retries_never_crosses_tenants() -> Outcome
{
    let mut seen = BTreeSet::new();
    for seed in 0..24_u64 {
        let first = interleaved(seed)?;
        assert_eq!(
            first,
            interleaved(seed)?,
            "seed {seed}: same seed, same trace"
        );
        seen.insert(first);
    }
    assert_eq!(seen.len(), 24, "every seed leaves its own trace");
    Ok(())
}
