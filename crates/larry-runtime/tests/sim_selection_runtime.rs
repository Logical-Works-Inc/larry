//! larry#64 part 2: one blind selection per plan rides the journal as an ordinary selector run, and
//! the runtime vets a lesson against the journal. There is no lesson store: it waits on Logical-DB.

#[path = "support/milli.rs"]
mod milli;
mod sim_kit;
mod support;

use larry_contracts::authority::Scope;
use larry_contracts::effect::{EffectOutcome, ReconciliationStrategy};
use larry_contracts::event::{CausalRef, EventSequence};
use larry_contracts::evidence::ImproverContext;
use larry_contracts::harness::{HarnessComponent, HarnessHash, HarnessTree};
use larry_contracts::ids::ContentDigest;
use larry_contracts::lesson::{Confidence, Lesson, LessonError};
use larry_contracts::recursion::{
    Label, Produced, RecursionMode, RecursionPlan, RecursionStep, Stage,
};
use larry_contracts::selection::{
    CandidateId, SelectionError, SelectionReason, SelectionStep, SelectorAnswer, prepare,
};
use larry_runtime::error::RuntimeError;
use larry_runtime::journal::Journal;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::projection::{FoldProjection, ProjectionBackend};
use larry_runtime::runtime::{replay as replay_journal, restore};

use milli::{proposal_milli, settings_milli};
use sim_kit::Sim;
use support::{TestRuntime, clock, command_id, operation, run, submit, tenant};

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

fn plan(mode: RecursionMode, seed: u64) -> Sim<RecursionPlan> {
    Ok(RecursionPlan::new(
        ContentDigest::over(&[b"task", &seed.to_be_bytes()]),
        mode,
        1,
        base(),
        4,
    )?)
}

/// A runtime over `journal` whose owner succeeds the seven steps of a one-round plan, then reports
/// `after` for the selector runs.
fn runtime_over(journal: Journal, after: Vec<EffectOutcome>) -> Sim<TestRuntime> {
    let mut outcomes: Vec<EffectOutcome> = (1..=7_u8)
        .map(|number| EffectOutcome::Succeeded {
            operation: operation(number),
        })
        .collect();
    outcomes.extend(after);
    let mut owner = ScriptedOwner::scripted(outcomes);
    for (number, label) in [
        (1_u8, Label::new(0, Stage::Propose, 0)),
        (2, Label::new(0, Stage::Propose, 1)),
        (7, Label::new(0, Stage::Compose, 0)),
    ] {
        owner = owner.producing(operation(number), Produced::Harness(made_by(label)?));
    }
    let mut runtime: TestRuntime = restore(
        settings_milli(&[Scope::FileRead], &[Scope::FileRead], 100)?,
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

/// Runs the plan's seven steps through the runtime.
fn settle(runtime: &mut TestRuntime, plan: RecursionPlan) -> Sim<()> {
    let mut number = 0_u8;
    loop {
        let ledger = runtime.recursion(tenant(1), &plan).cloned();
        let ready = ledger
            .clone()
            .unwrap_or_else(|| larry_contracts::recursion::RecursionLedger::new(plan))
            .next_steps();
        let Some(next) = ready.first() else {
            return Ok(());
        };
        number = number.saturating_add(1);
        let step = RecursionStep::new(plan, next.label, 0);
        let step = if next.label.stage() == Stage::Probe {
            step.with_candidate(
                ledger
                    .as_ref()
                    .and_then(|held| held.candidate(0, next.label.slot() >> 1))
                    .ok_or("candidate")?,
            )
        } else {
            step
        };
        let body = proposal_milli(
            run(number),
            tenant(1),
            operation(number),
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::Queryable,
        )
        .stepping(step);
        runtime.submit(submit(command_id(number), &body)?)?;
        runtime.tick()?;
    }
}

/// Submits a selector run `number` with `answer` and ticks once.
fn select(
    runtime: &mut TestRuntime,
    number: u8,
    plan: RecursionPlan,
    answer: SelectorAnswer,
) -> Sim<()> {
    let body = proposal_milli(
        run(number),
        tenant(1),
        operation(number),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    )
    .selecting(SelectionStep::new(plan, answer));
    runtime.submit(submit(command_id(number), &body)?)?;
    runtime.tick()?;
    Ok(())
}

fn chose(id: u8) -> SelectorAnswer {
    SelectorAnswer::Chose {
        id: CandidateId::new(id),
        reason: "most robust".to_owned(),
    }
}

#[test]
fn sim_selection_runtime_a_settled_plan_selects_once_and_replays() -> Sim<()> {
    for seed in 0..24_u64 {
        support::seeded(seed, || -> Sim<()> {
            let mode = if seed & 1 == 0 {
                RecursionMode::Parallel
            } else {
                RecursionMode::Sequential
            };
            let plan = plan(mode, seed)?;
            let mut runtime = runtime_over(Journal::new(4096)?, vec![succeed(8)])?;
            settle(&mut runtime, plan)?;
            select(&mut runtime, 8, plan, chose(0))?;
            let selection = runtime
                .selection(tenant(1), &plan)
                .cloned()
                .ok_or("a selection")?;
            assert!(
                !selection.is_fallback(),
                "seed {seed}: the selector's answer is used"
            );
            // The same result a pure replay of the plan's ledger gives, from the tenant and the plan.
            let ledger = runtime.recursion(tenant(1), &plan).ok_or("ledger")?;
            let expected = prepare(ledger, tenant(1), ImproverContext::assemble(Vec::new())?)?
                .decide(&chose(0));
            assert_eq!(
                selection, expected,
                "seed {seed}: derived from the plan and the run table's tenant"
            );
            // A second selection is refused before the write.
            let before = runtime.journal().len();
            let body = proposal_milli(
                run(9),
                tenant(1),
                operation(9),
                &[Scope::FileRead],
                1,
                ReconciliationStrategy::Queryable,
            )
            .selecting(SelectionStep::new(plan, chose(1)));
            runtime.submit(submit(command_id(9), &body)?)?;
            let error = runtime.tick().err().ok_or("a refusal")?;
            assert!(
                matches!(
                    error,
                    RuntimeError::Selection(SelectionError::AlreadySelected)
                ),
                "seed {seed}: {error}"
            );
            assert_eq!(
                runtime.journal().len(),
                before,
                "seed {seed}: the journal is unchanged"
            );
            assert!(
                runtime.journal().command(command_id(9)).is_none(),
                "seed {seed}: and holds no command"
            );
            // Replay yields the same winner.
            let mut folded = FoldProjection::default();
            replay_journal(&mut folded, runtime.journal())?;
            assert_eq!(
                folded.selection(tenant(1), &plan),
                Some(&selection),
                "seed {seed}: replay agrees"
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn sim_selection_runtime_the_durable_file_reopens_to_the_same_selection() -> Sim<()> {
    for seed in 0..6_u64 {
        support::seeded(seed, || -> Sim<()> {
            let path = std::env::temp_dir().join(format!(
                "larry-selection-{}-{seed}.journal",
                std::process::id()
            ));
            std::fs::remove_file(&path).ok();
            let plan = plan(RecursionMode::Sequential, seed)?;
            let selection = {
                let mut runtime = runtime_over(Journal::open(&path, 4096)?, vec![succeed(8)])?;
                settle(&mut runtime, plan)?;
                select(&mut runtime, 8, plan, SelectorAnswer::Failed)?;
                runtime
                    .selection(tenant(1), &plan)
                    .cloned()
                    .ok_or("a selection")?
            };
            assert!(
                selection.is_fallback(),
                "seed {seed}: a selector failure is a recorded fallback"
            );
            assert_eq!(
                selection.reason(),
                &SelectionReason::Fallback(larry_contracts::selection::NonAnswer::SelectorFailed),
                "seed {seed}"
            );
            let reopened = Journal::open(&path, 4096)?;
            let mut folded = FoldProjection::default();
            replay_journal(&mut folded, &reopened)?;
            assert_eq!(
                folded.selection(tenant(1), &plan),
                Some(&selection),
                "seed {seed}: the file replays to the same selection"
            );
            std::fs::remove_file(&path).ok();
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn sim_selection_runtime_an_unsettled_plan_is_refused_before_the_write() -> Sim<()> {
    for seed in 0..8_u64 {
        support::seeded(seed, || -> Sim<()> {
            let plan = plan(RecursionMode::Parallel, seed)?;
            let mut runtime = runtime_over(Journal::new(4096)?, vec![succeed(8)])?;
            let before = runtime.journal().len();
            let body = proposal_milli(
                run(8),
                tenant(1),
                operation(8),
                &[Scope::FileRead],
                1,
                ReconciliationStrategy::Queryable,
            )
            .selecting(SelectionStep::new(plan, chose(0)));
            runtime.submit(submit(command_id(8), &body)?)?;
            let error = runtime.tick().err().ok_or("a refusal")?;
            assert!(
                matches!(error, RuntimeError::Selection(SelectionError::NotSettled)),
                "seed {seed}: {error}"
            );
            assert_eq!(
                runtime.journal().len(),
                before,
                "seed {seed}: nothing written"
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn sim_selection_runtime_every_non_answer_falls_back_and_is_recorded() -> Sim<()> {
    use larry_contracts::selection::NonAnswer;
    let cases = [
        (SelectorAnswer::NoAnswer, NonAnswer::NoAnswer),
        (chose(200), NonAnswer::UnknownId),
        (
            SelectorAnswer::Chose {
                id: CandidateId::new(0),
                reason: String::new(),
            },
            NonAnswer::EmptyReason,
        ),
    ];
    for (index, (answer, kind)) in cases.into_iter().enumerate() {
        let plan = plan(RecursionMode::Parallel, u64::try_from(index)?)?;
        let mut runtime = runtime_over(Journal::new(4096)?, vec![succeed(8)])?;
        settle(&mut runtime, plan)?;
        select(&mut runtime, 8, plan, answer)?;
        let selection = runtime.selection(tenant(1), &plan).ok_or("a selection")?;
        assert_eq!(
            selection.reason(),
            &SelectionReason::Fallback(kind),
            "{kind:?} is recorded"
        );
        assert_eq!(
            Some(selection.winner()),
            selection.mapping().first().map(|entry| entry.1),
            "the seed's candidate wins"
        );
    }
    Ok(())
}

#[test]
fn sim_selection_runtime_a_failed_selector_run_records_nothing_and_allows_another() -> Sim<()> {
    let plan = plan(RecursionMode::Parallel, 1)?;
    let mut runtime = runtime_over(
        Journal::new(4096)?,
        vec![
            EffectOutcome::Failed {
                operation: operation(8),
            },
            succeed(9),
        ],
    )?;
    settle(&mut runtime, plan)?;
    select(&mut runtime, 8, plan, chose(0))?;
    assert!(
        runtime.selection(tenant(1), &plan).is_none(),
        "a failed selector run selected nothing"
    );
    select(&mut runtime, 9, plan, chose(0))?;
    assert!(
        runtime.selection(tenant(1), &plan).is_some(),
        "the next one does"
    );
    Ok(())
}

#[test]
fn runtime_vets_a_lesson_against_the_journal_and_the_tenant() -> Sim<()> {
    let plan = plan(RecursionMode::Parallel, 3)?;
    let mut runtime = runtime_over(Journal::new(4096)?, vec![succeed(8)])?;
    settle(&mut runtime, plan)?;
    // Another tenant's run, journalled.
    let other = proposal_milli(
        run(80),
        tenant(2),
        operation(80),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    runtime.submit(submit(command_id(80), &other)?)?;
    runtime.tick()?;
    let lesson_citing = |who: u8, cited: u8| -> Result<Lesson, LessonError> {
        Lesson::new(
            Some(tenant(who)),
            "web-forms",
            "cue",
            "lesson",
            vec![CausalRef::new(run(cited), EventSequence::new(0))],
            Some(Confidence::Medium),
            Some(base()),
        )
    };
    assert!(
        runtime.vet_lesson(&lesson_citing(1, 1)?).is_ok(),
        "a journalled run of the same tenant passes"
    );
    let unknown = runtime
        .vet_lesson(&lesson_citing(1, 99)?)
        .err()
        .ok_or("refused")?;
    assert!(
        matches!(
            unknown,
            RuntimeError::Lesson(LessonError::UnknownRun { .. })
        ),
        "an unknown run: {unknown}"
    );
    let foreign = runtime
        .vet_lesson(&lesson_citing(1, 80)?)
        .err()
        .ok_or("refused")?;
    assert!(
        matches!(
            foreign,
            RuntimeError::Lesson(LessonError::UnknownRun { .. })
        ),
        "another tenant's run: {foreign}"
    );
    assert!(
        runtime.vet_lesson(&lesson_citing(2, 80)?).is_ok(),
        "and it is known to its own tenant"
    );
    Ok(())
}
