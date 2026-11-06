//! Seeded budget-accounting simulations (D36, R71).
//!
//! Every test drives the real `Runtime` twice from one seed and asserts the
//! same fingerprint plus the exact remaining budget the reservations leave.
//! Budget is never created or destroyed except by a recorded reservation.

#[path = "sim_kit/seed.rs"]
mod seed;
mod sim_kit;
mod support;

use rocco_contracts::authority::Scope;
use rocco_contracts::effect::ReconciliationStrategy;
use rocco_contracts::ids::ContentDigest;
use rocco_contracts::state::RunState;
use rocco_runtime::owner::fixture::ScriptedOwner;
use rocco_runtime::projection::ProjectionBackend;

use seed::step;
use sim_kit::{Offer, Sim, read_work, reader, reopen, replay, trace, work};
use support::{
    TestRuntime, assert_state, command_id, operation, proposal, run, runtime_with, succeeded,
    tenant, unknown,
};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Wrapping seed evolution, independent of any clock or scheduler.
fn advance(seed: &mut u64) -> u64 {
    step(seed, 12_150_014_235_147_800_641, 19_775_109_225_716_931, 31)
}

/// E55: three sequential reservations leave exactly the unspent remainder.
#[test]
fn sim_budget_sequential_reservations_leave_remainder() -> Outcome {
    replay(0_u64..8, |seed| -> Sim<(u64, ContentDigest)> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([
                succeeded(operation(1)),
                succeeded(operation(2)),
                succeeded(operation(3)),
            ]),
            100,
        )?;
        for (number, cost) in [(1_u8, 10_u64), (2, 20), (3, 30)] {
            let body = work(
                number,
                1,
                number,
                cost,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            runtime.offer(number, &body)?;
            let tick = runtime.tick()?;
            assert_eq!(tick.receipts().len(), 1, "each run receipts once");
        }
        assert_eq!(
            runtime.remaining_budget(),
            40,
            "ten plus twenty plus thirty spent"
        );
        assert_eq!(runtime.projection().receipts().len(), 3, "three receipts");
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));
        assert_eq!(first_budget, 40, "seed {seed}: forty remain");
        Ok((first_budget, first_fp))
    })
}

/// D36: a denied reservation leaves the budget untouched.
#[test]
fn sim_budget_denied_reservation_untouched() -> Outcome {
    replay(8_u64..16, |seed| -> Sim<(u64, ContentDigest)> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 50)?;
        let body = read_work(4, 51, ReconciliationStrategy::Queryable);
        runtime.offer(4, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(tick.denied().len(), 1, "over-budget denied");
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));
        assert_eq!(first_budget, 50, "seed {seed}: denial reserves nothing");
        Ok((first_budget, first_fp))
    })
}

/// D36: ten unit-cost effects leave ninety of one hundred.
#[test]
fn sim_budget_ten_unit_costs_leave_ninety() -> Outcome {
    replay(16_u64..20, |seed| -> Sim<(u64, ContentDigest)> {
        let mut owners = Vec::new();
        for identity in 5_u8..15 {
            owners.push(succeeded(operation(identity)));
        }
        let mut runtime: TestRuntime = reader(ScriptedOwner::scripted(owners), 100)?;
        for number in 5_u8..15 {
            let body = work(
                number,
                1,
                number,
                1,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            runtime.offer(number, &body)?;
        }
        runtime.tick()?;
        assert_eq!(runtime.projection().receipts().len(), 10, "ten receipts");
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));
        assert_eq!(first_budget, 90, "seed {seed}: ninety remain");
        Ok((first_budget, first_fp))
    })
}

/// D36: spending the last unit authorizes, the next unit denies.
#[test]
fn sim_budget_last_unit_authorizes_next_denies() -> Outcome {
    replay(20_u64..26, |seed| -> Sim<(u64, ContentDigest)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(15))]), 1)?;
        let first = read_work(15, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(15, &first)?;
        let tick = runtime.tick()?;
        assert_eq!(tick.authorized(), &[operation(15)], "last unit authorizes");
        assert_eq!(runtime.remaining_budget(), 0, "budget exhausted exactly");
        let second = read_work(16, 1, ReconciliationStrategy::Queryable);
        runtime.offer(16, &second)?;
        let denied = runtime.tick()?;
        assert_eq!(denied.denied().len(), 1, "next unit denies");
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));
        assert_eq!(first_budget, 0, "seed {seed}: zero remain");
        Ok((first_budget, first_fp))
    })
}

/// D36: an open unknown holds its reservation across quiet ticks.
#[test]
fn sim_budget_open_unknown_holds_reservation() -> Outcome {
    replay(26_u64..32, |seed| -> Sim<(u64, ContentDigest)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([unknown(operation(17))]), 100)?;
        let body = read_work(17, 25, ReconciliationStrategy::Queryable);
        runtime.offer(17, &body)?;
        runtime.tick()?;
        runtime.tick()?;
        runtime.tick()?;
        assert_state(runtime.projection(), run(17), RunState::Reconciling)?;
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));
        assert_eq!(first_budget, 75, "seed {seed}: twenty-five held");
        Ok((first_budget, first_fp))
    })
}

/// D36: a receipt never refunds the spent reservation.
#[test]
fn sim_budget_receipt_never_refunds() -> Outcome {
    replay(32_u64..38, |seed| -> Sim<(u64, ContentDigest)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(18))]), 100)?;
        let body = read_work(18, 12, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(18, &body)?;
        runtime.tick()?;
        runtime.tick()?;
        assert_eq!(runtime.remaining_budget(), 88, "spent stays spent");
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));
        assert_eq!(first_budget, 88, "seed {seed}: eighty-eight remain");
        Ok((first_budget, first_fp))
    })
}

/// D36: a duplicate never reserves twice.
#[test]
fn sim_budget_duplicate_never_reserves_twice() -> Outcome {
    replay(38_u64..44, |seed| -> Sim<(u64, ContentDigest)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(19))]), 100)?;
        let body = read_work(19, 33, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(19, &body)?;
        runtime.tick()?;
        runtime.offer(19, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(tick.duplicates(), &[command_id(19)], "resubmit dedupes");
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));
        assert_eq!(first_budget, 67, "seed {seed}: sixty-seven remain");
        Ok((first_budget, first_fp))
    })
}

/// D36: seeded costs replay the same remainder on both runs.
#[test]
fn sim_budget_seeded_costs_replay_remainder() -> Outcome {
    replay(44_u64..56, |seed| -> Sim<(u64, ContentDigest)> {
        let mut random = seed.saturating_add(13);
        let mut costs = Vec::new();
        for _ in 0..3 {
            costs.push((advance(&mut random) & 7).saturating_add(1));
        }
        let mut owners = Vec::new();
        for identity in 20_u8..23 {
            owners.push(succeeded(operation(identity)));
        }
        let mut runtime: TestRuntime = reader(ScriptedOwner::scripted(owners), 100)?;
        let mut spent: u64 = 0;
        for (index, cost) in costs.iter().enumerate() {
            let number = u8::try_from(index)?.saturating_add(20);
            let body = work(
                number,
                1,
                number,
                *cost,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            runtime.offer(number, &body)?;
            spent = spent.saturating_add(*cost);
        }
        runtime.tick()?;
        assert_eq!(
            runtime.remaining_budget(),
            100_u64.saturating_sub(spent),
            "remainder matches the seeded sum"
        );
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));

        Ok((first_budget, first_fp))
    })
}

/// E72/D36: two open reservations survive a crash with the same remainder.
#[test]
fn sim_budget_two_open_reservations_survive_crash() -> Outcome {
    replay(56_u64..62, |seed| -> Sim<(u64, ContentDigest)> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([unknown(operation(24)), unknown(operation(25))]),
            100,
        )?;
        for (number, cost) in [(24_u8, 10_u64), (25, 15)] {
            let body = work(number, 1, number, cost, ReconciliationStrategy::Queryable);
            runtime.offer(number, &body)?;
        }
        runtime.tick()?;
        let restored = reopen(runtime.into_journal(), 100)?;
        let (first_budget, first_fp) = (restored.remaining_budget(), trace(restored.journal()));
        assert_eq!(first_budget, 75, "seed {seed}: seventy-five remain");
        Ok((first_budget, first_fp))
    })
}

/// D36: a capability denial holds the full budget.
#[test]
fn sim_budget_capability_denial_holds_full_budget() -> Outcome {
    replay(62_u64..68, |seed| -> Sim<(u64, ContentDigest)> {
        let mut runtime: TestRuntime = runtime_with(
            ScriptedOwner::new(),
            &[Scope::FileRead, Scope::FileWrite],
            &[Scope::FileRead],
            80,
        )?;
        let body = proposal(
            run(26),
            tenant(1),
            operation(26),
            &[Scope::FileWrite],
            40,
            ReconciliationStrategy::Queryable,
        );
        runtime.offer(26, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(tick.denied().len(), 1, "capability denies");
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));
        assert_eq!(first_budget, 80, "seed {seed}: eighty remain");
        Ok((first_budget, first_fp))
    })
}

/// D36: a policy denial holds the full budget.
#[test]
fn sim_budget_policy_denial_holds_full_budget() -> Outcome {
    replay(68_u64..74, |seed| -> Sim<(u64, ContentDigest)> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 80)?;
        let body = proposal(
            run(27),
            tenant(1),
            operation(27),
            &[Scope::NetworkEgress],
            40,
            ReconciliationStrategy::Queryable,
        );
        runtime.offer(27, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(tick.denied().len(), 1, "policy denies");
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));
        assert_eq!(first_budget, 80, "seed {seed}: eighty remain");
        Ok((first_budget, first_fp))
    })
}

/// D36: two tenants sharing one pool leave the pooled remainder.
#[test]
fn sim_budget_two_tenants_share_one_pool() -> Outcome {
    replay(74_u64..80, |seed| -> Sim<(u64, ContentDigest)> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(28)), succeeded(operation(29))]),
            100,
        )?;
        for (number, scope, cost) in [(28_u8, 1_u8, 30_u64), (29, 2, 25)] {
            let body = work(
                number,
                scope,
                number,
                cost,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            runtime.offer(number, &body)?;
        }
        runtime.tick()?;
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));
        assert_eq!(first_budget, 45, "seed {seed}: forty-five remain");
        Ok((first_budget, first_fp))
    })
}

/// D36: cancel never spends from the budget pool.
#[test]
fn sim_budget_cancel_never_spends() -> Outcome {
    use support::cancel;
    replay(80_u64..86, |seed| -> Sim<(u64, ContentDigest)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(30))]), 100)?;
        let body = read_work(30, 10, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(30, &body)?;
        runtime.tick()?;
        let gone = proposal(
            run(31),
            tenant(1),
            operation(31),
            &[],
            0,
            ReconciliationStrategy::RequiresHuman,
        );
        runtime.submit(cancel(command_id(31), &gone)?)?;
        runtime.tick()?;
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));
        assert_eq!(first_budget, 90, "seed {seed}: ninety remain");
        Ok((first_budget, first_fp))
    })
}

/// D36: interleaved ticks accrue reservations across the journal.
#[test]
fn sim_budget_interleaved_ticks_accrue() -> Outcome {
    replay(86_u64..92, |seed| -> Sim<(u64, ContentDigest)> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([
                succeeded(operation(32)),
                succeeded(operation(33)),
                succeeded(operation(34)),
            ]),
            100,
        )?;
        for (number, cost) in [(32_u8, 5_u64), (33, 5), (34, 5)] {
            let body = work(
                number,
                1,
                number,
                cost,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            runtime.offer(number, &body)?;
            let tick = runtime.tick()?;
            assert_eq!(tick.receipts().len(), 1, "each tick receipts once");
        }
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));
        assert_eq!(first_budget, 85, "seed {seed}: eighty-five remain");
        Ok((first_budget, first_fp))
    })
}

/// D36: a zero-cost success leaves a zero budget at zero.
#[test]
fn sim_budget_zero_cost_on_zero_budget_stays_zero() -> Outcome {
    replay(92_u64..98, |seed| -> Sim<(u64, ContentDigest)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(35))]), 0)?;
        let body = read_work(35, 0, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(35, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(tick.receipts().len(), 1, "zero cost still receipts");
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));
        assert_eq!(first_budget, 0, "seed {seed}: zero stays zero");
        Ok((first_budget, first_fp))
    })
}

/// D36: seeded tenants draw one pooled remainder across both runs.
#[test]
fn sim_budget_seeded_tenants_draw_one_pool() -> Outcome {
    replay(98_u64..106, |seed| -> Sim<(u64, ContentDigest)> {
        let mut random = seed.saturating_add(51);
        let first_cost = (advance(&mut random) & 15).saturating_add(1);
        let second_cost = (advance(&mut random) & 15).saturating_add(1);
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(36)), succeeded(operation(37))]),
            100,
        )?;
        for (number, scope, cost) in [(36_u8, 1_u8, first_cost), (37, 2, second_cost)] {
            let body = work(
                number,
                scope,
                number,
                cost,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            runtime.offer(number, &body)?;
        }
        runtime.tick()?;
        let expected = 100_u64
            .saturating_sub(first_cost)
            .saturating_sub(second_cost);
        assert_eq!(
            runtime.remaining_budget(),
            expected,
            "pool draws both costs"
        );
        let (first_budget, first_fp) = (runtime.remaining_budget(), trace(runtime.journal()));

        Ok((first_budget, first_fp))
    })
}
