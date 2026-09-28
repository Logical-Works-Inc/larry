//! E47 public runtime protocol-boundary falsifiers.

mod support;

use rocco_contracts::authority::Scope;
use rocco_contracts::command::Command;
use rocco_contracts::effect::ReconciliationStrategy;
use rocco_contracts::ids::ContentDigest;
use rocco_contracts::ids::{CommandId, OperationId, RunId, TenantId};
use rocco_contracts::state::RunState;
use rocco_runtime::error::RuntimeError;
use rocco_runtime::owner::fixture::ScriptedOwner;
use rocco_runtime::protocol::{
    MAX_PROTOCOL_BACKOFF_TICKS, MAX_PROTOCOL_VERSION, MIN_PROTOCOL_VERSION, ProtocolCapability,
    ProtocolError, ProtocolOffer, ProtocolVersionRange,
};

use support::{
    assert_state, command_id, durable_runtime_with, operation, proposal, run, submit, succeeded,
    tenant,
};

fn offer(
    tenant_id: TenantId,
    versions: ProtocolVersionRange,
    capabilities: &[ProtocolCapability],
    headless: bool,
    question: bool,
) -> ProtocolOffer {
    ProtocolOffer::new(
        tenant_id,
        versions,
        capabilities.iter().copied().collect(),
        headless,
        question,
    )
}

fn command_for(
    tenant_id: TenantId,
    id: CommandId,
    run_id: RunId,
) -> Result<Command, Box<dyn std::error::Error>> {
    let body = proposal(
        run_id,
        tenant_id,
        operation(1),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    submit(id, &body)
}

#[test]
fn e47_refusals_are_actionable_scoped_and_non_mutating() -> Result<(), Box<dyn std::error::Error>> {
    let tenant_a = tenant(1);
    let mut rt = durable_runtime_with(
        ScriptedOwner::scripted([succeeded(operation(1))]),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    let command = command_for(tenant_a, command_id(1), run(1))?;

    let unsupported = offer(
        tenant_a,
        ProtocolVersionRange::new(3, 5),
        &[ProtocolCapability::SubmitCommand],
        false,
        false,
    );
    let result = rt.submit_protocol(&unsupported, command.clone());
    assert!(matches!(
        &result,
        Err(RuntimeError::Protocol(ProtocolError::UnsupportedVersion {
            supported_min: MIN_PROTOCOL_VERSION,
            supported_max: MAX_PROTOCOL_VERSION,
            ..
        }))
    ));
    let message = result
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(message.contains("supported range is 1..=1"), "{message}");

    let reversed = offer(
        tenant_a,
        ProtocolVersionRange::new(5, 2),
        &[ProtocolCapability::SubmitCommand],
        false,
        false,
    );
    assert!(matches!(
        rt.submit_protocol(&reversed, command.clone()),
        Err(RuntimeError::Protocol(ProtocolError::InvalidVersionRange {
            min: 5,
            max: 2
        }))
    ));

    let absent = offer(tenant_a, ProtocolVersionRange::new(1, 1), &[], false, false);
    assert!(matches!(
        rt.submit_protocol(&absent, command.clone()),
        Err(RuntimeError::Protocol(ProtocolError::CapabilityMissing {
            capability: ProtocolCapability::SubmitCommand
        }))
    ));

    let missing_question = offer(
        tenant_a,
        ProtocolVersionRange::new(1, 1),
        &[ProtocolCapability::SubmitCommand],
        false,
        true,
    );
    assert!(matches!(
        rt.submit_protocol(&missing_question, command.clone()),
        Err(RuntimeError::Protocol(ProtocolError::CapabilityMissing {
            capability: ProtocolCapability::HumanQuestion
        }))
    ));

    let headless = offer(
        tenant_a,
        ProtocolVersionRange::new(1, 1),
        &[
            ProtocolCapability::SubmitCommand,
            ProtocolCapability::HumanQuestion,
        ],
        true,
        true,
    );
    assert!(matches!(
        rt.submit_protocol(&headless, command.clone()),
        Err(RuntimeError::Protocol(ProtocolError::HeadlessQuestion))
    ));

    let cross_tenant = command_for(tenant(2), command_id(2), run(1))?;
    let valid = offer(
        tenant_a,
        ProtocolVersionRange::new(0, 3),
        &[ProtocolCapability::SubmitCommand],
        false,
        false,
    );
    assert!(matches!(
        rt.submit_protocol(&valid, cross_tenant),
        Err(RuntimeError::Protocol(ProtocolError::TenantMismatch { .. }))
    ));
    assert_eq!(rt.journal().len(), 0);
    assert_eq!(rt.tick_count(), 0);

    let negotiated = rt.submit_protocol(&valid, command)?;
    assert_eq!(negotiated.tenant, tenant_a);
    assert_eq!(negotiated.version, MAX_PROTOCOL_VERSION);
    assert_eq!(
        rt.journal().len(),
        1,
        "durable admission journals exactly the negotiated command"
    );
    assert_eq!(rt.tick_count(), 0);
    let accepted = rt.tick()?;
    assert!(
        accepted.admitted().is_empty(),
        "admission happened durably at ingress; the tick never admits twice"
    );
    assert!(accepted.duplicates().is_empty());
    assert!(!rt.journal().is_empty());
    assert_state(rt.projection(), run(1), RunState::Completed)?;
    Ok(())
}

#[test]
fn e47_overload_backoff_is_bounded_deterministic_and_does_not_append()
-> Result<(), Box<dyn std::error::Error>> {
    let tenant_id = tenant(9);
    let command = command_for(tenant_id, command_id(9), run(9))?;
    let mut rt = durable_runtime_with(
        ScriptedOwner::new(),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    let valid = offer(
        tenant_id,
        ProtocolVersionRange::new(1, 1),
        &[ProtocolCapability::SubmitCommand],
        false,
        false,
    );
    // Distinct identities: a resubmitted identity is an idempotent duplicate
    // under durable admission and would never fill the queue.
    let distinct = |index: usize| -> Result<Command, Box<dyn std::error::Error>> {
        let mut identity = [0x47_u8; 16];
        identity[14..].copy_from_slice(&u16::try_from(index)?.to_be_bytes());
        let body = proposal(
            RunId::from_bytes(identity),
            tenant_id,
            OperationId::from_bytes(identity),
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::Queryable,
        );
        submit(CommandId::from_bytes(identity), &body)
    };
    for index in 0..rocco_runtime::MAX_READY_WORK {
        rt.submit_protocol(&valid, distinct(index)?)?;
    }
    let admitted = rt.journal().len();
    assert_eq!(admitted, rocco_runtime::MAX_READY_WORK);

    let first = rt.submit_protocol(&valid, command.clone());
    let second = rt.submit_protocol(&valid, command);
    let retry_delay = match first {
        Err(RuntimeError::Protocol(ProtocolError::Overloaded {
            capacity,
            retry_after_ticks,
        })) => {
            assert_eq!(capacity, rocco_runtime::MAX_READY_WORK);
            retry_after_ticks
        }
        other => return Err(format!("expected typed overload, got {other:?}").into()),
    };
    assert!(matches!(
        second,
        Err(RuntimeError::Protocol(ProtocolError::Overloaded {
            retry_after_ticks,
            ..
        })) if retry_after_ticks == retry_delay
    ));
    assert!((1..=MAX_PROTOCOL_BACKOFF_TICKS).contains(&retry_delay));
    assert_eq!(retry_delay, MAX_PROTOCOL_BACKOFF_TICKS);
    assert_eq!(
        rt.journal().len(),
        admitted,
        "overload refusals append nothing"
    );
    assert_eq!(rt.tick_count(), 0);

    for _ in 0..MAX_PROTOCOL_BACKOFF_TICKS {
        rt.tick()?;
    }
    let recovered_body = proposal(
        run(10),
        tenant_id,
        operation(10),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let recovered = submit(command_id(10), &recovered_body)?;
    let before = rt.journal().len();
    let negotiated = rt.submit_protocol(&valid, recovered)?;
    assert_eq!(negotiated.tenant, tenant_id);
    assert_eq!(negotiated.version, MAX_PROTOCOL_VERSION);
    assert_eq!(
        rt.journal().len(),
        before.saturating_add(1),
        "after back-off the same ingress durably admits exactly this command"
    );
    Ok(())
}

#[test]
fn e47_seeded_negotiation_replays_the_same_trace_for_one_thousand_seeds()
-> Result<(), Box<dyn std::error::Error>> {
    fn trace() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut outcomes = Vec::with_capacity(1_000);
        for seed in 0_u64..1_000 {
            let mut identity = [0_u8; 16];
            identity[..8].copy_from_slice(&seed.to_le_bytes());
            let tenant_id = TenantId::from_bytes([0x55; 16]);
            let command = command_for(
                tenant_id,
                CommandId::from_bytes(identity),
                RunId::from_bytes(identity),
            )?;
            let mut rt = durable_runtime_with(
                ScriptedOwner::new(),
                &[Scope::FileRead],
                &[Scope::FileRead],
                100,
            )?;
            let version = if seed & 1 == 0 { 1 } else { 2 };
            let candidate = offer(
                tenant_id,
                ProtocolVersionRange::new(version, version),
                &[ProtocolCapability::SubmitCommand],
                false,
                false,
            );
            let accepted = rt.submit_protocol(&candidate, command).is_ok();
            outcomes.push(u8::from(accepted));
            // A refusal appends nothing; an acceptance appends exactly one.
            assert_eq!(rt.journal().len(), usize::from(accepted));
        }
        Ok(outcomes)
    }

    let first = trace()?;
    let replayed = trace()?;
    assert_eq!(first, replayed);
    assert_eq!(
        ContentDigest::over(&[&first]),
        ContentDigest::over(&[&replayed]),
        "same seed set must produce the same trace hash"
    );
    let accepted = first.iter().filter(|outcome| **outcome == 1).count();
    assert_eq!(accepted, 500);
    Ok(())
}
