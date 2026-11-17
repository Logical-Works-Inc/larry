//! Seeded budget-accounting simulations (D36, R71).
//!
//! Every test drives the real `Runtime` twice from one seed and asserts the
//! same fingerprint plus the exact remaining budget the reservations leave.
//! Budget is never created or destroyed except by a recorded reservation.

#[path = "sim_kit/seed.rs"]
mod seed;
mod sim_kit;
mod support;

use larry_contracts::authority::Scope;
use larry_contracts::command::CommandKind;
use larry_contracts::effect::ReconciliationStrategy;
use larry_contracts::event::{Event, EventKind, EventSequence};
use larry_contracts::ids::{ContentDigest, TenantId};
use larry_contracts::replay::DenialReason;
use larry_contracts::state::RunState;
use larry_runtime::error::RuntimeError;
use larry_runtime::journal::Journal;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::payload::CommandAcceptedPayload;
use larry_runtime::projection::ProjectionBackend;
use larry_runtime::runtime::restore;

use seed::step;
use sim_kit::{Offer, Sim, read_work, reader, reopen, replay, trace, work};
use support::{
    TestRuntime, assert_state, clock, command_id, operation, proposal, run, runtime_with, settings,
    succeeded, tenant, unknown,
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
            runtime.remaining_budget(tenant(1)),
            40,
            "ten plus twenty plus thirty spent"
        );
        assert_eq!(runtime.projection().receipts().len(), 3, "three receipts");
        let (first_budget, first_fp) = (
            runtime.remaining_budget(tenant(1)),
            trace(runtime.journal()),
        );
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
        assert_eq!(
            runtime.remaining_budget(tenant(9)),
            50,
            "a tenant that never ran reads its whole budget"
        );
        let (first_budget, first_fp) = (
            runtime.remaining_budget(tenant(1)),
            trace(runtime.journal()),
        );
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
        let (first_budget, first_fp) = (
            runtime.remaining_budget(tenant(1)),
            trace(runtime.journal()),
        );
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
        assert_eq!(
            runtime.remaining_budget(tenant(1)),
            0,
            "budget exhausted exactly"
        );
        let second = read_work(16, 1, ReconciliationStrategy::Queryable);
        runtime.offer(16, &second)?;
        let denied = runtime.tick()?;
        assert_eq!(denied.denied().len(), 1, "next unit denies");
        let (first_budget, first_fp) = (
            runtime.remaining_budget(tenant(1)),
            trace(runtime.journal()),
        );
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
        let (first_budget, first_fp) = (
            runtime.remaining_budget(tenant(1)),
            trace(runtime.journal()),
        );
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
        assert_eq!(runtime.remaining_budget(tenant(1)), 88, "spent stays spent");
        let (first_budget, first_fp) = (
            runtime.remaining_budget(tenant(1)),
            trace(runtime.journal()),
        );
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
        let (first_budget, first_fp) = (
            runtime.remaining_budget(tenant(1)),
            trace(runtime.journal()),
        );
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
            runtime.remaining_budget(tenant(1)),
            100_u64.saturating_sub(spent),
            "remainder matches the seeded sum"
        );
        let (first_budget, first_fp) = (
            runtime.remaining_budget(tenant(1)),
            trace(runtime.journal()),
        );

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
        let (first_budget, first_fp) = (
            restored.remaining_budget(tenant(1)),
            trace(restored.journal()),
        );
        assert_eq!(first_budget, 75, "seed {seed}: seventy-five remain");
        Ok((first_budget, first_fp))
    })
}

/// D36: a capability denial and a policy denial each hold the full budget.
#[test]
fn sim_budget_denial_holds_full_budget() -> Outcome {
    replay(62_u64..74, |seed| -> Sim<(u64, u64)> {
        let mut capability: TestRuntime = runtime_with(
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
        capability.offer(26, &body)?;
        assert_eq!(capability.tick()?.denied().len(), 1, "capability denies");
        let mut policy: TestRuntime = reader(ScriptedOwner::new(), 80)?;
        let body = proposal(
            run(27),
            tenant(1),
            operation(27),
            &[Scope::NetworkEgress],
            40,
            ReconciliationStrategy::Queryable,
        );
        policy.offer(27, &body)?;
        assert_eq!(policy.tick()?.denied().len(), 1, "policy denies");
        let first_budget = capability.remaining_budget(tenant(1));
        let second_budget = policy.remaining_budget(tenant(1));
        assert_eq!(
            first_budget, 80,
            "seed {seed}: capability denial holds the budget"
        );
        assert_eq!(
            second_budget, 80,
            "seed {seed}: policy denial holds the budget"
        );
        Ok((first_budget, second_budget))
    })
}

/// D36 (larry#208): each tenant keeps its own remainder.
#[test]
fn sim_budget_two_tenants_keep_their_own_remainder() -> Outcome {
    replay(74_u64..80, |seed| -> Sim<(u64, u64, ContentDigest)> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(28)), succeeded(operation(29))]),
            100,
        )?;
        for (number, ten, cost) in [(28_u8, 1_u8, 30_u64), (29, 2, 25)] {
            let body = work(
                number,
                ten,
                number,
                cost,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            runtime.offer(number, &body)?;
        }
        runtime.tick()?;
        let (first, second) = (
            runtime.remaining_budget(tenant(1)),
            runtime.remaining_budget(tenant(2)),
        );
        assert_eq!(first, 70, "seed {seed}: tenant 1 spent thirty of its own");
        assert_eq!(
            second, 75,
            "seed {seed}: tenant 2 spent twenty-five of its own"
        );
        Ok((first, second, trace(runtime.journal())))
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
        let (first_budget, first_fp) = (
            runtime.remaining_budget(tenant(1)),
            trace(runtime.journal()),
        );
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
        let (first_budget, first_fp) = (
            runtime.remaining_budget(tenant(1)),
            trace(runtime.journal()),
        );
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
        let (first_budget, first_fp) = (
            runtime.remaining_budget(tenant(1)),
            trace(runtime.journal()),
        );
        assert_eq!(first_budget, 0, "seed {seed}: zero stays zero");
        Ok((first_budget, first_fp))
    })
}

/// D36 (larry#208): seeded tenants keep their own seeded remainders.
#[test]
fn sim_budget_seeded_tenants_keep_their_own_remainders() -> Outcome {
    replay(98_u64..106, |seed| -> Sim<(u64, u64)> {
        let mut random = seed.saturating_add(51);
        let first_cost = (advance(&mut random) & 15).saturating_add(1);
        let second_cost = (advance(&mut random) & 15).saturating_add(1);
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(36)), succeeded(operation(37))]),
            100,
        )?;
        for (number, ten, cost) in [(36_u8, 1_u8, first_cost), (37, 2, second_cost)] {
            let body = work(
                number,
                ten,
                number,
                cost,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            runtime.offer(number, &body)?;
        }
        runtime.tick()?;
        let first = runtime.remaining_budget(tenant(1));
        let second = runtime.remaining_budget(tenant(2));
        assert_eq!(
            first,
            100_u64.saturating_sub(first_cost),
            "seed {seed}: tenant 1 keeps its own"
        );
        assert_eq!(
            second,
            100_u64.saturating_sub(second_cost),
            "seed {seed}: tenant 2 keeps its own"
        );
        Ok((first, second))
    })
}

/// D36 (larry#208): one tenant spending its whole budget refuses only its own
/// next effect. Tenant A spends all 100 units; tenant B then submits a seeded
/// number of one-unit effects, and every one authorizes and receipts.
#[test]
fn sim_budget_is_per_tenant() -> Outcome {
    replay(106_u64..114, |seed| -> Sim<(u64, u64, usize)> {
        let mut random = seed.saturating_add(29);
        let effects = usize::try_from(advance(&mut random) & 3)?.saturating_add(1);
        let mut script = vec![succeeded(operation(50))];
        for index in 0..effects {
            let number = u8::try_from(index)?.saturating_add(51);
            script.push(succeeded(operation(number)));
        }
        let mut runtime: TestRuntime = reader(ScriptedOwner::scripted(script), 100)?;
        let spent = work(50, 1, 50, 100, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(50, &spent)?;
        runtime.tick()?;
        assert_eq!(
            runtime.remaining_budget(tenant(1)),
            0,
            "seed {seed}: A spends its whole budget"
        );
        assert_state(runtime.projection(), run(50), RunState::Completed)?;
        let next = work(49, 1, 49, 1, ReconciliationStrategy::Queryable);
        runtime.offer(49, &next)?;
        let denied = runtime.tick()?;
        assert_eq!(
            denied.denied(),
            &[(
                operation(49),
                DenialReason::BudgetExhausted {
                    required: 1,
                    remaining: 0
                }
            )],
            "A's own spent budget refuses its next effect"
        );
        for index in 0..effects {
            let number = u8::try_from(index)?.saturating_add(51);
            let body = work(
                number,
                2,
                number,
                1,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            runtime.offer(number, &body)?;
            let tick = runtime.tick()?;
            assert_eq!(
                tick.authorized(),
                &[operation(number)],
                "seed {seed}: B's effect {number} authorizes"
            );
            assert_eq!(
                tick.receipts().len(),
                1,
                "seed {seed}: B's effect {number} receipts"
            );
        }
        let left = runtime.remaining_budget(tenant(2));
        assert_eq!(
            left,
            100_u64.saturating_sub(u64::try_from(effects)?),
            "seed {seed}: B keeps its own remainder"
        );
        assert_eq!(
            runtime.remaining_budget(tenant(1)),
            0,
            "A stays spent while B works"
        );
        Ok((runtime.remaining_budget(tenant(1)), left, effects))
    })
}

/// D36 (larry#208): a journal whose one run names two tenants is refused at
/// restore with the typed mismatch, before any run state is trusted.
#[test]
fn sim_budget_restore_refuses_two_tenants_for_one_run() -> Outcome {
    replay(114_u64..116, |_| -> Sim<(usize, TenantId)> {
        let mut journal = Journal::new(64)?;
        for (sequence, ten) in [(0_u64, 1_u8), (1, 2)] {
            let payload = CommandAcceptedPayload::new(command_id(60), run(60), CommandKind::Submit);
            let event = Event::new(
                tenant(ten),
                run(60),
                EventSequence::new(sequence),
                Vec::new(),
                EventKind::CommandAccepted,
                lgwks_std::json::to_vec(&payload)?.into_boxed_slice(),
            )?;
            journal.record(event)?;
        }
        let refused: Result<TestRuntime, RuntimeError> = restore(
            settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
            ScriptedOwner::new(),
            clock()?,
            journal,
        );
        match refused {
            Err(RuntimeError::RunTenantMismatch {
                run: refused_run,
                expected,
                got,
            }) => {
                assert_eq!(refused_run, run(60), "the refusal names the run");
                assert_eq!(expected, tenant(1), "the first recorded tenant");
                assert_eq!(got, tenant(2), "the later event's tenant");
            }
            other => return Err(format!("expected RunTenantMismatch, got {other:?}").into()),
        }
        Ok((2, tenant(2)))
    })
}

/// D36 (larry#208): a restored runtime keeps each tenant's own remainder.
#[test]
fn sim_budget_restore_keeps_each_tenants_remainder() -> Outcome {
    replay(116_u64..122, |seed| -> Sim<(u64, u64)> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([unknown(operation(70)), unknown(operation(71))]),
            100,
        )?;
        for (number, ten, cost) in [(70_u8, 1_u8, 30_u64), (71, 2, 25)] {
            let body = work(number, ten, number, cost, ReconciliationStrategy::Queryable);
            runtime.offer(number, &body)?;
        }
        runtime.tick()?;
        let restored = reopen(runtime.into_journal(), 100)?;
        let first = restored.remaining_budget(tenant(1));
        let second = restored.remaining_budget(tenant(2));
        assert_eq!(
            first, 70_u64,
            "seed {seed}: tenant 1 keeps its own remainder"
        );
        assert_eq!(
            second, 75_u64,
            "seed {seed}: tenant 2 keeps its own remainder"
        );
        Ok((first, second))
    })
}
