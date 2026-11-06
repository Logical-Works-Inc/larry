//! E65 deterministic simulation families, driven through the public runtime.

use super::{
    AtomicOwner, BindingError, FixedClock, ProjectionBackend, ReconciliationStrategy, Result,
    Runtime, RuntimeError, SEEDS, Scope, TestRuntime, assert_swap, command, grant, holder, instant,
    replay_authorizations, restore, runtime, settings, success_journey, swap_journey,
    swapped_field, trace,
};

/// Resource/link/destination/body/adapter/schema/epoch swaps before admission.
#[test]
fn sim_e65_display_to_admission_swaps() -> Result {
    for seed in 0..SEEDS {
        assert_eq!(
            swap_journey(seed, 2)?,
            swap_journey(seed, 2)?,
            "seed {seed} replay"
        );
    }
    Ok(())
}

/// The durable reservation cannot authorize a changed dispatch target.
#[test]
fn sim_e65_admission_to_dispatch_swaps() -> Result {
    for seed in 0..SEEDS {
        assert_eq!(
            swap_journey(seed, 3)?,
            swap_journey(seed, 3)?,
            "seed {seed} replay"
        );
    }
    Ok(())
}

/// A race at the executing owner still refuses and preserves the sent witness.
#[test]
fn sim_e65_atomic_owner_refusal() -> Result {
    for seed in 0..SEEDS {
        assert_eq!(
            swap_journey(seed, 4)?,
            swap_journey(seed, 4)?,
            "seed {seed} replay"
        );
    }
    Ok(())
}

/// Positive controls and exact seeded trace replay across two tenant identities.
#[test]
fn sim_e65_bound_execution_and_replay() -> Result {
    for seed in 0..SEEDS {
        assert_eq!(
            success_journey(seed)?,
            success_journey(seed)?,
            "seed {seed} trace"
        );
    }
    Ok(())
}

/// Lost replies and restart may not rebind an idempotent retry to another target.
#[test]
fn sim_e65_retry_after_restore_keeps_binding() -> Result {
    for seed in 0..SEEDS {
        let mut runtime: TestRuntime = Runtime::new(
            settings(seed)?,
            AtomicOwner {
                lose_reply: true,
                ..AtomicOwner::new(seed, 0)
            },
            FixedClock::new(instant(seed)?),
        )?;
        let command = command(
            seed,
            1,
            &[Scope::FileRead],
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let display = runtime.display(
            &command,
            holder(),
            &[grant(1, &[Scope::FileRead], 3, seed)?],
        )?;
        runtime.submit_displayed(command, display)?;
        assert!(
            runtime.tick()?.receipts().is_empty(),
            "lost reply stays unknown"
        );
        let mut restored: TestRuntime = restore(
            settings(seed)?,
            AtomicOwner::new(seed, 1),
            FixedClock::new(instant(seed)?),
            runtime.into_journal(),
        )?;
        let before = trace(restored.journal());
        assert_swap(restored.tick(), swapped_field(seed));
        assert_eq!(
            trace(restored.journal()),
            before,
            "refused retry writes no invented outcome"
        );
        assert!(
            restored.owner().executed.is_empty(),
            "restoration must not swap targets"
        );
    }
    Ok(())
}

/// Separate grants must not contribute disjoint authority to one effect.
#[test]
fn sim_e65_overlapping_grants_never_union() -> Result {
    for seed in 0..SEEDS {
        let mut runtime = runtime(seed, 0)?;
        let command = command(
            seed,
            1,
            &[Scope::FileRead, Scope::FileWrite],
            ReconciliationStrategy::Queryable,
        )?;
        let split = [
            grant(1, &[Scope::FileRead, Scope::NetworkEgress], 10, seed)?,
            grant(1, &[Scope::FileWrite, Scope::NetworkEgress], 10, seed)?,
        ];
        assert!(
            matches!(
                runtime.display(&command, holder(), &split),
                Err(RuntimeError::Binding(BindingError::NoCoveringGrant))
            ),
            "disjoint scopes cannot stitch"
        );
        let cheap = grant(1, &[Scope::FileRead, Scope::FileWrite], 2, seed)?;
        assert!(
            matches!(
                runtime.display(&command, holder(), &[cheap.clone(), cheap]),
                Err(RuntimeError::Binding(BindingError::NoCoveringGrant))
            ),
            "budgets cannot add"
        );
        let foreign = grant(2, &[Scope::FileRead, Scope::FileWrite], 10, seed)?;
        assert!(
            matches!(
                runtime.display(&command, holder(), &[foreign]),
                Err(RuntimeError::Binding(BindingError::NoCoveringGrant))
            ),
            "tenant grants cannot cross"
        );
        let covering = grant(1, &[Scope::FileRead, Scope::FileWrite], 3, seed)?;
        let display = runtime.display(
            &command,
            holder(),
            &[split[0].clone(), covering, split[1].clone()],
        )?;
        runtime.submit_displayed(command, display)?;
        assert_eq!(
            runtime.tick()?.receipts().len(),
            1,
            "one sufficient grant must remain usable"
        );
        assert_eq!(
            runtime.projection().authorizations()[0]
                .pinned
                .budget
                .remaining,
            3,
            "selected ceiling is pinned"
        );
    }
    Ok(())
}

/// Losing the outcome write preserves the display in the authoritative intent.
#[test]
fn sim_e65_full_journal_restore_preserves_display() -> Result {
    for seed in 0..SEEDS {
        let mut limited = settings(seed)?;
        limited.journal_capacity = 6;
        let mut runtime: TestRuntime = Runtime::new(
            limited,
            AtomicOwner::new(seed, 0),
            FixedClock::new(instant(seed)?),
        )?;
        let command = command(
            seed,
            1,
            &[Scope::FileRead],
            ReconciliationStrategy::Queryable,
        )?;
        let display = runtime.display(
            &command,
            holder(),
            &[grant(1, &[Scope::FileRead], 3, seed)?],
        )?;
        let expected = *display.binding();
        runtime.submit_displayed(command, display)?;
        assert!(
            matches!(
                runtime.tick(),
                Err(RuntimeError::JournalFull { capacity: 6 })
            ),
            "fault must fall after reservation and before outcome, seed {seed}"
        );
        assert_eq!(
            runtime.owner().executed.len(),
            1,
            "effect happened before outcome loss"
        );
        let mut restored: TestRuntime = restore(
            settings(seed)?,
            AtomicOwner::new(seed, 1),
            FixedClock::new(instant(seed)?),
            runtime.into_journal(),
        )?;
        assert_eq!(
            restored.projection().intents()[0].binding(),
            Some(&expected),
            "display survives recovery"
        );
        assert_eq!(
            restored.tick()?.receipts().len(),
            1,
            "status of original operation remains queryable"
        );
        assert!(
            restored.owner().executed.is_empty(),
            "changed target cannot be dispatched by recovery"
        );
        assert!(
            replay_authorizations(restored.projection()).is_bit_identical(),
            "recovery preserves pinned authorization"
        );
    }
    Ok(())
}
