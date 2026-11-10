//! larry#61 part 2: a harness version becomes active only through the fence, per tenant, and a
//! journal replays to the same active hash. Every test drives the real `Runtime` through its public
//! surface with the scripted owner double; nothing here is owner evidence.

mod sim_kit;
mod support;

use larry_contracts::authority::Scope;
use larry_contracts::effect::{EffectOutcome, ReconciliationStrategy};
use larry_contracts::harness::{
    HarnessAdoption, HarnessAuthor, HarnessComponent, HarnessError, HarnessHash, HarnessTree,
    HarnessVersion, MAX_ADOPTED_TREE_BYTES, MAX_ADOPTED_VERSIONS, TenantHarness,
};
use larry_runtime::error::RuntimeError;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::projection::{FoldProjection, ProjectionBackend};
use larry_runtime::runtime::{TickReport, replay as replay_journal, restore};

use sim_kit::{Sim, replay};
use support::{
    TestRuntime, clock, command_id, operation, proposal, run, runtime_with, settings, submit,
    succeeded, tenant, unknown,
};

/// What a tenant's record shows: the active hash, the active tree's bytes, the adoptions in order.
type Snap = (Option<HarnessHash>, Option<Vec<u8>>, Vec<HarnessHash>);

fn writer(outcomes: Vec<EffectOutcome>) -> Sim<TestRuntime> {
    runtime_with(
        ScriptedOwner::scripted(outcomes),
        &[Scope::FileWrite],
        &[Scope::FileWrite],
        100,
    )
}

fn all_succeed(count: u8) -> Vec<EffectOutcome> {
    (1..=count)
        .map(|number| succeeded(operation(number)))
        .collect()
}

fn tree_of(seed: u64, variant: u8) -> Result<HarnessTree, HarnessError> {
    let mut tree = HarnessTree::empty();
    tree.insert(
        HarnessComponent::SystemPrompt,
        "main",
        format!("prompt {seed} {variant}").as_bytes(),
    )?;
    tree.insert(
        HarnessComponent::Skills,
        &format!("skill-{variant}"),
        &[variant; 40],
    )?;
    Ok(tree)
}

fn recording(tree: &HarnessTree, parent: Option<HarnessHash>) -> Sim<HarnessAdoption> {
    let version = HarnessVersion::new(tree, parent, HarnessAuthor::Human, "adopt")?;
    Ok(HarnessAdoption::recording(version, tree))
}

fn rolling_back(hash: HarnessHash, parent: Option<HarnessHash>) -> Sim<HarnessAdoption> {
    Ok(HarnessAdoption::rolling_back(HarnessVersion::naming(
        hash,
        parent,
        HarnessAuthor::Human,
        "roll back",
    )?))
}

fn body(
    number: u8,
    owner_tenant: u8,
    scopes: &[Scope],
    adoption: HarnessAdoption,
) -> larry_runtime::payload::WorkProposal {
    proposal(
        run(number),
        tenant(owner_tenant),
        operation(number),
        scopes,
        1,
        ReconciliationStrategy::Queryable,
    )
    .adopting(adoption)
}

/// Submits an adoption as command and run `number` for tenant `owner_tenant` and ticks once.
fn adopt(
    runtime: &mut TestRuntime,
    number: u8,
    owner_tenant: u8,
    adoption: HarnessAdoption,
) -> Sim<TickReport> {
    let work = body(number, owner_tenant, &[Scope::FileWrite], adoption);
    runtime.submit(submit(command_id(number), &work)?)?;
    Ok(runtime.tick()?)
}

/// Submits an adoption that must be refused, and returns the harness refusal. Nothing is written.
fn refused(
    runtime: &mut TestRuntime,
    number: u8,
    owner_tenant: u8,
    scopes: &[Scope],
    adoption: HarnessAdoption,
) -> Sim<HarnessError> {
    let before = runtime.journal().len();
    let work = body(number, owner_tenant, scopes, adoption);
    runtime.submit(submit(command_id(number), &work)?)?;
    let outcome = runtime.tick();
    assert_eq!(
        runtime.journal().len(),
        before,
        "a refused adoption writes nothing"
    );
    assert!(
        runtime.journal().command(command_id(number)).is_none(),
        "a refused adoption is not in the journal"
    );
    match outcome {
        Err(RuntimeError::Harness(refusal)) => Ok(refusal),
        other => Err(format!("expected a harness refusal, got {other:?}").into()),
    }
}

fn active_of(runtime: &TestRuntime, owner_tenant: u8) -> Option<HarnessHash> {
    runtime
        .harness(tenant(owner_tenant))
        .and_then(TenantHarness::active)
        .map(|(version, _)| version.hash())
}

fn snapshot(record: Option<&TenantHarness>) -> Snap {
    (
        record
            .and_then(TenantHarness::active)
            .map(|(version, _)| version.hash()),
        record
            .and_then(TenantHarness::active)
            .map(|(_, tree)| tree.encode()),
        record
            .map(|held| held.adoptions().map(HarnessVersion::hash).collect())
            .unwrap_or_default(),
    )
}

fn journey(seed: u64) -> Sim<Snap> {
    let mut runtime = writer(all_succeed(6))?;
    let mut parent = None;
    let mut hashes = Vec::new();
    for variant in 1..=5_u8 {
        let tree = tree_of(seed, variant)?;
        adopt(&mut runtime, variant, 1, recording(&tree, parent)?)?;
        assert_eq!(
            active_of(&runtime, 1),
            Some(tree.hash()),
            "seed {seed}: adoption {variant}"
        );
        parent = Some(tree.hash());
        hashes.push(tree.hash());
    }
    let target = *hashes.get(1).ok_or("five hashes")?;
    adopt(&mut runtime, 6, 1, rolling_back(target, parent)?)?;
    assert_eq!(
        active_of(&runtime, 1),
        Some(target),
        "seed {seed}: rollback adopts the earlier hash"
    );
    let live = snapshot(runtime.harness(tenant(1)));
    assert_eq!(live.2.len(), 6, "seed {seed}: six adoptions in order");

    let mut folded = FoldProjection::default();
    replay_journal(&mut folded, runtime.journal())?;
    assert_eq!(
        snapshot(folded.harness(tenant(1))),
        live,
        "seed {seed}: replay reproduces the active harness"
    );
    let restored: TestRuntime = restore(
        settings(&[Scope::FileWrite], &[Scope::FileWrite], 100)?,
        ScriptedOwner::new(),
        clock()?,
        runtime.into_journal(),
    )?;
    assert_eq!(
        snapshot(restored.harness(tenant(1))),
        live,
        "seed {seed}: restore reproduces it"
    );
    Ok(live)
}

#[test]
fn sim_harness_five_adoptions_and_a_rollback_replay_to_the_same_active_hash() -> Sim<()> {
    replay(0..32, journey)
}

fn raw_entry(tag: u8, name: &[u8], data: &[u8]) -> Result<Vec<u8>, std::num::TryFromIntError> {
    let mut out = vec![tag, u8::try_from(name.len())?];
    out.extend_from_slice(name);
    out.extend_from_slice(&u32::try_from(data.len())?.to_be_bytes());
    out.extend_from_slice(data);
    Ok(out)
}

fn raw_document(entries: &[Vec<u8>]) -> Result<Vec<u8>, std::num::TryFromIntError> {
    let mut out = vec![1];
    out.extend_from_slice(&u32::try_from(entries.len())?.to_be_bytes());
    for item in entries {
        out.extend_from_slice(item);
    }
    Ok(out)
}

/// An adoption whose tree bytes are exactly `bytes`, as a hostile writer could compose it.
fn hand_built(version: &HarnessVersion, bytes: &[u8]) -> Sim<HarnessAdoption> {
    let text = format!(
        "{{\"version\":{},\"tree\":{bytes:?}}}",
        lgwks_std::json::to_string(version)?
    );
    Ok(lgwks_std::json::from_str(&text)?)
}

#[test]
fn sim_harness_refusals_happen_before_the_journal_write_and_leave_the_harness_adopted() -> Sim<()> {
    for seed in 0..32_u64 {
        support::seeded(seed, || -> Sim<()> {
            let mut runtime = writer(all_succeed(20))?;
            let base = tree_of(seed, 1)?;
            adopt(&mut runtime, 1, 1, recording(&base, None)?)?;
            let held = active_of(&runtime, 1);
            assert_eq!(
                held,
                Some(base.hash()),
                "seed {seed}: the first harness is adopted"
            );

            // The reducer names an off-surface edit; the runtime names a tree that is not canonical.
            let version =
                HarnessVersion::new(&base, Some(base.hash()), HarnessAuthor::Proposer, "x")?;
            let write = &[Scope::FileWrite];
            let off = hand_built(&version, &raw_document(&[raw_entry(5, b"a/b", b"x")?])?)?;
            let unknown_kind = hand_built(&version, &raw_document(&[raw_entry(9, b"a", b"x")?])?)?;
            let unsorted = hand_built(
                &version,
                &raw_document(&[raw_entry(5, b"b", b"x")?, raw_entry(5, b"a", b"x")?])?,
            )?;
            assert_eq!(
                refused(&mut runtime, 2, 1, write, off)?,
                HarnessError::InvalidName {
                    component: HarnessComponent::Skills
                },
                "seed {seed}: a name off the surface"
            );
            assert_eq!(
                refused(&mut runtime, 3, 1, write, unknown_kind)?,
                HarnessError::UnknownComponent { tag: 9 },
                "seed {seed}"
            );
            assert_eq!(
                refused(&mut runtime, 4, 1, write, unsorted)?,
                HarnessError::Unsorted,
                "seed {seed}"
            );

            let other = tree_of(seed, 2)?;
            let mismatched = HarnessAdoption::recording(version, &other);
            assert_eq!(
                refused(&mut runtime, 5, 1, write, mismatched)?,
                HarnessError::HashMismatch,
                "seed {seed}"
            );
            let mut big = HarnessTree::empty();
            big.insert(
                HarnessComponent::Skills,
                "big",
                &vec![7; MAX_ADOPTED_TREE_BYTES],
            )?;
            let big_version =
                HarnessVersion::new(&big, Some(base.hash()), HarnessAuthor::Human, "big")?;
            assert!(
                matches!(
                    refused(
                        &mut runtime,
                        6,
                        1,
                        write,
                        HarnessAdoption::recording(big_version, &big)
                    )?,
                    HarnessError::AdoptedTreeTooLarge { .. }
                ),
                "seed {seed}: a tree past the record bound"
            );
            let next = tree_of(seed, 3)?;
            assert_eq!(
                refused(
                    &mut runtime,
                    7,
                    1,
                    &[Scope::FileRead],
                    recording(&next, Some(base.hash()))?
                )?,
                HarnessError::ScopeNotDeclared,
                "seed {seed}: the gate must decide it"
            );
            assert_eq!(
                refused(
                    &mut runtime,
                    8,
                    1,
                    write,
                    rolling_back(next.hash(), Some(base.hash()))?
                )?,
                HarnessError::UnknownVersion,
                "seed {seed}: rollback to a hash never adopted"
            );
            assert_eq!(
                active_of(&runtime, 1),
                held,
                "seed {seed}: the current harness stays adopted"
            );

            // A following valid adoption still works.
            adopt(&mut runtime, 9, 1, recording(&next, Some(base.hash()))?)?;
            assert_eq!(
                active_of(&runtime, 1),
                Some(next.hash()),
                "seed {seed}: the runtime carries on"
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn sim_harness_a_stale_parent_is_refused_after_the_first_adopts() -> Sim<()> {
    for seed in 0..32_u64 {
        support::seeded(seed, || -> Sim<()> {
            let mut runtime = writer(all_succeed(4))?;
            let (first, second) = (tree_of(seed, 1)?, tree_of(seed, 2)?);
            adopt(&mut runtime, 1, 1, recording(&first, None)?)?;
            // The second proposer read the same empty base.
            let stale = refused(
                &mut runtime,
                2,
                1,
                &[Scope::FileWrite],
                recording(&second, None)?,
            )?;
            assert_eq!(stale, HarnessError::StaleParent, "seed {seed}");
            assert_eq!(
                active_of(&runtime, 1),
                Some(first.hash()),
                "seed {seed}: the first wins"
            );
            adopt(&mut runtime, 3, 1, recording(&second, Some(first.hash()))?)?;
            assert_eq!(
                active_of(&runtime, 1),
                Some(second.hash()),
                "seed {seed}: rebased, it adopts"
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn sim_harness_two_tenants_with_one_tree_hold_separate_records() -> Sim<()> {
    for seed in 0..32_u64 {
        support::seeded(seed, || -> Sim<()> {
            let mut runtime = writer(all_succeed(8))?;
            let shared = tree_of(seed, 1)?;
            let (only_a, third) = (tree_of(seed, 2)?, tree_of(seed, 3)?);
            // Both tenants are queued before one tick, so their runs interleave in one runtime.
            let first = body(1, 1, &[Scope::FileWrite], recording(&shared, None)?);
            let second = body(2, 2, &[Scope::FileWrite], recording(&shared, None)?);
            runtime.submit(submit(command_id(1), &first)?)?;
            runtime.submit(submit(command_id(2), &second)?)?;
            runtime.tick()?;
            assert_eq!(
                active_of(&runtime, 1),
                Some(shared.hash()),
                "seed {seed}: tenant 1"
            );
            assert_eq!(
                active_of(&runtime, 2),
                Some(shared.hash()),
                "seed {seed}: tenant 2"
            );
            assert_eq!(shared.hash(), shared.hash(), "one tree, one hash");

            adopt(&mut runtime, 3, 1, recording(&only_a, Some(shared.hash()))?)?;
            assert_eq!(active_of(&runtime, 1), Some(only_a.hash()), "seed {seed}");
            assert_eq!(
                active_of(&runtime, 2),
                Some(shared.hash()),
                "seed {seed}: A never moves B"
            );
            let leak = refused(
                &mut runtime,
                4,
                2,
                &[Scope::FileWrite],
                rolling_back(only_a.hash(), Some(shared.hash()))?,
            )?;
            assert_eq!(
                leak,
                HarnessError::UnknownVersion,
                "seed {seed}: B cannot adopt what only A recorded"
            );
            // B's own adoption of the same tree A holds is B's record, not A's.
            adopt(&mut runtime, 5, 2, recording(&third, Some(shared.hash()))?)?;
            assert_eq!(
                active_of(&runtime, 1),
                Some(only_a.hash()),
                "seed {seed}: B never moves A"
            );
            assert_eq!(active_of(&runtime, 2), Some(third.hash()), "seed {seed}");
            assert_eq!(
                runtime
                    .harness(tenant(2))
                    .map(|held| held.adoptions().count()),
                Some(2),
                "seed {seed}: separate histories"
            );

            // The tenant is the run table's: a body that declares another tenant changes nothing.
            let spoof = proposal(
                run(6),
                tenant(2),
                operation(6),
                &[Scope::FileWrite],
                1,
                ReconciliationStrategy::Queryable,
            )
            .adopting(recording(&shared, Some(only_a.hash()))?);
            let command = larry_contracts::command::Command::new(
                command_id(6),
                tenant(1),
                Vec::new(),
                larry_contracts::command::CommandContent::new(
                    larry_contracts::command::CommandKind::Submit,
                    lgwks_std::json::to_vec(&spoof)?.into_boxed_slice(),
                ),
            )?;
            runtime.submit(command)?;
            runtime.tick()?;
            assert_eq!(
                active_of(&runtime, 1),
                Some(shared.hash()),
                "seed {seed}: it landed in the command's tenant"
            );
            assert_eq!(
                active_of(&runtime, 2),
                Some(third.hash()),
                "seed {seed}: and not in the declared one"
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn sim_harness_adoption_waits_for_the_effect_to_succeed() -> Sim<()> {
    for seed in 0..32_u64 {
        support::seeded(seed, || -> Sim<()> {
            let (first, second) = (tree_of(seed, 1)?, tree_of(seed, 2)?);
            // A gate denial: the worker lacks the write capability.
            let mut denied = runtime_with(
                ScriptedOwner::scripted(all_succeed(1)),
                &[Scope::FileWrite],
                &[Scope::FileRead],
                100,
            )?;
            let report = adopt(&mut denied, 1, 1, recording(&first, None)?)?;
            assert!(
                !report.denied().is_empty(),
                "seed {seed}: the gate denied it"
            );
            assert_eq!(
                active_of(&denied, 1),
                None,
                "seed {seed}: a denied adoption adopts nothing"
            );
            assert!(
                denied
                    .harness(tenant(1))
                    .is_none_or(|held| !held.in_flight()),
                "seed {seed}: and holds nothing in flight"
            );

            // The owner reports a definite failure, then the next adoption succeeds.
            let mut failing = writer(vec![
                EffectOutcome::Failed {
                    operation: operation(1),
                },
                succeeded(operation(2)),
            ])?;
            adopt(&mut failing, 1, 1, recording(&first, None)?)?;
            assert_eq!(
                active_of(&failing, 1),
                None,
                "seed {seed}: a failed effect adopts nothing"
            );
            adopt(&mut failing, 2, 1, recording(&second, None)?)?;
            assert_eq!(
                active_of(&failing, 1),
                Some(second.hash()),
                "seed {seed}: the base is still empty"
            );

            // An unknown outcome leaves the adoption open: nothing active, the tenant blocked.
            let mut open = writer(vec![unknown(operation(1)), succeeded(operation(2))])?;
            adopt(&mut open, 1, 1, recording(&first, None)?)?;
            assert_eq!(
                active_of(&open, 1),
                None,
                "seed {seed}: unknown adopts nothing"
            );
            let blocked = refused(
                &mut open,
                2,
                1,
                &[Scope::FileWrite],
                recording(&second, None)?,
            )?;
            assert_eq!(blocked, HarnessError::AdoptionInFlight, "seed {seed}");
            let replayed: TestRuntime = restore(
                settings(&[Scope::FileWrite], &[Scope::FileWrite], 100)?,
                ScriptedOwner::new(),
                clock()?,
                open.into_journal(),
            )?;
            assert_eq!(active_of(&replayed, 1), None, "seed {seed}: restore agrees");
            assert!(
                replayed
                    .harness(tenant(1))
                    .is_some_and(TenantHarness::in_flight),
                "seed {seed}: still open"
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn harness_the_sixty_fifth_tree_is_refused() -> Sim<()> {
    let mut runtime = writer(all_succeed(64))?;
    let mut parent = None;
    for number in 1..=64_u8 {
        let mut tree = HarnessTree::empty();
        tree.insert(HarnessComponent::Memory, "note", &[number; 8])?;
        adopt(&mut runtime, number, 1, recording(&tree, parent)?)?;
        parent = Some(tree.hash());
    }
    assert_eq!(MAX_ADOPTED_VERSIONS, 64, "the documented bound");
    let mut extra = HarnessTree::empty();
    extra.insert(HarnessComponent::Memory, "note", &[99; 8])?;
    assert_eq!(
        refused(
            &mut runtime,
            65,
            1,
            &[Scope::FileWrite],
            recording(&extra, parent)?
        )?,
        HarnessError::TooManyVersions {
            limit: MAX_ADOPTED_VERSIONS
        },
        "the sixty-fifth distinct tree"
    );
    Ok(())
}

#[test]
fn harness_old_journals_decode_unchanged() -> Sim<()> {
    let mut runtime = runtime_with(
        ScriptedOwner::scripted(all_succeed(1)),
        &[Scope::FileRead],
        &[Scope::FileRead],
        10,
    )?;
    let plain = proposal(
        run(1),
        tenant(1),
        operation(1),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    let bytes = lgwks_std::json::to_vec(&plain)?;
    assert!(
        !String::from_utf8(bytes.clone())?.contains("harness"),
        "a plain proposal encodes as it always did"
    );
    let decoded: larry_runtime::payload::WorkProposal = lgwks_std::json::from_slice(&bytes)?;
    assert!(decoded.harness().is_none(), "and decodes with no adoption");
    runtime.submit(submit(command_id(1), &plain)?)?;
    runtime.tick()?;
    for entry in runtime.journal().entries() {
        if let larry_runtime::journal::JournalEntry::Event(ref event) = *entry {
            assert!(
                !String::from_utf8_lossy(event.payload()).contains("harness"),
                "a plain run's records carry no harness field"
            );
        }
    }
    let mut folded = FoldProjection::default();
    replay_journal(&mut folded, runtime.journal())?;
    assert!(
        folded.harness(tenant(1)).is_none(),
        "and fold to no harness record"
    );
    Ok(())
}
