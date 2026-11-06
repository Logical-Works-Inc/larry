//! Cross-crate construction and canonical identity regression tests.

use std::collections::BTreeSet;
use std::time::SystemTime;

use larry_contracts::authority::{BudgetUnit, Scope, SecurityEpoch};
use larry_contracts::command::{CommandContent, CommandKind};
use larry_contracts::effect::{
    EffectIntent, ReconciliationStrategy, TerminalDisposition, TerminalReceipt,
};
use larry_contracts::event::{CausalRef, Event, EventKind, EventSequence};
use larry_contracts::ids::{ContentDigest, EffectId, OperationId, RunId, TenantId};
use larry_contracts::replay::{
    BudgetSnapshot, CapabilitySnapshot, Decision, DenialReason, EffectRequest, PinnedInputs,
    PolicySnapshot, ReplayRecord, ValiditySnapshot, replay_log,
};

#[test]
fn canonical_kind_tags_preserve_existing_command_and_event_digests()
-> Result<(), Box<dyn std::error::Error>> {
    for (kind, tag) in [
        (CommandKind::Submit, 0u8),
        (CommandKind::Steer, 1),
        (CommandKind::Cancel, 2),
    ] {
        let content = CommandContent::new(kind, Box::from(b"payload".as_slice()));
        assert_eq!(kind.tag(), tag, "command tags must remain stable");
        assert_eq!(content.digest(), ContentDigest::over(&[&[tag], b"payload"]));
    }
    let run = RunId::from_bytes([1; 16]);
    let tenant = TenantId::from_bytes([2; 16]);
    let parent = CausalRef::new(run, EventSequence::FIRST);
    assert_eq!(parent.run, run);
    assert_eq!(parent.sequence, EventSequence::FIRST);
    for (kind, tag) in [
        (EventKind::CommandAccepted, 0u8),
        (EventKind::StateTransition, 1),
        (EventKind::EffectIntentRecorded, 2),
        (EventKind::EffectOutcomeRecorded, 3),
        (EventKind::TerminalReceiptPublished, 4),
    ] {
        let event = Event::new(
            tenant,
            run,
            EventSequence::FIRST,
            vec![parent],
            kind,
            Box::from(b"payload".as_slice()),
        )?;
        assert_eq!(kind.tag(), tag, "event tags must remain stable");
        assert_eq!(
            event.payload_digest(),
            ContentDigest::over(&[&[tag], b"payload"])
        );
        assert_eq!(event.causal_parents(), &[parent]);
    }
    Ok(())
}

#[test]
fn intent_construction_preserves_scope_and_receipt_identity() {
    let operation = OperationId::from_bytes([3; 16]);
    let run = RunId::from_bytes([1; 16]);
    let tenant = TenantId::from_bytes([2; 16]);
    let identity = EffectId::from_bytes([4; 16]);
    let epoch = SecurityEpoch::new(9);
    let intent = EffectIntent::new(
        identity,
        operation,
        run,
        tenant,
        ReconciliationStrategy::Queryable,
        epoch,
    );
    assert_eq!(intent.id, identity);
    assert_eq!(intent.operation, operation);
    assert_eq!(intent.run, run);
    assert_eq!(intent.tenant, tenant);
    assert_eq!(intent.strategy, ReconciliationStrategy::Queryable);
    assert_eq!(intent.admission_epoch, epoch);
    for (disposition, tag) in [
        (TerminalDisposition::Succeeded, 0u8),
        (TerminalDisposition::Failed, 1),
        (TerminalDisposition::UnknownPermanent, 2),
        (TerminalDisposition::Compensated, 3),
    ] {
        let receipt = TerminalReceipt::publish(operation, run, tenant, disposition);
        assert_eq!(disposition.tag(), tag);
        assert_eq!(
            receipt.digest(),
            ContentDigest::over(&[
                operation.as_bytes(),
                run.as_bytes(),
                tenant.as_bytes(),
                &[tag]
            ])
        );
    }
}

#[test]
fn constructed_snapshots_still_deny_expired_authority_and_replay_divergence() {
    let request = EffectRequest::new(
        OperationId::from_bytes([3; 16]),
        BTreeSet::from([Scope::FileRead]),
        1,
        BudgetUnit::Requests,
        SystemTime::UNIX_EPOCH,
    );
    let pinned = PinnedInputs::new(
        request,
        PolicySnapshot::new(7, BTreeSet::from([Scope::FileRead])),
        BudgetSnapshot::new(10, BudgetUnit::Requests),
        CapabilitySnapshot::new(BTreeSet::from([Scope::FileRead])),
        ValiditySnapshot::new(SystemTime::UNIX_EPOCH),
    );
    assert_eq!(pinned.policy.revision, 7);
    assert_eq!(pinned.budget.remaining, 10);
    assert_eq!(
        pinned.decide(),
        Decision::Denied {
            reason: DenialReason::AuthorityExpired
        }
    );
    let report = replay_log(&[ReplayRecord::new(pinned, Decision::Authorized)]);
    assert_eq!(report.divergences(), 1);
    assert!(
        !report.is_bit_identical(),
        "an authorized recording must diverge from expired authority"
    );
}

#[cfg(feature = "serde")]
#[test]
fn unrecognized_wire_variants_are_rejected_instead_of_authorized() {
    let command = lgwks_std::json::from_str::<CommandKind>("\"FutureCommand\"");
    let event = lgwks_std::json::from_str::<EventKind>("\"FutureEvent\"");
    let decision = lgwks_std::json::from_str::<Decision>("\"FutureAuthorization\"");
    assert!(command.is_err(), "unsupported commands must not decode");
    assert!(
        event.is_err(),
        "unsupported journal vocabulary must not decode"
    );
    assert!(
        decision.is_err(),
        "unsupported authorization must not decode"
    );
}
