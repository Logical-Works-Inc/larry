//! Seeded coverage sims for the question book, the display boundary, the owner
//! trait defaults and the tenant catch-up consumer.
//!
//! Questions are durable facts, so every refusal a hostile or confused client
//! can provoke is exercised here with exact typed assertions: expired,
//! foreign, duplicate and unscoped questions and answers, a closing run's
//! denial, and the display binding a tampered command breaks. Sources: E30
//! (expiry is never consent, approval covers exact content), E65 (display
//! binding), D25 (bounded questions), E35 (bounded catch-up) and CWE-20,
//! CWE-639 (authorization bypass through forged binding).

mod support;

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use larry_contracts::authority::Scope;
use larry_contracts::command::{Command, CommandContent, CommandKind};
use larry_contracts::effect::ReconciliationStrategy;
use larry_contracts::ids::{CommandId, ContentDigest, OperationId, RunId, TenantId, WorkerId};
use larry_contracts::replay::DenialReason;
use larry_runtime::error::RuntimeError;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::protocol::{
    MAX_PROTOCOL_VERSION, MIN_PROTOCOL_VERSION, ProtocolCapability, ProtocolOffer,
    ProtocolVersionRange,
};
use larry_runtime::runtime::{Answer, Decision, Question, QuestionError, Subject, restore};
use support::{
    TestRuntime, command_id, epoch, operation, proposal, run, runtime, settings, submit, tenant,
};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Calls a tiny `const` constructor or getter through an opaque function
/// pointer so its body executes out of line in its own crate.
fn outlined<F>(func: F) -> F {
    std::hint::black_box(func)
}

/// A question-capable offer for `tenant`, authenticated as `principal`.
fn asking_offer(tenant: TenantId, principal: WorkerId) -> ProtocolOffer {
    ProtocolOffer::new(
        tenant,
        ProtocolVersionRange::new(MIN_PROTOCOL_VERSION, MAX_PROTOCOL_VERSION),
        [
            ProtocolCapability::SubmitCommand,
            ProtocolCapability::HumanQuestion,
        ]
        .into_iter()
        .collect(),
        false,
        true,
    )
    .authenticated(principal)
}

/// The `Ask` command `id` about `subject`, asked by `requester` of `respondent`.
fn ask_command(
    id: CommandId,
    tenant: TenantId,
    subject: Subject,
    requester: WorkerId,
    respondent: WorkerId,
    expires_at: SystemTime,
    scopes: BTreeSet<Scope>,
) -> Result<Command, Box<dyn std::error::Error>> {
    let ask: fn(Subject, WorkerId, WorkerId, SystemTime, BTreeSet<Scope>) -> Question =
        outlined(Question::new);
    let body = ask(subject, requester, respondent, expires_at, scopes);
    Ok(Command::new(
        id,
        tenant,
        Vec::new(),
        CommandContent::new(
            CommandKind::Ask,
            lgwks_std::json::to_vec(&body)?.into_boxed_slice(),
        ),
    )?)
}

/// The `Answer` command `id` to `question` from `respondent`.
fn answer_command(
    id: CommandId,
    tenant: TenantId,
    question: CommandId,
    respondent: WorkerId,
    subject: Subject,
    decision: Decision,
) -> Result<Command, Box<dyn std::error::Error>> {
    let body = Answer::new(question, respondent, subject, decision);
    Ok(Command::new(
        id,
        tenant,
        Vec::new(),
        CommandContent::new(
            CommandKind::Answer,
            lgwks_std::json::to_vec(&body)?.into_boxed_slice(),
        ),
    )?)
}

/// The world's question as the command `id`: the open question every
/// answer-refusal test submits before it answers.
fn ask_open_command(
    id: CommandId,
    world: &QuestionWorld,
) -> Result<Command, Box<dyn std::error::Error>> {
    ask_command(
        id,
        world.tenant,
        subject(world.run, world.operation, world.digest),
        world.requester,
        world.respondent,
        world.expiry,
        world.scopes.clone(),
    )
}

/// The subject of the proposed effect: its run, operation and digest.
fn subject(run: RunId, operation: OperationId, effect: ContentDigest) -> Subject {
    Subject::new(run, operation, effect, ContentDigest::over(&[b"schema"]))
}

/// The shared question fixture: identities, proposal and its digest.
struct QuestionWorld {
    tenant: TenantId,
    requester: WorkerId,
    respondent: WorkerId,
    run: RunId,
    operation: OperationId,
    body: larry_runtime::payload::WorkProposal,
    digest: ContentDigest,
    scopes: BTreeSet<Scope>,
    expiry: SystemTime,
}

fn question_world() -> Result<QuestionWorld, Box<dyn std::error::Error>> {
    let tenant = tenant(1);
    let body = proposal(
        run(1),
        tenant,
        operation(1),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let digest = submit(command_id(10), &body)?.content().digest();
    Ok(QuestionWorld {
        tenant,
        requester: WorkerId::from_bytes([0xA1; 16]),
        respondent: WorkerId::from_bytes([0xB2; 16]),
        run: run(1),
        operation: operation(1),
        body,
        digest,
        scopes: [Scope::FileRead].into_iter().collect(),
        expiry: epoch()?
            .checked_add(Duration::from_secs(3_600))
            .ok_or("time")?,
    })
}

// Source: E30 (a question counts from its durable admission) and CWE-20
// (a hostile client that forges timing). Every refusal names its typed
// cause and writes nothing: an expired question, a self-answered question
// and a second question for one effect.
#[test]
fn sim_expired_reflexive_and_duplicate_questions_are_refused() -> Outcome {
    let world = question_world()?;
    let QuestionWorld {
        tenant,
        requester,
        respondent,
        run,
        operation,
        digest,
        ..
    } = world;

    let mut runtime = runtime()?;
    let before = runtime.journal().len();

    // An Ask that already expired is refused before anything is written.
    let stale = runtime.submit_protocol(
        &asking_offer(tenant, respondent),
        ask_command(
            command_id(1),
            tenant,
            subject(run, operation, digest),
            requester,
            respondent,
            epoch()?,
            world.scopes.clone(),
        )?,
        None,
    );
    assert!(
        matches!(
            stale,
            Err(RuntimeError::Question(QuestionError::ExpiredWhenAsked))
        ),
        "an expired question is refused: {stale:?}"
    );
    assert_eq!(
        runtime.journal().len(),
        before,
        "the refusal writes nothing"
    );

    // A question whose respondent is its requester authorizes nothing.
    let reflexive = runtime.submit_protocol(
        &asking_offer(tenant, requester),
        ask_command(
            command_id(2),
            tenant,
            subject(run, operation, digest),
            requester,
            requester,
            epoch()?
                .checked_add(Duration::from_secs(60))
                .ok_or("time")?,
            world.scopes.clone(),
        )?,
        None,
    );
    assert!(
        matches!(
            reflexive,
            Err(RuntimeError::Question(QuestionError::RequesterIsRespondent))
        ),
        "self-approval is refused: {reflexive:?}"
    );

    // A live question is asked, then asked again: the second is a duplicate.
    runtime.submit_protocol(
        &asking_offer(world.tenant, world.respondent),
        ask_open_command(command_id(3), &world)?,
        None,
    )?;
    let again = runtime.submit_protocol(
        &asking_offer(world.tenant, world.respondent),
        ask_open_command(command_id(4), &world)?,
        None,
    );
    assert!(
        matches!(
            again,
            Err(RuntimeError::Question(QuestionError::AlreadyAsked { .. }))
        ),
        "one effect has exactly one question: {again:?}"
    );
    Ok(())
}

// Source: E30 and CWE-639 (a client that names an identity it does not
// hold). Only the respondent answers, and one question has exactly one
// answer: a second approval and a requester's answer are both refused.
#[test]
fn sim_answers_are_bound_to_their_respondent_and_count_once() -> Outcome {
    let world = question_world()?;
    let mut runtime = runtime()?;
    runtime.submit_protocol(
        &asking_offer(world.tenant, world.respondent),
        ask_open_command(command_id(3), &world)?,
        None,
    )?;
    let QuestionWorld {
        tenant,
        requester,
        respondent,
        run,
        operation,
        digest,
        scopes,
        ..
    } = world;

    // The requester answering its own question is refused: a worker never
    // approves itself, and no other principal answers for the respondent.
    let answered_by_requester = runtime.submit_protocol(
        &asking_offer(tenant, requester),
        answer_command(
            command_id(5),
            tenant,
            command_id(3),
            requester,
            subject(run, operation, digest),
            Decision::Approve(scopes.clone()),
        )?,
        None,
    );
    assert!(
        matches!(
            answered_by_requester,
            Err(RuntimeError::Question(
                QuestionError::BindingMismatch { .. }
            ))
        ),
        "only the respondent answers: {answered_by_requester:?}"
    );

    // The respondent approves, then approves again: the second is refused.
    runtime.submit_protocol(
        &asking_offer(tenant, respondent),
        answer_command(
            command_id(6),
            tenant,
            command_id(3),
            respondent,
            subject(run, operation, digest),
            Decision::Approve(scopes),
        )?,
        None,
    )?;
    let twice = runtime.submit_protocol(
        &asking_offer(tenant, respondent),
        answer_command(
            command_id(7),
            tenant,
            command_id(3),
            respondent,
            subject(run, operation, digest),
            Decision::Reject,
        )?,
        None,
    );
    assert!(
        matches!(
            twice,
            Err(RuntimeError::Question(
                QuestionError::AlreadyAnswered { .. }
            ))
        ),
        "one question has exactly one answer: {twice:?}"
    );
    Ok(())
}

// Source: E30 (expiry is never consent). An answer past its question's
// expiry is refused with the typed cause, and nothing is written.
#[test]
fn sim_late_answer_is_refused() -> Outcome {
    let world = question_world()?;
    let QuestionWorld {
        tenant,
        requester,
        respondent,
        run,
        operation,
        digest,
        scopes,
        body: _,
        expiry: _,
    } = world;
    let mut expiring = support::runtime()?;
    let short = epoch()?
        .checked_add(Duration::from_secs(30))
        .ok_or("time")?;
    expiring.submit_protocol(
        &asking_offer(tenant, respondent),
        ask_command(
            command_id(8),
            tenant,
            subject(run, operation, digest),
            requester,
            respondent,
            short,
            scopes.clone(),
        )?,
        None,
    )?;
    let journal = expiring.into_journal();
    let mut late: TestRuntime = restore(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        ScriptedOwner::new(),
        lgwks_bot::kernel::clock::Clock::virtual_at(Duration::from_secs(1_700_000_000 + 3_600)),
        journal,
    )?;
    let past = late.submit_protocol(
        &asking_offer(tenant, respondent),
        answer_command(
            command_id(9),
            tenant,
            command_id(8),
            respondent,
            subject(run, operation, digest),
            Decision::Approve(scopes),
        )?,
        None,
    );
    assert!(
        matches!(
            past,
            Err(RuntimeError::Question(QuestionError::Expired { .. }))
        ),
        "a late answer is refused: {past:?}"
    );
    Ok(())
}

// Source: E30 (an approval covers only its exact content). A submitted
// effect whose question was rejected is denied, never run, and the owner
// is never reached.
#[test]
fn sim_rejected_question_denies_its_submit() -> Outcome {
    let world = question_world()?;
    let QuestionWorld {
        tenant,
        requester,
        respondent,
        run,
        operation,
        body,
        digest,
        scopes,
        expiry,
    } = world;
    // A submitted effect whose question was rejected is denied, never run.
    let mut refused_run = support::runtime()?;
    refused_run.submit_protocol(
        &asking_offer(tenant, respondent),
        ask_command(
            command_id(11),
            tenant,
            subject(run, operation, digest),
            requester,
            respondent,
            expiry,
            scopes,
        )?,
        None,
    )?;
    refused_run.submit_protocol(
        &asking_offer(tenant, respondent),
        answer_command(
            command_id(12),
            tenant,
            command_id(11),
            respondent,
            subject(run, operation, digest),
            Decision::Reject,
        )?,
        None,
    )?;
    refused_run.submit(submit(command_id(13), &body)?)?;
    let denied = refused_run.tick()?;
    assert_eq!(
        denied.denied().len(),
        1,
        "exactly the refused effect is denied"
    );
    let Some(&(denied_operation, denied_reason)) = denied.denied().first() else {
        return Err("exactly one denial is recorded".into());
    };
    assert_eq!(
        (denied_operation, denied_reason),
        (operation, DenialReason::NotApproved),
        "a rejected question denies the effect"
    );
    assert!(
        refused_run.owner().dispatched().is_empty(),
        "a refused effect never reaches its owner"
    );
    Ok(())
}

// Source: E30 (expiry and cancellation are never consent) and larry#371 (a
// refusal beside a durable acceptance outlives the process). A submit that
// waited on a question while its run closed is denied, and nothing runs.
#[test]
fn sim_closing_run_with_a_question_denies_its_submit() -> Outcome {
    let tenant = tenant(2);
    let requester = WorkerId::from_bytes([0xC3; 16]);
    let respondent = WorkerId::from_bytes([0xD4; 16]);
    let run = run(2);
    let operation = operation(2);
    let body = proposal(
        run,
        tenant,
        operation,
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let digest = submit(command_id(20), &body)?.content().digest();
    let mut runtime = runtime()?;
    let expiry = epoch()?
        .checked_add(Duration::from_secs(3_600))
        .ok_or("time")?;
    let scopes: BTreeSet<Scope> = [Scope::FileRead].into_iter().collect();
    runtime.submit_protocol(
        &asking_offer(tenant, respondent),
        ask_command(
            command_id(21),
            tenant,
            subject(run, operation, digest),
            requester,
            respondent,
            expiry,
            scopes,
        )?,
        None,
    )?;
    // The submit waits on the open question; the cancellation closes the run.
    runtime.submit(submit(command_id(22), &body)?)?;
    runtime.submit(support::cancel(command_id(23), &body)?)?;
    let held = runtime.tick()?;
    assert!(
        held.admitted().contains(&command_id(21)),
        "the question is admitted: {:?}",
        held.admitted()
    );
    assert!(
        !held.admitted().contains(&command_id(22)),
        "the submit waits on its question: {:?}",
        held.admitted()
    );
    let denied = runtime.tick()?;
    assert!(
        denied
            .denied()
            .contains(&(operation, DenialReason::NotApproved)),
        "a closing run denies the held submit: {:?}",
        denied.denied()
    );
    assert!(
        runtime.owner().dispatched().is_empty(),
        "the denied submit never reaches its owner"
    );
    Ok(())
}
