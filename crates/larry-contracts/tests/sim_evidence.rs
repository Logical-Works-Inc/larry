//! larry#63: no grader verdict, rubric or score reaches an improver. A scripted proposer double
//! counts the contexts it is handed, so a refusal is shown by the absence of a call. The type
//! boundary itself is the `compile_fail` doctest in `evidence/mod.rs`; this file checks the scan
//! and the runtime-tagged door.

use larry_contracts::evidence::{
    AnyEvidence, ContextError, Deliverable, Evidence, EvidenceError, EvidenceKind, Execution,
    Grader, ImproverContext, ImproverItem, LeakError, LeakKind, MAX_CONTEXT_BYTES,
    MAX_CONTEXT_ITEMS, MAX_EVIDENCE_BYTES, Source,
};

type Outcome = Result<(), Box<dyn std::error::Error>>;

/// A scripted proposer: it records every path it was given.
#[derive(Default)]
struct Proposer {
    calls: usize,
    seen: Vec<String>,
}

impl Proposer {
    fn receive(&mut self, items: &[ImproverItem]) {
        self.calls = self.calls.saturating_add(1);
        self.seen
            .extend(items.iter().map(|item| item.path().to_owned()));
    }
}

/// The guarded path: assemble first, call the proposer only with a context, record a refusal.
fn dispatch(
    proposer: &mut Proposer,
    refusals: &mut Vec<LeakError>,
    items: Vec<ImproverItem>,
) -> Result<(), ContextError> {
    match ImproverContext::assemble(items) {
        Ok(context) => {
            proposer.receive(context.items());
            Ok(())
        }
        Err(refusal) => {
            if let ContextError::Leak(ref leak) = refusal {
                refusals.push(leak.clone());
            }
            Err(refusal)
        }
    }
}

fn execution(path: &str, text: &str) -> Result<ImproverItem, EvidenceError> {
    Ok(ImproverItem::new(Evidence::<Execution>::new(
        path,
        text.as_bytes(),
    )?))
}

fn deliverable(path: &str, text: &str) -> Result<ImproverItem, EvidenceError> {
    Ok(ImproverItem::new(Evidence::<Deliverable>::new(
        path,
        text.as_bytes(),
    )?))
}

fn source(path: &str, text: &str) -> Result<ImproverItem, EvidenceError> {
    Ok(ImproverItem::new(Evidence::<Source>::new(
        path,
        text.as_bytes(),
    )?))
}

/// The three plants from the issue, in a clean company of items.
type Plant = (Vec<ImproverItem>, &'static str, LeakKind);

fn planted() -> Result<Vec<Plant>, EvidenceError> {
    let mut cases = Vec::new();
    for (path, kind, make) in [
        (
            "rollout/trial-1/result.json",
            LeakKind::GraderVerdict,
            execution(
                "rollout/trial-1/result.json",
                "{\"verdict\": \"fail\", \"steps\": 4}",
            )?,
        ),
        (
            "deliverable/answer.md",
            LeakKind::RubricLine,
            deliverable(
                "deliverable/answer.md",
                "The fix is below.\nRubric: must name the file\n",
            )?,
        ),
        (
            "lesson/l1.txt",
            LeakKind::Score,
            execution("lesson/l1.txt", "Next time check the diff. score: 7/10")?,
        ),
    ] {
        cases.push((
            vec![
                source("source/task.md", "Fix the parser")?,
                make,
                execution("rollout/trial-1/run.log", "exit 0")?,
            ],
            path,
            kind,
        ));
    }
    Ok(cases)
}

#[test]
fn sim_evidence_falsifier_each_plant_is_refused_before_any_proposer_call() -> Outcome {
    for (items, path, kind) in planted()? {
        let (mut proposer, mut refusals) = (Proposer::default(), Vec::new());
        let result = dispatch(&mut proposer, &mut refusals, items);
        assert_eq!(
            result,
            Err(ContextError::Leak(LeakError::new(path, kind))),
            "{path} is refused with its specific error"
        );
        assert_eq!(proposer.calls, 0, "{path}: no proposer call happened");
        assert_eq!(refusals.len(), 1, "{path}: the refusal is recorded");
        assert_eq!(
            refusals.first().map(|leak| leak.path.as_str()),
            Some(path),
            "{path}: named"
        );
    }
    Ok(())
}

/// Negative control: with the guard removed (an unguarded helper that exists only here), the same
/// plants reach the proposer, so the guard is what refuses them.
#[test]
fn sim_evidence_negative_control_without_the_guard_the_plants_reach_the_proposer() -> Outcome {
    for (items, path, _) in planted()? {
        let mut proposer = Proposer::default();
        proposer.receive(&items);
        assert_eq!(
            proposer.calls, 1,
            "{path}: unguarded, the proposer is called"
        );
        assert!(
            proposer.seen.iter().any(|seen| seen == path),
            "{path}: and it received the plant"
        );
    }
    Ok(())
}

#[test]
fn evidence_a_grader_cannot_enter_a_context_through_the_runtime_door_either() -> Outcome {
    let grader = Evidence::<Grader>::new("rollout/verdict", b"pass")?;
    let any = AnyEvidence::from(grader);
    assert_eq!(
        any.into_improver(),
        Err(LeakError::new(
            "rollout/verdict",
            LeakKind::GraderProvenance
        )),
        "grader provenance is refused"
    );
    for kind in [
        EvidenceKind::Execution,
        EvidenceKind::Deliverable,
        EvidenceKind::Source,
    ] {
        let safe = AnyEvidence::new(kind, "rollout/a", b"x")?.into_improver()?;
        assert_eq!(safe.kind(), kind, "{kind:?} converts");
    }
    assert_eq!(EvidenceKind::Grader.tag(), 3, "tags are stable");
    Ok(())
}

#[test]
fn evidence_paths_and_bounds_are_typed_refusals() -> Outcome {
    for path in [
        "", "/a", "a//b", "a/b/c/d", "A/b", "a/..", "-a", ".hidden", "a b", "a\\b",
    ] {
        assert_eq!(
            Evidence::<Source>::new(path, b"x"),
            Err(EvidenceError::InvalidPath),
            "{path:?}"
        );
    }
    assert!(
        Evidence::<Source>::new("rollout/run-1_a.log", b"x").is_ok(),
        "a valid path"
    );
    let big = vec![b'a'; MAX_EVIDENCE_BYTES.saturating_add(1)];
    assert_eq!(
        Evidence::<Source>::new("a", &big),
        Err(EvidenceError::TooLarge {
            limit: MAX_EVIDENCE_BYTES,
            actual: MAX_EVIDENCE_BYTES.saturating_add(1)
        }),
        "an item past its bound"
    );
    assert!(
        Evidence::<Source>::new("a", big.get(..MAX_EVIDENCE_BYTES).ok_or("slice")?).is_ok(),
        "an item at its bound"
    );
    let many: Vec<ImproverItem> = (0..=MAX_CONTEXT_ITEMS)
        .map(|index| source(&format!("s/f{index}"), "x"))
        .collect::<Result<_, _>>()?;
    assert_eq!(
        ImproverContext::assemble(many),
        Err(ContextError::TooManyItems {
            limit: MAX_CONTEXT_ITEMS,
            actual: MAX_CONTEXT_ITEMS.saturating_add(1)
        }),
        "too many items"
    );
    let heavy: Vec<ImproverItem> = (0..17)
        .map(|index| source(&format!("s/f{index}"), &"a".repeat(MAX_EVIDENCE_BYTES)))
        .collect::<Result<_, _>>()?;
    assert!(
        matches!(ImproverContext::assemble(heavy), Err(ContextError::TooLarge { limit, .. }) if limit == MAX_CONTEXT_BYTES),
        "too many bytes"
    );
    Ok(())
}

#[test]
fn evidence_clean_look_alikes_pass_and_markers_in_every_shape_are_refused() -> Outcome {
    for clean in [
        "the scoreboard is on screen",
        "exit code 0",
        "scores were discussed",
        "a rubric-free day",
    ] {
        assert!(
            ImproverContext::assemble(vec![execution("a", clean)?]).is_ok(),
            "{clean:?} is clean"
        );
    }
    let refused = [
        ("{\"passed\": true}", LeakKind::GraderVerdict),
        ("{ \"reward\" :1 }", LeakKind::GraderVerdict),
        ("  - Verdict = fail", LeakKind::GraderVerdict),
        ("> RUBRIC item 3", LeakKind::RubricLine),
        ("{\"rubric\": []}", LeakKind::RubricLine),
        ("{\"score\": 0.5}", LeakKind::Score),
        ("Score 3", LeakKind::Score),
        ("final score=9/10", LeakKind::Score),
    ];
    for (text, kind) in refused {
        assert_eq!(
            ImproverContext::assemble(vec![execution("a", text)?]),
            Err(ContextError::Leak(LeakError::new("a", kind))),
            "{text:?}"
        );
    }
    Ok(())
}

/// A seeded stream: wrapping arithmetic only.
struct Rng(u64);

impl Rng {
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
}

const WORDS: [&str; 8] = [
    "alpha",
    "build",
    "compile",
    "output",
    "tests ran",
    "exit 0",
    "diff",
    "patch",
];
const LEAKS: [(&str, LeakKind); 9] = [
    ("{\"verdict\": \"pass\"}", LeakKind::GraderVerdict),
    ("{\"passed\":true}", LeakKind::GraderVerdict),
    ("verdict: fail", LeakKind::GraderVerdict),
    ("reward = 1", LeakKind::GraderVerdict),
    ("Rubric: names the file", LeakKind::RubricLine),
    ("- rubric item 2", LeakKind::RubricLine),
    ("score: 7/10", LeakKind::Score),
    ("{\"score\": 0.5}", LeakKind::Score),
    ("Score = 3", LeakKind::Score),
];

fn clean_text(rng: &mut Rng) -> Result<String, Box<dyn std::error::Error>> {
    let lines = rng.below(4).saturating_add(1);
    let mut picked = Vec::new();
    for _ in 0..lines {
        picked.push(*WORDS.get(rng.below(WORDS.len())).ok_or("word")?);
    }
    Ok(picked.join("\n"))
}

/// One seeded world: clean items, optionally one planted leak, and what the guarded path did.
fn world(
    seed: u64,
    plant: bool,
) -> Result<(usize, Option<LeakError>, usize), Box<dyn std::error::Error>> {
    let mut rng = Rng(seed);
    let count = rng.below(12).saturating_add(1);
    let victim = rng.below(count);
    let (leak_text, expected) = *LEAKS.get(rng.below(LEAKS.len())).ok_or("leak")?;
    let mut items = Vec::new();
    for index in 0..count {
        let mut text = clean_text(&mut rng)?;
        if plant && index == victim {
            let at = rng.below(text.lines().count().saturating_add(1));
            let mut lines: Vec<&str> = text.lines().collect();
            lines.insert(at, leak_text);
            text = lines.join("\n");
        }
        let path = format!("rollout/f{index}");
        items.push(match rng.below(3) {
            0 => execution(&path, &text)?,
            1 => deliverable(&path, &text)?,
            _ => source(&path, &text)?,
        });
    }
    let (mut proposer, mut refusals) = (Proposer::default(), Vec::new());
    let result = dispatch(&mut proposer, &mut refusals, items);
    if plant {
        let want = LeakError::new(&format!("rollout/f{victim}"), expected);
        assert_eq!(
            result,
            Err(ContextError::Leak(want.clone())),
            "seed {seed}: the plant is refused"
        );
        assert_eq!(refusals, vec![want], "seed {seed}: and recorded");
        assert_eq!(proposer.calls, 0, "seed {seed}: before any proposer call");
    } else {
        assert_eq!(result, Ok(()), "seed {seed}: clean evidence assembles");
        assert_eq!(proposer.calls, 1, "seed {seed}: and reaches the proposer");
    }
    Ok((count, refusals.into_iter().next(), proposer.seen.len()))
}

#[test]
fn sim_evidence_clean_assembles_and_one_planted_leak_is_always_refused() -> Outcome {
    for seed in 0..320_u64 {
        let clean = world(seed, false)?;
        let hostile = world(seed, true)?;
        assert_eq!(clean.0, hostile.0, "seed {seed}: same shape");
        assert!(
            hostile.1.is_some() && clean.1.is_none(),
            "seed {seed}: refusal only with the plant"
        );
    }
    Ok(())
}

#[test]
fn sim_evidence_same_seed_same_result() -> Outcome {
    for seed in 0..320_u64 {
        assert_eq!(
            world(seed, true)?,
            world(seed, true)?,
            "seed {seed}: hostile"
        );
        assert_eq!(
            world(seed, false)?,
            world(seed, false)?,
            "seed {seed}: clean"
        );
    }
    Ok(())
}
