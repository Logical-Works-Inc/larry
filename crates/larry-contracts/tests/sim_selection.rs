//! larry#64 part 1: blind seeded selection and the evidence-backed lesson record. One seed derives
//! the task, tenant, mode, k and the selector's answer; same seed, same selection. No store is built:
//! that waits on Logical-DB.

use std::collections::BTreeSet;

use larry_contracts::event::{CausalRef, EventSequence};
use larry_contracts::evidence::{Evidence, Execution, ImproverContext, ImproverItem};
use larry_contracts::harness::{HarnessComponent, HarnessError, HarnessHash, HarnessTree};
use larry_contracts::ids::{ContentDigest, RunId, TenantId};
use larry_contracts::lesson::{
    Confidence, Lesson, LessonError, LessonField, LessonView, MAX_CUE_BYTES, MAX_EVIDENCE, RunIndex,
};
use larry_contracts::recursion::{
    Label, RecursionLedger, RecursionMode, RecursionPlan, Stage, StepOutcome,
};
use larry_contracts::selection::{
    CandidateId, MAX_SELECTION_REASON_BYTES, NonAnswer, SelectionError, SelectionReason,
    SelectorAnswer, prepare, selection_seed,
};

type Outcome = Result<(), Box<dyn std::error::Error>>;

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

/// A ledger driven to completion with every step a success.
fn settled(
    task: ContentDigest,
    mode: RecursionMode,
    rounds: u8,
) -> Result<RecursionLedger, Box<dyn std::error::Error>> {
    let plan = RecursionPlan::new(task, mode, rounds, HarnessTree::empty().hash(), 4)?;
    let mut ledger = RecursionLedger::new(plan);
    loop {
        let ready = ledger.next_steps();
        if ready.is_empty() {
            return Ok(ledger);
        }
        for step in ready {
            ledger.record_intent(step.label, step.attempt)?;
            let produced = (step.label.stage() != Stage::Probe)
                .then(|| made_by(step.label))
                .transpose()?;
            ledger.record_outcome(step.label, StepOutcome::Succeeded { produced })?;
        }
    }
}

fn context() -> Result<ImproverContext, Box<dyn std::error::Error>> {
    Ok(ImproverContext::assemble(vec![ImproverItem::new(
        Evidence::<Execution>::new("rollout/a", b"exit 0")?,
    )])?)
}

fn tenant_of(seed: u64) -> TenantId {
    let mut bytes = [0_u8; 16];
    bytes
        .iter_mut()
        .zip(seed.to_be_bytes().iter().cycle())
        .for_each(|(slot, byte)| *slot = *byte);
    TenantId::from_bytes(bytes)
}

fn world(seed: u64) -> Result<(RecursionLedger, TenantId), Box<dyn std::error::Error>> {
    let mode = if seed & 1 == 0 {
        RecursionMode::Parallel
    } else {
        RecursionMode::Sequential
    };
    let rounds = u8::try_from((seed >> 1) % 8)?.saturating_add(1);
    let task = ContentDigest::over(&[b"task", &seed.to_be_bytes()]);
    Ok((settled(task, mode, rounds)?, tenant_of(seed)))
}

#[test]
fn sim_selection_same_seed_same_selection_over_1000_seeds() -> Outcome {
    for seed in 0..1_000_u64 {
        let pick = |ledger: &RecursionLedger, tenant| -> Result<_, Box<dyn std::error::Error>> {
            let prepared = prepare(ledger, tenant, context()?)?;
            let count = prepared.selector_input().ids().len();
            let id = CandidateId::new(u8::try_from(seed % u64::try_from(count.max(1))?)?);
            Ok(prepared.decide(&SelectorAnswer::Chose {
                id,
                reason: "most robust".to_owned(),
            }))
        };
        let (ledger, tenant) = world(seed)?;
        let (first, second) = (pick(&ledger, tenant)?, pick(&world(seed)?.0, tenant)?);
        assert_eq!(first, second, "seed {seed}: same seed, same selection");
        assert!(!first.is_fallback(), "seed {seed}: a valid answer is used");
        let hashes: BTreeSet<_> = first.mapping().iter().map(|entry| entry.1).collect();
        let ids: BTreeSet<_> = first.mapping().iter().map(|entry| entry.0).collect();
        assert_eq!(
            (hashes.len(), ids.len()),
            (first.mapping().len(), first.mapping().len()),
            "seed {seed}: a permutation"
        );
        assert!(
            hashes.contains(&first.winner()),
            "seed {seed}: the winner was delivered"
        );
        assert_eq!(
            first.seed(),
            selection_seed(
                ledger.plan().task(),
                ledger.plan().mode(),
                ledger.plan().rounds(),
                tenant
            ),
            "seed {seed}: derived from the plan and tenant only"
        );
    }
    Ok(())
}

#[test]
fn sim_selection_task_k_mode_and_tenant_each_change_the_shuffle() -> Outcome {
    let task = ContentDigest::over(&[b"t"]);
    let tenant = tenant_of(1);
    let base = selection_seed(task, RecursionMode::Parallel, 4, tenant);
    assert_ne!(
        base,
        selection_seed(
            ContentDigest::over(&[b"u"]),
            RecursionMode::Parallel,
            4,
            tenant
        ),
        "task"
    );
    assert_ne!(
        base,
        selection_seed(task, RecursionMode::Sequential, 4, tenant),
        "mode"
    );
    assert_ne!(
        base,
        selection_seed(task, RecursionMode::Parallel, 5, tenant),
        "k"
    );
    assert_ne!(
        base,
        selection_seed(task, RecursionMode::Parallel, 4, tenant_of(2)),
        "tenant"
    );
    let mut orders = BTreeSet::new();
    for seed in 0..64_u64 {
        let ledger = settled(
            ContentDigest::over(&[b"same", &seed.to_be_bytes()]),
            RecursionMode::Parallel,
            6,
        )?;
        let prepared = prepare(&ledger, tenant_of(seed), context()?)?;
        orders.insert(
            prepared
                .decide(&SelectorAnswer::NoAnswer)
                .mapping()
                .iter()
                .map(|entry| entry.1)
                .collect::<Vec<_>>(),
        );
    }
    assert!(
        orders.len() > 8,
        "different inputs shuffle differently: {}",
        orders.len()
    );
    Ok(())
}

#[test]
fn sim_selection_the_selector_never_sees_a_hash() -> Outcome {
    for seed in 0..200_u64 {
        let (ledger, tenant) = world(seed)?;
        let prepared = prepare(&ledger, tenant, context()?)?;
        let shown = format!("{:?}", prepared.selector_input());
        let chosen = prepared.clone().decide(&SelectorAnswer::NoAnswer);
        for entry in chosen.mapping() {
            assert!(
                !shown.contains(&entry.1.to_hex()),
                "seed {seed}: a candidate hash reached the selector"
            );
        }
        assert_eq!(
            prepared.selector_input().ids().len(),
            chosen.mapping().len(),
            "seed {seed}: ids only"
        );
    }
    Ok(())
}

#[test]
fn sim_selection_every_non_answer_is_recorded_and_falls_back_without_error() -> Outcome {
    for seed in 0..64_u64 {
        let (ledger, tenant) = world(seed)?;
        let prepared = prepare(&ledger, tenant, context()?)?;
        let first_hash = prepared
            .clone()
            .decide(&SelectorAnswer::NoAnswer)
            .mapping()
            .first()
            .map(|entry| entry.1)
            .ok_or("a candidate")?;
        let cases = [
            (SelectorAnswer::NoAnswer, NonAnswer::NoAnswer),
            (SelectorAnswer::Failed, NonAnswer::SelectorFailed),
            (
                SelectorAnswer::Chose {
                    id: CandidateId::new(200),
                    reason: "x".to_owned(),
                },
                NonAnswer::UnknownId,
            ),
            (
                SelectorAnswer::Chose {
                    id: CandidateId::new(0),
                    reason: "  ".to_owned(),
                },
                NonAnswer::EmptyReason,
            ),
            (
                SelectorAnswer::Chose {
                    id: CandidateId::new(0),
                    reason: "r".repeat(MAX_SELECTION_REASON_BYTES + 1),
                },
                NonAnswer::ReasonTooLong,
            ),
        ];
        for (answer, kind) in cases {
            let selection = prepared.clone().decide(&answer);
            assert_eq!(
                selection.reason(),
                &SelectionReason::Fallback(kind),
                "seed {seed}: {kind:?} is recorded"
            );
            assert!(
                selection.is_fallback() && selection.winner() == first_hash,
                "seed {seed}: the seed's candidate wins"
            );
        }
        let named = CandidateId::new(0);
        let chosen = prepared.clone().decide(&SelectorAnswer::Chose {
            id: named,
            reason: "why".to_owned(),
        });
        assert_eq!(
            chosen.reason(),
            &SelectionReason::Selector("why".to_owned()),
            "seed {seed}"
        );
    }
    Ok(())
}

#[test]
fn selection_waits_for_the_recursion_to_settle() -> Outcome {
    let plan = RecursionPlan::new(
        ContentDigest::over(&[b"t"]),
        RecursionMode::Parallel,
        2,
        HarnessTree::empty().hash(),
        2,
    )?;
    let fresh = RecursionLedger::new(plan);
    assert_eq!(
        prepare(&fresh, tenant_of(1), context()?).err(),
        Some(SelectionError::NotSettled),
        "nothing delivered yet"
    );
    Ok(())
}

struct Known(BTreeSet<RunId>);

impl RunIndex for Known {
    fn knows(&self, run: RunId) -> bool {
        self.0.contains(&run)
    }
}

fn reference(byte: u8) -> CausalRef {
    CausalRef::new(RunId::from_bytes([byte; 16]), EventSequence::new(0))
}

fn lesson_with(
    tenant: Option<TenantId>,
    domain: &str,
    cue: &str,
    text: &str,
    evidence: Vec<CausalRef>,
    confidence: Option<Confidence>,
    adopted: Option<HarnessHash>,
) -> Result<Lesson, LessonError> {
    Lesson::new(tenant, domain, cue, text, evidence, confidence, adopted)
}

#[test]
fn lesson_refusals_name_each_missing_or_out_of_bounds_field() -> Outcome {
    let (tenant, hash) = (Some(tenant_of(1)), Some(HarnessTree::empty().hash()));
    let (low, proof) = (Some(Confidence::Low), vec![reference(1)]);
    let ok = |domain: &str| lesson_with(tenant, domain, "cue", "text", proof.clone(), low, hash);
    assert!(ok("web-forms").is_ok(), "a complete lesson");
    let missing = |field| Err(LessonError::Missing { field });
    assert_eq!(
        lesson_with(None, "d", "c", "t", proof.clone(), low, hash),
        missing(LessonField::Tenant),
        "tenant"
    );
    assert_eq!(ok(""), missing(LessonField::Domain), "domain");
    assert_eq!(
        lesson_with(tenant, "d", " ", "t", proof.clone(), low, hash),
        missing(LessonField::Cue),
        "cue"
    );
    assert_eq!(
        lesson_with(tenant, "d", "c", "", proof.clone(), low, hash),
        missing(LessonField::Lesson),
        "lesson"
    );
    assert_eq!(
        lesson_with(tenant, "d", "c", "t", Vec::new(), low, hash),
        missing(LessonField::Evidence),
        "no evidence"
    );
    assert_eq!(
        lesson_with(tenant, "d", "c", "t", proof.clone(), None, hash),
        missing(LessonField::Confidence),
        "confidence"
    );
    assert_eq!(
        lesson_with(tenant, "d", "c", "t", proof.clone(), low, None),
        missing(LessonField::Adopted),
        "adopted"
    );
    for bad in ["Up", "-x", "a b", "a/b"] {
        assert_eq!(ok(bad), Err(LessonError::InvalidDomain), "{bad}");
    }
    let long = "c".repeat(MAX_CUE_BYTES + 1);
    assert_eq!(
        lesson_with(tenant, "d", &long, "t", proof.clone(), low, hash),
        Err(LessonError::TooLong {
            field: LessonField::Cue,
            limit: MAX_CUE_BYTES,
            actual: MAX_CUE_BYTES + 1
        }),
        "cue bound"
    );
    let many: Vec<CausalRef> = (0..=u8::try_from(MAX_EVIDENCE)?).map(reference).collect();
    assert_eq!(
        lesson_with(tenant, "d", "c", "t", many, low, hash),
        Err(LessonError::TooMuchEvidence {
            limit: MAX_EVIDENCE,
            actual: MAX_EVIDENCE + 1
        }),
        "evidence bound"
    );
    let kept = ok("d")?;
    assert_eq!(
        kept.adopted(),
        HarnessTree::empty().hash(),
        "the full adopted hash is kept, not a summary"
    );
    let known = Known([RunId::from_bytes([1; 16])].into_iter().collect());
    assert_eq!(kept.vet(&known), Ok(()), "a known run");
    let citing = lesson_with(
        tenant,
        "d",
        "c",
        "t",
        vec![reference(1), reference(9)],
        low,
        hash,
    )?;
    assert_eq!(
        citing.vet(&known),
        Err(LessonError::UnknownRun {
            run: RunId::from_bytes([9; 16])
        }),
        "an unknown run is refused"
    );
    Ok(())
}

#[test]
fn sim_lesson_scope_never_returns_another_tenants_lesson_over_seeded_interleavings() -> Outcome {
    for seed in 0..300_u64 {
        let tenants = [tenant_of(seed), tenant_of(seed ^ 0xFF)];
        let domains = ["alpha", "beta"];
        let mut written = Vec::new();
        let mut state = seed;
        for turn in 0..24_u8 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_038_195_672_373);
            let who = usize::try_from((state >> 33) & 1)?;
            let where_ = usize::try_from((state >> 40) & 1)?;
            written.push((
                who,
                where_,
                lesson_with(
                    tenants.get(who).copied(),
                    domains.get(where_).copied().ok_or("domain")?,
                    "cue",
                    &format!("lesson {turn}"),
                    vec![reference(turn)],
                    Some(Confidence::Medium),
                    Some(HarnessTree::empty().hash()),
                )?,
            ));
        }
        for (who, tenant) in tenants.iter().enumerate() {
            for (where_, domain) in domains.iter().enumerate() {
                let lessons: Vec<Lesson> = written.iter().map(|entry| entry.2.clone()).collect();
                let view = LessonView::for_scope(*tenant, domain, &lessons);
                let expected = written
                    .iter()
                    .filter(|entry| entry.0 == who && entry.1 == where_)
                    .count();
                assert_eq!(
                    view.lessons().len(),
                    expected,
                    "seed {seed}: exactly this tenant's lessons in this domain"
                );
                assert!(
                    view.lessons()
                        .iter()
                        .all(|lesson| lesson.tenant() == *tenant && lesson.domain() == *domain),
                    "seed {seed}: no other tenant or domain"
                );
            }
        }
    }
    Ok(())
}
