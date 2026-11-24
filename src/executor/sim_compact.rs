//! Seeded simulations of a terminal session's auto-compaction (larry#235).
//!
//! Source: tool-result clearing as Anthropic's context editing and Microsoft Agent
//! Framework's `ToolResultCompactionStrategy` define it (old tool output dropped,
//! the call kept), and RFC 9110 15.5.14 (413 Content Too Large). Requirement
//! proved: every request a session sends fits its window, the head turns never
//! change, no step disappears without being counted, and one seed replays one
//! conversation. The window a gateway's refusal names is `sim_refusal`'s
//! (larry#236).

use std::error::Error as StdError;
use std::io::Write;
use std::time::Duration;

use lgwks_bot::kernel::clock::Clock;
use lgwks_macros::Error;
use lgwks_std::hash::Hasher;
use lgwks_std::json;
use lgwks_std::seeded::Seeded;

use super::super::session::sim::text;
use super::{Answer, Conversation, KEEP_EXCHANGES, MAX_RECORD_LINES, Overflow, quarters};
use crate::eval::percentile;
use crate::model::{
    MAX_MESSAGES, Message, ModelClient, ModelError, Request, Role, WindowRefusal, sim_config,
};
use crate::sim_error::SimError;

/// What every simulation returns.
type TestResult = Result<(), Box<dyn StdError>>;

/// Seeds each simulation runs: every request is encoded whole, so 24 seeds of up
/// to [`STEPS`] steps keep a debug run within the mutation gate's budget.
const SEEDS: u64 = 24;

/// Most steps one simulated session takes: past [`super::MAX_EXCHANGES`], so the
/// fold down to its low-water mark runs.
const STEPS: u64 = 200;

/// The system turn every simulated conversation opens with.
const SYSTEM: &str = "You are Larry, a terminal agent.";

/// Why a simulated session could not run to its end.
#[derive(Debug, Error)]
enum DriveError {
    /// The seeded input could not be built.
    #[error("seeded input: {0}")]
    Sim(#[from] SimError),
    /// The client could not be built.
    #[error("client: {0}")]
    Model(#[from] ModelError),
    /// The conversation could not be sent.
    #[error("overflow: {0}")]
    Overflow(#[from] Overflow),
    /// A request lacked what a check reads.
    #[error("missing: {0}")]
    Missing(&'static str),
}

/// A drawn number below `bound`, as a length.
fn upto(rng: &mut Seeded, bound: u64) -> Result<usize, SimError> {
    Ok(usize::try_from(rng.below(bound)?)?)
}

/// Step `index` of `tenant`'s session: a command that ran, or a refused reply.
/// Output may carry control bytes, which JSON escapes sixfold.
fn step(rng: &mut Seeded, tenant: &str, index: usize) -> Result<(String, Answer), SimError> {
    let len = upto(rng, 120)?;
    let command = format!("{tenant} {index} {}", text(rng, len)?);
    let reply = format!("{{\"command\":{}}}", json::to_string(&command)?);
    if rng.below(8)? == 0 {
        let fault = format!("the reply is not JSON ({})", text(rng, len)?);
        return Ok((reply, Answer::Refused { fault }));
    }
    let out = upto(rng, 3000)?;
    let controls = "\u{1}".repeat(upto(rng, 512)?);
    let output = format!("stdout:\n{}{controls}\nstderr:\n", text(rng, out)?);
    let answer = Answer::Observed {
        step: index,
        command,
        status: format!("exit status {}", rng.below(3)?),
        output: Some(output),
    };
    Ok((reply, answer))
}

/// A client that never sends, whose route bound is `window` bytes.
fn client(window: usize) -> Result<ModelClient, ModelError> {
    Ok(ModelClient::new(
        sim_config(Some(window))?,
        Clock::virtual_at(Duration::ZERO),
        |_| false,
    ))
}

/// One seeded session of `tenant`: a window, an instruction and up to 300 steps,
/// with an occasional gateway refusal that shrinks the window. `check` sees every
/// request the session would send.
fn drive(
    seed: u64,
    tenant: &str,
    mut check: impl FnMut(&Conversation, &Request) -> Result<(), DriveError>,
) -> Result<Conversation, DriveError> {
    let mut rng = Seeded::from_seed(seed);
    let window = 16_384_usize.saturating_add(upto(&mut rng, 32_769)?);
    let model = client(window)?;
    let len = upto(&mut rng, 2048)?;
    let task = format!("Task for {tenant}:\n{}", text(&mut rng, len)?);
    let mut conversation = Conversation::new(SYSTEM, task, model.request_bound());
    let steps = upto(&mut rng, STEPS)?.saturating_add(1);
    for index in 0..steps {
        let request = conversation.prepare(&model)?;
        check(&conversation, &request)?;
        let refused = quarters(request.bytes(), 3) >= 16_384 && conversation.tally.shrinks < 2;
        if rng.below(16).map_err(SimError::Draw)? == 0 && refused {
            conversation.shrink(request.bytes(), WindowRefusal::default())?;
            let shorter = conversation.prepare(&model)?;
            check(&conversation, &shorter)?;
        }
        let (reply, answer) = step(&mut rng, tenant, index)?;
        conversation.push(reply, answer);
    }
    let last = conversation.prepare(&model)?;
    check(&conversation, &last)?;
    Ok(conversation)
}

/// Every request fits its window and the client's message bound, the head
/// turns are byte-identical, the newest step is never folded, and across the
/// seeds every stage of compaction runs.
#[test]
fn sim_compaction_keeps_every_request_in_its_window_for_every_seed() -> TestResult {
    let mut stages = [0_usize; 4];
    for seed in 0..SEEDS {
        let mut head: Option<Vec<Message>> = None;
        let conversation = drive(seed, "tenant", |conversation, request| {
            let len = request.bytes();
            assert!(len <= conversation.window, "seed {seed}: {len} over");
            let count = request.messages();
            assert!(count <= MAX_MESSAGES, "seed {seed}: message bound");
            if head.is_none() {
                let opening = conversation.render();
                head = Some(
                    opening
                        .get(..2)
                        .ok_or(DriveError::Missing("the head turns"))?
                        .to_vec(),
                );
            }
            assert!(
                conversation.exchanges.len() >= KEEP_EXCHANGES || conversation.tally.folded == 0,
                "seed {seed}: the newest step is never folded"
            );
            Ok(())
        })?;
        let end = conversation.render();
        assert_eq!(
            end.get(..2),
            head.as_deref(),
            "seed {seed}: the head turns never change"
        );
        let tally = conversation.tally;
        let unlisted = tally.folded.saturating_sub(conversation.record.len());
        let counts = [tally.runs, tally.cleared, tally.folded, unlisted];
        for (stage, count) in stages.iter_mut().zip(counts) {
            *stage = stage.saturating_add(count);
        }
    }
    assert!(
        stages.iter().all(|count| *count > 0),
        "every stage ran: {stages:?}"
    );
    Ok(())
}

/// Every pushed step is whole or folded, and the record never passes its bound:
/// nothing leaves the conversation uncounted.
#[test]
fn sim_no_step_is_lost_silently_for_every_seed() -> TestResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed.wrapping_add(1 << 32));
        let steps = upto(&mut rng, STEPS)?.saturating_add(1);
        let model = client(16_384_usize.saturating_add(upto(&mut rng, 32_768)?))?;
        let mut conversation =
            Conversation::new(SYSTEM, String::from("task"), model.request_bound());
        for index in 0..steps {
            let (reply, answer) = step(&mut rng, "tenant", index)?;
            conversation.push(reply, answer);
            conversation.prepare(&model)?;
            let held = conversation
                .exchanges
                .len()
                .saturating_add(conversation.tally.folded);
            assert_eq!(held, index.saturating_add(1), "seed {seed}: step {index}");
            assert!(
                conversation.record.len() <= MAX_RECORD_LINES.min(conversation.tally.folded),
                "seed {seed}: the record holds only folds, within its bound"
            );
        }
    }
    Ok(())
}

/// The BLAKE3 hash over every request of one seeded session.
fn trace(seed: u64) -> Result<String, DriveError> {
    let mut hasher = Hasher::new();
    drive(seed, "tenant", |_, request| {
        hasher.write_framed(request.body());
        Ok(())
    })?;
    Ok(hasher.finalize().to_hex())
}

/// One seed replays one conversation, byte for byte; another seed does not.
#[test]
fn sim_same_seed_same_render_hash() -> TestResult {
    for seed in 0..4 {
        assert_eq!(trace(seed)?, trace(seed)?, "seed {seed} replays");
        assert_ne!(
            trace(seed)?,
            trace(seed.wrapping_add(1000))?,
            "seed {seed} differs"
        );
    }
    Ok(())
}

/// Appends `count` refused replies, each carrying `fault`.
fn refuse(conversation: &mut Conversation, count: usize, fault: &str) {
    for _ in 0..count {
        conversation.push(
            String::from("{}"),
            Answer::Refused {
                fault: fault.to_owned(),
            },
        );
    }
}

/// `count` refused replies of one byte, the cheapest exchange a session holds.
fn tiny(conversation: &mut Conversation, count: usize) {
    refuse(conversation, count, "x");
}

/// Every bound is exact. A request at 3/4 of its window is sent as it is and
/// one byte more compacts it. A request at the window is sent and one byte
/// less window refuses it. 120 exchanges stay whole and the 121st folds down to
/// 96. The newest 4 outputs stay. The record keeps 64 lines. A forget drops
/// exactly the lines that cover its excess. No record turn exists before a fold.
#[test]
fn every_compaction_bound_is_exact() -> TestResult {
    let model = client(1_048_576)?;
    let mut task = String::from("task");
    for _ in 0..3 {
        let len = model
            .encode(&Conversation::new(SYSTEM, task.clone(), 1).render())
            .bytes();
        if len.rem_euclid(3) == 0 {
            break;
        }
        task.push('x');
    }
    let len = model
        .encode(&Conversation::new(SYSTEM, task.clone(), 1).render())
        .bytes();
    assert_eq!(len.rem_euclid(3), 0, "a request length divisible by 3");
    let window = len.div_euclid(3).saturating_mul(4);

    let mut at = Conversation::new(SYSTEM, task.clone(), window);
    assert_eq!(at.prepare(&model)?.bytes(), len, "sent as it is");
    assert_eq!(at.tally.runs, 0, "at 3/4 of the window nothing compacts");
    assert_eq!(at.render().len(), 2, "no record turn before a fold");
    let mut over = Conversation::new(SYSTEM, format!("{task}x"), window);
    over.prepare(&model)?;
    assert_eq!(over.tally.runs, 1, "one byte past 3/4 compacts");

    let mut full = Conversation::new(SYSTEM, task.clone(), len);
    assert_eq!(
        full.prepare(&model)?.bytes(),
        len,
        "a request at the window is sent"
    );
    let mut short = Conversation::new(SYSTEM, task, len.saturating_sub(1));
    assert!(
        matches!(short.prepare(&model), Err(Overflow::Irreducible { .. })),
        "one byte less window refuses it"
    );

    let mut many = Conversation::new(SYSTEM, String::from("task"), 1_048_576);
    tiny(&mut many, super::MAX_EXCHANGES);
    many.prepare(&model)?;
    assert_eq!(many.tally.runs, 0, "120 exchanges stay whole");
    tiny(&mut many, 1);
    many.prepare(&model)?;
    assert_eq!(
        many.exchanges.len(),
        super::LOW_EXCHANGES,
        "the 121st folds to 96"
    );
    let seen = format!("{:?}", many.render());
    assert!(!seen.contains("not listed"), "every fold is still listed");

    let mut kept = observations(6);
    kept.clear(super::KEEP_OBSERVED);
    assert_eq!(kept.tally.cleared, 2, "the newest 4 outputs stay");

    let mut long = Conversation::new(SYSTEM, String::from("task"), 1_048_576);
    tiny(&mut long, 200);
    long.fold(usize::MAX);
    assert_eq!(
        long.exchanges.len(),
        KEEP_EXCHANGES,
        "everything but the newest folds"
    );
    assert_eq!(
        long.record.len(),
        MAX_RECORD_LINES,
        "the record keeps 64 lines"
    );
    let two = long
        .record
        .iter()
        .take(2)
        .fold(0_usize, |all, line| all.saturating_add(line.len()));
    long.forget(two);
    assert_eq!(
        long.record.len(),
        MAX_RECORD_LINES.saturating_sub(2),
        "a forget drops exactly the lines that cover its excess"
    );
    Ok(())
}

/// A conversation of `count` commands that ran, each with a short output.
fn observations(count: usize) -> Conversation {
    let mut conversation = Conversation::new(SYSTEM, String::from("task"), 1_048_576);
    for index in 0..count {
        conversation.push(
            String::from("{}"),
            Answer::Observed {
                step: index,
                command: String::from("true"),
                status: String::from("exit status 0"),
                output: Some(String::from("stdout:\nok\nstderr:\n")),
            },
        );
    }
    conversation
}

/// A conversation of `count` refused replies, each carrying `fault`.
fn refusals(count: usize, fault: &str) -> Conversation {
    let mut conversation = Conversation::new(SYSTEM, String::from("task"), 1_048_576);
    refuse(&mut conversation, count, fault);
    conversation
}

/// Each stage stops at its target, not one byte short of it: a request exactly
/// at its target folds, clears and forgets nothing. A first fold also adds the
/// record turn its estimate cannot see, so a second pass re-measures and folds
/// on: the request ends under its target with no record line forgotten. A
/// first forget adds the "not listed" line, so a second pass forgets on.
#[test]
fn every_compaction_stage_re_measures_until_its_target() -> TestResult {
    let model = client(1_048_576)?;
    let mut exact = observations(3);
    let len = model.encode(&exact.render()).bytes();
    assert_eq!(exact.compact(&model, len).bytes(), len, "at its target");
    let tally = exact.tally;
    assert_eq!(
        (tally.folded, tally.cleared),
        (0, 0),
        "nothing folds or clears"
    );

    let mut folds = refusals(10, "x");
    let target = model.encode(&folds.render()).bytes().saturating_sub(1);
    let left = folds.compact(&model, target).bytes();
    assert!(left <= target, "{left} over {target}");
    assert!(folds.tally.folded > 0, "it folded");
    assert_eq!(
        folds.record.len(),
        folds.tally.folded,
        "folding alone reached the target: no line forgotten"
    );

    let mut full = refusals(super::MAX_EXCHANGES, "x");
    let over = model.encode(&full.render()).bytes().saturating_sub(10);
    full.compact(&model, over);
    assert!(
        full.exchanges.len() > super::LOW_EXCHANGES,
        "at the exchange bound a fold covers only its excess: {}",
        full.exchanges.len()
    );

    let mut lines = refusals(MAX_RECORD_LINES.saturating_add(1), "x");
    lines.fold(usize::MAX);
    assert_eq!(lines.record.len(), lines.tally.folded, "every fold listed");
    let whole = model.encode(&lines.render()).bytes();
    let first = lines.record.front().ok_or("a record line to forget")?.len();
    let target = whole.saturating_sub(first.saturating_add(1));
    let left = lines.compact(&model, target).bytes();
    assert!(left <= target, "{left} over {target}");
    Ok(())
}

/// An instruction over the window is refused typed, naming both lengths.
#[test]
fn an_irreducible_conversation_is_refused_typed() -> TestResult {
    let model = client(1024)?;
    let mut conversation = Conversation::new(SYSTEM, "x".repeat(4096), model.request_bound());
    match conversation.prepare(&model) {
        Err(Overflow::Irreducible { len, window }) => {
            assert_eq!(window, 1024, "the route's bound");
            assert!(len > 4096, "the whole request: {len}");
        }
        other => return Err(format!("expected Irreducible, got {other:?}").into()),
    }
    Ok(())
}

/// Two tenants' sessions compact at once on two threads; neither conversation
/// ever carries the other's text, its record included.
#[test]
fn sim_two_sessions_compacting_at_once_never_share_a_line() -> TestResult {
    let run = |seed: u64, own: &'static str, other: &'static str| -> Result<(), String> {
        let ended = drive(seed, own, |_, _| Ok(())).map_err(|cause| cause.to_string())?;
        let seen = format!("{:?}", ended.render());
        assert!(!seen.contains(other), "{own} saw {other}");
        assert!(seen.contains(own), "{own} sees its own task");
        Ok(())
    };
    for seed in 0..8 {
        let (first, second) = std::thread::scope(|scope| {
            let one = scope.spawn(|| run(seed, "tenant-amber", "tenant-cobalt"));
            let two = scope.spawn(|| run(seed, "tenant-cobalt", "tenant-amber"));
            (one.join(), two.join())
        });
        match (first, second) {
            (Ok(one), Ok(two)) => {
                one?;
                two?;
            }
            _ => return Err("a tenant thread panicked".into()),
        }
    }
    Ok(())
}

/// The bytes each request of a 256-step session with 4 KiB outputs sends at a
/// 1 MiB window, compacted, against the same session sent whole: p50, p95 and
/// p99 with the sample count. Every byte is a prompt byte a provider bills.
#[test]
fn sim_compaction_request_bytes_p50_p95_p99() -> TestResult {
    let model = client(1_048_576)?;
    let mut conversation = Conversation::new(SYSTEM, String::from("task"), model.request_bound());
    let head = conversation.head.clone();
    let base = model.encode(&head).bytes();
    let (mut sizes, mut sent, mut whole, mut whole_bytes) = (Vec::new(), 0_usize, base, 0_usize);
    for index in 0..256_usize {
        let len = conversation.prepare(&model)?.bytes();
        sizes.push(u64::try_from(len)?);
        sent = sent.saturating_add(len);
        whole_bytes = whole_bytes.saturating_add(whole);
        let command = format!("cat part-{index}");
        let reply = format!("{{\"command\":{}}}", json::to_string(&command)?);
        let answer = Answer::Observed {
            step: index,
            command,
            status: String::from("exit status 0"),
            output: Some(format!("stdout:\n{}\nstderr:\n", "a".repeat(4096))),
        };
        let mut pair = head.to_vec();
        pair.push(Message::new(Role::Assistant, reply.clone()));
        pair.push(Message::new(Role::User, answer.text()));
        whole = whole.saturating_add(model.encode(&pair).bytes().saturating_sub(base));
        conversation.push(reply, answer);
    }
    sizes.sort_unstable();
    let rank = |percent: usize| percentile(&sizes, percent).ok_or("no samples");
    assert!(conversation.tally.runs > 0, "the session compacted");
    let trigger = u64::try_from(quarters(1_048_576, 3))?;
    assert!(rank(100)? <= trigger, "no request past the trigger");
    assert!(sent < whole_bytes, "compaction sends fewer bytes in all");
    writeln!(
        std::io::stdout().lock(),
        "session request bytes ({} samples, 1 MiB window, 4 KiB outputs): p50 {} p95 {} p99 {}; {sent} bytes in all against {whole_bytes} sent whole; {} runs, {} cleared, {} folded",
        sizes.len(),
        rank(50)?,
        rank(95)?,
        rank(99)?,
        conversation.tally.runs,
        conversation.tally.cleared,
        conversation.tally.folded
    )?;
    Ok(())
}
