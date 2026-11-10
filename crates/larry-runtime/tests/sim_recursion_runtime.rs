//! larry#62 part 2: the recursion ledger rides the runtime fold. A step is an ordinary submitted
//! run whose intent carries a `RecursionStep`; the fold keeps the tenant's ledger, admission asks the
//! same rules before the journal write, and a receipt settles the step. Each step here runs on a
//! runtime restored from the previous journal, so every position between steps is a restart.

mod sim_kit;
mod support;

use std::time::Instant;

use larry_contracts::authority::Scope;
use larry_contracts::effect::{EffectOutcome, ReconciliationStrategy};
use larry_contracts::harness::{HarnessComponent, HarnessHash, HarnessTree};
use larry_contracts::ids::ContentDigest;
use larry_contracts::recursion::{
    Label, Produced, RecursionError, RecursionLedger, RecursionMode, RecursionPlan, RecursionStep,
    Stage, StepState,
};
use larry_runtime::error::RuntimeError;
use larry_runtime::journal::Journal;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::payload::WorkProposal;
use larry_runtime::projection::{FoldProjection, ProjectionBackend};
use larry_runtime::runtime::{replay as replay_journal, restore};

use sim_kit::Sim;
use support::{
    TestRuntime, cancel, clock, command_id, operation, proposal, run, settings, submit, tenant,
};

const MODES: [RecursionMode; 2] = [RecursionMode::Parallel, RecursionMode::Sequential];

fn base() -> HarnessHash {
    HarnessTree::empty().hash()
}

fn made_by(label: Label) -> Sim<HarnessHash> {
    let mut tree = HarnessTree::empty();
    tree.insert(
        HarnessComponent::Memory,
        &format!(
            "n{}-{}-{}",
            label.round(),
            label.stage().tag(),
            label.slot()
        ),
        b"x",
    )?;
    Ok(tree.hash())
}

fn plan(mode: RecursionMode, rounds: u8, width: usize) -> Sim<RecursionPlan> {
    Ok(RecursionPlan::new(
        ContentDigest::over(&[b"pinned task"]),
        mode,
        rounds,
        base(),
        width,
    )?)
}

/// The step reference a ledger would schedule `label` with.
fn reference(
    ledger: Option<&RecursionLedger>,
    plan: RecursionPlan,
    label: Label,
    attempt: u8,
) -> Sim<RecursionStep> {
    let step = RecursionStep::new(plan, label, attempt);
    Ok(match label.stage() {
        Stage::Probe => {
            let candidate = ledger
                .and_then(|held| held.candidate(label.round(), label.slot() >> 1))
                .unwrap_or_else(base);
            step.with_candidate(candidate)
        }
        _ => step,
    })
}

fn body(number: u8, step: RecursionStep) -> WorkProposal {
    proposal(
        run(number),
        tenant(1),
        operation(number),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    )
    .stepping(step)
}

/// Restores a runtime from `journal` with an owner that will report `outcome`, reaches the authority
/// source, submits the step and ticks once.
fn execute(
    journal: Journal,
    number: u8,
    step: RecursionStep,
    outcome: Option<EffectOutcome>,
) -> Sim<TestRuntime> {
    let produced = if step.label.stage() == Stage::Probe {
        None
    } else {
        Some(Produced::Harness(made_by(step.label)?))
    };
    execute_with(journal, number, step, outcome, produced)
}

/// As [`execute`], with the owner reporting `produced` for the step.
fn execute_with(
    journal: Journal,
    number: u8,
    step: RecursionStep,
    outcome: Option<EffectOutcome>,
    produced: Option<Produced>,
) -> Sim<TestRuntime> {
    let mut owner = ScriptedOwner::scripted(outcome);
    if let Some(made) = produced {
        owner = owner.producing(operation(number), made);
    }
    let mut runtime: TestRuntime = restore(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        owner,
        clock()?,
        journal,
    )?;
    let epoch = runtime.fence().epoch()?;
    runtime.fence_mut().authenticate(epoch)?;
    runtime.submit(submit(command_id(number), &body(number, step))?)?;
    runtime.tick()?;
    Ok(runtime)
}

fn empty_journal() -> Sim<Journal> {
    Ok(Journal::new(4096)?)
}

fn ledger_of(runtime: &TestRuntime, plan: &RecursionPlan) -> Option<RecursionLedger> {
    runtime.recursion(tenant(1), plan).cloned()
}

fn succeeded(number: u8) -> Option<EffectOutcome> {
    Some(EffectOutcome::Succeeded {
        operation: operation(number),
    })
}

/// Drives a plan to completion, one restored runtime per step, and returns each stage's timings.
fn drive(seed: u64, timings: &mut [Vec<u128>; 3]) -> Sim<()> {
    let mode = *MODES.get(usize::try_from(seed & 1)?).ok_or("mode")?;
    let rounds = u8::try_from((seed >> 1) % 3)?.saturating_add(1);
    let width = usize::try_from((seed >> 3) % 3)?.saturating_add(1);
    let plan = plan(mode, rounds, width)?;
    let mut journal = empty_journal()?;
    let mut number = 0_u8;
    let mut runtime: Option<TestRuntime> = None;
    loop {
        let held = runtime.as_ref().and_then(|live| ledger_of(live, &plan));
        let ready = held
            .clone()
            .unwrap_or_else(|| RecursionLedger::new(plan))
            .next_steps();
        let Some(next) = ready.first() else { break };
        number = number.saturating_add(1);
        let step = reference(held.as_ref(), plan, next.label, next.attempt)?;
        let outcome = if (seed >> 7).wrapping_add(u64::from(number)) % 7 == 0 {
            Some(EffectOutcome::Failed {
                operation: operation(number),
            })
        } else {
            succeeded(number)
        };
        let started = Instant::now();
        let after = execute(journal, number, step, outcome)?;
        if let Some(slot) = timings.get_mut(usize::from(next.label.stage().tag()) % 3) {
            slot.push(started.elapsed().as_micros());
        }
        journal = after.journal().clone();
        runtime = Some(after);
    }
    let live = runtime
        .as_ref()
        .and_then(|held| ledger_of(held, &plan))
        .ok_or("a ledger")?;
    let count = usize::from(rounds);
    assert!(live.is_complete(), "seed {seed}: complete");
    assert_eq!(
        live.rollouts(),
        count.saturating_mul(4),
        "seed {seed}: R = 4k"
    );
    assert_eq!(
        live.sessions(),
        count.saturating_mul(3),
        "seed {seed}: P = 3k"
    );
    let mut folded = FoldProjection::default();
    replay_journal(&mut folded, &journal)?;
    assert_eq!(
        folded.recursion(tenant(1), &plan),
        Some(&live),
        "seed {seed}: the rebuilt fold equals the live ledger"
    );
    assert_eq!(
        RecursionLedger::replay(plan, live.facts())?,
        live,
        "seed {seed}: and replays from its own facts"
    );
    Ok(())
}

#[test]
fn sim_recursion_runtime_a_plan_completes_through_restarts_at_every_step() -> Sim<()> {
    let mut timings: [Vec<u128>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for seed in 0..48_u64 {
        support::seeded(seed, || drive(seed, &mut timings))?;
    }
    for (stage, mut samples) in timings.into_iter().enumerate() {
        samples.sort_unstable();
        let at = |part: usize| {
            let last = samples.len().saturating_sub(1);
            let rank = samples
                .len()
                .saturating_mul(part)
                .checked_div(100)
                .map_or(last, |wanted| wanted.min(last));
            samples.get(rank).copied()
        };
        let (p50, p99) = (at(50), at(99));
        assert!(
            p50.is_some() && p99.is_some_and(|micros| micros < 2_000_000),
            "stage {stage}: p99 {p99:?} us, p50 {p50:?} us"
        );
    }
    Ok(())
}

/// A step refused before the journal write: the error, and nothing written.
fn refused(runtime: &mut TestRuntime, number: u8, step: RecursionStep) -> Sim<RuntimeError> {
    let before = runtime.journal().len();
    runtime.submit(submit(command_id(number), &body(number, step))?)?;
    let outcome = runtime.tick();
    assert_eq!(
        runtime.journal().len(),
        before,
        "a refused step writes nothing"
    );
    assert!(
        runtime.journal().command(command_id(number)).is_none(),
        "and its command is not journalled"
    );
    outcome.err().ok_or_else(|| "expected a refusal".into())
}

fn is_ledger_refusal(error: &RuntimeError, want: &RecursionError) -> bool {
    matches!(error, RuntimeError::Recursion(got) if got == want)
}

#[test]
fn sim_recursion_runtime_refuses_before_the_journal_write() -> Sim<()> {
    for seed in 0..16_u64 {
        support::seeded(seed, || -> Sim<()> {
            let plan = plan(RecursionMode::Parallel, 1, 1)?;
            let mut journal = empty_journal()?;
            let mut number = 0_u8;
            let mut ledger: Option<RecursionLedger> = None;
            let mut live: Option<TestRuntime> = None;
            // Settle both proposals, then run three probes; the fourth stays ambiguous below.
            let labels = [
                Label::new(0, Stage::Propose, 0),
                Label::new(0, Stage::Propose, 1),
                Label::new(0, Stage::Probe, 0),
                Label::new(0, Stage::Probe, 1),
                Label::new(0, Stage::Probe, 2),
                Label::new(0, Stage::Probe, 3),
                Label::new(0, Stage::Compose, 0),
            ];
            for label in labels {
                number = number.saturating_add(1);
                let step = reference(ledger.as_ref(), plan, label, 0)?;
                let after = execute(journal, number, step, succeeded(number))?;
                ledger = ledger_of(&after, &plan);
                journal = after.journal().clone();
                live = Some(after);
            }
            let mut runtime = live.ok_or("a runtime")?;
            assert_eq!(
                ledger
                    .as_ref()
                    .map(|held| (held.rollouts(), held.sessions())),
                Some((4, 3)),
                "seed {seed}: the identity is spent"
            );
            let duplicate = reference(ledger.as_ref(), plan, Label::new(0, Stage::Probe, 0), 0)?;
            let error = refused(&mut runtime, 100, duplicate)?;
            assert!(
                is_ledger_refusal(
                    &error,
                    &RecursionError::DuplicateLabel {
                        label: Label::new(0, Stage::Probe, 0)
                    }
                ),
                "seed {seed}: duplicate StepKey: {error}"
            );
            let overflow = reference(ledger.as_ref(), plan, Label::new(0, Stage::Probe, 4), 0)?;
            let error = refused(&mut runtime, 101, overflow)?;
            assert!(
                is_ledger_refusal(&error, &RecursionError::RolloutBudget { limit: 4 }),
                "seed {seed}: rollout 4k + 1: {error}"
            );
            let session = reference(ledger.as_ref(), plan, Label::new(0, Stage::Propose, 2), 0)?;
            let error = refused(&mut runtime, 102, session)?;
            assert!(
                is_ledger_refusal(&error, &RecursionError::SessionBudget { limit: 3 }),
                "seed {seed}: session 3k + 1: {error}"
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn sim_recursion_runtime_width_overflow_and_a_wrong_candidate_are_refused() -> Sim<()> {
    for seed in 0..16_u64 {
        support::seeded(seed, || -> Sim<()> {
            let plan = plan(RecursionMode::Parallel, 1, 1)?;
            let mut journal = empty_journal()?;
            let mut ledger: Option<RecursionLedger> = None;
            let mut number = 0_u8;
            for label in [
                Label::new(0, Stage::Propose, 0),
                Label::new(0, Stage::Propose, 1),
            ] {
                number = number.saturating_add(1);
                let step = reference(ledger.as_ref(), plan, label, 0)?;
                let after = execute(journal, number, step, succeeded(number))?;
                ledger = ledger_of(&after, &plan);
                journal = after.journal().clone();
            }
            number = number.saturating_add(1);
            let first = reference(ledger.as_ref(), plan, Label::new(0, Stage::Probe, 0), 0)?;
            let mut runtime = execute(
                journal,
                number,
                first,
                Some(EffectOutcome::Unknown {
                    operation: operation(number),
                }),
            )?;
            ledger = ledger_of(&runtime, &plan);
            let early = reference(ledger.as_ref(), plan, Label::new(0, Stage::Compose, 0), 0)?;
            let error = refused(&mut runtime, 49, early)?;
            assert!(
                is_ledger_refusal(
                    &error,
                    &RecursionError::DependencyUnsettled {
                        label: Label::new(0, Stage::Compose, 0)
                    }
                ),
                "seed {seed}: unsettled dependency: {error}"
            );
            let second = reference(ledger.as_ref(), plan, Label::new(0, Stage::Probe, 1), 0)?;
            let error = refused(&mut runtime, 50, second)?;
            assert!(
                is_ledger_refusal(&error, &RecursionError::WidthExceeded { limit: 1 }),
                "seed {seed}: width overflow: {error}"
            );
            let wrong = RecursionStep::new(plan, Label::new(0, Stage::Probe, 2), 0)
                .with_candidate(made_by(Label::new(9, Stage::Probe, 0))?);
            let error = refused(&mut runtime, 51, wrong)?;
            assert!(
                matches!(
                    error,
                    RuntimeError::Recursion(
                        RecursionError::WidthExceeded { .. }
                            | RecursionError::CandidateMismatch { .. }
                    )
                ),
                "seed {seed}: {error}"
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn sim_recursion_runtime_an_unknown_step_stays_open_across_a_restart_for_every_stage() -> Sim<()> {
    for stage in [Stage::Propose, Stage::Probe, Stage::Compose] {
        for seed in 0..8_u64 {
            support::seeded(seed, || -> Sim<()> {
                let plan = plan(RecursionMode::Sequential, 1, 4)?;
                let mut journal = empty_journal()?;
                let mut ledger: Option<RecursionLedger> = None;
                let mut number = 0_u8;
                loop {
                    let ready = ledger
                        .clone()
                        .unwrap_or_else(|| RecursionLedger::new(plan))
                        .next_steps();
                    let next = ready.into_iter().find(|candidate| {
                        candidate.label.stage() == stage || ledger.as_ref().is_none_or(|_| true)
                    });
                    let Some(next) = next else {
                        return Err("a step".into());
                    };
                    number = number.saturating_add(1);
                    let step = reference(ledger.as_ref(), plan, next.label, 0)?;
                    let hit = next.label.stage() == stage;
                    let outcome = if hit {
                        Some(EffectOutcome::Unknown {
                            operation: operation(number),
                        })
                    } else {
                        succeeded(number)
                    };
                    let after = execute(journal, number, step.clone(), outcome)?;
                    journal = after.journal().clone();
                    ledger = ledger_of(&after, &plan);
                    if !hit {
                        continue;
                    }
                    // Crash here: between the intent and any receipt.
                    let mut folded = FoldProjection::default();
                    replay_journal(&mut folded, &journal)?;
                    let restored = folded
                        .recursion(tenant(1), &plan)
                        .ok_or("restored ledger")?;
                    assert_eq!(
                        Some(restored),
                        ledger.as_ref(),
                        "seed {seed} {stage:?}: replay equals live"
                    );
                    assert!(
                        matches!(restored.state(next.label), Some(StepState::InFlight { .. })),
                        "seed {seed} {stage:?}: open, never succeeded"
                    );
                    assert!(
                        restored
                            .next_steps()
                            .iter()
                            .all(|again| again.label != next.label),
                        "seed {seed} {stage:?}: not issued again"
                    );
                    let mut runtime: TestRuntime = restore(
                        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
                        ScriptedOwner::new(),
                        clock()?,
                        journal.clone(),
                    )?;
                    let again = refused(&mut runtime, 200, step)?;
                    assert!(
                        matches!(
                            again,
                            RuntimeError::Recursion(RecursionError::DuplicateLabel { .. })
                        ),
                        "seed {seed} {stage:?}: no StepKey staged twice: {again}"
                    );
                    let count = restored.rollouts().saturating_add(restored.sessions());
                    assert!(count <= 7, "seed {seed} {stage:?}: counts within 4k + 3k");
                    return Ok(());
                }
            })?;
        }
    }
    Ok(())
}

#[test]
fn sim_recursion_runtime_cancel_stops_new_probes_and_the_slot_frees_when_the_step_settles()
-> Sim<()> {
    for seed in 0..16_u64 {
        support::seeded(seed, || -> Sim<()> {
            let plan = plan(RecursionMode::Parallel, 1, 2)?;
            let mut journal = empty_journal()?;
            let mut ledger: Option<RecursionLedger> = None;
            let mut number = 0_u8;
            for label in [
                Label::new(0, Stage::Propose, 0),
                Label::new(0, Stage::Propose, 1),
            ] {
                number = number.saturating_add(1);
                let step = reference(ledger.as_ref(), plan, label, 0)?;
                let after = execute(journal, number, step, succeeded(number))?;
                ledger = ledger_of(&after, &plan);
                journal = after.journal().clone();
            }
            number = number.saturating_add(1);
            let probe = reference(ledger.as_ref(), plan, Label::new(0, Stage::Probe, 0), 0)?;
            let held = body(number, probe);
            let mut runtime = execute(
                journal,
                number,
                held.recursion().cloned().ok_or("step")?,
                Some(EffectOutcome::Unknown {
                    operation: operation(number),
                }),
            )?;
            assert_eq!(
                ledger_of(&runtime, &plan).ok_or("ledger")?.open_probes(),
                1,
                "seed {seed}: the probe holds a slot"
            );
            runtime.submit(cancel(command_id(90), &held)?)?;
            runtime.tick()?;
            let after = ledger_of(&runtime, &plan).ok_or("ledger")?;
            assert!(after.is_cancelled(), "seed {seed}: the plan stops");
            assert_eq!(
                after.open_probes(),
                1,
                "seed {seed}: the unsettled probe holds its slot until it settles"
            );
            assert!(after.next_steps().is_empty(), "seed {seed}: no new probes");
            let next = reference(Some(&after), plan, Label::new(0, Stage::Probe, 1), 0)?;
            let error = refused(&mut runtime, 91, next)?;
            assert!(
                is_ledger_refusal(&error, &RecursionError::Cancelled),
                "seed {seed}: {error}"
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn sim_recursion_runtime_a_tenant_holds_a_bounded_number_of_plans() -> Sim<()> {
    for seed in 0..4_u64 {
        support::seeded(seed, || -> Sim<()> {
            let mut journal = empty_journal()?;
            let mut number = 0_u8;
            for variant in 0..16_u8 {
                number = number.saturating_add(1);
                let task = ContentDigest::over(&[b"task", &[variant]]);
                let one = RecursionPlan::new(task, RecursionMode::Parallel, 1, base(), 1)?;
                let step = reference(None, one, Label::new(0, Stage::Propose, 0), 0)?;
                journal = execute(journal, number, step, succeeded(number))?
                    .journal()
                    .clone();
            }
            let extra = RecursionPlan::new(
                ContentDigest::over(&[b"task", &[99]]),
                RecursionMode::Parallel,
                1,
                base(),
                1,
            )?;
            let step = reference(None, extra, Label::new(0, Stage::Propose, 0), 0)?;
            let mut runtime: TestRuntime = restore(
                settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
                ScriptedOwner::new(),
                clock()?,
                journal,
            )?;
            let error = refused(&mut runtime, 200, step)?;
            assert!(
                matches!(error, RuntimeError::RecursionPlanLimit { capacity: 16 }),
                "seed {seed}: {error}"
            );
            Ok(())
        })?;
    }
    Ok(())
}

fn durable_path(name: &str, seed: u64) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "larry-recursion-{}-{name}-{seed}.journal",
        std::process::id()
    ));
    std::fs::remove_file(&path).ok();
    path
}

/// One step on a runtime restored from the journal file at `path`: opened from disk, run, dropped.
fn execute_on_disk(
    path: &std::path::Path,
    number: u8,
    step: RecursionStep,
    outcome: Option<EffectOutcome>,
    produced: Option<Produced>,
) -> Sim<()> {
    let journal = Journal::open(path, 4096)?;
    let after = execute_with(journal, number, step, outcome, produced)?;
    drop(after);
    Ok(())
}

fn on_disk_ledger(path: &std::path::Path, plan: &RecursionPlan) -> Sim<Option<RecursionLedger>> {
    let journal = Journal::open(path, 4096)?;
    let mut folded = FoldProjection::default();
    replay_journal(&mut folded, &journal)?;
    Ok(folded.recursion(tenant(1), plan).cloned())
}

#[test]
fn sim_recursion_runtime_the_durable_file_restarts_between_intent_and_receipt_for_every_stage()
-> Sim<()> {
    for stage in [Stage::Propose, Stage::Probe, Stage::Compose] {
        for seed in 0..6_u64 {
            support::seeded(seed, || -> Sim<()> {
                let plan = plan(RecursionMode::Sequential, 1, 4)?;
                let path = durable_path("restart", seed.wrapping_add(u64::from(stage.tag()) << 8));
                let mut number = 0_u8;
                loop {
                    let held = on_disk_ledger(&path, &plan).ok().flatten();
                    let ready = held
                        .clone()
                        .unwrap_or_else(|| RecursionLedger::new(plan))
                        .next_steps();
                    let next = ready.first().ok_or("a step")?.clone();
                    number = number.saturating_add(1);
                    let step = reference(held.as_ref(), plan, next.label, 0)?;
                    let hit = next.label.stage() == stage;
                    let outcome = if hit {
                        Some(EffectOutcome::Unknown {
                            operation: operation(number),
                        })
                    } else {
                        succeeded(number)
                    };
                    let produced =
                        (next.label.stage() != Stage::Probe).then(|| Produced::Harness(base()));
                    execute_on_disk(&path, number, step.clone(), outcome, produced)?;
                    if !hit {
                        continue;
                    }
                    // The process died here. Everything below is read back from the file.
                    let restored = on_disk_ledger(&path, &plan)?.ok_or("a ledger on disk")?;
                    assert!(
                        matches!(restored.state(next.label), Some(StepState::InFlight { .. })),
                        "seed {seed} {stage:?}: open, never succeeded"
                    );
                    assert!(
                        restored
                            .next_steps()
                            .iter()
                            .all(|again| again.label != next.label),
                        "seed {seed} {stage:?}: not issued again"
                    );
                    let before = Journal::open(&path, 4096)?.len();
                    let mut runtime: TestRuntime = restore(
                        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
                        ScriptedOwner::new(),
                        clock()?,
                        Journal::open(&path, 4096)?,
                    )?;
                    let again = refused(&mut runtime, 200, step)?;
                    assert!(
                        matches!(
                            again,
                            RuntimeError::Recursion(RecursionError::DuplicateLabel { .. })
                        ),
                        "seed {seed} {stage:?}: no StepKey staged twice: {again}"
                    );
                    drop(runtime);
                    assert_eq!(
                        Journal::open(&path, 4096)?.len(),
                        before,
                        "seed {seed} {stage:?}: the file did not grow"
                    );
                    std::fs::remove_file(&path).ok();
                    return Ok(());
                }
            })?;
        }
    }
    Ok(())
}

#[test]
fn sim_recursion_runtime_an_invalid_proposal_retries_once_then_falls_back() -> Sim<()> {
    use larry_contracts::harness::HarnessError;
    for seed in 0..8_u64 {
        support::seeded(seed, || -> Sim<()> {
            let plan = plan(RecursionMode::Parallel, 1, 2)?;
            let label = Label::new(0, Stage::Propose, 0);
            let refusal = Produced::Refused(HarnessError::OffSurface);
            // First attempt: the proposer's edit fails the harness reducer.
            let first = execute_with(
                empty_journal()?,
                1,
                RecursionStep::new(plan, label, 0),
                succeeded(1),
                Some(refusal.clone()),
            )?;
            let ledger = ledger_of(&first, &plan).ok_or("ledger")?;
            assert!(
                matches!(ledger.state(label), Some(StepState::RetryPending { .. })),
                "seed {seed}: one retry is pending"
            );
            let retry = ledger
                .next_steps()
                .into_iter()
                .find(|step| step.label == label)
                .ok_or("retry offered")?;
            assert_eq!(
                (retry.attempt, retry.carried),
                (1, Some(HarnessError::OffSurface)),
                "seed {seed}: it carries the typed error"
            );
            // The retry is refused too: fall back to the round's base.
            let second = execute_with(
                first.journal().clone(),
                2,
                RecursionStep::new(plan, label, 1),
                succeeded(2),
                Some(refusal),
            )?;
            let ledger = ledger_of(&second, &plan).ok_or("ledger")?;
            assert_eq!(
                ledger.state(label),
                Some(&StepState::FellBack),
                "seed {seed}: fallback is a recorded fact"
            );
            assert_eq!(
                ledger.candidate(0, 0),
                Some(base()),
                "seed {seed}: the round's base harness"
            );
            assert_eq!(
                ledger.sessions(),
                1,
                "seed {seed}: the retry is the same session"
            );
            let mut folded = FoldProjection::default();
            replay_journal(&mut folded, second.journal())?;
            assert_eq!(
                folded.recursion(tenant(1), &plan),
                Some(&ledger),
                "seed {seed}: the rebuilt fold agrees"
            );
            // A third attempt is not allowed.
            let mut runtime = second;
            let error = refused(&mut runtime, 3, RecursionStep::new(plan, label, 1))?;
            assert!(
                matches!(error, RuntimeError::Recursion(_)),
                "seed {seed}: {error}"
            );
            // A succeeded proposer that reports no harness produced nothing: it falls back too.
            let other = Label::new(0, Stage::Propose, 1);
            let none = execute_with(
                runtime.journal().clone(),
                4,
                RecursionStep::new(plan, other, 0),
                succeeded(4),
                None,
            )?;
            let ledger = ledger_of(&none, &plan).ok_or("ledger")?;
            assert_eq!(
                ledger.state(other),
                Some(&StepState::Failed),
                "seed {seed}: no produced hash is a failed session"
            );
            assert_eq!(
                ledger.candidate(0, 1),
                Some(base()),
                "seed {seed}: and the candidate is the base"
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn sim_recursion_runtime_cancel_with_an_unknown_effect_releases_the_slot_when_reconciliation_settles_it()
-> Sim<()> {
    for seed in 0..8_u64 {
        support::seeded(seed, || -> Sim<()> {
            let plan = plan(RecursionMode::Parallel, 1, 2)?;
            let mut journal = empty_journal()?;
            let mut ledger: Option<RecursionLedger> = None;
            let mut number = 0_u8;
            for label in [
                Label::new(0, Stage::Propose, 0),
                Label::new(0, Stage::Propose, 1),
            ] {
                number = number.saturating_add(1);
                let step = reference(ledger.as_ref(), plan, label, 0)?;
                let after = execute(journal, number, step, succeeded(number))?;
                ledger = ledger_of(&after, &plan);
                journal = after.journal().clone();
            }
            number = number.saturating_add(1);
            let probe = body(
                number,
                reference(ledger.as_ref(), plan, Label::new(0, Stage::Probe, 0), 0)?,
            );
            // The owner first says unknown, and answers the status query with success.
            let owner = ScriptedOwner::scripted([
                EffectOutcome::Unknown {
                    operation: operation(number),
                },
                EffectOutcome::Succeeded {
                    operation: operation(number),
                },
            ]);
            let mut runtime: TestRuntime = restore(
                settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
                owner,
                clock()?,
                journal,
            )?;
            let epoch = runtime.fence().epoch()?;
            runtime.fence_mut().authenticate(epoch)?;
            runtime.submit(submit(command_id(number), &probe)?)?;
            runtime.tick()?;
            assert_eq!(
                ledger_of(&runtime, &plan).ok_or("ledger")?.open_probes(),
                1,
                "seed {seed}: unknown holds the slot"
            );
            runtime.submit(cancel(command_id(90), &probe)?)?;
            let mut ticks = 0_u8;
            while ledger_of(&runtime, &plan).ok_or("ledger")?.open_probes() > 0 && ticks < 6 {
                runtime.tick()?;
                ticks = ticks.saturating_add(1);
            }
            let after = ledger_of(&runtime, &plan).ok_or("ledger")?;
            assert!(after.is_cancelled(), "seed {seed}: the plan stopped");
            assert_eq!(
                after.open_probes(),
                0,
                "seed {seed}: reconciliation settled it and the slot is free"
            );
            assert!(
                after.next_steps().is_empty(),
                "seed {seed}: and no new probe is offered"
            );
            Ok(())
        })?;
    }
    Ok(())
}
