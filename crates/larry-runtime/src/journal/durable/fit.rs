//! The largest allowed adoption fits every journal record it writes, with a quarter to spare.
//!
//! `MAX_ADOPTED_TREE_BYTES` is chosen from this measurement, not from the harness value's own
//! bounds: lgwks_bot caps a record at 64 KiB and Larry's records spell bytes as JSON numbers
//! (ADR 0029).

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use larry_contracts::authority::{BudgetUnit, Scope};
use larry_contracts::command::{Command, CommandContent, CommandKind};
use larry_contracts::effect::{EffectOutcome, ReconciliationStrategy};
use larry_contracts::harness::{
    HarnessAdoption, HarnessAuthor, HarnessComponent, HarnessTree, HarnessVersion,
    MAX_ADOPTED_TREE_BYTES,
};
use larry_contracts::ids::{CommandId, OperationId, RunId, TenantId};
use larry_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};
use lgwks_bot::clock::Clock;

use super::{MAX_RECORD_BYTES, encode};
use crate::owner::fixture::ScriptedOwner;
use crate::payload::WorkProposal;
use crate::projection::FoldProjection;
use crate::runtime::{Runtime, RuntimeSettings};

type Outcome = Result<(), Box<dyn std::error::Error>>;

#[test]
fn the_largest_adoption_fits_every_record_it_writes_with_a_quarter_to_spare() -> Outcome {
    let scopes: BTreeSet<Scope> = [
        Scope::FileRead,
        Scope::FileWrite,
        Scope::NetworkEgress,
        Scope::ProcessSpawn,
        Scope::ModelInvoke,
    ]
    .into_iter()
    .collect();
    let start = Duration::from_secs(1_700_000_000);
    let valid_until = SystemTime::UNIX_EPOCH
        .checked_add(start)
        .and_then(|base| base.checked_add(Duration::from_secs(3_600)))
        .ok_or("validity")?;
    let settings = RuntimeSettings::with_defaults(
        PolicySnapshot::new(1, scopes.clone()),
        CapabilitySnapshot::new(scopes.clone()),
        ValiditySnapshot::new(valid_until),
        BudgetUnit::Requests,
        10,
    );
    let owner = ScriptedOwner::scripted([
        EffectOutcome::Succeeded {
            operation: OperationId::from_bytes([1; 16]),
        },
        EffectOutcome::Succeeded {
            operation: OperationId::from_bytes([2; 16]),
        },
    ]);
    let mut runtime: Runtime<FoldProjection, ScriptedOwner> =
        Runtime::new(settings, owner, Clock::virtual_at(start))?;
    let tenant = TenantId::from_bytes([3; 16]);

    let mut small = HarnessTree::empty();
    small.insert(HarnessComponent::SystemPrompt, "main", b"first")?;
    let mut largest = HarnessTree::empty();
    // Header 5 + tag, name length, name and data length 7 = 12 bytes around the data.
    largest.insert(
        HarnessComponent::Skills,
        "a",
        &vec![0xff; MAX_ADOPTED_TREE_BYTES.saturating_sub(12)],
    )?;
    assert_eq!(
        largest.encode().len(),
        MAX_ADOPTED_TREE_BYTES,
        "the tree is the largest allowed"
    );
    let control_reason = "\u{1}".repeat(512);
    let first = HarnessVersion::new(&small, None, HarnessAuthor::Human, "first")?;
    let second = HarnessVersion::new(
        &largest,
        Some(small.hash()),
        HarnessAuthor::Proposer,
        &control_reason,
    )?;
    for (number, adoption) in [
        (1_u8, HarnessAdoption::recording(first, &small)),
        (2_u8, HarnessAdoption::recording(second, &largest)),
    ] {
        let work = WorkProposal::new(
            RunId::from_bytes([number; 16]),
            tenant,
            OperationId::from_bytes([number; 16]),
            scopes.clone(),
            1,
            BudgetUnit::Requests,
            ReconciliationStrategy::Queryable,
        )
        .adopting(adoption);
        runtime.submit(Command::new(
            CommandId::from_bytes([number; 16]),
            tenant,
            Vec::new(),
            CommandContent::new(
                CommandKind::Submit,
                lgwks_std::json::to_vec(&work)?.into_boxed_slice(),
            ),
        )?)?;
        runtime.tick()?;
    }
    assert_eq!(
        runtime
            .harness(tenant)
            .and_then(|held| held.active())
            .map(|(version, _)| version.hash()),
        Some(largest.hash()),
        "the largest adoption was adopted"
    );
    let widest = runtime
        .journal()
        .entries()
        .iter()
        .map(|entry| encode(entry).map(|bytes| bytes.len()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .ok_or("the journal holds entries")?;
    assert!(
        widest.saturating_mul(4) <= MAX_RECORD_BYTES.saturating_mul(3),
        "the widest record is {widest} bytes of {MAX_RECORD_BYTES}: keep a quarter spare"
    );
    Ok(())
}

/// A recursion step's references (plan, label, candidate, produced hash) fit every record they are
/// written into, with a quarter to spare (larry#62).
#[test]
fn the_widest_recursion_step_fits_every_record_it_writes_with_a_quarter_to_spare() -> Outcome {
    use larry_contracts::harness::HarnessHash;
    use larry_contracts::ids::ContentDigest;
    use larry_contracts::recursion::{Label, RecursionMode, RecursionPlan, RecursionStep, Stage};

    let scopes: BTreeSet<Scope> = [
        Scope::FileRead,
        Scope::FileWrite,
        Scope::NetworkEgress,
        Scope::ProcessSpawn,
        Scope::ModelInvoke,
    ]
    .into_iter()
    .collect();
    let start = Duration::from_secs(1_700_000_000);
    let valid_until = SystemTime::UNIX_EPOCH
        .checked_add(start)
        .and_then(|base| base.checked_add(Duration::from_secs(3_600)))
        .ok_or("validity")?;
    let settings = RuntimeSettings::with_defaults(
        PolicySnapshot::new(1, scopes.clone()),
        CapabilitySnapshot::new(scopes.clone()),
        ValiditySnapshot::new(valid_until),
        BudgetUnit::Requests,
        20,
    );
    let outcomes: Vec<EffectOutcome> = (1..=7_u8)
        .map(|number| EffectOutcome::Succeeded {
            operation: OperationId::from_bytes([number; 16]),
        })
        .collect();
    let mut runtime: Runtime<FoldProjection, ScriptedOwner> = Runtime::new(
        settings,
        ScriptedOwner::scripted(outcomes),
        Clock::virtual_at(start),
    )?;
    let tenant = TenantId::from_bytes([3; 16]);
    let base: HarnessHash = HarnessTree::empty().hash();
    let plan = RecursionPlan::new(
        ContentDigest::over(&[b"task"]),
        RecursionMode::Sequential,
        8,
        base,
        16,
    )?;
    let labels = [
        Label::new(0, Stage::Propose, 0),
        Label::new(0, Stage::Propose, 1),
        Label::new(0, Stage::Probe, 0),
        Label::new(0, Stage::Probe, 1),
        Label::new(0, Stage::Probe, 2),
        Label::new(0, Stage::Probe, 3),
        Label::new(0, Stage::Compose, 0),
    ];
    for (index, label) in labels.into_iter().enumerate() {
        let number = u8::try_from(index)?.saturating_add(1);
        let mut tree = HarnessTree::empty();
        tree.insert(HarnessComponent::Memory, &format!("n{number}"), b"x")?;
        let step = RecursionStep::new(plan, label, 0);
        let step = if label.stage() == Stage::Probe {
            step.with_candidate(
                runtime
                    .recursion(tenant, &plan)
                    .and_then(|held| held.candidate(0, label.slot() >> 1))
                    .ok_or("candidate")?,
            )
        } else {
            step.producing(tree.hash())
        };
        let work = WorkProposal::new(
            RunId::from_bytes([number; 16]),
            tenant,
            OperationId::from_bytes([number; 16]),
            scopes.clone(),
            1,
            BudgetUnit::Requests,
            ReconciliationStrategy::Queryable,
        )
        .stepping(step);
        runtime.submit(Command::new(
            CommandId::from_bytes([number; 16]),
            tenant,
            Vec::new(),
            CommandContent::new(
                CommandKind::Submit,
                lgwks_std::json::to_vec(&work)?.into_boxed_slice(),
            ),
        )?)?;
        runtime.tick()?;
    }
    assert!(
        runtime
            .recursion(tenant, &plan)
            .and_then(|held| held.state(Label::new(0, Stage::Compose, 0)))
            .is_some_and(larry_contracts::recursion::StepState::is_settled),
        "round 0 completed"
    );
    let widest = runtime
        .journal()
        .entries()
        .iter()
        .map(|entry| encode(entry).map(|bytes| bytes.len()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .ok_or("the journal holds entries")?;
    assert!(
        widest.saturating_mul(4) <= MAX_RECORD_BYTES.saturating_mul(3),
        "the widest record is {widest} bytes of {MAX_RECORD_BYTES}"
    );
    Ok(())
}
