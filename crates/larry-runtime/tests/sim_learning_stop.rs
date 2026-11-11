//! larry#66 part 2: one stop halts a tenant's recursion plans. The stop rides an ordinary `Cancel`
//! whose body names the plans, so no new command or event kind exists. In-flight steps still settle
//! and free their slots; another tenant's plans keep running; replay, including a reopened journal
//! file, rebuilds the stop.

#[path = "support/milli.rs"]
mod milli;
mod sim_kit;
mod support;

use larry_contracts::authority::{BudgetUnit, Scope};
use larry_contracts::effect::{EffectOutcome, ReconciliationStrategy};
use larry_contracts::harness::{HarnessComponent, HarnessHash, HarnessTree};
use larry_contracts::ids::ContentDigest;
use larry_contracts::recursion::{
    Label, PlanScope, Produced, RecursionMode, RecursionPlan, RecursionStep, Stage, StopError,
};
use larry_runtime::error::RuntimeError;
use larry_runtime::journal::Journal;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::payload::WorkProposal;
use larry_runtime::projection::{FoldProjection, ProjectionBackend};
use larry_runtime::runtime::{replay as replay_journal, restore};

use milli::{proposal_milli, settings_milli};
use sim_kit::Sim;
use support::{TestRuntime, clock, command_id, operation, run, submit, tenant};

fn base() -> HarnessHash {
    HarnessTree::empty().hash()
}

fn plan(task: u64) -> Sim<RecursionPlan> {
    Ok(RecursionPlan::new(
        ContentDigest::over(&[b"task", &task.to_be_bytes()]),
        RecursionMode::Parallel,
        1,
        base(),
        4,
    )?)
}

/// A runtime whose owner follows `script` and produces a harness for every proposer step.
fn runtime_over(journal: Journal, script: Vec<EffectOutcome>) -> Sim<TestRuntime> {
    let mut owner = ScriptedOwner::scripted(script);
    for number in 1..=40_u8 {
        let mut tree = HarnessTree::empty();
        tree.insert(HarnessComponent::Memory, &format!("n{number}"), b"x")?;
        owner = owner.producing(operation(number), Produced::Harness(tree.hash()));
    }
    let mut runtime: TestRuntime = restore(
        settings_milli(&[Scope::FileRead], &[Scope::FileRead], 1_000_000)?,
        owner,
        clock()?,
        journal,
    )?;
    let epoch = runtime.fence().epoch()?;
    runtime.fence_mut().authenticate(epoch)?;
    Ok(runtime)
}

fn succeed(number: u8) -> EffectOutcome {
    EffectOutcome::Succeeded {
        operation: operation(number),
    }
}

/// Submits step `label` of `plan` as run `number` for tenant `who` and ticks.
fn step(
    runtime: &mut TestRuntime,
    number: u8,
    who: u8,
    plan: RecursionPlan,
    label: Label,
) -> Sim<()> {
    let mut reference = RecursionStep::new(plan, label, 0);
    if label.stage() == Stage::Probe {
        let candidate = runtime
            .recursion(tenant(who), &plan)
            .and_then(|held| held.candidate(0, label.slot() >> 1))
            .ok_or("candidate")?;
        reference = reference.with_candidate(candidate);
    }
    let body = proposal_milli(
        run(number),
        tenant(who),
        operation(number),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    )
    .stepping(reference);
    runtime.submit(submit(command_id(number), &body)?)?;
    runtime.tick()?;
    Ok(())
}

fn propose(
    runtime: &mut TestRuntime,
    number: u8,
    who: u8,
    plan: RecursionPlan,
    slot: u8,
) -> Sim<()> {
    step(
        runtime,
        number,
        who,
        plan,
        Label::new(0, Stage::Propose, slot),
    )
}

#[test]
fn sim_learning_stop_halts_new_steps_while_in_flight_ones_settle_and_free_their_slots() -> Sim<()> {
    for seed in 0..16_u64 {
        support::seeded(seed, || -> Sim<()> {
            let one = plan(seed)?;
            // Two proposals succeed; the probe is unknown, then the status query says success.
            let mut runtime = runtime_over(
                Journal::new(4096)?,
                vec![
                    succeed(1),
                    succeed(2),
                    EffectOutcome::Unknown {
                        operation: operation(3),
                    },
                    succeed(3),
                ],
            )?;
            propose(&mut runtime, 1, 1, one, 0)?;
            propose(&mut runtime, 2, 1, one, 1)?;
            step(&mut runtime, 3, 1, one, Label::new(0, Stage::Probe, 0))?;
            assert_eq!(
                runtime
                    .recursion(tenant(1), &one)
                    .ok_or("ledger")?
                    .open_probes(),
                1,
                "seed {seed}: a probe holds its slot"
            );

            runtime.stop_learning(tenant(1), PlanScope::One(one.identity()))?;
            let stopped = runtime.recursion(tenant(1), &one).ok_or("ledger")?;
            assert!(stopped.is_cancelled(), "seed {seed}: the plan is stopped");
            assert!(
                stopped.next_steps().is_empty(),
                "seed {seed}: nothing is offered"
            );
            // A new intent is refused before the write.
            let before = runtime.journal().len();
            let late = WorkProposal::new(
                run(9),
                tenant(1),
                operation(9),
                [Scope::FileRead].into_iter().collect(),
                1,
                BudgetUnit::MilliCents,
                ReconciliationStrategy::Queryable,
            )
            .stepping(
                RecursionStep::new(one, Label::new(0, Stage::Probe, 1), 0)
                    .with_candidate(stopped.candidate(0, 0).ok_or("candidate")?),
            );
            runtime.submit(submit(command_id(9), &late)?)?;
            let error = runtime.tick().err().ok_or("a refusal")?;
            assert!(
                matches!(
                    error,
                    RuntimeError::Recursion(larry_contracts::recursion::RecursionError::Cancelled)
                ),
                "seed {seed}: {error}"
            );
            assert_eq!(
                runtime.journal().len(),
                before,
                "seed {seed}: nothing was written"
            );
            // The in-flight probe still settles, and its slot frees.
            runtime.tick()?;
            assert_eq!(
                runtime
                    .recursion(tenant(1), &one)
                    .ok_or("ledger")?
                    .open_probes(),
                0,
                "seed {seed}: it settled and freed its slot"
            );
            // A second stop does nothing: refused before the write.
            let before = runtime.journal().len();
            let again = runtime
                .stop_learning(tenant(1), PlanScope::One(one.identity()))
                .err()
                .ok_or("refused")?;
            assert!(
                matches!(again, RuntimeError::Stop(StopError::AlreadyStopped)),
                "seed {seed}: {again}"
            );
            assert_eq!(
                runtime.journal().len(),
                before,
                "seed {seed}: idempotent, nothing written"
            );
            // The stop comes from the run table's tenant: tenant 2 cannot name tenant 1's plan.
            let foreign = runtime
                .stop_learning(tenant(2), PlanScope::One(one.identity()))
                .err()
                .ok_or("refused")?;
            assert!(
                matches!(foreign, RuntimeError::Stop(StopError::UnknownPlan)),
                "seed {seed}: {foreign}"
            );
            // Replay rebuilds the stop.
            let mut folded = FoldProjection::default();
            replay_journal(&mut folded, runtime.journal())?;
            assert_eq!(
                folded.recursion(tenant(1), &one),
                runtime.recursion(tenant(1), &one),
                "seed {seed}: replay equals live"
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn sim_learning_stop_all_covers_this_tenants_plans_and_no_others() -> Sim<()> {
    for seed in 0..16_u64 {
        support::seeded(seed, || -> Sim<()> {
            let (first, second, other) = (
                plan(seed)?,
                plan(seed.wrapping_add(1000))?,
                plan(seed.wrapping_add(2000))?,
            );
            let mut runtime = runtime_over(Journal::new(4096)?, (1..=10).map(succeed).collect())?;
            propose(&mut runtime, 1, 1, first, 0)?;
            propose(&mut runtime, 2, 1, second, 0)?;
            propose(&mut runtime, 3, 2, other, 0)?;
            runtime.stop_learning(tenant(1), PlanScope::All)?;
            for held in [&first, &second] {
                assert!(
                    runtime
                        .recursion(tenant(1), held)
                        .ok_or("ledger")?
                        .is_cancelled(),
                    "seed {seed}: tenant 1's plans stop"
                );
            }
            let running = runtime.recursion(tenant(2), &other).ok_or("ledger")?;
            assert!(
                !running.is_cancelled() && !running.next_steps().is_empty(),
                "seed {seed}: tenant 2's plan keeps running"
            );
            propose(&mut runtime, 4, 2, other, 1)?;
            assert_eq!(
                runtime
                    .recursion(tenant(2), &other)
                    .ok_or("ledger")?
                    .sessions(),
                2,
                "seed {seed}: and still takes steps"
            );
            let again = runtime
                .stop_learning(tenant(1), PlanScope::All)
                .err()
                .ok_or("refused")?;
            assert!(
                matches!(again, RuntimeError::Stop(StopError::NothingToStop)),
                "seed {seed}: {again}"
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn sim_learning_stop_survives_a_reopened_journal_file() -> Sim<()> {
    for seed in 0..6_u64 {
        support::seeded(seed, || -> Sim<()> {
            let path = std::env::temp_dir()
                .join(format!("larry-stop-{}-{seed}.journal", std::process::id()));
            std::fs::remove_file(&path).ok();
            let one = plan(seed)?;
            {
                let mut runtime = runtime_over(Journal::open(&path, 4096)?, vec![succeed(1)])?;
                propose(&mut runtime, 1, 1, one, 0)?;
                runtime.stop_learning(tenant(1), PlanScope::One(one.identity()))?;
            }
            let reopened = Journal::open(&path, 4096)?;
            let mut folded = FoldProjection::default();
            replay_journal(&mut folded, &reopened)?;
            assert!(
                folded
                    .recursion(tenant(1), &one)
                    .is_some_and(|held| held.is_cancelled()),
                "seed {seed}: the file replays to a stopped plan"
            );
            std::fs::remove_file(&path).ok();
            Ok(())
        })?;
    }
    Ok(())
}
