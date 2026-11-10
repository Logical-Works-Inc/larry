//! larry#62 part 1: the recursion budget identity is a reducer, not a final assert. One seed
//! decides outcome order, failures, unknowns, proposer validity, cancel points and probe width;
//! same seed, same trace hash. No clock, no threads, no hash maps.

use std::collections::BTreeSet;
use std::time::Instant;

use larry_contracts::evidence::{Evidence, Execution, ImproverContext, ImproverItem};
use larry_contracts::harness::{
    HarnessComponent, HarnessEdit, HarnessError, HarnessHash, HarnessOp, HarnessPolicy,
    HarnessTree, validate_edit,
};
use larry_contracts::ids::ContentDigest;
use larry_contracts::recursion::{
    Label, MAX_K, PlanError, RecursionError, RecursionFact, RecursionLedger, RecursionMode,
    RecursionPlan, RecursionStep, Refusal, Stage, Step, StepOutcome, StepState,
};

type Outcome = Result<(), Box<dyn std::error::Error>>;

const MODES: [RecursionMode; 2] = [RecursionMode::Parallel, RecursionMode::Sequential];

fn base() -> HarnessHash {
    HarnessTree::empty().hash()
}

/// A distinct hash per label, standing in for what a proposer or composer produced.
fn made_by(label: Label) -> Result<HarnessHash, HarnessError> {
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

fn plan(mode: RecursionMode, rounds: u8, width: usize) -> Result<RecursionPlan, PlanError> {
    RecursionPlan::new(
        ContentDigest::over(&[b"pinned task"]),
        mode,
        rounds,
        base(),
        width,
    )
}

fn success(label: Label) -> Result<StepOutcome, HarnessError> {
    let produced = if label.stage() == Stage::Probe {
        None
    } else {
        Some(made_by(label)?)
    };
    Ok(StepOutcome::Succeeded { produced })
}

#[test]
fn sim_recursion_every_k_in_both_modes_totals_the_identity_and_refuses_one_more() -> Outcome {
    for mode in MODES {
        for rounds in 1..=MAX_K {
            let mut ledger = RecursionLedger::new(plan(mode, rounds, 3)?);
            loop {
                let ready = ledger.next_steps();
                if ready.is_empty() {
                    break;
                }
                for step in ready {
                    ledger.record_intent(step.label, step.attempt)?;
                    ledger.record_outcome(step.label, success(step.label)?)?;
                }
            }
            let count = usize::from(rounds);
            let (rollouts, sessions) = (count.saturating_mul(4), count.saturating_mul(3));
            assert!(ledger.is_complete(), "{mode:?} k={rounds}: complete");
            assert_eq!(ledger.rollouts(), rollouts, "{mode:?} k={rounds}: R = 4k");
            assert_eq!(ledger.sessions(), sessions, "{mode:?} k={rounds}: P = 3k");
            let before = ledger.clone();
            assert_eq!(
                ledger.record_intent(Label::new(0, Stage::Probe, 4), 0),
                Err(RecursionError::RolloutBudget { limit: rollouts }),
                "{mode:?} k={rounds}: rollout 4k + 1"
            );
            assert_eq!(
                ledger.record_intent(Label::new(0, Stage::Propose, 2), 0),
                Err(RecursionError::SessionBudget { limit: sessions }),
                "{mode:?} k={rounds}: session 3k + 1"
            );
            assert_eq!(
                ledger.record_intent(Label::new(0, Stage::Probe, 0), 0),
                Err(RecursionError::DuplicateLabel {
                    label: Label::new(0, Stage::Probe, 0)
                }),
                "{mode:?} k={rounds}: a duplicate is refused"
            );
            assert_eq!(
                ledger, before,
                "{mode:?} k={rounds}: refusals count nothing"
            );
        }
    }
    assert_eq!(
        plan(RecursionMode::Parallel, 0, 1),
        Err(PlanError::Rounds { got: 0 }),
        "k = 0"
    );
    assert_eq!(
        plan(RecursionMode::Parallel, 9, 1),
        Err(PlanError::Rounds { got: 9 }),
        "k = 9"
    );
    assert_eq!(
        plan(RecursionMode::Parallel, 1, 0),
        Err(PlanError::Width { got: 0 }),
        "width 0"
    );
    assert_eq!(
        plan(RecursionMode::Parallel, 1, 17),
        Err(PlanError::Width { got: 17 }),
        "width 17"
    );
    Ok(())
}

/// A seeded stream: wrapping arithmetic only.
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ (mixed >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        let (Ok(wide), true) = (u64::try_from(bound), bound > 0) else {
            return 0;
        };
        let scaled = (self.next() >> 32).wrapping_mul(wide) >> 32;
        let Ok(value) = usize::try_from(scaled) else {
            return 0;
        };
        value
    }

    /// True about `part` times in `of`.
    fn chance(&mut self, part: usize, of: usize) -> bool {
        self.below(of) < part
    }
}

/// What one seeded run left behind.
struct Run {
    ledger: RecursionLedger,
    trace: ContentDigest,
    cancelled_at_idle: bool,
}

fn trace_of(facts: &[RecursionFact]) -> ContentDigest {
    facts
        .iter()
        .fold(ContentDigest::over(&[b"trace"]), |digest, fact| {
            ContentDigest::over(&[digest.as_bytes(), format!("{fact:?}").as_bytes()])
        })
}

/// Drives a ledger to quiescence under one seed, checking the invariants after every move.
fn drive(seed: u64) -> Result<Run, Box<dyn std::error::Error>> {
    let mut rng = Seeded(seed);
    let mode = *MODES.get(rng.below(2)).ok_or("mode")?;
    let rounds = u8::try_from(rng.below(usize::from(MAX_K)))?.saturating_add(1);
    let width = rng.below(6).saturating_add(1);
    let mut ledger = RecursionLedger::new(plan(mode, rounds, width)?);
    let cancel_after = rng.chance(1, 4).then(|| rng.below(60));
    let (mut open, mut ambiguous): (Vec<Step>, Vec<Label>) = (Vec::new(), Vec::new());
    let mut counted = BTreeSet::new();
    let mut moves = 0_usize;
    let limit = usize::from(rounds).saturating_mul(56);
    while moves < limit.saturating_mul(4) {
        moves = moves.saturating_add(1);
        if cancel_after == Some(moves) && !ledger.is_cancelled() {
            ledger.cancel();
            assert!(
                ledger.next_steps().is_empty(),
                "seed {seed}: cancel stops new steps"
            );
            assert_eq!(
                ledger
                    .record_intent(Label::new(0, Stage::Probe, 0), 0)
                    .err(),
                Some(RecursionError::Cancelled),
                "seed {seed}: and refuses an intent"
            );
        }
        let ready = ledger.next_steps();
        let start = !ready.is_empty() && (open.is_empty() || rng.chance(1, 2));
        if start {
            let step = ready.get(rng.below(ready.len())).ok_or("step")?.clone();
            if step.attempt == 1 {
                assert_eq!(
                    step.carried,
                    Some(HarnessError::OffSurface),
                    "seed {seed}: retry carries the error"
                );
            }
            if step.label.stage() == Stage::Propose
                && step.label.round() > 0
                && mode == RecursionMode::Sequential
            {
                let before = Label::new(step.label.round().saturating_sub(1), Stage::Compose, 0);
                assert!(
                    ledger.state(before).is_some_and(StepState::is_settled),
                    "seed {seed}: nested rounds wait"
                );
            }
            ledger.record_intent(step.label, step.attempt)?;
            if step.attempt == 0 {
                assert!(
                    counted.insert(step.label),
                    "seed {seed}: no label counted twice"
                );
            }
            open.push(step);
        } else if !open.is_empty() {
            let at = rng.below(open.len());
            let step = open.swap_remove(at);
            let label = step.label;
            if label.stage() == Stage::Propose && rng.chance(1, 4) {
                match ledger.record_refusal(label, HarnessError::OffSurface)? {
                    Refusal::Retry => assert!(
                        ledger.is_cancelled()
                            || ledger
                                .next_steps()
                                .iter()
                                .any(|next| next.label == label && next.attempt == 1),
                        "seed {seed}"
                    ),
                    Refusal::FellBack => assert_eq!(
                        ledger.state(label),
                        Some(&StepState::FellBack),
                        "seed {seed}"
                    ),
                    _ => return Err("unknown refusal".into()),
                }
            } else if rng.chance(1, 8) {
                ledger.record_outcome(label, StepOutcome::Unknown)?;
                assert_eq!(
                    ledger.state(label),
                    Some(&StepState::Ambiguous),
                    "seed {seed}: unknown stays ambiguous"
                );
                ambiguous.push(label);
            } else if rng.chance(1, 8) {
                ledger.record_outcome(label, StepOutcome::Failed)?;
            } else {
                ledger.record_outcome(label, success(label)?)?;
            }
        } else if let Some(label) = ambiguous.pop() {
            let outcome = if rng.chance(1, 2) {
                success(label)?
            } else {
                StepOutcome::Failed
            };
            ledger.record_outcome(label, outcome)?;
        } else {
            break;
        }
        assert!(
            ledger.rollouts() <= ledger.plan().rollout_limit(),
            "seed {seed}: rollouts within 4k"
        );
        assert!(
            ledger.sessions() <= ledger.plan().session_limit(),
            "seed {seed}: sessions within 3k"
        );
        assert!(
            ledger.open_probes() <= width,
            "seed {seed}: width never exceeded"
        );
    }
    let facts = ledger.facts();
    assert_eq!(
        RecursionLedger::replay(*ledger.plan(), facts)?,
        ledger,
        "seed {seed}: replay equals live"
    );
    if !ledger.is_cancelled() {
        assert!(
            ledger.is_complete(),
            "seed {seed}: an uncancelled run completes"
        );
        assert_eq!(
            ledger.rollouts(),
            usize::from(rounds).saturating_mul(4),
            "seed {seed}: R = 4k"
        );
        assert_eq!(
            ledger.sessions(),
            usize::from(rounds).saturating_mul(3),
            "seed {seed}: P = 3k"
        );
    }
    Ok(Run {
        trace: trace_of(facts),
        cancelled_at_idle: ledger.is_cancelled(),
        ledger,
    })
}

#[test]
fn sim_recursion_seeded_driver_holds_every_invariant_and_replays() -> Outcome {
    let seeds = 1_200_u64;
    let started = Instant::now();
    let mut cancelled = 0_usize;
    for seed in 0..seeds {
        let (first, second) = (drive(seed)?, drive(seed)?);
        assert_eq!(
            first.trace, second.trace,
            "seed {seed}: same seed, same trace hash"
        );
        assert_eq!(first.ledger, second.ledger, "seed {seed}: same ledger");
        cancelled = cancelled.saturating_add(usize::from(first.cancelled_at_idle));
    }
    assert!(
        cancelled > 0 && cancelled < 1_200,
        "cancel points are exercised: {cancelled}"
    );
    // Each seed runs twice; a generous floor that only a broken loop would miss.
    let millis = started.elapsed().as_millis().max(1);
    assert!(
        u128::from(seeds)
            .saturating_mul(2_000)
            .checked_div(millis)
            .is_some_and(|rate| rate > 20),
        "{seeds} seeds twice in {millis} ms"
    );
    Ok(())
}

#[test]
fn sim_recursion_different_seeds_make_different_traces() -> Outcome {
    let traces: BTreeSet<_> = (0..64_u64)
        .map(|seed| drive(seed).map(|run| run.trace))
        .collect::<Result<_, _>>()?;
    assert!(
        traces.len() > 32,
        "seeds drive different worlds: {}",
        traces.len()
    );
    Ok(())
}

#[test]
fn recursion_a_duplicate_label_is_refused_before_it_is_counted() -> Outcome {
    let mut ledger = RecursionLedger::new(plan(RecursionMode::Parallel, 2, 4)?);
    let label = Label::new(0, Stage::Propose, 0);
    ledger.record_intent(label, 0)?;
    ledger.record_outcome(label, success(label)?)?;
    let probe = Label::new(0, Stage::Probe, 0);
    ledger.record_intent(probe, 0)?;
    let (sessions, rollouts, facts) = (ledger.sessions(), ledger.rollouts(), ledger.facts().len());
    assert_eq!(
        ledger.record_intent(probe, 0),
        Err(RecursionError::DuplicateLabel { label: probe }),
        "twice"
    );
    assert_eq!(
        ledger.record_intent(label, 0),
        Err(RecursionError::DuplicateLabel { label }),
        "settled, still a duplicate"
    );
    assert_eq!(
        (ledger.sessions(), ledger.rollouts(), ledger.facts().len()),
        (sessions, rollouts, facts),
        "nothing counted or recorded"
    );
    assert_eq!(
        ledger.record_intent(Label::new(0, Stage::Probe, 2), 0),
        Err(RecursionError::DependencyUnsettled {
            label: Label::new(0, Stage::Probe, 2)
        }),
        "a probe waits for its candidate"
    );
    assert_eq!(
        ledger.record_intent(Label::new(2, Stage::Propose, 0), 0),
        Err(RecursionError::NotInPlan {
            label: Label::new(2, Stage::Propose, 0)
        }),
        "round 2 is not in a k = 2 plan"
    );
    assert_ne!(
        ledger.plan().step_key(label),
        ledger.plan().step_key(probe),
        "keys differ by label"
    );
    assert_eq!(
        ledger.plan().step_key(label),
        ledger.plan().step_key(Label::new(0, Stage::Propose, 0)),
        "and are stable"
    );
    Ok(())
}

#[test]
fn recursion_an_invalid_proposal_retries_once_with_the_typed_error_then_falls_back() -> Outcome {
    let mut ledger = RecursionLedger::new(plan(RecursionMode::Sequential, 1, 2)?);
    let label = Label::new(0, Stage::Propose, 0);
    // The refusal is the real one from the harness reducer.
    let edit = HarnessEdit::new(vec![HarnessOp::Put {
        path: "bin/x".to_owned(),
        bytes: vec![1],
    }])?;
    let refusal = validate_edit(&HarnessTree::empty(), &edit, &HarnessPolicy::new())
        .err()
        .ok_or("refused")?;
    assert_eq!(
        refusal,
        HarnessError::OffSurface,
        "the reducer's specific error"
    );
    ledger.record_intent(label, 0)?;
    assert_eq!(
        ledger.record_refusal(label, refusal.clone())?,
        Refusal::Retry,
        "first refusal: one retry"
    );
    let retry = ledger
        .next_steps()
        .into_iter()
        .find(|step| step.label == label)
        .ok_or("retry offered")?;
    assert_eq!(
        (retry.attempt, retry.carried),
        (1, Some(refusal)),
        "the retry carries the error"
    );
    assert_eq!(
        ledger.record_intent(label, 2),
        Err(RecursionError::NotAwaitingRetry { label }),
        "only one retry"
    );
    ledger.record_intent(label, 1)?;
    assert_eq!(
        ledger.record_intent(label, 1),
        Err(RecursionError::NotAwaitingRetry { label }),
        "not twice"
    );
    let second = HarnessError::InvalidName {
        component: HarnessComponent::Skills,
    };
    assert_eq!(
        ledger.record_refusal(label, second)?,
        Refusal::FellBack,
        "second refusal: fall back"
    );
    assert_eq!(
        ledger.state(label),
        Some(&StepState::FellBack),
        "recorded as a fact"
    );
    assert!(
        matches!(ledger.facts().last(), Some(RecursionFact::FellBack { .. })),
        "the last fact is the fallback"
    );
    assert_eq!(
        ledger.candidate(0, 0),
        Some(base()),
        "the candidate is the round's base harness"
    );
    assert_eq!(
        ledger.sessions(),
        1,
        "the retry is the same session, counted once"
    );
    assert_eq!(
        ledger
            .record_refusal(Label::new(0, Stage::Probe, 0), HarnessError::OffSurface)
            .err(),
        Some(RecursionError::NotProposal {
            label: Label::new(0, Stage::Probe, 0)
        }),
        "only proposals"
    );
    assert_eq!(
        RecursionLedger::replay(*ledger.plan(), ledger.facts())?,
        ledger,
        "and it replays"
    );
    Ok(())
}

#[test]
fn recursion_unknown_is_ambiguous_blocks_its_dependents_and_is_never_rescheduled() -> Outcome {
    let mut ledger = RecursionLedger::new(plan(RecursionMode::Parallel, 1, 4)?);
    for slot in 0..2 {
        let label = Label::new(0, Stage::Propose, slot);
        ledger.record_intent(label, 0)?;
        ledger.record_outcome(label, success(label)?)?;
    }
    let mut probes = Vec::new();
    for slot in 0..4 {
        let label = Label::new(0, Stage::Probe, slot);
        ledger.record_intent(label, 0)?;
        probes.push(label);
    }
    ledger.record_outcome(Label::new(0, Stage::Probe, 0), StepOutcome::Unknown)?;
    for label in probes.iter().skip(1) {
        ledger.record_outcome(*label, StepOutcome::Failed)?;
    }
    assert_eq!(
        ledger.state(Label::new(0, Stage::Probe, 0)),
        Some(&StepState::Ambiguous),
        "ambiguous"
    );
    assert_eq!(
        ledger.open_probes(),
        1,
        "an ambiguous probe still holds an executor"
    );
    assert!(
        ledger.next_steps().is_empty(),
        "the composer waits and nothing is rescheduled"
    );
    let compose = Label::new(0, Stage::Compose, 0);
    assert_eq!(
        ledger.record_intent(compose, 0),
        Err(RecursionError::DependencyUnsettled { label: compose }),
        "blocked"
    );
    ledger.record_outcome(Label::new(0, Stage::Probe, 0), StepOutcome::Unknown)?;
    ledger.record_outcome(
        Label::new(0, Stage::Probe, 0),
        StepOutcome::Succeeded { produced: None },
    )?;
    assert_eq!(
        ledger.next_steps().len(),
        1,
        "a reconciliation unblocks the composer"
    );
    assert_eq!(
        ledger.record_outcome(Label::new(0, Stage::Probe, 0), StepOutcome::Failed),
        Err(RecursionError::NotInFlight {
            label: Label::new(0, Stage::Probe, 0)
        }),
        "a settled step does not settle again"
    );
    let context = ImproverContext::assemble(vec![ImproverItem::new(Evidence::<Execution>::new(
        "rollout/a",
        b"exit 0",
    )?)])?;
    let input = ledger.compose_input(compose, context.clone())?;
    assert_eq!(
        (input.label(), input.base(), input.context()),
        (compose, base(), &context),
        "the composer's door is an improver context"
    );
    assert_eq!(
        ledger
            .compose_input(Label::new(0, Stage::Probe, 0), context)
            .err(),
        Some(RecursionError::NotComposable {
            label: Label::new(0, Stage::Probe, 0)
        }),
        "only a compose step"
    );
    Ok(())
}

#[test]
fn sim_recursion_probe_width_queues_overflow_without_dropping_any() -> Outcome {
    for width in 1..=4_usize {
        let mut ledger = RecursionLedger::new(plan(RecursionMode::Parallel, 3, width)?);
        for round in 0..3 {
            for slot in 0..2 {
                let label = Label::new(round, Stage::Propose, slot);
                ledger.record_intent(label, 0)?;
                ledger.record_outcome(label, success(label)?)?;
            }
        }
        let offered = ledger.next_steps();
        assert_eq!(
            offered.len(),
            width,
            "width {width}: only the free width is offered"
        );
        for step in &offered {
            ledger.record_intent(step.label, 0)?;
        }
        let overflow = Label::new(2, Stage::Probe, 3);
        assert_eq!(
            ledger.record_intent(overflow, 0),
            Err(RecursionError::WidthExceeded { limit: width }),
            "width {width}: full"
        );
        assert!(
            ledger.next_steps().is_empty(),
            "width {width}: nothing more until one settles"
        );
        let first = offered.first().ok_or("one offered")?.label;
        ledger.record_outcome(first, success(first)?)?;
        assert_eq!(
            ledger.next_steps().len(),
            1,
            "width {width}: a freed slot admits exactly one"
        );
    }
    Ok(())
}

#[test]
fn sim_recursion_a_crash_at_every_position_leaves_the_step_open_and_unrepeated() -> Outcome {
    for seed in 0..48_u64 {
        let run = drive(seed)?;
        let facts = run.ledger.facts();
        for cut in 0..=facts.len() {
            let prefix = facts.get(..cut).ok_or("prefix")?;
            let restored = RecursionLedger::replay(*run.ledger.plan(), prefix)?;
            for (index, fact) in prefix.iter().enumerate() {
                let RecursionFact::IntentRecorded { label, attempt } = *fact else {
                    continue;
                };
                let rest = prefix.get(index.saturating_add(1)..).ok_or("rest")?;
                let reported = rest.iter().any(|later| match *later {
                    RecursionFact::OutcomeRecorded { label: seen, .. }
                    | RecursionFact::ProposalRefused { label: seen, .. } => seen == label,
                    _ => false,
                });
                let state = restored.state(label).ok_or("an intent has a state")?;
                if !reported {
                    assert_eq!(
                        state,
                        &StepState::InFlight { attempt },
                        "seed {seed} cut {cut}: in flight, never done"
                    );
                }
                assert!(
                    !matches!(state, StepState::Done { .. }) || reported,
                    "seed {seed} cut {cut}: never succeeded without an outcome"
                );
            }
            for step in restored.next_steps() {
                let again = restored.state(step.label);
                assert!(
                    again.is_none()
                        || (step.attempt == 1
                            && matches!(again, Some(StepState::RetryPending { .. }))),
                    "seed {seed} cut {cut}: no step with an intent is issued again"
                );
            }
            let mut resumed = restored.clone();
            for step in resumed.next_steps() {
                resumed.record_intent(step.label, step.attempt)?;
            }
        }
    }
    Ok(())
}

#[test]
fn recursion_facts_and_steps_roundtrip_through_their_encoding_and_stay_small() -> Outcome {
    let label = Label::new(7, Stage::Compose, 0);
    let hash = made_by(label)?;
    let facts = [
        RecursionFact::IntentRecorded { label, attempt: 1 },
        RecursionFact::OutcomeRecorded {
            label,
            outcome: StepOutcome::Succeeded {
                produced: Some(hash),
            },
        },
        RecursionFact::OutcomeRecorded {
            label,
            outcome: StepOutcome::Succeeded { produced: None },
        },
        RecursionFact::OutcomeRecorded {
            label,
            outcome: StepOutcome::Failed,
        },
        RecursionFact::OutcomeRecorded {
            label,
            outcome: StepOutcome::Unknown,
        },
        RecursionFact::ProposalRefused {
            label,
            error: HarnessError::OffSurface,
        },
        RecursionFact::ProposalRefused {
            label,
            error: HarnessError::Reserved {
                component: HarnessComponent::Scripts,
                name: "x".repeat(64),
            },
        },
        RecursionFact::ProposalRefused {
            label,
            error: HarnessError::Oversize {
                component: HarnessComponent::Plugins,
                limit: 65_536,
                actual: usize::MAX,
            },
        },
        RecursionFact::FellBack { label },
        RecursionFact::Cancelled,
    ];
    let mut widest = 0_usize;
    for fact in facts {
        let bytes = lgwks_std::json::to_vec(&fact)?;
        widest = widest.max(bytes.len());
        let back: RecursionFact = lgwks_std::json::from_slice(&bytes)?;
        assert_eq!(back, fact, "{fact:?} roundtrips");
    }
    assert!(widest < 512, "a fact encodes small: {widest}");
    let step = RecursionStep::new(plan(RecursionMode::Sequential, 8, 16)?, label, 1)
        .with_candidate(hash)
        .producing(hash);
    let bytes = lgwks_std::json::to_vec(&step)?;
    assert_eq!(
        lgwks_std::json::from_slice::<RecursionStep>(&bytes)?,
        step,
        "a step reference roundtrips"
    );
    assert!(bytes.len() < 1024, "and encodes small: {}", bytes.len());
    assert_eq!(
        step.plan.validated(),
        Ok(step.plan),
        "a decoded plan is re-checked"
    );
    Ok(())
}
