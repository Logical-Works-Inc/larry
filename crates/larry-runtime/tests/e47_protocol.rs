//! E47 public runtime protocol-boundary falsifiers.

mod support;

/// The atomic-compare-and-dispatch owner double and its helpers.
#[path = "support/atomic.rs"]
mod atomic;

use std::time::Duration;

use larry_contracts::authority::{Budget, BudgetUnit, Grant, Scope};
use larry_contracts::command::Command;
use larry_contracts::effect::ReconciliationStrategy;
use larry_contracts::ids::ContentDigest;
use larry_contracts::ids::{CommandId, RunId, TenantId};
use larry_contracts::state::RunState;
use larry_runtime::error::RuntimeError;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::projection::FoldProjection;
use larry_runtime::protocol::{
    MAX_PROTOCOL_VERSION, MIN_PROTOCOL_VERSION, ProtocolCapability, ProtocolError, ProtocolOffer,
    ProtocolVersionRange,
};
use larry_runtime::runtime::Runtime;

use atomic::{AtomicOwner, holder};
use support::{
    assert_state, clock, command_id, epoch, operation, proposal, run, runtime, runtime_with,
    settings, submit, succeeded, tenant,
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
    let mut rt = runtime_with(
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
    let result = rt.submit_protocol(&unsupported, command.clone(), None);
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
        .ok_or("an unsupported version range must be refused")?
        .to_string();
    assert!(message.contains("supported range is 1..=1"), "{message}");

    let reversed = offer(
        tenant_a,
        ProtocolVersionRange::new(5, 2),
        &[ProtocolCapability::SubmitCommand],
        false,
        false,
    );
    assert!(matches!(
        rt.submit_protocol(&reversed, command.clone(), None),
        Err(RuntimeError::Protocol(ProtocolError::InvalidVersionRange {
            min: 5,
            max: 2
        }))
    ));

    let absent = offer(tenant_a, ProtocolVersionRange::new(1, 1), &[], false, false);
    assert!(matches!(
        rt.submit_protocol(&absent, command.clone(), None),
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
        rt.submit_protocol(&missing_question, command.clone(), None),
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
        rt.submit_protocol(&headless, command.clone(), None),
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
        rt.submit_protocol(&valid, cross_tenant, None),
        Err(RuntimeError::Protocol(ProtocolError::TenantMismatch { .. }))
    ));
    assert_eq!(rt.journal().len(), 0);
    assert_eq!(rt.tick_count(), 0);

    let negotiated_version = rt.submit_protocol(&valid, command, None)?;
    assert_eq!(negotiated_version, MAX_PROTOCOL_VERSION);
    assert_eq!(rt.journal().len(), 0, "queueing is not journal acceptance");
    assert_eq!(rt.tick_count(), 0);
    let accepted = rt.tick()?;
    assert_eq!(accepted.admitted(), &[command_id(1)]);
    assert!(accepted.duplicates().is_empty());
    assert!(!rt.journal().is_empty());
    assert_state(rt.projection(), run(1), RunState::Completed)?;
    Ok(())
}

#[test]
fn e47_overload_backoff_is_bounded_deterministic_and_does_not_append()
-> Result<(), Box<dyn std::error::Error>> {
    // MAX_PROTOCOL_BACKOFF_TICKS is crate-internal; the public journeys
    // assert its literal, which the protocol module's compile-time assert
    // pins to the drain bound.
    const BACKOFF_TICKS: u32 = 4;
    let tenant_id = tenant(9);
    let command = command_for(tenant_id, command_id(9), run(9))?;
    let mut rt = runtime_with(
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
    for _ in 0..larry_runtime::MAX_READY_WORK {
        rt.submit_protocol(&valid, command.clone(), None)?;
    }

    let first = rt.submit_protocol(&valid, command.clone(), None);
    let second = rt.submit_protocol(&valid, command, None);
    let retry_delay = match first {
        Err(RuntimeError::Protocol(ProtocolError::Overloaded {
            capacity,
            retry_after_ticks,
        })) => {
            assert_eq!(capacity, larry_runtime::MAX_READY_WORK);
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
    assert!((1..=BACKOFF_TICKS).contains(&retry_delay));
    assert_eq!(retry_delay, BACKOFF_TICKS);
    assert_eq!(rt.journal().len(), 0);
    assert_eq!(rt.tick_count(), 0);

    for _ in 0..BACKOFF_TICKS {
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
    let negotiated_version = rt.submit_protocol(&valid, recovered, None)?;
    assert_eq!(negotiated_version, MAX_PROTOCOL_VERSION);
    assert_eq!(rt.tick()?.admitted(), &[command_id(10)]);
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
            let mut rt = runtime()?;
            let version = if seed & 1 == 0 { 1 } else { 2 };
            let candidate = offer(
                tenant_id,
                ProtocolVersionRange::new(version, version),
                &[ProtocolCapability::SubmitCommand],
                false,
                false,
            );
            outcomes.push(u8::from(
                rt.submit_protocol(&candidate, command, None).is_ok(),
            ));
            assert_eq!(rt.journal().len(), 0);
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

/// A displayed command submitted through the protocol ingress dispatches its
/// bound effect exactly once, and an offer scoped to another tenant is
/// refused with the queue and the journal unchanged (E47 meets E65).
#[test]
fn e47_a_displayed_command_through_protocol_ingress_dispatches_its_bound_effect()
-> Result<(), Box<dyn std::error::Error>> {
    let tenant_a = tenant(3);
    let mut rt: Runtime<FoldProjection, AtomicOwner> = Runtime::new(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        AtomicOwner::new(7, 0),
        clock()?,
    )?;
    let command = command_for(tenant_a, command_id(1), run(1))?;
    let expiry = epoch()?
        .checked_add(Duration::from_secs(7_200))
        .ok_or("fixture grant expiry overflow")?;
    let grant = Grant::new(
        tenant_a,
        holder(),
        [Scope::FileRead].into_iter().collect(),
        Budget::new(5, BudgetUnit::Requests),
        expiry,
        0,
    )?;
    let display = rt.display(&command, holder(), &[grant])?;

    // An offer scoped to another tenant is refused before queueing: the
    // journal stays empty and the queue gains nothing to admit.
    let foreign = offer(
        tenant(4),
        ProtocolVersionRange::new(MIN_PROTOCOL_VERSION, MAX_PROTOCOL_VERSION),
        &[ProtocolCapability::SubmitCommand],
        false,
        false,
    );
    assert!(matches!(
        rt.submit_protocol(&foreign, command.clone(), Some(display.clone())),
        Err(RuntimeError::Protocol(ProtocolError::TenantMismatch { .. }))
    ));
    assert_eq!(rt.journal().len(), 0);
    assert_eq!(rt.tick_count(), 0);

    let negotiated_version = rt.submit_protocol(
        &offer(
            tenant_a,
            ProtocolVersionRange::new(MIN_PROTOCOL_VERSION, MAX_PROTOCOL_VERSION),
            &[ProtocolCapability::SubmitCommand],
            false,
            false,
        ),
        command,
        Some(display),
    )?;
    assert_eq!(negotiated_version, MAX_PROTOCOL_VERSION);
    let accepted = rt.tick()?;
    // Exactly one command was queued across the whole journey: the refused
    // offer contributed nothing.
    assert_eq!(accepted.admitted(), &[command_id(1)]);
    assert!(accepted.duplicates().is_empty());
    // The bound effect dispatched once and the run completed on its outcome.
    assert_eq!(rt.owner().executed().len(), 1);
    assert_eq!(
        rt.owner()
            .executed()
            .first()
            .map(|&(_, dispatched, _)| dispatched),
        Some(operation(1))
    );
    assert_state(rt.projection(), run(1), RunState::Completed)?;
    Ok(())
}
