//! Falsifiers for E71: revocation's boundary, the offline exposure bound and
//! the treatment of an effect that had already been dispatched (R87, D48).
//!
//! Every assertion inspects a real value — a concrete refusal, a concrete call
//! list at the owner boundary, a concrete effect phase — never `is_err()`
//! alone. The owner is the scripted fixture double; nothing here reaches Moo,
//! Sentinel, Nova or a network.

mod support;

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use rocco_contracts::authority::{Scope, SecurityEpoch};
use rocco_contracts::command::{Command, CommandContent, CommandKind};
use rocco_contracts::effect::{EffectOutcome, MAX_EFFECT_ATTEMPTS, ReconciliationStrategy};
use rocco_contracts::event::EventKind;
use rocco_contracts::ids::{OperationId, RunId, TenantId};
use rocco_contracts::replay::ValiditySnapshot;
use rocco_contracts::state::RunState;
use rocco_runtime::clock::{Clock, FixedClock};
use rocco_runtime::journal::JournalEntry;
use rocco_runtime::owner::fixture::ScriptedOwner;
use rocco_runtime::projection::{EffectPhase, FoldProjection, ProjectionBackend};
use rocco_runtime::runtime::{Runtime, RuntimeSettings, restore};
use rocco_runtime::{FenceError, FenceRefusal, RevocationFence};

use support::{
    TestRuntime, assert_state, command_id, epoch, operation, proposal, run, runtime_with, settings,
    submit, succeeded, tenant,
};

/// A clock the test moves by hand, so that "time passed" and "the clock ran
/// backwards" are asserted as input changes instead of waited for.
#[derive(Clone)]
struct ManualClock(Rc<Cell<SystemTime>>);

impl ManualClock {
    fn new(at: SystemTime) -> Self {
        Self(Rc::new(Cell::new(at)))
    }

    fn set(&self, at: SystemTime) {
        self.0.set(at);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> SystemTime {
        self.0.get()
    }
}

/// The runtime under test with a clock the test controls.
type ManualRuntime = Runtime<FoldProjection, ScriptedOwner, ManualClock>;

/// Settings whose fence permits `bound` mutating reaches between
/// authentications and stands at `at_epoch`.
fn settings_at(
    at_epoch: SecurityEpoch,
    bound: u32,
) -> Result<RuntimeSettings, Box<dyn std::error::Error>> {
    let mut base = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
    base.security_epoch = at_epoch;
    base.offline_dispatch_bound = bound;
    Ok(base)
}

/// [`settings_at`] with authority validity far enough out that the E30 expiry
/// check cannot confound a probe that deliberately moves the clock by a year.
fn settings_that_outlive_a_year(
    at_epoch: SecurityEpoch,
    bound: u32,
) -> Result<RuntimeSettings, Box<dyn std::error::Error>> {
    let mut base = settings_at(at_epoch, bound)?;
    base.validity = ValiditySnapshot::new(
        epoch()?
            .checked_add(Duration::from_secs(10 * 365 * 24 * 3_600))
            .ok_or("fixture validity overflow")?,
    );
    Ok(base)
}

/// A `Submit` for one file-read effect, in `run_byte`'s run.
///
/// Each run admits one effect and walks its own lifecycle, so a test that
/// needs several effects open at once gives each its own run.
///
/// # Errors
///
/// Returns the contract's ordering error for an over-long causal-parent list,
/// which cannot happen here but is not silently discarded.
fn work_in(
    run_byte: u8,
    id: u8,
    op: OperationId,
    strategy: ReconciliationStrategy,
) -> Result<Command, Box<dyn std::error::Error>> {
    let body = proposal(
        run(run_byte),
        tenant(run_byte),
        op,
        &[Scope::FileRead],
        5,
        strategy,
    );
    submit(command_id(id), &body)
}

/// [`work_in`] for run 1.
///
/// # Errors
///
/// As [`work_in`].
fn work(
    id: u8,
    op: OperationId,
    strategy: ReconciliationStrategy,
) -> Result<Command, Box<dyn std::error::Error>> {
    work_in(1, id, op, strategy)
}

/// A retry-safe effect with an ambiguous outcome stays open, which is the state
/// the retry falsifiers below start from.
///
/// Returns the runtime and the journal-independent fact that the effect is
/// open, so a caller cannot accidentally assert on a resolved effect.
fn open_retry_safe_effect(
    bound: u32,
) -> Result<(TestRuntime, OperationId), Box<dyn std::error::Error>> {
    let mut rt: TestRuntime = Runtime::new(
        settings_at(SecurityEpoch::INITIAL, bound)?,
        ScriptedOwner::new(),
        FixedClock::new(epoch()?),
    )?;
    rt.submit(work(
        1,
        operation(1),
        ReconciliationStrategy::IdempotentRetrySafe,
    )?)?;
    let tick = rt.tick()?;
    assert_eq!(
        tick.authorized(),
        &[operation(1)],
        "the fixture must reserve the intended operation"
    );
    assert_eq!(
        rt.projection().effect_phase(operation(1)),
        Some(EffectPhase::Observed {
            outcome: EffectOutcome::Unknown {
                operation: operation(1)
            }
        }),
        "an ambiguous outcome under a retry-safe strategy must stay open"
    );
    Ok((rt, operation(1)))
}

/// Revoking an operation while its command is still queued stops it reaching
/// the owner at all.
#[test]
fn e71_revocation_before_admission_fences_the_effect_at_the_boundary()
-> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime_with(
        ScriptedOwner::new(),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
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
    let mut rt = runtime_with(
        ScriptedOwner::new(),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
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
    let mut rt = ManualRuntime::new(
        settings_at(SecurityEpoch::INITIAL, 1)?,
        ScriptedOwner::new(),
        ManualClock::new(epoch()?),
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

/// Neither elapsed time nor a reversed clock recharges offline authority. The
/// fence holds no clock, so "sleep" and "rollback" are the same non-event.
#[test]
fn e71_neither_sleeping_nor_a_rolled_back_clock_recharges_offline_authority()
-> Result<(), Box<dyn std::error::Error>> {
    let clock = ManualClock::new(epoch()?);
    let mut rt = ManualRuntime::new(
        settings_that_outlive_a_year(SecurityEpoch::INITIAL, 1)?,
        ScriptedOwner::new(),
        clock.clone(),
    )?;
    rt.submit(work(1, operation(1), ReconciliationStrategy::Queryable)?)?;
    rt.tick()?;
    assert_eq!(rt.fence().offline_spent(), 1);

    // Sleep: a year of wall time passes with no authority contact, and a fresh
    // probe is still refused.
    clock.set(
        epoch()?
            .checked_add(Duration::from_secs(365 * 24 * 3_600))
            .ok_or("fixture time overflow")?,
    );
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
        )]
    );

    // Rollback: the clock runs backwards past where it started, and a second
    // fresh probe is refused just the same.
    clock.set(
        epoch()?
            .checked_sub(Duration::from_secs(365 * 24 * 3_600))
            .ok_or("fixture time underflow")?,
    );
    rt.submit(work_in(
        3,
        3,
        operation(3),
        ReconciliationStrategy::Queryable,
    )?)?;
    let after_rollback = rt.tick()?;
    assert_eq!(
        after_rollback.fenced(),
        &[(
            operation(3),
            FenceRefusal::OfflineExposureExhausted { bound: 1 }
        )]
    );
    assert_eq!(
        rt.owner().dispatched(),
        &[operation(1)],
        "time is not evidence of freshness in either direction"
    );
    Ok(())
}

/// A replayed older authority snapshot is absorbed: the fence never moves
/// backwards, so stale evidence cannot re-authorize anything (E71).
#[test]
fn e71_a_replayed_older_snapshot_never_lowers_the_fence() -> Result<(), Box<dyn std::error::Error>>
{
    let mut rt = runtime_with(
        ScriptedOwner::new(),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
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
    let mut rt = runtime_with(
        ScriptedOwner::new(),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
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
    let mut early = runtime_with(
        ScriptedOwner::new(),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    early.submit(submit(command_id(1), &body)?)?;
    early.tick()?;
    let mut late: TestRuntime = Runtime::new(
        settings_at(SecurityEpoch::new(7), 64)?,
        ScriptedOwner::new(),
        FixedClock::new(epoch()?),
    )?;
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

/// The tenant a run belongs to is the one the runtime recorded for that run,
/// never the one the proposal body declares, so a hostile generator cannot
/// record an effect — or re-attribute a run after restore — to a tenant it
/// does not own (D42, E63).
///
/// This is the same rule as the admission epoch below: the body's `tenant`
/// field is a declaration like any other, and the runtime consults its own
/// run table instead. The envelope tenant and the body's claim are different
/// here on purpose, which is the only way the two sources can be told apart.
#[test]
fn e63_a_proposal_cannot_declare_its_own_tenant() -> Result<(), Box<dyn std::error::Error>> {
    // The envelope says tenant 1. The body claims tenant 2.
    let body = proposal(
        run(1),
        tenant(2),
        operation(1),
        &[Scope::FileRead],
        5,
        ReconciliationStrategy::Queryable,
    );
    let payload = lgwks_std::json::to_vec(&body)?;
    let envelope = Command::new(
        command_id(1),
        tenant(1),
        Vec::new(),
        CommandContent::new(CommandKind::Submit, payload.into_boxed_slice()),
    )?;

    let mut rt = runtime_with(
        ScriptedOwner::scripted([succeeded(operation(1))]),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    rt.submit(envelope)?;
    let tick = rt.tick()?;
    assert_eq!(tick.authorized(), &[operation(1)]);

    assert_eq!(
        rt.projection()
            .intents()
            .first()
            .map(|intent| intent.tenant),
        Some(tenant(1)),
        "the durable intent carries the run's recorded tenant, not the body's claim"
    );
    let receipts = rt.projection().receipts();
    assert_eq!(receipts.len(), 1);
    assert_eq!(
        receipts
            .first()
            .map(rocco_contracts::effect::TerminalReceipt::tenant),
        Some(tenant(1))
    );

    // A restart must not re-attribute the run to the declared tenant: the
    // restored run table is the same table the journal recorded.
    let journal = rt.into_journal();
    let restored: TestRuntime = restore(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        ScriptedOwner::new(),
        FixedClock::new(epoch()?),
        journal,
    )?;
    let receipt = rocco_contracts::effect::TerminalReceipt::publish(
        operation(3),
        run(1),
        restored
            .projection()
            .run_tenant(run(1))
            .ok_or("the restored run must carry a tenant")?,
        rocco_contracts::effect::TerminalDisposition::Succeeded,
    );
    assert_eq!(
        receipt.tenant(),
        tenant(1),
        "the restored run table attributes the run to the tenant the journal recorded"
    );
    Ok(())
}

/// A restored runtime brings each effect's admission epoch back with it, so a
/// host whose authority has moved on recovers with those effects already fenced
/// instead of re-dispatching them on a decision that no longer stands
/// (D48, E71). Restoring at the *same* generation is the differential: the
/// epoch fences what it withdrew, not everything.
#[test]
fn e71_restore_fences_only_the_effects_the_new_generation_left_behind()
-> Result<(), Box<dyn std::error::Error>> {
    let reopened = |at_epoch: SecurityEpoch| -> Result<TestRuntime, Box<dyn std::error::Error>> {
        let (original, _) = open_retry_safe_effect(64)?;
        Ok(restore(
            settings_at(at_epoch, 64)?,
            ScriptedOwner::new(),
            FixedClock::new(epoch()?),
            original.into_journal(),
        )?)
    };

    let mut advanced = reopened(SecurityEpoch::new(1))?;
    assert_eq!(advanced.fence().epoch(), SecurityEpoch::new(1));
    let tick = advanced.tick()?;
    assert_eq!(
        tick.fenced(),
        &[(
            operation(1),
            FenceRefusal::StaleEpoch {
                admitted: SecurityEpoch::INITIAL,
                current: SecurityEpoch::new(1)
            }
        )],
        "the recovered effect is fenced, not re-dispatched"
    );
    assert_eq!(advanced.owner().dispatched(), &[]);

    let mut same = reopened(SecurityEpoch::INITIAL)?;
    let unrestored_tick = same.tick()?;
    assert!(
        unrestored_tick.fenced().is_empty(),
        "the same generation does not fence an effect admitted under it"
    );
    assert_eq!(
        same.owner().dispatched(),
        &[operation(1)],
        "the retry proceeds, because nothing withdrew its authority"
    );
    Ok(())
}

/// A fenced effect has already reserved its budget, and this slice releases
/// nothing. The direction is deliberate — a reservation that is not released
/// fails closed — and it is pinned here so a later work package has to change
/// this test to change the behaviour.
///
/// It also pins the consequence that follows from the closed event vocabulary:
/// the journal cannot record "reserved but never dispatched", so a fenced
/// first dispatch is *indistinguishable from a crash in the dispatch window*
/// and the runtime treats it as such — it queries, and never re-dispatches an
/// effect whose strategy is not retry-safe.
#[test]
fn e71_a_fenced_effect_keeps_its_reservation_and_that_is_a_known_limit()
-> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime_with(
        ScriptedOwner::new(),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    assert_eq!(rt.remaining_budget(), 100);
    rt.submit(work(1, operation(1), ReconciliationStrategy::Queryable)?)?;
    assert!(rt.fence_mut().revoke(operation(1)));
    let tick = rt.tick()?;

    assert_eq!(tick.fenced().len(), 1);
    assert_eq!(
        rt.remaining_budget(),
        95,
        "budget is reserved at admission, before the fence runs, so a fenced \
         effect holds its reservation; releasing it would need a journal fact \
         the closed vocabulary does not have"
    );
    assert_eq!(
        rt.projection().effect_phase(operation(1)),
        Some(EffectPhase::Reserved),
        "the reservation is conservative, not lost: the intent is durable"
    );

    // The ambiguity is resolved the only way the vocabulary allows: ask. A
    // query cannot cause the change the fence refused, so containment holds
    // whether or not the effect had actually reached the endpoint.
    let next = rt.tick()?;
    assert_eq!(
        next.reconciled(),
        &[operation(1)],
        "the reserved effect is resolved by querying, and the tick says so"
    );
    assert!(
        next.fenced().is_empty(),
        "an observation is not a reach the fence gates"
    );
    assert_eq!(
        rt.owner().reconciled(),
        &[operation(1)],
        "the reserved effect is queried, not re-dispatched"
    );
    assert_eq!(
        rt.owner().dispatched(),
        &[],
        "the revoked reach is never taken"
    );
    assert_eq!(
        rt.projection().effect_phase(operation(1)),
        Some(EffectPhase::Observed {
            outcome: EffectOutcome::Unknown {
                operation: operation(1)
            }
        })
    );
    Ok(())
}

/// An endpoint's answer is evidence about an effect this runtime actually
/// sent, and about nothing else. An effect whose only reach was refused never
/// left the process, so the status answer that follows — `Succeeded` here,
/// which is the worst case — cannot resolve it, cannot publish a receipt and
/// cannot carry the run to `Completed`.
///
/// The journal cannot tell a refused reach from a crash inside the dispatch
/// window, and this test exists because the *runtime* can: a refused reach
/// calls no owner, so the live effect table still reads "never sent", while a
/// restored reservation is restored as possibly-sent and keeps its full
/// treatment. Reading the endpoint's answer as truth here would manufacture
/// success for an effect that provably never happened (D42, R88).
#[test]
fn e71_an_observation_cannot_resolve_an_effect_that_was_never_sent()
-> Result<(), Box<dyn std::error::Error>> {
    // The offline bound is spent, so the only reach this effect ever gets is
    // the refused one: `dispatch` is never called on the owner.
    let mut rt: TestRuntime = Runtime::new(
        settings_at(SecurityEpoch::INITIAL, 0)?,
        // The endpoint answers "succeeded" to the status query.
        ScriptedOwner::scripted([succeeded(operation(1))]),
        FixedClock::new(epoch()?),
    )?;
    rt.submit(work(1, operation(1), ReconciliationStrategy::Queryable)?)?;

    let refused = rt.tick()?;
    assert_eq!(
        refused.fenced(),
        &[(
            operation(1),
            FenceRefusal::OfflineExposureExhausted { bound: 0 }
        )]
    );
    assert_eq!(
        rt.owner().dispatched(),
        &[],
        "the mutating reach was refused, so the effect never left the process"
    );
    assert_eq!(
        rt.projection().effect_phase(operation(1)),
        Some(EffectPhase::Reserved)
    );

    // The next tick queries the endpoint. The query itself is the
    // already-dispatched treatment and stays open; what it may not do is
    // become the effect's disposition.
    let next = rt.tick()?;
    assert_eq!(next.reconciled(), &[operation(1)]);
    assert_eq!(
        rt.owner().reconciled(),
        &[operation(1)],
        "the status query is still sent, so the refusal stays visible"
    );
    assert_eq!(
        rt.owner().dispatched(),
        &[],
        "and the dispatch still never happens"
    );
    assert_eq!(next.receipts(), &[], "no reach earned a receipt");
    assert_eq!(
        next.receipts().len(),
        0,
        "an effect that never left this process is not receipted as a success"
    );
    assert_state(rt.projection(), run(1), RunState::Reconciling)?;
    assert_eq!(
        rt.projection().effect_phase(operation(1)),
        Some(EffectPhase::Observed {
            outcome: succeeded(operation(1))
        }),
        "the answer is recorded as what the owner said, and resolves nothing"
    );

    // It stays open and reported for as long as the reconciliation budget
    // allows, so a host that leaves the fence shut watches the effect stay
    // unresolved rather than lose sight of it.
    for rest in 1..MAX_EFFECT_ATTEMPTS {
        let tick = rt.tick()?;
        assert_eq!(tick.reconciled(), &[operation(1)], "query {rest}");
        assert_eq!(tick.receipts(), &[]);
    }
    assert_eq!(
        rt.owner().reconciled().len(),
        usize::from(MAX_EFFECT_ATTEMPTS),
        "the query budget is spent and bounded, not unbounded"
    );

    // Budget spent: quiet, and still unresolved rather than resolved wrongly.
    let quiet = rt.tick()?;
    assert_eq!(quiet.reconciled(), &[]);
    assert_eq!(quiet.receipts(), &[]);
    assert_eq!(
        rt.projection().receipts().len(),
        0,
        "no tick in the effect's lifetime publishes a receipt for it"
    );
    assert_state(rt.projection(), run(1), RunState::Reconciling)?;
    Ok(())
}

/// A zero bound is a kill switch: no reach is permitted, and authentication
/// does not lift it, because the bound is a ceiling on reaches between
/// authentications and not a credit that authentication hands out.
#[test]
fn e71_a_zero_bound_stops_every_effect_permanently() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt: TestRuntime = Runtime::new(
        settings_at(SecurityEpoch::INITIAL, 0)?,
        ScriptedOwner::new(),
        FixedClock::new(epoch()?),
    )?;
    rt.submit(work(1, operation(1), ReconciliationStrategy::Queryable)?)?;
    let refused = rt.tick()?;
    assert_eq!(
        refused.fenced(),
        &[(
            operation(1),
            FenceRefusal::OfflineExposureExhausted { bound: 0 }
        )]
    );

    assert_eq!(
        rt.fence_mut().authenticate(SecurityEpoch::INITIAL),
        SecurityEpoch::INITIAL
    );
    assert_eq!(
        rt.fence().offline_spent(),
        0,
        "authentication cleared the count"
    );
    rt.submit(work_in(
        2,
        2,
        operation(2),
        ReconciliationStrategy::Queryable,
    )?)?;
    let again = rt.tick()?;
    assert_eq!(
        again.fenced(),
        &[(
            operation(2),
            FenceRefusal::OfflineExposureExhausted { bound: 0 }
        )],
        "and the reach is still refused: zero is a ceiling, not a credit"
    );
    assert_eq!(rt.owner().dispatched(), &[]);
    Ok(())
}

/// The fence can only ever refuse. An effect the reducer denied is denied
/// whether the fence is open or closed, so the two checks cannot be traded
/// against each other — the freshness check is not a second way in.
#[test]
fn the_fence_cannot_authorize_what_the_reducer_denied() -> Result<(), Box<dyn std::error::Error>> {
    // The policy allows nothing, so the gate denies before the fence is
    // consulted at all, even though the fence is wide open.
    let mut rt = runtime_with(ScriptedOwner::new(), &[], &[Scope::FileRead], 100)?;
    rt.submit(work(1, operation(1), ReconciliationStrategy::Queryable)?)?;
    let tick = rt.tick()?;
    assert_eq!(tick.denied().len(), 1);
    assert!(
        tick.fenced().is_empty(),
        "a denied effect never reaches the fence"
    );
    assert_eq!(rt.owner().dispatched(), &[]);
    Ok(())
}

/// A run whose effect was fenced neither silently completes nor silently fails.
#[test]
fn a_fenced_effect_leaves_the_run_visibly_unfinished() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime_with(
        ScriptedOwner::new(),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    rt.submit(work(
        1,
        operation(1),
        ReconciliationStrategy::RequiresHuman,
    )?)?;
    assert!(rt.fence_mut().revoke(operation(1)));
    rt.tick()?;
    let state = rt.projection().run_state(run(1));
    assert_eq!(state, Some(RunState::Reconciling));
    assert!(!state.is_some_and(RunState::is_terminal));
    assert!(rt.projection().receipts().is_empty());
    Ok(())
}

/// Epoch exhaustion refuses rather than wrapping, because a wrapped epoch would
/// alias a withdrawn generation with a live one.
#[test]
fn e71_epoch_exhaustion_refuses_at_the_runtime_boundary() -> Result<(), Box<dyn std::error::Error>>
{
    let mut fence = RevocationFence::new(SecurityEpoch::new(u64::MAX), 64);
    assert_eq!(fence.advance_epoch(), Err(FenceError::EpochExhausted));
    assert_eq!(fence.epoch(), SecurityEpoch::new(u64::MAX));
    Ok(())
}

/// The host surface is the withdrawal and the refresh; the runtime's own
/// mutating operations on the fence are crate-internal, so a caller cannot
/// pre-clear a reach or assert that a dispatch happened.
#[test]
fn the_host_control_surface_is_withdrawal_refresh_and_reading()
-> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime_with(
        ScriptedOwner::new(),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    let fence: &mut RevocationFence = rt.fence_mut();
    assert!(!fence.is_revoked(operation(9)));
    assert!(fence.revoke(operation(9)));
    assert!(fence.is_revoked(operation(9)));
    assert_eq!(fence.offline_bound(), rocco_runtime::MAX_OFFLINE_DISPATCHES);
    assert_eq!(fence.offline_spent(), 0);
    assert_eq!(fence.epoch(), SecurityEpoch::INITIAL);
    assert_eq!(
        fence.authenticate(SecurityEpoch::new(2)),
        SecurityEpoch::new(2)
    );
    assert_eq!(fence.advance_epoch()?, SecurityEpoch::new(3));
    // Refusals and epoch failures render as sentences, not as Debug output.
    assert_eq!(
        FenceRefusal::Revoked {
            operation: operation(9)
        }
        .to_string(),
        format!("operation {} was revoked by name", operation(9))
    );
    assert_eq!(
        FenceError::EpochExhausted.to_string(),
        "security epoch space is exhausted; it never wraps"
    );
    // The tenant and run of the frozen effect are untouched by any of this.
    assert_eq!(TenantId::from_bytes([1; 16]), tenant(1));
    assert_eq!(RunId::from_bytes([1; 16]), run(1));
    Ok(())
}
