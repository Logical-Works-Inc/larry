//! Seeded multi-tenant isolation simulations (E37, E63, E70).
//! Every test drives the real `Runtime` twice from one seed with two or more
//! tenants and asserts the same counts plus the caller-visible isolation.

#[path = "sim_kit/seed.rs"]
mod seed;
mod sim_kit;
mod support;

use rocco_contracts::authority::Scope;
use rocco_contracts::effect::ReconciliationStrategy;
use rocco_contracts::state::RunState;
use rocco_runtime::error::RuntimeError;
use rocco_runtime::owner::fixture::ScriptedOwner;
use rocco_runtime::projection::ProjectionBackend;

use rocco_runtime::runtime::TickReport;

use seed::step;
use sim_kit::{Offer, Sim, read_work, reader, reopen, replay, work};
use support::{
    TestRuntime, assert_state, command_id, operation, proposal, run, succeeded, tenant, unknown,
};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;
/// Tenant tally: journal length, receipts folded and runs tracked.
type TenantTally = (usize, usize, usize);

/// Wrapping seed evolution, independent of any clock or scheduler.
fn tenant_mix(seed: &mut u64) -> u64 {
    step(seed, 15_407_997_324_636_748_307, 9_561_242_260_961_875, 19)
}

/// Counts every recorded fact plus the folded view sizes.
fn tenant_tally(runtime: &TestRuntime) -> TenantTally {
    let view = runtime.projection();
    (
        runtime.journal().len(),
        view.receipts().len(),
        view.run_count(),
    )
}

/// Runtime with `FileRead` policy, capability and a budget of 100.
fn mk_tenant(owner: ScriptedOwner) -> Result<TestRuntime, Box<dyn std::error::Error>> {
    reader(owner, 100)
}

/// Submits one tenant-scoped unit of work and ticks once.
fn tender(
    rt: &mut TestRuntime,
    id: u8,
    ten: u8,
    scopes: &[Scope],
    cost: u64,
    strat: ReconciliationStrategy,
) -> Result<TickReport, Box<dyn std::error::Error>> {
    let body = proposal(run(id), tenant(ten), operation(id), scopes, cost, strat);
    rt.offer(id, &body)?;
    Ok(rt.tick()?)
}

/// E63: the same run bytes under a second tenant are refused as trespass.
#[test]
fn sim_multitenant_same_run_bytes_refused_for_second_tenant() -> Outcome {
    replay(0_u64..8, |_| -> Sim<TenantTally> {
        let mut rt = mk_tenant(ScriptedOwner::scripted([
            succeeded(operation(1)),
            succeeded(operation(2)),
        ]))?;
        tender(
            &mut rt,
            1,
            1,
            &[Scope::FileRead],
            5,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        assert_state(rt.projection(), run(1), RunState::Completed)?;
        let before = tenant_tally(&rt);
        let second = work(1, 2, 2, 5, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(2, &second)?;
        assert!(
            matches!(rt.tick(), Err(RuntimeError::RunTenantMismatch { .. })),
            "second tenant refused"
        );
        assert_eq!(
            tenant_tally(&rt),
            before,
            "refusal records no trespass fact"
        );
        Ok(before)
    })
}

/// E37: each receipt is bound to the tenant that reserved it.
#[test]
fn sim_multitenant_receipts_carry_their_tenant() -> Outcome {
    replay(8_u64..16, |_| -> Sim<TenantTally> {
        let mut rt = mk_tenant(ScriptedOwner::scripted([
            succeeded(operation(3)),
            succeeded(operation(4)),
        ]))?;
        tender(
            &mut rt,
            3,
            1,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        tender(
            &mut rt,
            4,
            2,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        for receipt in rt.projection().receipts() {
            let want = if receipt.operation() == operation(3) {
                tenant(1)
            } else {
                tenant(2)
            };
            assert_eq!(receipt.tenant(), want, "receipt binds its tenant");
        }
        Ok(tenant_tally(&rt))
    })
}

/// E63: an envelope tenant that disagrees with the run's tenant is refused.
#[test]
fn sim_multitenant_run_tenant_mismatch_refused() -> Outcome {
    use rocco_contracts::command::{Command, CommandContent, CommandKind};
    replay(16_u64..24, |_| -> Sim<TenantTally> {
        let mut rt = mk_tenant(ScriptedOwner::scripted([succeeded(operation(5))]))?;
        tender(
            &mut rt,
            5,
            1,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let before = tenant_tally(&rt);
        let trespass = work(5, 2, 6, 1, ReconciliationStrategy::IdempotentRetrySafe);
        let cmd = Command::new(
            command_id(6),
            tenant(2),
            Vec::new(),
            CommandContent::new(
                CommandKind::Submit,
                lgwks_std::json::to_vec(&trespass)?.into_boxed_slice(),
            ),
        )?;
        match rt.submit(cmd) {
            Err(RuntimeError::Command(_)) | Err(RuntimeError::RunTenantMismatch { .. }) => {}
            Ok(()) => assert!(rt.tick().is_err(), "trespassing submit refused"),
            Err(other) => return Err(other.into()),
        }
        assert_eq!(
            tenant_tally(&rt),
            before,
            "refusal records no trespass fact"
        );
        assert_state(rt.projection(), run(5), RunState::Completed)?;
        Ok(before)
    })
}

/// E63: a foreign cancel is refused and changes no run state.
#[test]
fn sim_multitenant_foreign_cancel_refused() -> Outcome {
    replay(24_u64..32, |_| -> Sim<TenantTally> {
        let mut rt = mk_tenant(ScriptedOwner::scripted([succeeded(operation(7))]))?;
        tender(
            &mut rt,
            7,
            1,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let before = tenant_tally(&rt);
        let foreign = read_work(7, 1, ReconciliationStrategy::IdempotentRetrySafe);
        let cmd = rocco_contracts::command::Command::new(
            command_id(8),
            tenant(2),
            Vec::new(),
            rocco_contracts::command::CommandContent::new(
                rocco_contracts::command::CommandKind::Cancel,
                lgwks_std::json::to_vec(&foreign)?.into_boxed_slice(),
            ),
        )?;
        rt.submit(cmd)?;
        assert!(
            matches!(rt.tick(), Err(RuntimeError::RunTenantMismatch { .. })),
            "foreign cancel refused"
        );
        assert_eq!(tenant_tally(&rt), before, "refusal records no foreign fact");
        assert_state(rt.projection(), run(7), RunState::Completed)?;
        Ok(before)
    })
}

/// E71: revoking one tenant's operation leaves the other's reach intact.
#[test]
fn sim_multitenant_revoke_is_per_operation() -> Outcome {
    replay(32_u64..40, |_| -> Sim<TenantTally> {
        let mut rt = mk_tenant(ScriptedOwner::scripted([succeeded(operation(9))]))?;
        for (num, ten) in [(8_u8, 1_u8), (9, 2)] {
            let body = work(
                num,
                ten,
                num,
                1,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            rt.offer(num, &body)?;
        }
        assert!(
            rt.fence_mut().revoke(operation(8)),
            "withdrawal reports the change"
        );
        let tick = rt.tick()?;
        assert_eq!(tick.fenced().len(), 1, "only the named operation fenced");
        assert_eq!(
            tick.fenced()[0].0,
            operation(8),
            "fence names the operation"
        );
        assert_eq!(
            rt.owner().dispatched(),
            &[operation(9)],
            "the other tenant still dispatches"
        );
        Ok(tenant_tally(&rt))
    })
}

/// E37: one tenant's denial leaves the other tenant's run complete.
#[test]
fn sim_multitenant_denial_is_per_tenant() -> Outcome {
    use rocco_contracts::replay::DenialReason;
    replay(40_u64..48, |_| -> Sim<TenantTally> {
        let mut rt = mk_tenant(ScriptedOwner::scripted([succeeded(operation(11))]))?;
        let tick = tender(
            &mut rt,
            10,
            1,
            &[Scope::NetworkEgress],
            1,
            ReconciliationStrategy::Queryable,
        )?;
        assert_eq!(
            tick.denied(),
            &[(
                operation(10),
                DenialReason::PolicyRefuses {
                    scope: Scope::NetworkEgress
                }
            )],
            "only the trespassing scope denied"
        );
        tender(
            &mut rt,
            11,
            2,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        assert_state(rt.projection(), run(10), RunState::Failed)?;
        assert_state(rt.projection(), run(11), RunState::Completed)?;
        Ok(tenant_tally(&rt))
    })
}

/// E37: unknown-permanent in one tenant blocks no sibling tenant.
#[test]
fn sim_multitenant_unknown_permanent_is_per_tenant() -> Outcome {
    replay(48_u64..56, |_| -> Sim<TenantTally> {
        let mut rt = mk_tenant(ScriptedOwner::scripted([
            unknown(operation(12)),
            succeeded(operation(13)),
        ]))?;
        tender(
            &mut rt,
            12,
            1,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::RequiresHuman,
        )?;
        assert_state(rt.projection(), run(12), RunState::OutcomeUnknown)?;
        tender(
            &mut rt,
            13,
            2,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        assert_state(rt.projection(), run(13), RunState::Completed)?;
        Ok(tenant_tally(&rt))
    })
}

/// E37: five tenants complete independently in one tick.
#[test]
fn sim_multitenant_five_tenants_complete_together() -> Outcome {
    replay(56_u64..60, |_| -> Sim<(usize, TenantTally)> {
        let mut rt = mk_tenant(ScriptedOwner::scripted([
            succeeded(operation(14)),
            succeeded(operation(15)),
            succeeded(operation(16)),
            succeeded(operation(17)),
            succeeded(operation(18)),
        ]))?;
        for (num, ten) in [(14_u8, 1_u8), (15, 2), (16, 3), (17, 4), (18, 5)] {
            let body = work(
                num,
                ten,
                num,
                1,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            rt.offer(num, &body)?;
        }
        let tick = rt.tick()?;
        assert_eq!(tick.receipts().len(), 5, "every tenant receipts");
        assert_eq!(rt.remaining_budget(), 95, "five reservations held");
        Ok((tick.receipts().len(), tenant_tally(&rt)))
    })
}

/// E37: reservations draw from one pool across tenants.
#[test]
fn sim_multitenant_reservations_share_one_pool() -> Outcome {
    replay(60_u64..66, |seed| -> Sim<(u64, TenantTally)> {
        let mut rt = mk_tenant(ScriptedOwner::scripted([succeeded(operation(19))]))?;
        tender(
            &mut rt,
            19,
            1,
            &[Scope::FileRead],
            90,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let second = work(20, 2, 20, 20, ReconciliationStrategy::Queryable);
        rt.offer(20, &second)?;
        let tick = rt.tick()?;
        assert_eq!(tick.authorized().len(), 0, "the pool is spent");
        let (first_budget, first_fp) = (rt.remaining_budget(), tenant_tally(&rt));
        assert_eq!(first_budget, 10, "seed {seed}: ten units remain");
        Ok((first_budget, first_fp))
    })
}

/// E63: seeded tenants complete independently with disjoint identities.
#[test]
fn sim_multitenant_seeded_tenants_complete() -> Outcome {
    replay(66_u64..74, |seed| -> Sim<TenantTally> {
        let mut random = seed.saturating_add(17);
        let left = u8::try_from(tenant_mix(&mut random) & 0x7F)?;
        let right = left.saturating_add(1);
        let mut rt = mk_tenant(ScriptedOwner::scripted([
            succeeded(operation(left)),
            succeeded(operation(right)),
        ]))?;
        for (num, ten) in [(left, left), (right, right)] {
            tender(
                &mut rt,
                num,
                ten,
                &[Scope::FileRead],
                1,
                ReconciliationStrategy::IdempotentRetrySafe,
            )?;
        }
        Ok(tenant_tally(&rt))
    })
}

/// E37: effect counts stay scoped to each tenant's own run.
#[test]
fn sim_multitenant_effect_counts_stay_scoped() -> Outcome {
    replay(74_u64..80, |seed| -> Sim<(usize, usize)> {
        let mut rt = mk_tenant(ScriptedOwner::scripted([
            succeeded(operation(21)),
            succeeded(operation(22)),
        ]))?;
        tender(
            &mut rt,
            21,
            1,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        tender(
            &mut rt,
            22,
            2,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        assert_eq!(
            rt.projection().effect_count(run(21)),
            1,
            "first run counts one effect"
        );
        assert_eq!(
            rt.projection().effect_count(run(22)),
            1,
            "second run counts one effect"
        );
        let (first_one, first_two) = (
            rt.projection().effect_count(run(21)),
            rt.projection().effect_count(run(22)),
        );
        assert_eq!(
            (first_one, first_two),
            (1, 1),
            "seed {seed}: one effect each"
        );
        Ok((first_one, first_two))
    })
}

/// E37: the journal holds both tenants' commands and both replay.
#[test]
fn sim_multitenant_journal_holds_both_tenants() -> Outcome {
    replay(80_u64..86, |_| -> Sim<(usize, TenantTally)> {
        let mut rt = mk_tenant(ScriptedOwner::scripted([
            succeeded(operation(23)),
            succeeded(operation(24)),
        ]))?;
        tender(
            &mut rt,
            23,
            1,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        tender(
            &mut rt,
            24,
            2,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        assert!(rt.journal().len() > 2, "both tenants recorded");
        Ok((rt.journal().len(), tenant_tally(&rt)))
    })
}

/// E63: a conflicting cancel identity from another tenant is refused.
#[test]
fn sim_multitenant_conflicting_cancel_across_tenants_refused() -> Outcome {
    use support::cancel;
    replay(86_u64..92, |_| -> Sim<TenantTally> {
        let mut rt = mk_tenant(ScriptedOwner::new())?;
        let body = proposal(
            run(25),
            tenant(1),
            operation(25),
            &[],
            0,
            ReconciliationStrategy::RequiresHuman,
        );
        rt.submit(cancel(command_id(25), &body)?)?;
        rt.tick()?;
        let foreign = proposal(
            run(25),
            tenant(2),
            operation(25),
            &[],
            0,
            ReconciliationStrategy::RequiresHuman,
        );
        match rt.submit(cancel(command_id(25), &foreign)?) {
            Err(RuntimeError::Command(_)) => {}
            Ok(()) => {
                let result = rt.tick();
                assert!(
                    matches!(
                        &result,
                        Err(RuntimeError::Command(_)) | Err(RuntimeError::RunTenantMismatch { .. })
                    ),
                    "cross-tenant cancel refused, got {result:?}"
                );
            }
            Err(other) => return Err(other.into()),
        }
        assert_state(rt.projection(), run(25), RunState::Cancelled)?;
        Ok(tenant_tally(&rt))
    })
}

/// E37: a cancel in one tenant leaves the sibling tenant's run complete.
#[test]
fn sim_multitenant_cancel_leaves_sibling_complete() -> Outcome {
    use support::cancel;
    replay(92_u64..98, |_| -> Sim<TenantTally> {
        let mut rt = mk_tenant(ScriptedOwner::scripted([succeeded(operation(27))]))?;
        tender(
            &mut rt,
            27,
            2,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let gone = proposal(
            run(26),
            tenant(1),
            operation(26),
            &[],
            0,
            ReconciliationStrategy::RequiresHuman,
        );
        rt.submit(cancel(command_id(26), &gone)?)?;
        rt.tick()?;
        assert_state(rt.projection(), run(26), RunState::Cancelled)?;
        assert_state(rt.projection(), run(27), RunState::Completed)?;
        Ok(tenant_tally(&rt))
    })
}

/// E37: restore preserves every tenant's folded run state.
#[test]
fn sim_multitenant_restore_preserves_every_tenant() -> Outcome {
    replay(98_u64..104, |_| -> Sim<TenantTally> {
        let mut rt = mk_tenant(ScriptedOwner::scripted([
            succeeded(operation(28)),
            succeeded(operation(29)),
        ]))?;
        tender(
            &mut rt,
            28,
            1,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        tender(
            &mut rt,
            29,
            2,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let restored = reopen(rt.into_journal(), 100)?;
        assert_state(restored.projection(), run(28), RunState::Completed)?;
        assert_state(restored.projection(), run(29), RunState::Completed)?;
        assert_eq!(
            restored.projection().receipts().len(),
            2,
            "both receipts kept"
        );
        Ok(tenant_tally(&restored))
    })
}

/// E37: duplicate resubmits in one tenant never disturb the other.
#[test]
fn sim_multitenant_duplicate_never_disturbs_sibling() -> Outcome {
    replay(104_u64..110, |_| -> Sim<TenantTally> {
        let mut rt = mk_tenant(ScriptedOwner::scripted([
            succeeded(operation(30)),
            succeeded(operation(31)),
        ]))?;
        tender(
            &mut rt,
            30,
            1,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        tender(
            &mut rt,
            31,
            2,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let repeat = read_work(30, 1, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(30, &repeat)?;
        assert_eq!(rt.tick()?.duplicates(), &[command_id(30)], "repeat dedupes");
        assert_state(rt.projection(), run(31), RunState::Completed)?;
        Ok(tenant_tally(&rt))
    })
}

/// E70: two tenants sharing one owner script each consume their own outcome.
#[test]
fn sim_multitenant_owner_script_consumed_in_order() -> Outcome {
    replay(110_u64..116, |_| -> Sim<TenantTally> {
        let mut rt = mk_tenant(ScriptedOwner::scripted([
            unknown(operation(32)),
            succeeded(operation(33)),
            succeeded(operation(32)),
        ]))?;
        let slow = read_work(32, 1, ReconciliationStrategy::Queryable);
        rt.offer(32, &slow)?;
        let fast = work(33, 2, 33, 1, ReconciliationStrategy::Queryable);
        rt.offer(33, &fast)?;
        rt.tick()?;
        assert_state(rt.projection(), run(32), RunState::Reconciling)?;
        assert_state(rt.projection(), run(33), RunState::Completed)?;
        assert_eq!(
            rt.tick()?.reconciled(),
            &[operation(32)],
            "slow tenant reconciles"
        );
        assert_state(rt.projection(), run(32), RunState::Completed)?;
        Ok(tenant_tally(&rt))
    })
}

/// E37: five tenants' receipts each bind exactly their own tenant.
#[test]
fn sim_multitenant_five_receipts_bind_five_tenants() -> Outcome {
    replay(116_u64..120, |_| -> Sim<usize> {
        let mut rt = mk_tenant(ScriptedOwner::scripted([
            succeeded(operation(34)),
            succeeded(operation(35)),
            succeeded(operation(36)),
            succeeded(operation(37)),
            succeeded(operation(38)),
        ]))?;
        for (num, ten) in [(34_u8, 1_u8), (35, 2), (36, 3), (37, 4), (38, 5)] {
            tender(
                &mut rt,
                num,
                ten,
                &[Scope::FileRead],
                1,
                ReconciliationStrategy::IdempotentRetrySafe,
            )?;
        }
        for (num, ten) in [(34_u8, 1_u8), (35, 2), (36, 3), (37, 4), (38, 5)] {
            let found = rt
                .projection()
                .receipts()
                .iter()
                .filter(|rec| rec.operation() == operation(num) && rec.tenant() == tenant(ten))
                .count();
            assert_eq!(found, 1, "receipt binds operation {num} to tenant {ten}");
        }
        Ok(rt.projection().receipts().len())
    })
}

/// E37: one tenant's fence withdrawal never names another tenant's operation.
#[test]
fn sim_multitenant_fence_names_only_its_operation() -> Outcome {
    replay(120_u64..126, |_| -> Sim<TenantTally> {
        let mut rt = mk_tenant(ScriptedOwner::scripted([succeeded(operation(40))]))?;
        for (num, ten) in [(39_u8, 1_u8), (40, 2)] {
            let body = work(
                num,
                ten,
                num,
                1,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            rt.offer(num, &body)?;
        }
        assert!(
            rt.fence_mut().revoke(operation(39)),
            "withdrawal reports the change"
        );
        assert!(
            !rt.fence().is_revoked(operation(40)),
            "sibling stands unrevoked"
        );
        let tick = rt.tick()?;
        assert_eq!(tick.fenced().len(), 1, "only one refusal recorded");
        assert_eq!(
            tick.fenced()[0].0,
            operation(39),
            "fence names its operation"
        );
        Ok(tenant_tally(&rt))
    })
}
