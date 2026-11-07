//! Falsifiers for E71: revocation's boundary, the offline exposure bound and
//! the treatment of an effect that had already been dispatched (R87, D48).
//!
//! Every assertion inspects a real value — a concrete refusal, a concrete call
//! list at the owner boundary, a concrete effect phase — never `is_err()`
//! alone. The owner is the scripted fixture double; nothing here reaches Moo,
//! Sentinel, Nova or a network.

mod fence_kit;
mod support;

use std::time::Duration;

use larry_contracts::authority::{Scope, SecurityEpoch};
use larry_contracts::effect::{EffectOutcome, ReconciliationStrategy};
use larry_contracts::event::EventKind;
use larry_contracts::state::RunState;
use larry_runtime::FenceRefusal;
use larry_runtime::journal::JournalEntry;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::projection::{EffectPhase, ProjectionBackend};
use larry_runtime::runtime::{Runtime, restore};

use fence_kit::{
    open_retry_safe_effect, runtime_at, settings_at, settings_that_outlive_a_year, work, work_in,
};
use support::{
    TestRuntime, clock, command_id, epoch, operation, proposal, run, runtime, submit, tenant,
};

/// Revoking an operation while its command is still queued stops it reaching
/// the owner at all.
#[test]
fn e71_revocation_before_admission_fences_the_effect_at_the_boundary()
-> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime()?;
    // Queued, not yet admitted: the command is in the ready queue and the
    // journal knows nothing about it.
    rt.submit(work(1, operation(1), ReconciliationStrategy::Queryable)?)?;
    assert_eq!(rt.journal().len(), 0);
    assert!(rt.fence_mut().revoke(operation(1)));

    let tick = rt.tick()?;

    assert_eq!(tick.admitted(), &[command_id(1)]);
    assert_eq!(
        tick.fenced(),
        &[(
            operation(1),
            FenceRefusal::Revoked {
                operation: operation(1)
            }
        )]
    );
    assert_eq!(
        rt.projection().effect_phase(operation(1)),
        Some(EffectPhase::Reserved),
        "the durable intent is recorded; `Reserved` is only ever produced by \
         folding a recorded intent"
    );
    assert_eq!(rt.owner().dispatched(), &[], "the owner was never called");
    assert_eq!(rt.owner().reconciled(), &[]);
    assert!(tick.outcomes().is_empty(), "no outcome was invented");
    assert!(
        tick.receipts().is_empty(),
        "a fenced effect is not a failure"
    );
    assert_eq!(
        rt.projection().run_state(run(1)),
        Some(RunState::Reconciling),
        "a refused reach is not a failed run; the effect simply did not happen"
    );
    Ok(())
}

/// The ordering the check depends on: the fence sits *after* the durable
/// reservation and *before* the owner call, so the window D48 names is the
/// window this closes.
#[test]
fn e71_the_fence_sits_between_the_durable_intent_and_the_owner_call()
-> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime()?;
    rt.submit(work(1, operation(1), ReconciliationStrategy::Queryable)?)?;
    assert!(rt.fence_mut().revoke(operation(1)));
    let tick = rt.tick()?;

    let intents_recorded = rt
        .journal()
        .entries()
        .iter()
        .filter(|entry| match **entry {
            JournalEntry::Event(ref event) => event.kind() == EventKind::EffectIntentRecorded,
            _ => false,
        })
        .count();
    assert_eq!(
        intents_recorded, 1,
        "the intent must be durable before the fence is consulted"
    );
    assert!(
        rt.owner().dispatched().is_empty(),
        "and before the owner call"
    );
    assert_eq!(tick.fenced().len(), 1);
    assert!(tick.outcomes().is_empty());
    Ok(())
}

/// A refused reach is a non-event: the tick writes no journal entry, dispatches
/// nothing and publishes nothing, and reports the refusal on every tick rather
/// than once, so a stuck effect is never inferred from an absence.
#[test]
fn e71_a_fenced_retry_is_a_non_event_reported_every_tick() -> Result<(), Box<dyn std::error::Error>>
{
    let (mut rt, op) = open_retry_safe_effect(64)?;
    let before = rt.journal().len();
    assert!(rt.fence_mut().revoke(op));

    let tick = rt.tick()?;
    assert_eq!(
        tick.fenced(),
        &[(op, FenceRefusal::Revoked { operation: op })]
    );
    assert!(tick.is_idle(), "a refusal changes nothing");
    assert_eq!(rt.journal().len(), before, "and writes nothing");
    assert_eq!(rt.owner().dispatched(), &[op], "only the original dispatch");

    let again = rt.tick()?;
    assert_eq!(
        again.fenced(),
        &[(op, FenceRefusal::Revoked { operation: op })]
    );
    assert_eq!(rt.journal().len(), before);
    assert_eq!(rt.owner().dispatched(), &[op]);
    Ok(())
}

/// The offline exposure bound is spent by mutating reaches and recharged only
/// by authenticating against the authority source (E71 partition).
#[test]
fn e71_a_partition_spends_the_offline_bound_and_authentication_recharges_it()
-> Result<(), Box<dyn std::error::Error>> {
    let mut rt: TestRuntime = Runtime::new(
        settings_at(SecurityEpoch::INITIAL, 1)?,
        ScriptedOwner::new(),
        clock()?,
    )?;

    rt.submit(work(1, operation(1), ReconciliationStrategy::Queryable)?)?;
    let first = rt.tick()?;
    assert_eq!(first.authorized(), &[operation(1)]);
    assert_eq!(rt.fence().offline_spent(), 1);

    // The authority source is unreachable, so the next reach is refused. The
    // probe is a fresh command in its own run, because a command is admitted
    // exactly once and a second `Submit` into a reconciling run is refused by
    // the run-state algebra.
    rt.submit(work_in(
        2,
        2,
        operation(2),
        ReconciliationStrategy::IdempotentRetrySafe,
    )?)?;
    let refused = rt.tick()?;
    assert_eq!(
        refused.fenced(),
        &[(
            operation(2),
            FenceRefusal::OfflineExposureExhausted { bound: 1 }
        )]
    );
    assert_eq!(rt.owner().dispatched(), &[operation(1)]);

    // Reaching the authority source recharges it, and the reserved effect goes
    // out on its declared retry path.
    assert_eq!(
        rt.fence_mut().authenticate(SecurityEpoch::INITIAL),
        SecurityEpoch::INITIAL
    );
    assert_eq!(rt.fence().offline_spent(), 0);
    let third = rt.tick()?;
    assert!(third.fenced().is_empty());
    assert_eq!(
        rt.owner().dispatched(),
        &[operation(1), operation(2)],
        "a retry-safe effect reserved while fenced is dispatched once the fence opens"
    );
    Ok(())
}

/// One year, the span both halves of the sleep-and-rollback falsifier move time by.
const YEAR: Duration = Duration::from_secs(365 * 24 * 3_600);

/// What a fresh probe meets once a slept history is restored under a clock placed at `placed`:
/// the fence's refusals, the reaches the owner saw and the offline allowance spent.
type Probe = (
    Vec<(larry_contracts::ids::OperationId, FenceRefusal)>,
    Vec<larry_contracts::ids::OperationId>,
    u32,
);

/// Spends the offline bound, sleeps a year (asserting the next probe is still refused), then
/// restores that history under a clock placed at `placed` and submits one more probe.
fn probe_after_sleep_restored_at(placed: Duration) -> Result<Probe, Box<dyn std::error::Error>> {
    let held = clock()?;
    let mut rt: TestRuntime = Runtime::new(
        settings_that_outlive_a_year(SecurityEpoch::INITIAL, 1)?,
        ScriptedOwner::new(),
        held.clone(),
    )?;
    rt.submit(work(1, operation(1), ReconciliationStrategy::Queryable)?)?;
    rt.tick()?;
    assert_eq!(
        rt.fence().offline_spent(),
        1,
        "the one reach spent the bound"
    );

    // Sleep: a year passes with no authority contact, and a fresh probe is still refused.
    held.advance(YEAR)?;
    rt.submit(work_in(
        2,
        2,
        operation(2),
        ReconciliationStrategy::Queryable,
    )?)?;
    let after_sleep = rt.tick()?;
    assert_eq!(
        after_sleep.fenced(),
        &[(
            operation(2),
            FenceRefusal::OfflineExposureExhausted { bound: 1 }
        )],
        "a year of sleep recharges nothing"
    );
    assert_eq!(
        rt.owner().dispatched(),
        &[operation(1)],
        "only the first reach went out"
    );

    let mut restored: TestRuntime = restore(
        settings_that_outlive_a_year(SecurityEpoch::INITIAL, 1)?,
        ScriptedOwner::new(),
        lgwks_bot::clock::Clock::virtual_at(placed),
        rt.into_journal(),
    )?;
    restored.submit(work_in(
        3,
        3,
        operation(3),
        ReconciliationStrategy::Queryable,
    )?)?;
    let tick = restored.tick()?;
    Ok((
        tick.fenced().to_vec(),
        restored.owner().dispatched().to_vec(),
        restored.fence().offline_spent(),
    ))
}

/// Neither elapsed time nor a rolled-back clock recharges offline authority. The fence holds
/// no clock, so "sleep" and "rollback" are the same non-event. The estate clock never reads
/// earlier within a process (larry#74), so a rollback reaches a runtime only across a restart:
/// the same history restored under a clock placed a year before the epoch meets exactly the
/// fence it meets restored at the epoch.
#[test]
fn e71_neither_sleeping_nor_a_rolled_back_clock_recharges_offline_authority()
-> Result<(), Box<dyn std::error::Error>> {
    let origin = epoch()?.duration_since(std::time::SystemTime::UNIX_EPOCH)?;
    let rolled_back = origin.checked_sub(YEAR).ok_or("fixture time underflow")?;
    let at_origin = probe_after_sleep_restored_at(origin)?;
    let at_rollback = probe_after_sleep_restored_at(rolled_back)?;
    assert_eq!(
        at_rollback, at_origin,
        "time is not evidence of freshness in either direction"
    );
    Ok(())
}

/// A replayed older authority snapshot is absorbed: the fence never moves
/// backwards, so stale evidence cannot re-authorize anything (E71).
#[test]
fn e71_a_replayed_older_snapshot_never_lowers_the_fence() -> Result<(), Box<dyn std::error::Error>>
{
    let mut rt = runtime()?;
    assert_eq!(rt.fence().epoch(), SecurityEpoch::INITIAL);
    assert_eq!(rt.fence_mut().advance_epoch()?, SecurityEpoch::new(1));
    // The snapshot the attacker holds says generation zero.
    assert_eq!(
        rt.fence_mut().authenticate(SecurityEpoch::INITIAL),
        SecurityEpoch::new(1)
    );
    assert_eq!(rt.fence().epoch(), SecurityEpoch::new(1));
    // A genuine advance still moves in the other direction.
    assert_eq!(
        rt.fence_mut().authenticate(SecurityEpoch::new(4)),
        SecurityEpoch::new(4)
    );
    assert_eq!(rt.fence().epoch(), SecurityEpoch::new(4));
    Ok(())
}

/// Revocation stops new external change; it does not stop the runtime from
/// finding out what already happened. This is the "already-dispatched
/// treatment" D48 requires, and the two effects differ only in that one would
/// re-dispatch while the other only queries.
#[test]
fn e71_an_advanced_epoch_fences_the_retry_but_not_the_status_query()
-> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime()?;
    // Run 1: a retry-safe effect, whose retry would cause a second change.
    let retry_body = proposal(
        run(1),
        tenant(1),
        operation(1),
        &[Scope::FileRead],
        5,
        ReconciliationStrategy::IdempotentRetrySafe,
    );
    rt.submit(submit(command_id(1), &retry_body)?)?;
    // Run 2: a queryable effect, whose reconciliation only observes.
    let query_body = proposal(
        run(2),
        tenant(2),
        operation(2),
        &[Scope::FileRead],
        5,
        ReconciliationStrategy::Queryable,
    );
    rt.submit(submit(command_id(2), &query_body)?)?;
    let first = rt.tick()?;
    assert_eq!(first.authorized(), &[operation(1), operation(2)]);
    assert_eq!(rt.owner().dispatched(), &[operation(1), operation(2)]);
    assert!(rt.owner().reconciled().is_empty());

    // Revocation lands after both effects are already out.
    assert_eq!(rt.fence_mut().advance_epoch()?, SecurityEpoch::new(1));
    let second = rt.tick()?;

    assert_eq!(
        second.fenced(),
        &[(
            operation(1),
            FenceRefusal::StaleEpoch {
                admitted: SecurityEpoch::INITIAL,
                current: SecurityEpoch::new(1)
            }
        )],
        "only the re-dispatch is refused"
    );
    assert_eq!(
        rt.owner().dispatched(),
        &[operation(1), operation(2)],
        "no second change was caused"
    );
    assert_eq!(
        rt.owner().reconciled(),
        &[operation(2)],
        "the status query still went out, so the effect can still be resolved"
    );
    assert_eq!(second.reconciled(), &[operation(2)]);
    // The refused effect stays open rather than acquiring a disposition.
    assert_eq!(
        rt.projection().effect_phase(operation(1)),
        Some(EffectPhase::Observed {
            outcome: EffectOutcome::Unknown {
                operation: operation(1)
            }
        })
    );
    assert!(rt.projection().receipts().is_empty());
    Ok(())
}

/// The admission epoch is read from the fence, never from the command, so a
/// hostile generator cannot name a generation that outlives a revocation
/// (D42, E63).
#[test]
fn e71_a_proposal_cannot_declare_its_own_admission_epoch() -> Result<(), Box<dyn std::error::Error>>
{
    // The same proposal bytes, admitted under two different fences, record two
    // different epochs. Nothing in the command chose either.
    let body = proposal(
        run(1),
        tenant(1),
        operation(1),
        &[Scope::FileRead],
        5,
        ReconciliationStrategy::Queryable,
    );
    let mut early = runtime()?;
    early.submit(submit(command_id(1), &body)?)?;
    early.tick()?;
    let mut late = runtime_at(SecurityEpoch::new(7), 64, ScriptedOwner::new())?;
    late.submit(submit(command_id(1), &body)?)?;
    late.tick()?;

    let recorded = |rt: &TestRuntime| -> Option<SecurityEpoch> {
        rt.projection()
            .intents()
            .first()
            .map(|intent| intent.admission_epoch)
    };
    assert_eq!(recorded(&early), Some(SecurityEpoch::INITIAL));
    assert_eq!(recorded(&late), Some(SecurityEpoch::new(7)));
    Ok(())
}
