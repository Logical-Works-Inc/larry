//! E30 public-runtime approval question falsifiers.

mod support;

use std::time::Duration;

use rocco_contracts::authority::Scope;
use rocco_contracts::effect::ReconciliationStrategy;
use rocco_contracts::ids::{ContentDigest, WorkerId};
use rocco_contracts::state::RunState;
use rocco_runtime::approval::{
    ApprovalAnswer, ApprovalDecision, ApprovalEffectBinding, ApprovalQuestionError,
    ApprovalQuestionId, ApprovalQuestionParties, ApprovalQuestionSpec, ApprovalQuestionStatus,
};
use rocco_runtime::error::RuntimeError;
use support::{
    assert_state, cancel, command_id, epoch, operation, proposal, run, runtime, runtime_with,
    submit, succeeded, tenant,
};

/// A deterministic worker identity from one byte.
fn worker(byte: u8) -> WorkerId {
    WorkerId::from_bytes([byte; 16])
}

/// A stable digest of one test label or seed payload.
fn digest(label: &[u8]) -> ContentDigest {
    ContentDigest::over(&[label])
}

/// Derives distinct deterministic fixture bytes by seed and identity domain.
fn id_byte(seed: u64, domain: u8) -> u8 {
    u8::try_from(seed.wrapping_add(u64::from(domain)) % 251).unwrap_or_default()
}

/// Builds a question binding with the one schema used by this falsifier.
fn question_spec(
    tenant_id: rocco_contracts::ids::TenantId,
    run_id: rocco_contracts::ids::RunId,
    operation_id: rocco_contracts::ids::OperationId,
    requester: WorkerId,
    respondent: WorkerId,
    effect_hash: ContentDigest,
    expires_at: std::time::SystemTime,
) -> Result<ApprovalQuestionSpec, ApprovalQuestionError> {
    ApprovalQuestionSpec::new(
        ApprovalQuestionParties::new(tenant_id, run_id, requester, respondent),
        ApprovalEffectBinding::new(operation_id, effect_hash, digest(b"schema-v1")),
        expires_at,
        vec![Scope::FileRead],
    )
}

/// Calls the public runtime path for one question request.
fn open_question(
    runtime: &mut support::TestRuntime,
    spec: ApprovalQuestionSpec,
    client: Option<WorkerId>,
) -> Result<ApprovalQuestionId, RuntimeError> {
    runtime.open_approval_question(spec, client)
}

/// Exact input identity sent with an answer in these scenarios.
#[derive(Clone, Copy)]
struct AnswerBinding {
    /// Runtime-local question key.
    question: ApprovalQuestionId,
    /// Authenticated tenant scope.
    tenant: rocco_contracts::ids::TenantId,
    /// Only authorized respondent.
    respondent: WorkerId,
    /// Effect operation under decision.
    operation: rocco_contracts::ids::OperationId,
    /// Effect payload binding.
    effect: ContentDigest,
    /// Answer schema revision binding.
    schema: ContentDigest,
}

impl AnswerBinding {
    /// Binds an answer to the test schema version.
    fn new(
        question: ApprovalQuestionId,
        tenant: rocco_contracts::ids::TenantId,
        respondent: WorkerId,
        operation: rocco_contracts::ids::OperationId,
        effect: ContentDigest,
    ) -> Self {
        Self {
            question,
            tenant,
            respondent,
            operation,
            effect,
            schema: digest(b"schema-v1"),
        }
    }
}

/// Sends a bound answer through the public Runtime API.
fn answer_question(
    runtime: &mut support::TestRuntime,
    binding: AnswerBinding,
    decision: ApprovalDecision,
) -> Result<(), RuntimeError> {
    runtime.answer_approval_question(ApprovalAnswer::new(
        binding.question,
        binding.tenant,
        binding.respondent,
        ApprovalEffectBinding::new(binding.operation, binding.effect, binding.schema),
        decision,
    ))
}

/// Runs one seeded concurrent, cancellation and exact-binding scenario.
fn concurrent_case(seed: u64) -> Result<ContentDigest, Box<dyn std::error::Error>> {
    let mut trace = Vec::new();
    let first_run = run(id_byte(seed, 1));
    let second_run = run(id_byte(seed, 2));
    let first_tenant = tenant(id_byte(seed, 10));
    let second_tenant = tenant(id_byte(seed, 11));
    let first_operation = operation(id_byte(seed, 20));
    let second_operation = operation(id_byte(seed, 21));
    let first_respondent = worker(id_byte(seed, 31));
    let second_respondent = worker(id_byte(seed, 32));
    let first_body = proposal(
        first_run,
        first_tenant,
        first_operation,
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let second_body = proposal(
        second_run,
        second_tenant,
        second_operation,
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let mut rt = runtime()?;
    rt.submit(submit(command_id(1), &first_body)?)?;
    rt.submit(submit(command_id(2), &second_body)?)?;
    rt.tick()?;
    assert_state(rt.projection(), first_run, RunState::Reconciling)?;
    let expiry = epoch()?
        .checked_add(Duration::from_secs(60))
        .ok_or("expiry overflow")?;
    let first_effect = digest(&seed.to_be_bytes());
    let second_effect = digest(&seed.saturating_add(1).to_be_bytes());
    let first_spec = question_spec(
        first_tenant,
        first_run,
        first_operation,
        worker(id_byte(seed, 30)),
        first_respondent,
        first_effect,
        expiry,
    )?;
    let wrong_client = open_question(&mut rt, first_spec.clone(), Some(second_respondent));
    assert!(
        matches!(
            wrong_client,
            Err(RuntimeError::ApprovalQuestion(
                ApprovalQuestionError::ClientIsNotRespondent { .. }
            ))
        ),
        "an unauthenticated client cannot open another worker's question"
    );
    let first_question = open_question(&mut rt, first_spec, Some(first_respondent))?;
    let second_question = open_question(
        &mut rt,
        question_spec(
            second_tenant,
            second_run,
            second_operation,
            worker(id_byte(seed, 30)),
            second_respondent,
            second_effect,
            expiry,
        )?,
        Some(second_respondent),
    )?;
    let first_binding = AnswerBinding::new(
        first_question,
        first_tenant,
        first_respondent,
        first_operation,
        first_effect,
    );
    let wrong_respondent = answer_question(
        &mut rt,
        AnswerBinding {
            respondent: second_respondent,
            ..first_binding
        },
        ApprovalDecision::Approve(vec![Scope::FileRead]),
    );
    assert!(
        matches!(
            wrong_respondent,
            Err(RuntimeError::ApprovalQuestion(
                ApprovalQuestionError::BindingMismatch { .. }
            ))
        ),
        "only the bound respondent may answer"
    );
    let wrong_effect = answer_question(
        &mut rt,
        AnswerBinding {
            effect: second_effect,
            ..first_binding
        },
        ApprovalDecision::Approve(vec![Scope::FileRead]),
    );
    assert!(
        matches!(
            wrong_effect,
            Err(RuntimeError::ApprovalQuestion(
                ApprovalQuestionError::BindingMismatch { .. }
            ))
        ),
        "an answer for another effect cannot resolve this question"
    );
    let wrong_schema = answer_question(
        &mut rt,
        AnswerBinding {
            schema: digest(b"schema-v2"),
            ..first_binding
        },
        ApprovalDecision::Approve(vec![Scope::FileRead]),
    );
    assert!(
        matches!(
            wrong_schema,
            Err(RuntimeError::ApprovalQuestion(
                ApprovalQuestionError::BindingMismatch { .. }
            ))
        ),
        "an answer using another schema revision is stale"
    );
    answer_question(
        &mut rt,
        first_binding,
        ApprovalDecision::Approve(vec![Scope::FileRead]),
    )?;
    let second_binding = AnswerBinding::new(
        second_question,
        second_tenant,
        second_respondent,
        second_operation,
        second_effect,
    );
    answer_question(
        &mut rt,
        second_binding,
        ApprovalDecision::Approve(vec![Scope::FileRead]),
    )?;
    rt.submit(cancel(command_id(3), &second_body)?)?;
    rt.tick()?;
    assert_eq!(
        rt.approval_question_status(first_question)?,
        ApprovalQuestionStatus::Approved(vec![Scope::FileRead]),
        "cancelling the other run leaves this wait unaffected"
    );
    assert_eq!(
        rt.approval_question_status(second_question)?,
        ApprovalQuestionStatus::Cancelled,
        "cancellation discards a queued answer"
    );
    let late_answer = answer_question(
        &mut rt,
        second_binding,
        ApprovalDecision::Approve(vec![Scope::FileRead]),
    );
    assert!(
        matches!(
            late_answer,
            Err(RuntimeError::ApprovalQuestion(
                ApprovalQuestionError::AlreadyResolved { .. }
            ))
        ),
        "a response after cancellation is refused"
    );
    assert_eq!(
        rt.owner().dispatched(),
        &[first_operation, second_operation],
        "answering questions does not dispatch another effect"
    );
    trace.extend_from_slice(&first_question.value().to_be_bytes());
    trace.extend_from_slice(&second_question.value().to_be_bytes());
    trace.push(u8::from(matches!(
        rt.approval_question_status(first_question)?,
        ApprovalQuestionStatus::Approved(_)
    )));
    trace.push(u8::from(matches!(
        rt.approval_question_status(second_question)?,
        ApprovalQuestionStatus::Cancelled
    )));
    Ok(digest(&trace))
}

#[test]
fn e30_concurrent_questions_keep_independent_exact_bindings_under_1000_seeds()
-> Result<(), Box<dyn std::error::Error>> {
    for seed in 0_u64..1_000 {
        assert_eq!(
            concurrent_case(seed)?,
            concurrent_case(seed)?,
            "seed {seed} replays identically"
        );
    }
    Ok(())
}

/// Runs one seeded same-operation cross-tenant isolation scenario.
fn tenant_case(seed: u64) -> Result<ContentDigest, Box<dyn std::error::Error>> {
    let shared_operation = operation(id_byte(seed, 20));
    let first_run = run(id_byte(seed, 1));
    let second_run = run(id_byte(seed, 2));
    let first_tenant = tenant(id_byte(seed, 10));
    let second_tenant = tenant(id_byte(seed, 11));
    let first_respondent = worker(id_byte(seed, 31));
    let second_respondent = worker(id_byte(seed, 32));
    let first_effect = digest(&seed.to_be_bytes());
    let second_effect = digest(&seed.saturating_add(1).to_be_bytes());
    let expiry = epoch()?
        .checked_add(Duration::from_secs(60))
        .ok_or("expiry overflow")?;
    let first_body = proposal(
        first_run,
        first_tenant,
        shared_operation,
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let second_body = proposal(
        second_run,
        second_tenant,
        shared_operation,
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let mut first = runtime()?;
    let mut second = runtime()?;
    first.submit(submit(command_id(1), &first_body)?)?;
    second.submit(submit(command_id(2), &second_body)?)?;
    first.tick()?;
    second.tick()?;
    let first_question = open_question(
        &mut first,
        question_spec(
            first_tenant,
            first_run,
            shared_operation,
            worker(id_byte(seed, 30)),
            first_respondent,
            first_effect,
            expiry,
        )?,
        Some(first_respondent),
    )?;
    let second_question = open_question(
        &mut second,
        question_spec(
            second_tenant,
            second_run,
            shared_operation,
            worker(id_byte(seed, 30)),
            second_respondent,
            second_effect,
            expiry,
        )?,
        Some(second_respondent),
    )?;
    let cross_tenant = answer_question(
        &mut first,
        AnswerBinding::new(
            first_question,
            second_tenant,
            second_respondent,
            shared_operation,
            second_effect,
        ),
        ApprovalDecision::Approve(vec![Scope::FileRead]),
    );
    assert!(
        matches!(
            cross_tenant,
            Err(RuntimeError::ApprovalQuestion(
                ApprovalQuestionError::BindingMismatch { .. }
            ))
        ),
        "tenant identity is part of the answer binding"
    );
    answer_question(
        &mut second,
        AnswerBinding::new(
            second_question,
            second_tenant,
            second_respondent,
            shared_operation,
            second_effect,
        ),
        ApprovalDecision::Approve(vec![Scope::FileRead]),
    )?;
    second.tick()?;
    assert_eq!(
        first.approval_question_status(first_question)?,
        ApprovalQuestionStatus::Pending,
        "the same operation ID in another tenant cannot consume this question"
    );
    assert_eq!(
        second.approval_question_status(second_question)?,
        ApprovalQuestionStatus::Approved(vec![Scope::FileRead]),
        "the owning tenant can still resolve its own question"
    );
    let mut trace = Vec::new();
    trace.extend_from_slice(&first_question.value().to_be_bytes());
    trace.extend_from_slice(&second_question.value().to_be_bytes());
    trace.extend_from_slice(first_effect.as_bytes());
    trace.extend_from_slice(second_effect.as_bytes());
    Ok(digest(&trace))
}

#[test]
fn e30_identical_operation_ids_remain_tenant_isolated_across_1000_seeds()
-> Result<(), Box<dyn std::error::Error>> {
    for seed in 0_u64..1_000 {
        assert_eq!(
            tenant_case(seed)?,
            tenant_case(seed)?,
            "seed {seed} replays identically"
        );
    }
    Ok(())
}

#[test]
fn e30_headless_refusal_and_expiry_never_become_consent_or_block_another_wait()
-> Result<(), Box<dyn std::error::Error>> {
    let sample_success = succeeded(operation(3));
    assert_eq!(
        sample_success.operation(),
        operation(3),
        "owner outcomes preserve the exact operation identity"
    );
    let first_run = run(1);
    let second_run = run(2);
    let third_run = run(3);
    let first_tenant = tenant(1);
    let second_tenant = tenant(2);
    let third_tenant = tenant(3);
    let first_body = proposal(
        first_run,
        first_tenant,
        operation(1),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let second_body = proposal(
        second_run,
        second_tenant,
        operation(2),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let third_body = proposal(
        third_run,
        third_tenant,
        operation(3),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let mut rt = runtime_with(
        rocco_runtime::owner::fixture::ScriptedOwner::new(),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    rt.submit(submit(command_id(1), &first_body)?)?;
    rt.submit(submit(command_id(2), &second_body)?)?;
    rt.submit(submit(command_id(3), &third_body)?)?;
    rt.tick()?;

    let now = epoch()?;
    let future = now
        .checked_add(Duration::from_secs(60))
        .ok_or("expiry overflow")?;
    let first_effect = digest(b"first-effect");
    let second_effect = digest(b"second-effect");
    let first_question = open_question(
        &mut rt,
        question_spec(
            first_tenant,
            first_run,
            operation(1),
            worker(11),
            worker(21),
            first_effect,
            future,
        )?,
        Some(worker(21)),
    )?;
    let second_question = open_question(
        &mut rt,
        question_spec(
            second_tenant,
            second_run,
            operation(2),
            worker(12),
            worker(22),
            second_effect,
            future,
        )?,
        Some(worker(22)),
    )?;
    let headless = open_question(
        &mut rt,
        question_spec(
            third_tenant,
            third_run,
            operation(3),
            worker(13),
            worker(23),
            digest(b"third-effect"),
            now,
        )?,
        None,
    );
    assert!(
        matches!(
            headless,
            Err(RuntimeError::ApprovalQuestion(ApprovalQuestionError::NoInteractiveClient { operation: got_operation })) if got_operation == operation(3)
        ),
        "headless callers receive typed AwaitingAuthority instead of consent"
    );

    answer_question(
        &mut rt,
        AnswerBinding::new(
            first_question,
            first_tenant,
            worker(21),
            operation(1),
            first_effect,
        ),
        ApprovalDecision::Approve(vec![Scope::FileRead]),
    )?;
    answer_question(
        &mut rt,
        AnswerBinding::new(
            second_question,
            second_tenant,
            worker(22),
            operation(2),
            second_effect,
        ),
        ApprovalDecision::Approve(vec![Scope::FileRead]),
    )?;
    rt.tick()?;
    assert_eq!(
        rt.approval_question_status(first_question)?,
        ApprovalQuestionStatus::Approved(vec![Scope::FileRead]),
        "the first unrelated wait resolves independently"
    );
    assert_eq!(
        rt.approval_question_status(second_question)?,
        ApprovalQuestionStatus::Approved(vec![Scope::FileRead]),
        "the second unrelated wait resolves independently"
    );

    let expired = open_question(
        &mut rt,
        question_spec(
            first_tenant,
            first_run,
            operation(1),
            worker(11),
            worker(21),
            digest(b"expired-effect"),
            now,
        )?,
        Some(worker(21)),
    )?;
    answer_question(
        &mut rt,
        AnswerBinding::new(
            expired,
            first_tenant,
            worker(21),
            operation(1),
            digest(b"expired-effect"),
        ),
        ApprovalDecision::Approve(vec![Scope::FileRead]),
    )?;
    rt.tick()?;
    assert_eq!(
        rt.approval_question_status(expired)?,
        ApprovalQuestionStatus::TimedOut,
        "an answer at the pinned expiry instant never becomes consent"
    );
    assert_eq!(
        rt.approval_question_status(first_question)?,
        ApprovalQuestionStatus::Approved(vec![Scope::FileRead]),
        "expiry of another question does not alter this result"
    );
    assert_eq!(
        rt.owner().dispatched(),
        &[operation(1), operation(2), operation(3)],
        "approval answers do not trigger another owner reach"
    );
    Ok(())
}

#[test]
fn e30_same_operation_isolated_between_tenants_and_question_capacity_is_bounded()
-> Result<(), Box<dyn std::error::Error>> {
    let now = epoch()?;
    let expiry = now
        .checked_add(Duration::from_secs(60))
        .ok_or("expiry overflow")?;
    let shared_operation = operation(40);
    let first_body = proposal(
        run(40),
        tenant(40),
        shared_operation,
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let second_body = proposal(
        run(41),
        tenant(41),
        shared_operation,
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let mut first = runtime()?;
    let mut second = runtime()?;
    first.submit(submit(command_id(40), &first_body)?)?;
    second.submit(submit(command_id(41), &second_body)?)?;
    first.tick()?;
    second.tick()?;
    let first_effect = digest(b"tenant-40-effect");
    let second_effect = digest(b"tenant-41-effect");
    let first_question = open_question(
        &mut first,
        question_spec(
            tenant(40),
            run(40),
            shared_operation,
            worker(40),
            worker(50),
            first_effect,
            expiry,
        )?,
        Some(worker(50)),
    )?;
    let second_question = open_question(
        &mut second,
        question_spec(
            tenant(41),
            run(41),
            shared_operation,
            worker(41),
            worker(51),
            second_effect,
            expiry,
        )?,
        Some(worker(51)),
    )?;

    let first_binding = AnswerBinding::new(
        first_question,
        tenant(40),
        worker(50),
        shared_operation,
        first_effect,
    );
    let second_binding = AnswerBinding::new(
        second_question,
        tenant(41),
        worker(51),
        shared_operation,
        second_effect,
    );
    let cross_tenant = answer_question(
        &mut first,
        second_binding,
        ApprovalDecision::Approve(vec![Scope::FileRead]),
    );
    assert!(
        matches!(
            cross_tenant,
            Err(RuntimeError::ApprovalQuestion(
                ApprovalQuestionError::BindingMismatch { .. }
            ))
        ),
        "a tenant cannot answer another tenant's question"
    );
    let invalid_scope = answer_question(
        &mut first,
        first_binding,
        ApprovalDecision::Approve(vec![Scope::FileWrite]),
    );
    assert!(
        matches!(
            invalid_scope,
            Err(RuntimeError::ApprovalQuestion(
                ApprovalQuestionError::ScopeOutsideQuestion { .. }
            ))
        ),
        "approval is limited to the question's allowed scope"
    );

    let repeated_question = question_spec(
        tenant(40),
        run(40),
        shared_operation,
        worker(40),
        worker(50),
        first_effect,
        expiry,
    )?;
    for _ in 1..rocco_runtime::approval::MAX_APPROVAL_QUESTIONS {
        open_question(&mut first, repeated_question.clone(), Some(worker(50)))?;
    }
    let overflow = open_question(&mut first, repeated_question, Some(worker(50)));
    assert!(
        matches!(
            overflow,
            Err(RuntimeError::ApprovalQuestion(ApprovalQuestionError::CapacityReached { capacity }))
                if capacity == rocco_runtime::approval::MAX_APPROVAL_QUESTIONS
        ),
        "question retention refuses overflow rather than evicting a wait"
    );

    answer_question(
        &mut second,
        second_binding,
        ApprovalDecision::Approve(vec![Scope::FileRead]),
    )?;
    second.tick()?;
    assert_eq!(
        second.approval_question_status(second_question)?,
        ApprovalQuestionStatus::Approved(vec![Scope::FileRead]),
        "the correct tenant still resolves its own question"
    );
    assert_eq!(
        first.approval_question_status(first_question)?,
        ApprovalQuestionStatus::Pending,
        "the other tenant's question remains pending"
    );
    let orphan = runtime()?;
    let missing_question = orphan.approval_question_status(first_question);
    assert!(
        matches!(
            missing_question,
            Err(RuntimeError::ApprovalQuestion(
                ApprovalQuestionError::UnknownQuestion { .. }
            ))
        ),
        "question identities do not cross runtime lifetimes"
    );
    let oversized_schema = ApprovalQuestionSpec::new(
        ApprovalQuestionParties::new(tenant(40), run(40), worker(40), worker(50)),
        ApprovalEffectBinding::new(shared_operation, first_effect, digest(b"schema-v1")),
        expiry,
        vec![Scope::FileRead; 17],
    );
    assert!(
        matches!(
            oversized_schema,
            Err(ApprovalQuestionError::ScopeListTooLarge {
                capacity: rocco_runtime::approval::MAX_APPROVAL_ANSWER_SCOPES,
                ..
            })
        ),
        "question schemas reject unbounded scope lists"
    );
    Ok(())
}
