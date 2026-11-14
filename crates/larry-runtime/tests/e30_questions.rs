//! E30 (larry#98, specification D25, Cell 02 "Durable scoped question;
//! expiry is not consent"): concurrent questions keep their exact respondent
//! and effect, an unrelated wait is unaffected, expiry and cancellation never
//! become consent, a headless client is refused typed, and a question and its
//! answer survive a crash because they are journaled commands.
//!
//! Every case drives the shipped `Runtime` through `submit_protocol`, `tick`
//! and `question`; the owner is the scripted fixture double.

mod support;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use larry_contracts::authority::Scope;
use larry_contracts::command::{Command, CommandAcceptance, CommandContent, CommandKind};
use larry_contracts::effect::ReconciliationStrategy;
use larry_contracts::ids::{CommandId, ContentDigest, RunId, TenantId, WorkerId};
use larry_contracts::replay::DenialReason;
use larry_runtime::error::RuntimeError;
use larry_runtime::journal::Journal;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::protocol::{
    ProtocolCapability, ProtocolError, ProtocolOffer, ProtocolVersionRange,
};
use larry_runtime::runtime::{
    Answer, Decision, Question, QuestionError, Runtime, Subject, restore,
};
use lgwks_bot::clock::Clock;

use support::{
    TestRuntime, cancel, clock, command_id, epoch, operation, proposal, run, seeded, settings,
    submit, succeeded, tenant,
};

type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Seconds every question here stays open unless a case says otherwise.
const OPEN_SECS: u64 = 600;

fn worker(byte: u8) -> WorkerId {
    WorkerId::from_bytes([byte; 16])
}

fn schema() -> ContentDigest {
    ContentDigest::over(&[b"larry/e30/schema/approve-file-read/v1"])
}

fn offer(
    tenant_id: TenantId,
    capabilities: &[ProtocolCapability],
    headless: bool,
) -> ProtocolOffer {
    ProtocolOffer::new(
        tenant_id,
        ProtocolVersionRange::new(1, 1),
        capabilities.iter().copied().collect(),
        headless,
        false,
    )
}

/// A client with a human at it.
fn human(tenant_id: TenantId) -> ProtocolOffer {
    offer(
        tenant_id,
        &[
            ProtocolCapability::SubmitCommand,
            ProtocolCapability::HumanQuestion,
        ],
        false,
    )
}

fn world(owner: ScriptedOwner) -> Result<(TestRuntime, Clock), Box<dyn std::error::Error>> {
    let clock = clock()?;
    let rt = Runtime::new(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        owner,
        clock.clone(),
    )?;
    Ok((rt, clock))
}

/// One `FileRead` effect on `run` for `tenant`, costing `cost`.
fn work(
    id: u8,
    tenant_id: TenantId,
    run_id: RunId,
    op: u8,
    cost: u64,
) -> Result<Command, Box<dyn std::error::Error>> {
    let body = proposal(
        run_id,
        tenant_id,
        operation(op),
        &[Scope::FileRead],
        cost,
        ReconciliationStrategy::Queryable,
    );
    submit(command_id(id), &body)
}

fn command(
    id: CommandId,
    tenant_id: TenantId,
    kind: CommandKind,
    body: &impl serde_body::Body,
) -> Result<Command, Box<dyn std::error::Error>> {
    let payload = body.bytes()?.into_boxed_slice();
    Ok(Command::new(
        id,
        tenant_id,
        Vec::new(),
        CommandContent::new(kind, payload),
    )?)
}

/// The JSON body of a question or an answer.
mod serde_body {
    pub trait Body {
        fn bytes(&self) -> Result<Vec<u8>, lgwks_std::json::Error>;
    }
    impl Body for larry_runtime::runtime::Question {
        fn bytes(&self) -> Result<Vec<u8>, lgwks_std::json::Error> {
            lgwks_std::json::to_vec(self)
        }
    }
    impl Body for larry_runtime::runtime::Answer {
        fn bytes(&self) -> Result<Vec<u8>, lgwks_std::json::Error> {
            lgwks_std::json::to_vec(self)
        }
    }
}

/// The subject naming `work` exactly.
fn subject_of(work: &Command, run_id: RunId, op: u8) -> Subject {
    Subject::new(run_id, operation(op), work.content().digest(), schema())
}

/// Asks `respondent` about `subject` for `secs` from the fixture epoch.
fn ask(
    id: u8,
    tenant_id: TenantId,
    subject: Subject,
    respondent: WorkerId,
    secs: u64,
) -> Result<Command, Box<dyn std::error::Error>> {
    let expires = epoch()?
        .checked_add(Duration::from_secs(secs))
        .ok_or("fixture expiry overflow")?;
    let question = Question::new(
        subject,
        worker(0xA0),
        respondent,
        expires,
        [Scope::FileRead].into_iter().collect(),
    );
    command(command_id(id), tenant_id, CommandKind::Ask, &question)
}

fn answer(
    id: u8,
    tenant_id: TenantId,
    question: u8,
    respondent: WorkerId,
    subject: Subject,
    decision: Decision,
) -> Result<Command, Box<dyn std::error::Error>> {
    let body = Answer::new(command_id(question), respondent, subject, decision);
    command(command_id(id), tenant_id, CommandKind::Answer, &body)
}

fn approve_read() -> Decision {
    Decision::Approve([Scope::FileRead].into_iter().collect())
}

/// The question refusal `result` carries, if any: a protocol submission answers
/// the acceptance and the version, and only a refusal is interesting here
/// (larry#182 defect 3).
fn refused(result: Result<(CommandAcceptance, u16), RuntimeError>) -> Option<QuestionError> {
    match result {
        Err(RuntimeError::Question(error)) => Some(error),
        _ => None,
    }
}

/// Two questions at once, one per respondent; every misrouted answer is
/// refused with nothing written, one approval releases only its own effect,
/// and a second tenant's effect with the same operation bytes stays held.
#[test]
fn e30_concurrent_questions_keep_exact_bindings_and_unrelated_waits() -> Outcome {
    let owner = ScriptedOwner::scripted([succeeded(operation(1))]);
    let (mut rt, _clock) = world(owner)?;
    let (t1, t2) = (tenant(1), tenant(2));
    let (r1, r2) = (worker(1), worker(2));
    let work_a = work(10, t1, run(1), 1, 1)?;
    let work_b = work(11, t1, run(2), 2, 1)?;
    let work_c = work(12, t2, run(3), 1, 1)?;
    let (sa, sb, sc) = (
        subject_of(&work_a, run(1), 1),
        subject_of(&work_b, run(2), 2),
        subject_of(&work_c, run(3), 1),
    );
    rt.submit_protocol(&human(t1), ask(20, t1, sa, r1, OPEN_SECS)?, None)?;
    rt.submit_protocol(&human(t1), ask(21, t1, sb, r2, OPEN_SECS)?, None)?;
    rt.submit_protocol(&human(t2), ask(22, t2, sc, r1, OPEN_SECS)?, None)?;
    for (offer_tenant, item) in [(t1, work_a), (t1, work_b), (t2, work_c)] {
        rt.submit_protocol(&human(offer_tenant), item, None)?;
    }
    let held = rt.tick()?;
    assert!(
        held.authorized().is_empty(),
        "every effect waits for its answer"
    );
    assert_eq!(rt.owner().dispatched(), &[]);

    let before = rt.journal().len();
    let wrong_respondent = answer(30, t1, 20, r2, sa, approve_read())?;
    let wrong_effect = answer(31, t1, 20, r1, sb, approve_read())?;
    let wrong_tenant = answer(32, t2, 20, r1, sa, approve_read())?;
    for misrouted in [wrong_respondent, wrong_effect] {
        assert_eq!(
            refused(rt.submit_protocol(&human(t1), misrouted, None)),
            Some(QuestionError::BindingMismatch {
                question: command_id(20)
            })
        );
    }
    // Another tenant cannot even reach the question: the run is tenant 1's.
    assert!(matches!(
        rt.submit_protocol(&human(t2), wrong_tenant, None),
        Err(RuntimeError::RunTenantMismatch { .. })
    ));
    let outside = Decision::Approve([Scope::FileWrite].into_iter().collect());
    assert_eq!(
        refused(rt.submit_protocol(&human(t1), answer(33, t1, 20, r1, sa, outside)?, None)),
        Some(QuestionError::ScopeOutsideQuestion {
            question: command_id(20),
            scope: Scope::FileWrite
        })
    );
    assert_eq!(
        rt.journal().len(),
        before,
        "a refused answer writes nothing"
    );

    rt.submit_protocol(
        &human(t1),
        answer(34, t1, 20, r1, sa, approve_read())?,
        None,
    )?;
    let released = rt.tick()?;
    assert_eq!(
        released.authorized(),
        &[operation(1)],
        "only the approved effect leaves"
    );
    assert_eq!(
        rt.owner().dispatched(),
        &[operation(1)],
        "the other wait and another tenant's same operation stay held"
    );
    assert_eq!(
        refused(rt.submit_protocol(
            &human(t1),
            answer(35, t1, 20, r1, sa, Decision::Reject)?,
            None
        )),
        Some(QuestionError::AlreadyAnswered {
            question: command_id(20)
        })
    );

    rt.submit_protocol(
        &human(t1),
        answer(36, t1, 21, r2, sb, Decision::Reject)?,
        None,
    )?;
    let rejected = rt.tick()?;
    assert_eq!(
        rejected.denied(),
        &[(operation(2), DenialReason::NotApproved)]
    );
    assert_eq!(
        rt.owner().dispatched(),
        &[operation(1)],
        "a rejection reaches no owner"
    );
    // Tenant 2's question was never touched by tenant 1's answers: it still
    // takes its own respondent's answer.
    rt.submit_protocol(
        &human(t2),
        answer(37, t2, 22, r1, sc, Decision::Reject)?,
        None,
    )?;
    Ok(())
}

/// Expiry is never consent, and an answer past it or after the run's
/// cancellation is refused typed with nothing written.
#[test]
fn e30_expiry_and_cancellation_never_become_consent() -> Outcome {
    let (mut rt, clock) = world(ScriptedOwner::new())?;
    let t1 = tenant(1);
    let r1 = worker(1);
    let late = work(10, t1, run(1), 1, 1)?;
    let gone = work(11, t1, run(2), 2, 1)?;
    let (sl, sg) = (subject_of(&late, run(1), 1), subject_of(&gone, run(2), 2));
    rt.submit_protocol(&human(t1), ask(20, t1, sl, r1, 10)?, None)?;
    rt.submit_protocol(&human(t1), ask(21, t1, sg, r1, OPEN_SECS)?, None)?;
    rt.submit_protocol(&human(t1), late, None)?;
    rt.submit_protocol(&human(t1), gone, None)?;
    assert!(rt.tick()?.authorized().is_empty());

    clock.advance(Duration::from_secs(10))?;
    let before = rt.journal().len();
    assert_eq!(
        refused(rt.submit_protocol(
            &human(t1),
            answer(30, t1, 20, r1, sl, approve_read())?,
            None
        )),
        Some(QuestionError::Expired {
            question: command_id(20)
        })
    );
    assert_eq!(rt.journal().len(), before);

    let stop = proposal(
        run(2),
        t1,
        operation(2),
        &[],
        0,
        ReconciliationStrategy::Queryable,
    );
    rt.submit_protocol(&human(t1), cancel(command_id(40), &stop)?, None)?;
    assert_eq!(
        refused(rt.submit_protocol(
            &human(t1),
            answer(31, t1, 21, r1, sg, approve_read())?,
            None
        )),
        Some(QuestionError::RunClosed { run: run(2) }),
        "an answer after cancellation is refused even before the cancel is reduced"
    );
    let settled = rt.tick()?;
    assert!(
        settled
            .denied()
            .contains(&(operation(1), DenialReason::NotApproved)),
        "a timed-out question denies its work: {settled:?}"
    );
    assert!(
        settled
            .denied()
            .contains(&(operation(2), DenialReason::NotApproved)),
        "a cancelled question denies its work: {settled:?}"
    );
    assert_eq!(
        rt.owner().dispatched(),
        &[],
        "neither timeout nor cancellation reached the owner"
    );
    Ok(())
}

/// A question needs a human at the other end: a headless client, a client
/// without `HumanQuestion`, and the local queue are each refused typed, and
/// nothing is written.
#[test]
fn e30_a_headless_client_is_refused_and_nothing_is_written() -> Outcome {
    let (mut rt, _clock) = world(ScriptedOwner::new())?;
    let t1 = tenant(1);
    let item = work(10, t1, run(1), 1, 1)?;
    let question = ask(20, t1, subject_of(&item, run(1), 1), worker(1), OPEN_SECS)?;
    let headless = offer(
        t1,
        &[
            ProtocolCapability::SubmitCommand,
            ProtocolCapability::HumanQuestion,
        ],
        true,
    );
    assert!(matches!(
        rt.submit_protocol(&headless, question.clone(), None),
        Err(RuntimeError::Protocol(ProtocolError::HeadlessQuestion))
    ));
    let plain = offer(t1, &[ProtocolCapability::SubmitCommand], false);
    assert!(matches!(
        rt.submit_protocol(&plain, question.clone(), None),
        Err(RuntimeError::Protocol(ProtocolError::CapabilityMissing {
            capability: ProtocolCapability::HumanQuestion
        }))
    ));
    assert!(matches!(
        rt.submit(question),
        Err(RuntimeError::UnsupportedCommand(CommandKind::Ask))
    ));
    assert!(rt.journal().is_empty());
    let subject = subject_of(&item, run(1), 1);
    assert_eq!(
        refused(rt.submit_protocol(
            &human(t1),
            answer(30, t1, 20, worker(1), subject, approve_read())?,
            None
        )),
        Some(QuestionError::UnknownQuestion {
            question: command_id(20)
        }),
        "a refused question was never asked"
    );
    Ok(())
}

/// An approval admits only the exact `Submit` it names and only within the
/// scopes it approved; a different body for the same operation is denied.
#[test]
fn e30_an_approval_covers_only_its_exact_content() -> Outcome {
    let (mut rt, _clock) = world(ScriptedOwner::new())?;
    let t1 = tenant(1);
    let shown = work(10, t1, run(1), 1, 1)?;
    let swapped = work(11, t1, run(1), 1, 7)?;
    let subject = subject_of(&shown, run(1), 1);
    rt.submit_protocol(
        &human(t1),
        ask(20, t1, subject, worker(1), OPEN_SECS)?,
        None,
    )?;
    rt.submit_protocol(&human(t1), swapped, None)?;
    rt.submit_protocol(
        &human(t1),
        answer(30, t1, 20, worker(1), subject, approve_read())?,
        None,
    )?;
    let tick = rt.tick()?;
    assert_eq!(
        tick.denied(),
        &[(operation(1), DenialReason::NotApproved)],
        "{tick:?}"
    );
    assert_eq!(rt.owner().dispatched(), &[]);
    assert_eq!(rt.remaining_budget(), 100, "a denied swap reserves nothing");
    Ok(())
}

/// The pending set of questions is bounded; past it an `Ask` is refused
/// typed, and answering one makes room again.
#[test]
fn e30_pending_questions_refuse_past_their_bound_and_recover() -> Outcome {
    let (mut rt, _clock) = world(ScriptedOwner::new())?;
    let t1 = tenant(1);
    let mut asked = Vec::new();
    let mut refusal = None;
    // Ask until the runtime refuses; the bound is read from the refusal, and
    // a bound that never binds below 1024 fails the case.
    for index in 0..1024_usize {
        let byte = u8::try_from(index % 250)?;
        let run_id = RunId::from_bytes(wide(index, 0x51));
        let id = CommandId::from_bytes(wide(index, 0x52));
        let op = larry_contracts::ids::OperationId::from_bytes(wide(index, 0x53));
        let subject = Subject::new(run_id, op, ContentDigest::over(&[&[byte]]), schema());
        let question = Question::new(
            subject,
            worker(0),
            worker(1),
            epoch()?
                .checked_add(Duration::from_secs(OPEN_SECS))
                .ok_or("overflow")?,
            BTreeSet::new(),
        );
        let item = command(id, t1, CommandKind::Ask, &question)?;
        match rt.submit_protocol(&human(t1), item.clone(), None) {
            Ok(_) => asked.push((id, subject)),
            Err(error) => {
                refusal = Some((index, error, item));
                break;
            }
        }
        if index % 32 == 31 {
            rt.tick()?;
        }
    }
    let (filled, error, extra) = refusal.ok_or("the pending bound never bound below 1024")?;
    assert!(filled > 0, "the first question was refused: {error:?}");
    assert!(
        matches!(
            error,
            RuntimeError::Question(QuestionError::PendingFull { capacity }) if capacity == filled
        ),
        "refused at {filled}: {error:?}"
    );
    let (first, subject) = asked.first().copied().ok_or("nothing asked")?;
    let reject = Answer::new(first, worker(1), subject, Decision::Reject);
    rt.submit_protocol(
        &human(t1),
        command(command_id(253), t1, CommandKind::Answer, &reject)?,
        None,
    )?;
    rt.submit_protocol(&human(t1), extra, None)?;
    Ok(())
}

/// Sixteen identity bytes distinct for every `index` under `lead`.
fn wide(index: usize, lead: u8) -> [u8; 16] {
    let mut bytes = [lead; 16];
    for (slot, byte) in bytes.iter_mut().skip(8).zip(index.to_be_bytes()) {
        *slot = byte;
    }
    bytes
}

fn journal_file(seed: u64) -> Result<PathBuf, std::io::Error> {
    let path =
        std::env::temp_dir().join(format!("larry-e30-{}-{seed}.journal", std::process::id()));
    match std::fs::remove_file(&path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(path),
    }
}

fn reopen(
    path: &std::path::Path,
    owner: ScriptedOwner,
) -> Result<TestRuntime, Box<dyn std::error::Error>> {
    let settings = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
    let journal = Journal::open(path, settings.journal_capacity)?;
    let mut rt: TestRuntime = restore(settings, owner, clock()?, journal)?;
    let epoch_now = rt.fence().epoch()?;
    rt.fence_mut().authenticate(epoch_now)?;
    Ok(rt)
}

/// For every seed: a question and its held work survive a kill before the
/// answer, the restored runtime still holds the work, an answer acknowledged
/// and then lost to a second kill is in the file, and the effect is
/// dispatched exactly once, after the approval and never before.
#[test]
fn sim_e30_a_question_and_its_answer_survive_a_crash() -> Outcome {
    for seed in 0..32_u64 {
        seeded(seed, || -> Outcome {
            let path = journal_file(seed)?;
            let t1 = tenant(1);
            let item = work(10, t1, run(1), 1, 1)?;
            let subject = subject_of(&item, run(1), 1);
            {
                let mut first = reopen(&path, ScriptedOwner::new())?;
                first.submit_protocol(
                    &human(t1),
                    ask(20, t1, subject, worker(1), OPEN_SECS)?,
                    None,
                )?;
                first.submit_protocol(&human(t1), item, None)?;
                if seed % 2 == 1 {
                    assert!(
                        first.tick()?.authorized().is_empty(),
                        "seed {seed}: held before the answer"
                    );
                }
            }
            {
                let mut second = reopen(&path, ScriptedOwner::new())?;
                assert!(
                    second.tick()?.authorized().is_empty(),
                    "seed {seed}: a restart releases nothing"
                );
                second.submit_protocol(
                    &human(t1),
                    answer(30, t1, 20, worker(1), subject, approve_read())?,
                    None,
                )?;
            }
            let mut third = reopen(&path, ScriptedOwner::scripted([succeeded(operation(1))]))?;
            let tick = third.tick()?;
            assert_eq!(tick.authorized(), &[operation(1)], "seed {seed}");
            assert_eq!(
                third.owner().dispatched(),
                &[operation(1)],
                "seed {seed}: once"
            );
            drop(third);
            std::fs::remove_file(&path)?;
            Ok(())
        })?;
    }
    Ok(())
}
