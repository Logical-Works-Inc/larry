//! E71 public-runtime falsifiers. All authority and effect owners are visibly
//! synthetic; the runtime, journal, reducer, fence and replay are real code.

use std::time::{Duration, SystemTime};

use rocco_contracts::authority::{BudgetUnit, Scope, SecurityEpoch};
use rocco_contracts::command::{Command, CommandContent, CommandKind};
use rocco_contracts::effect::{EffectOutcome, ReconciliationStrategy};
use rocco_contracts::ids::{CommandId, ContentDigest, OperationId, RunId, TenantId};
use rocco_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};
use rocco_runtime::FenceRefusal;
use rocco_runtime::authority::fixture::FixtureAuthority;
use rocco_runtime::clock::FixedClock;
use rocco_runtime::journal::JournalEntry;
use rocco_runtime::owner::fixture::ScriptedOwner;
use rocco_runtime::payload::WorkProposal;
use rocco_runtime::projection::{FoldProjection, ProjectionBackend, replay_authorizations};
use rocco_runtime::runtime::{Runtime, RuntimeSettings, restore};

/// The real runtime instantiated with the permitted deterministic owners.
type Subject = Runtime<FoldProjection, ScriptedOwner, FixedClock>;
/// Fallible tests never panic to unwrap fixture setup.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
/// Each simulation sweeps this many independently replayed seeds.
const SEEDS: u64 = 1_000;

/// Independent portable identities, not a product identity convention.
fn bytes(value: u64) -> [u8; 16] {
    u128::from(value).to_be_bytes()
}

/// One seed determines the clock, policy revision and simulated actor IDs.
fn settings(seed: u64) -> TestResult<RuntimeSettings> {
    let at = instant(seed)?;
    Ok(RuntimeSettings::with_defaults(
        PolicySnapshot::new(
            seed.saturating_add(1),
            [Scope::FileRead].into_iter().collect(),
        ),
        CapabilitySnapshot::new([Scope::FileRead].into_iter().collect()),
        ValiditySnapshot::new(
            at.checked_add(Duration::from_secs(60))
                .ok_or("time overflow")?,
        ),
        BudgetUnit::Requests,
        100,
    ))
}

/// Portable simulated time, with bounded inputs.
fn instant(seed: u64) -> TestResult<SystemTime> {
    Ok(SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(seed))
        .ok_or("time overflow")?)
}

/// Explicit provisioning is separate from the runtime's supplied snapshots.
fn source(settings: &RuntimeSettings, epoch: u64, bound: u32) -> TestResult<FixtureAuthority> {
    let mut source = FixtureAuthority::new(SecurityEpoch::new(epoch), bound);
    for value in [1, 2] {
        source.trust(
            TenantId::from_bytes(bytes(value)),
            settings.policy.clone(),
            settings.capabilities.clone(),
            settings.validity,
        )?;
    }
    Ok(source)
}

/// One real command with caller-controlled proposals but no authority claim.
fn command(tenant: u64, operation: u64, strategy: ReconciliationStrategy) -> TestResult<Command> {
    let body = WorkProposal::new(
        RunId::from_bytes(bytes(operation)),
        TenantId::from_bytes(bytes(tenant)),
        OperationId::from_bytes(bytes(operation)),
        [Scope::FileRead].into_iter().collect(),
        1,
        BudgetUnit::Requests,
        strategy,
    );
    Ok(Command::new(
        CommandId::from_bytes(bytes(operation)),
        body.tenant,
        Vec::new(),
        CommandContent::new(
            CommandKind::Submit,
            lgwks_std::json::to_vec(&body)?.into_boxed_slice(),
        ),
    )?)
}

/// Builds a real runtime at its public entrypoint.
fn subject(settings: RuntimeSettings, source: FixtureAuthority, seed: u64) -> TestResult<Subject> {
    Ok(Runtime::new(
        settings,
        ScriptedOwner::new(),
        FixedClock::new(instant(seed)?),
        source,
    )?)
}

/// Trace covers journal bytes, externally visible refusals, and owner calls.
fn trace(subject: &Subject, report: &rocco_runtime::runtime::TickReport) -> ContentDigest {
    let mut data = Vec::new();
    for entry in subject.journal().entries() {
        if let JournalEntry::Event(ref event) = *entry {
            data.extend_from_slice(event.payload_digest().as_bytes());
        }
    }
    data.extend_from_slice(format!("{report:?}").as_bytes());
    for operation in subject.owner().dispatched() {
        data.extend_from_slice(operation.as_bytes());
    }
    ContentDigest::over(&[&data])
}

/// Every seed is executed twice, including faults, and its complete trace agrees.
fn sweep(run: impl Fn(u64) -> TestResult<ContentDigest>) -> TestResult {
    for seed in 0..SEEDS {
        assert_eq!(
            run(seed)?,
            run(seed)?,
            "E71 exact replay failed for seed {seed}"
        );
    }
    Ok(())
}

/// Replayed decisions remain reproducible but cannot authorize after revocation.
#[test]
fn sim_e71_restart_rechecks_source_epoch() -> TestResult {
    sweep(|seed| {
        let settings = settings(seed)?;
        let admitted = seed.saturating_add(1);
        let reported = if seed & 1 == 0 {
            seed.saturating_add(2)
        } else {
            seed
        };
        let mut original = subject(settings.clone(), source(&settings, admitted, 4)?, seed)?;
        original.submit(command(1, 10, ReconciliationStrategy::IdempotentRetrySafe)?)?;
        original.tick()?;
        assert_eq!(
            original.owner().dispatched().len(),
            1,
            "control must dispatch"
        );
        let mut resumed: Subject = restore(
            settings.clone(),
            ScriptedOwner::new(),
            FixedClock::new(instant(seed)?),
            original.into_journal(),
            source(&settings, reported, 4)?,
        )?;
        assert!(
            replay_authorizations(resumed.projection()).is_bit_identical(),
            "old decisions still replay"
        );
        let tick = resumed.tick()?;
        assert_eq!(
            tick.fenced(),
            &[(
                OperationId::from_bytes(bytes(10)),
                FenceRefusal::StaleEpoch {
                    admitted: SecurityEpoch::new(admitted),
                    current: SecurityEpoch::new(reported)
                }
            )]
        );
        assert!(
            resumed.owner().dispatched().is_empty(),
            "restart must not spend stale authority"
        );
        Ok(trace(&resumed, &tick))
    })
}

/// Same epoch is insufficient: each exact snapshot is authenticated separately.
#[test]
fn sim_e71_old_snapshots_refused_after_restart() -> TestResult {
    sweep(|seed| {
        let old = settings(seed)?;
        let mut current = old.clone();
        match seed.checked_rem(3).ok_or("invalid divisor")? {
            0 => {
                current.policy = PolicySnapshot::new(
                    seed.saturating_add(2),
                    [Scope::FileRead].into_iter().collect(),
                )
            }
            1 => {
                current.capabilities = CapabilitySnapshot::new(
                    [Scope::FileRead, Scope::FileWrite].into_iter().collect(),
                )
            }
            _ => {
                current.validity = ValiditySnapshot::new(
                    instant(seed)?
                        .checked_add(Duration::from_secs(120))
                        .ok_or("time overflow")?,
                )
            }
        }
        let mut original = subject(old.clone(), source(&old, seed, 8)?, seed)?;
        original.submit(command(1, 10, ReconciliationStrategy::IdempotentRetrySafe)?)?;
        original.tick()?;
        // Even current host settings must not replace the old reservation's inputs.
        let mut resumed: Subject = restore(
            current.clone(),
            ScriptedOwner::new(),
            FixedClock::new(instant(seed)?),
            original.into_journal(),
            source(&current, seed, 8)?,
        )?;
        let tick = resumed.tick()?;
        assert_eq!(
            tick.fenced(),
            &[(
                OperationId::from_bytes(bytes(10)),
                FenceRefusal::StaleSnapshots
            )]
        );
        assert!(
            resumed.owner().dispatched().is_empty(),
            "old snapshots cannot authorize a retry"
        );
        assert!(
            replay_authorizations(resumed.projection()).is_bit_identical(),
            "replay does not rewrite inputs"
        );
        resumed.submit(command(1, 11, ReconciliationStrategy::Queryable)?)?;
        let control = resumed.tick()?;
        assert_eq!(
            resumed.owner().dispatched(),
            &[OperationId::from_bytes(bytes(11))],
            "independently current authority still works"
        );
        Ok(trace(&resumed, &control))
    })
}

/// Restart while partitioned never refills a spent or partially spent lease.
#[test]
fn sim_e71_partition_restart_requires_fresh_source() -> TestResult {
    sweep(|seed| {
        let settings = settings(seed)?;
        let bound = u32::try_from(seed.checked_rem(4).ok_or("invalid divisor")?)?.saturating_add(1);
        let mut original = subject(settings.clone(), source(&settings, seed, bound)?, seed)?;
        original.submit(command(1, 10, ReconciliationStrategy::IdempotentRetrySafe)?)?;
        original.tick()?;
        let mut offline = source(&settings, seed, bound)?;
        offline.set_connected(false);
        let mut resumed: Subject = restore(
            settings,
            ScriptedOwner::new(),
            FixedClock::new(SystemTime::UNIX_EPOCH),
            original.into_journal(),
            offline,
        )?;
        let tick = resumed.tick()?;
        assert_eq!(
            tick.fenced(),
            &[(
                OperationId::from_bytes(bytes(10)),
                FenceRefusal::AuthorityUnavailable
            )]
        );
        assert!(
            resumed.owner().dispatched().is_empty(),
            "clock rollback and reconstruction cannot create authority"
        );
        resumed.authority_mut().set_connected(true);
        resumed
            .authority_mut()
            .authenticate(SecurityEpoch::new(seed))?;
        let control = resumed.tick()?;
        assert_eq!(
            resumed.owner().dispatched().len(),
            1,
            "authenticated recovery can retry"
        );
        Ok(trace(&resumed, &control))
    })
}

/// Tenant-qualified withdrawal never contaminates the same operation bytes.
#[test]
fn sim_e71_scoped_withdrawal_isolates_identical_resources() -> TestResult {
    sweep(|seed| {
        let settings = settings(seed)?;
        let mut hashes = Vec::new();
        // The existing runtime operation table is globally keyed (E63).
        // Independent runtimes exercise the authority boundary without changing it.
        for tenant in [1, 2] {
            let mut authority = source(&settings, seed, 8)?;
            authority.withdraw(
                TenantId::from_bytes(bytes(1)),
                OperationId::from_bytes(bytes(10)),
            )?;
            let mut runtime = subject(settings.clone(), authority, seed)?;
            runtime.submit(command(tenant, 10, ReconciliationStrategy::Queryable)?)?;
            let tick = runtime.tick()?;
            assert_eq!(
                runtime.owner().dispatched().is_empty(),
                tenant == 1,
                "withdrawal scope must be tenant-specific"
            );
            if tenant == 1 {
                assert_eq!(
                    tick.fenced(),
                    &[(
                        OperationId::from_bytes(bytes(10)),
                        FenceRefusal::Revoked {
                            operation: OperationId::from_bytes(bytes(10))
                        }
                    )]
                );
            }
            hashes.extend_from_slice(trace(&runtime, &tick).as_bytes());
        }
        Ok(ContentDigest::over(&[&hashes]))
    })
}

/// A partition permits only the declared exposure; stale contact cannot refill it.
#[test]
fn sim_e71_partition_budget_and_stale_contact() -> TestResult {
    sweep(|seed| {
        let mut settings = settings(seed)?;
        // Alternate which owner supplies the stricter ceiling.
        let source_bound = if seed & 1 == 0 {
            settings.offline_dispatch_bound = u32::MAX;
            1
        } else {
            settings.offline_dispatch_bound = 1;
            u32::MAX
        };
        let current = seed.saturating_add(1);
        let mut runtime = subject(
            settings.clone(),
            source(&settings, current, source_bound)?,
            seed,
        )?;
        runtime.submit(command(1, 10, ReconciliationStrategy::Queryable)?)?;
        runtime.tick()?;
        runtime.authority_mut().set_connected(false);
        runtime.submit(command(1, 11, ReconciliationStrategy::IdempotentRetrySafe)?)?;
        let tick = runtime.tick()?;
        assert_eq!(
            tick.fenced(),
            &[(
                OperationId::from_bytes(bytes(11)),
                FenceRefusal::OfflineExposureExhausted { bound: 1 }
            )]
        );
        runtime.authority_mut().set_connected(true);
        assert_eq!(
            runtime
                .authority_mut()
                .authenticate(SecurityEpoch::new(seed)),
            Err(FenceRefusal::StaleEpoch {
                admitted: SecurityEpoch::new(seed),
                current: SecurityEpoch::new(current)
            })
        );
        assert_eq!(
            runtime.fence().offline_spent(),
            1,
            "replay must not refund exposure"
        );
        let again = runtime.tick()?;
        assert_eq!(again.fenced(), tick.fenced());
        assert_eq!(
            runtime.owner().dispatched().len(),
            1,
            "only the original reach happened"
        );
        Ok(trace(&runtime, &again))
    })
}

/// No source tenant record means no effect, however permissive host settings are.
#[test]
fn e71_missing_tenant_and_old_settings_cannot_authenticate() -> TestResult {
    let mut settings = settings(1)?;
    settings.security_epoch = SecurityEpoch::new(u64::MAX);
    let source = FixtureAuthority::new(SecurityEpoch::INITIAL, 8);
    let mut runtime = subject(settings, source, 1)?;
    runtime.submit(command(1, 10, ReconciliationStrategy::Queryable)?)?;
    let tick = runtime.tick()?;
    assert_eq!(
        tick.fenced(),
        &[(
            OperationId::from_bytes(bytes(10)),
            FenceRefusal::AuthorityTenant {
                tenant: TenantId::from_bytes(bytes(1))
            }
        )]
    );
    assert_eq!(
        runtime
            .projection()
            .intents()
            .first()
            .map(|intent| intent.admission_epoch),
        Some(SecurityEpoch::INITIAL)
    );
    assert!(
        runtime.owner().dispatched().is_empty(),
        "host epoch cannot create authority"
    );
    Ok(())
}

/// Revocation leaves status queries usable, including disconnected recovery.
#[test]
fn e71_partitioned_restore_can_still_query_a_sent_effect() -> TestResult {
    let settings = settings(1)?;
    let mut original = subject(settings.clone(), source(&settings, 0, 4)?, 1)?;
    original.submit(command(1, 10, ReconciliationStrategy::Queryable)?)?;
    original.tick()?;
    let mut offline = source(&settings, 1, 4)?;
    offline.set_connected(false);
    let operation = OperationId::from_bytes(bytes(10));
    let mut resumed: Subject = restore(
        settings,
        ScriptedOwner::scripted([EffectOutcome::Succeeded { operation }]),
        FixedClock::new(instant(1)?),
        original.into_journal(),
        offline,
    )?;
    let tick = resumed.tick()?;
    assert_eq!(
        tick.receipts().len(),
        1,
        "a sent effect can still be resolved"
    );
    assert_eq!(resumed.owner().reconciled(), &[operation]);
    assert!(
        resumed.owner().dispatched().is_empty(),
        "query never mutates"
    );
    Ok(())
}

/// Bounded authority storage refuses growth without evicting old withdrawals.
#[test]
fn e71_authority_saturation_and_recovery_preserve_revocation() -> TestResult {
    let settings = settings(1)?;
    let mut source = FixtureAuthority::new(SecurityEpoch::INITIAL, 4);
    for value in 0..rocco_runtime::MAX_TRACKED_RUNS {
        source.trust(
            TenantId::from_bytes(bytes(u64::try_from(value)?)),
            settings.policy.clone(),
            settings.capabilities.clone(),
            settings.validity,
        )?;
    }
    let extra = TenantId::from_bytes(bytes(u64::MAX));
    assert_eq!(
        source.trust(
            extra,
            settings.policy.clone(),
            settings.capabilities.clone(),
            settings.validity
        ),
        Err(FenceRefusal::AuthorityCapacity)
    );
    let tenant = TenantId::from_bytes(bytes(1));
    for value in 0..rocco_runtime::MAX_JOURNAL_ENTRIES {
        source.withdraw(
            tenant,
            OperationId::from_bytes(bytes(u64::try_from(value)?)),
        )?;
    }
    assert_eq!(
        source.withdraw(tenant, OperationId::from_bytes(bytes(u64::MAX))),
        Err(FenceRefusal::AuthorityCapacity)
    );
    assert!(
        !source.withdraw(tenant, OperationId::from_bytes(bytes(10)))?,
        "idempotent withdrawal works at capacity"
    );
    source.set_connected(false);
    assert_eq!(
        source.trust(
            tenant,
            settings.policy.clone(),
            settings.capabilities.clone(),
            settings.validity
        ),
        Err(FenceRefusal::AuthorityUnavailable)
    );
    source.set_connected(true);
    let mut runtime = subject(settings, source, 1)?;
    runtime.submit(command(1, 10, ReconciliationStrategy::Queryable)?)?;
    let tick = runtime.tick()?;
    assert_eq!(
        tick.fenced(),
        &[(
            OperationId::from_bytes(bytes(10)),
            FenceRefusal::Revoked {
                operation: OperationId::from_bytes(bytes(10))
            }
        )]
    );
    assert!(
        runtime.owner().dispatched().is_empty(),
        "saturation never erases withdrawal"
    );
    Ok(())
}

/// Replacement fixture that delivers revocation at the actual reach boundary.
#[derive(Debug)]
struct RevokeAtReach(FixtureAuthority);

impl rocco_runtime::authority::AuthoritySource for RevokeAtReach {
    fn begin_session(&mut self, offline_bound: u32) {
        self.0.begin_session(offline_bound);
    }

    fn fence(&self) -> &rocco_runtime::RevocationFence {
        self.0.fence()
    }

    fn authenticate_reach(
        &mut self,
        intent: &rocco_contracts::effect::EffectIntent,
        pinned: &rocco_contracts::replay::PinnedInputs,
    ) -> Result<&mut rocco_runtime::RevocationFence, FenceRefusal> {
        self.0
            .advance_epoch()
            .map_err(|_| FenceRefusal::AuthorityUnavailable)?;
        self.0.authenticate_reach(intent, pinned)
    }
}

/// Revocation arriving after reservation fences the first dispatch too.
#[test]
fn sim_e71_source_is_consulted_between_reservation_and_dispatch() -> TestResult {
    sweep(|seed| {
        let settings = settings(seed)?;
        let authority = RevokeAtReach(source(&settings, seed, 8)?);
        let mut runtime: Runtime<FoldProjection, ScriptedOwner, FixedClock, RevokeAtReach> =
            Runtime::new(
                settings,
                ScriptedOwner::new(),
                FixedClock::new(instant(seed)?),
                authority,
            )?;
        runtime.submit(command(1, 10, ReconciliationStrategy::Queryable)?)?;
        let tick = runtime.tick()?;
        assert_eq!(
            runtime.projection().intents().len(),
            1,
            "reservation must exist before the reach"
        );
        assert_eq!(
            tick.fenced(),
            &[(
                OperationId::from_bytes(bytes(10)),
                FenceRefusal::StaleEpoch {
                    admitted: SecurityEpoch::new(seed),
                    current: SecurityEpoch::new(seed.saturating_add(1))
                }
            )]
        );
        assert!(
            runtime.owner().dispatched().is_empty(),
            "source replacement must still enforce the fence"
        );
        Ok(ContentDigest::over(&[format!("{tick:?}").as_bytes()]))
    })
}

/// A queued proposal cannot relabel old source evidence with a newly read epoch.
#[test]
fn sim_e71_queued_snapshots_require_new_source_evidence() -> TestResult {
    sweep(|seed| {
        let settings = settings(seed)?;
        let mut runtime = subject(settings.clone(), source(&settings, seed, 4)?, seed)?;
        runtime.submit(command(1, 10, ReconciliationStrategy::IdempotentRetrySafe)?)?;
        let next = runtime.authority_mut().advance_epoch()?;
        let tick = runtime.tick()?;
        assert_eq!(
            tick.fenced(),
            &[(
                OperationId::from_bytes(bytes(10)),
                FenceRefusal::StaleSnapshots
            )]
        );
        assert!(
            runtime.owner().dispatched().is_empty(),
            "queued snapshots must not acquire new authority by relabeling their epoch"
        );
        runtime.authority_mut().authenticate(next)?;
        let retry = runtime.tick()?;
        assert_eq!(
            retry.fenced(),
            tick.fenced(),
            "epoch contact alone cannot renew the old snapshot evidence"
        );
        runtime.authority_mut().trust(
            TenantId::from_bytes(bytes(1)),
            settings.policy,
            settings.capabilities,
            settings.validity,
        )?;
        let control = runtime.tick()?;
        assert_eq!(
            runtime.owner().dispatched().len(),
            1,
            "only independently renewed source evidence authorizes"
        );
        assert!(
            replay_authorizations(runtime.projection()).is_bit_identical(),
            "renewal never rewrites a recorded decision"
        );
        Ok(trace(&runtime, &control))
    })
}
