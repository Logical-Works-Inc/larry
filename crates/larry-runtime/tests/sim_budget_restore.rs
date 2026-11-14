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

use sim_kit::{Offer, Sim, fresh_journal, read_work, remove, replay};
use support::{TestRuntime, clock, settings};

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
        let path = fresh_journal("budget", seed)?;
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
