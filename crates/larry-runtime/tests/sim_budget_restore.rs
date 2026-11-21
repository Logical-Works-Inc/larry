//! larry#182 defect 2: a restart must spend the dispatch budget the journal
//! recorded, never recharge it.
//!
//! Source: idempotency-key retries (Stripe, `docs.stripe.com/api/idempotent_requests`)
//! and the transactional-outbox rule that a record of an effect is committed
//! with the effect itself (microservices.io Pattern: Transactional outbox). One
//! `EffectKey` names one physical dispatch, so the number of dispatches an
//! effect has made must be a fold over recorded facts, not live state a process
//! restart resets.
//!
//! Requirement proved: an `IdempotentRetrySafe` effect whose owner answers
//! `Unknown` every time, restored N times over one journal file, sees at most
//! `MAX_EFFECT_ATTEMPTS` owner dispatches in total, and no attempt number is
//! dispatched twice.

mod sim_kit;
mod support;

use std::path::Path;

use larry_contracts::authority::Scope;
use larry_contracts::effect::{MAX_EFFECT_ATTEMPTS, ReconciliationStrategy};
use larry_runtime::journal::Journal;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::runtime::restore;

use sim_kit::{Offer, Sim, fresh_journal, read_work, reader, remove, replay};
use support::{TestRuntime, clock, operation, settings};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// The `FileRead` runtime over the durable journal at `path`, a fresh owner
/// answering `Unknown` forever.
fn durable(path: &Path) -> Sim<TestRuntime> {
    let settings = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
    let journal = Journal::open(path, settings.journal_capacity)?;
    Ok(restore(settings, ScriptedOwner::new(), clock()?, journal)?)
}

/// Ticks `runtime` `count` times and returns how many dispatches its owner saw.
fn ticks(runtime: &mut TestRuntime, count: usize) -> Result<usize, Box<dyn std::error::Error>> {
    for _ in 0..count {
        runtime.tick()?;
    }
    Ok(runtime.owner().dispatched().len())
}

/// An `IdempotentRetrySafe` effect that reports `Unknown` every time spends its
/// dispatches once; every later restart must add none, so no `EffectKey` names
/// a second physical dispatch (larry#182, ADR 0017).
#[test]
fn sim_a_restart_spends_the_recorded_dispatch_budget() -> Outcome {
    replay(0_u64..8, |seed| -> Sim<(usize, usize)> {
        let (_guard, path) = fresh_journal("budget", seed)?;
        // Process 1: submit one retry-safe effect and drain its budget. Four
        // ticks is more than the three attempts, so the effect leaves
        // reconciliation with its dispatch budget spent and its outcome unknown.
        let mut first = durable(&path)?;
        first.offer(
            1,
            &read_work(1, 1, ReconciliationStrategy::IdempotentRetrySafe),
        )?;
        let spent = ticks(&mut first, 4)?;
        assert!(
            (1..=usize::from(MAX_EFFECT_ATTEMPTS)).contains(&spent),
            "seed {seed}: one process spends between one and the budget of \
             {MAX_EFFECT_ATTEMPTS} dispatches, saw {spent}"
        );
        drop(first);

        // Restart N times, 1 to 8 by the seed, over the same journal file. The
        // host reaches its authority source after each restart, exactly as
        // `sim_crash_new_run_after_restore_completes` does, so the fence is not
        // what holds the dispatch back.
        let restarts = 1_usize.saturating_add(usize::try_from(seed % 8)?);
        let mut total = spent;
        for _ in 0..restarts {
            let mut restarted = durable(&path)?;
            let epoch = restarted.fence().epoch()?;
            restarted.fence_mut().authenticate(epoch)?;
            total = total.saturating_add(ticks(&mut restarted, 4)?);
            drop(restarted);
        }
        remove(&path)?;
        // The budget was spent in process 1, so the only way a restart
        // dispatches again is by resetting the recorded count to zero, which
        // also reuses attempt 1: the same `EffectKey` for a second physical
        // dispatch. Bounding the total by the budget forbids both.
        assert!(
            total <= usize::from(MAX_EFFECT_ATTEMPTS),
            "seed {seed}: {restarts} restarts must add no dispatch past the record, saw {total}"
        );
        Ok((spent, restarts))
    })?;
    Ok(())
}

/// The journal a crash leaves when it lands inside the owner call of the
/// `reach`th dispatch: every earlier record is durable, the reach is not. The
/// sim drives the real reach and then restores from the snapshot taken before
/// it, which is the exact state a process that died there would leave.
fn crash_after_reaches(seed: u64) -> Sim<(usize, usize)> {
    let reach = match seed.checked_rem(u64::from(MAX_EFFECT_ATTEMPTS)) {
        Some(offset) => usize::try_from(offset)?.saturating_add(1),
        None => 1,
    };
    let mut runtime = reader(ScriptedOwner::new(), 100)?;
    runtime.offer(
        1,
        &read_work(1, 1, ReconciliationStrategy::IdempotentRetrySafe),
    )?;
    let mut snapshot = runtime.journal().clone();
    for _ in 0..reach {
        snapshot = runtime.journal().clone();
        runtime.tick()?;
    }
    let before_crash = runtime.owner().dispatched().len();
    assert_eq!(
        before_crash, reach,
        "seed {seed}: every attempt before the crash reached the owner"
    );
    let mut restored: TestRuntime = restore(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        ScriptedOwner::new(),
        clock()?,
        snapshot,
    )?;
    let epoch = restored.fence().epoch()?;
    restored.fence_mut().authenticate(epoch)?;
    for _ in 0..=usize::from(MAX_EFFECT_ATTEMPTS) {
        restored.tick()?;
    }
    let total = before_crash.saturating_add(restored.owner().dispatched().len());
    assert!(
        total <= usize::from(MAX_EFFECT_ATTEMPTS),
        "seed {seed}: a crash inside dispatch {reach} must not buy an attempt past \
         the budget of {MAX_EFFECT_ATTEMPTS}, saw {total}"
    );
    Ok((reach, total))
}

/// A never-sent Queryable effect left open spends its reconciliation budget on
/// `Unknown` records; a restart must not recharge that budget (larry#332).
fn restart_unknowns(seed: u64) -> Sim<usize> {
    let mut runtime = reader(ScriptedOwner::new(), 100)?;
    runtime.offer(2, &read_work(2, 1, ReconciliationStrategy::Queryable))?;
    assert!(
        runtime.fence_mut().revoke(operation(2))?,
        "seed {seed}: the first mutating reach is fenced, so the effect was never sent"
    );
    runtime.tick()?;
    for _ in 0..usize::from(MAX_EFFECT_ATTEMPTS).saturating_mul(2) {
        runtime.tick()?;
    }
    let mut restored: TestRuntime = restore(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        ScriptedOwner::new(),
        clock()?,
        runtime.into_journal(),
    )?;
    let epoch = restored.fence().epoch()?;
    restored.fence_mut().authenticate(epoch)?;
    let before = restored.journal().len();
    for _ in 0..usize::from(MAX_EFFECT_ATTEMPTS).saturating_mul(2) {
        restored.tick()?;
    }
    let added = restored.journal().len().saturating_sub(before);
    assert_eq!(
        added, 0,
        "seed {seed}: a restart must not buy an `Unknown` record past the budget"
    );
    Ok(added)
}

/// larry#332: for 1,000 seeds, a crash at every dispatch point leaves total
/// owner reaches at or below `MAX_EFFECT_ATTEMPTS`, and a restart adds no
/// `Unknown` record past the reconciliation budget. One seed decides every
/// identity, the clock and the crash point; the same seed replays exactly.
#[test]
fn sim_a_crash_at_every_point_never_spends_past_the_budget() -> Outcome {
    replay(0_u64..1_000, |seed| -> Sim<(usize, usize, usize)> {
        let (reach, dispatched) = crash_after_reaches(seed)?;
        let unknowns = restart_unknowns(seed)?;
        Ok((reach, dispatched, unknowns))
    })?;
    Ok(())
}
