//! E65 public-runtime journeys. The runtime is real; the deterministic owner
//! models an atomic compare-and-dispatch, not an integrated external service.

use std::time::{Duration, SystemTime};

use rocco_contracts::authority::{Budget, BudgetUnit, Grant, Scope, SecurityEpoch};
use rocco_contracts::binding::{
    BindingError, BindingField, EffectTarget, MAX_BOUND_FIELD_BYTES, ResolvedEffect,
};
use rocco_contracts::command::{Command, CommandContent, CommandKind};
use rocco_contracts::effect::{EffectIntent, EffectOutcome, ReconciliationStrategy};
use rocco_contracts::ids::{CommandId, ContentDigest, OperationId, RunId, TenantId, WorkerId};
use rocco_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};
use rocco_runtime::clock::FixedClock;
use rocco_runtime::error::RuntimeError;
use rocco_runtime::journal::{Journal, JournalEntry};
use rocco_runtime::owner::EffectOwner;
use rocco_runtime::payload::WorkProposal;
use rocco_runtime::projection::{FoldProjection, ProjectionBackend, replay_authorizations};
use rocco_runtime::runtime::{Runtime, RuntimeSettings, restore};

/// Each simulation family exhausts this deterministic seed interval.
const SEEDS: u64 = 1_024;
/// Public runtime under the replacement owner contract.
type TestRuntime = Runtime<FoldProjection, AtomicOwner, FixedClock>;
/// Fallible public journeys avoid panic-based fixture construction.
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A deterministic owner whose fourth observation is the atomic effect boundary.
#[derive(Debug, Clone)]
struct AtomicOwner {
    /// Single seed controls bytes, identity, time and fault location.
    seed: u64,
    /// Observation number that swaps one field; zero never swaps.
    change_on: u64,
    /// Observations already made.
    observations: u64,
    /// Whether the effect's response is deliberately lost.
    lose_reply: bool,
    /// Bounded by the runtime queue/workload in these tests.
    executed: Vec<(TenantId, OperationId, ResolvedEffect)>,
}

impl AtomicOwner {
    /// Starts an isolated owner without an ambient clock or network.
    fn new(seed: u64, change_on: u64) -> Self {
        Self {
            seed,
            change_on,
            observations: 0,
            lose_reply: false,
            executed: Vec::new(),
        }
    }

    /// Atomically observes the same scripted state used by execution.
    fn observe(&mut self) -> std::result::Result<ResolvedEffect, BindingError> {
        self.observations = self.observations.saturating_add(1);
        resolved(
            self.seed,
            self.change_on != 0 && self.observations >= self.change_on,
        )
    }
}

impl EffectOwner for AtomicOwner {
    fn resolve(
        &mut self,
        _intent: &EffectIntent,
    ) -> std::result::Result<Option<ResolvedEffect>, BindingError> {
        self.observe().map(Some)
    }

    fn dispatch_bound(
        &mut self,
        intent: &EffectIntent,
        _at: SystemTime,
    ) -> std::result::Result<EffectOutcome, BindingError> {
        let current = self.observe()?;
        intent
            .binding()
            .ok_or(BindingError::MissingDisplay)?
            .resolved()
            .verify(&current)?;
        // The validated resolved value itself is executed, never a second lookup.
        self.executed
            .push((intent.tenant, intent.operation, current));
        Ok(if self.lose_reply {
            EffectOutcome::Unknown {
                operation: intent.operation,
            }
        } else {
            EffectOutcome::Succeeded {
                operation: intent.operation,
            }
        })
    }

    fn dispatch(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
        // Any legacy dispatch is an observable test failure, not a fallback.
        self.observations = u64::MAX;
        EffectOutcome::Unknown {
            operation: intent.operation,
        }
    }

    fn reconcile(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
        EffectOutcome::Succeeded {
            operation: intent.operation,
        }
    }
}

/// Seed-selected field with a separate oracle from the digest implementation.
fn swapped_field(seed: u64) -> BindingField {
    match seed & 7 {
        0 => BindingField::Resource,
        1 => BindingField::Symlink,
        2 => BindingField::Recipient,
        3 => BindingField::Account,
        4 => BindingField::Payload,
        5 => BindingField::Adapter,
        6 => BindingField::Schema,
        _ => BindingField::PolicyEpoch,
    }
}

/// Opaque fixture identities vary by seed; exactly one field changes.
fn resolved(seed: u64, changed: bool) -> std::result::Result<ResolvedEffect, BindingError> {
    let original = seed.to_be_bytes();
    let replacement = seed.saturating_add(1).to_be_bytes();
    let field = seed & 7;
    let bytes = |index| {
        if changed && field == index {
            replacement.as_slice()
        } else {
            original.as_slice()
        }
    };
    let target = EffectTarget::new(bytes(0), bytes(1), bytes(2), bytes(3), bytes(4), bytes(5))?;
    Ok(ResolvedEffect::new(
        target,
        if changed && field == 6 { 2 } else { 1 },
        SecurityEpoch::new(u64::from(changed && field == 7)),
    ))
}

/// Seed-controlled time remains within portable SystemTime ranges.
fn instant(seed: u64) -> Result<SystemTime> {
    Ok(SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(seed.saturating_add(100)))
        .ok_or("fixture instant overflow")?)
}

/// Deterministic identity bytes, disjoint across tenants and commands.
fn identity(number: u64, tenant: u8) -> [u8; 16] {
    let mut bytes = [tenant; 16];
    bytes[..8].copy_from_slice(&number.to_be_bytes());
    bytes
}

/// Host-authenticated holder used by both tenants without sharing their grants.
fn holder() -> WorkerId {
    WorkerId::from_bytes([42; 16])
}

/// One independently covering host grant.
fn grant(tenant: u8, scopes: &[Scope], budget: u64, seed: u64) -> Result<Grant> {
    Ok(Grant::new(
        TenantId::from_bytes([tenant; 16]),
        holder(),
        scopes.iter().copied().collect(),
        Budget::new(budget, BudgetUnit::Requests),
        instant(seed.saturating_add(1_000))?,
        0,
    )?)
}

/// Builds the actual serialized public command.
fn command(
    number: u64,
    tenant: u8,
    scopes: &[Scope],
    strategy: ReconciliationStrategy,
) -> Result<Command> {
    let bytes = identity(number, tenant);
    let tenant = TenantId::from_bytes([tenant; 16]);
    let proposal = WorkProposal::new(
        RunId::from_bytes(bytes),
        tenant,
        OperationId::from_bytes(bytes),
        scopes.iter().copied().collect(),
        3,
        BudgetUnit::Requests,
        strategy,
    );
    Ok(Command::new(
        CommandId::from_bytes(bytes),
        tenant,
        Vec::new(),
        CommandContent::new(
            CommandKind::Submit,
            lgwks_std::json::to_vec(&proposal)?.into_boxed_slice(),
        ),
    )?)
}

/// Host snapshot admits all test scopes; each display still needs one grant.
fn settings(seed: u64) -> Result<RuntimeSettings> {
    let scopes = [Scope::FileRead, Scope::FileWrite, Scope::NetworkEgress];
    Ok(RuntimeSettings::with_defaults(
        PolicySnapshot::new(17, scopes.into_iter().collect()),
        CapabilitySnapshot::new(scopes.into_iter().collect()),
        ValiditySnapshot::new(instant(seed.saturating_add(2_000))?),
        BudgetUnit::Requests,
        10_000,
    ))
}

/// Fresh public runtime, with no shared persistent or approval state.
fn runtime(seed: u64, change_on: u64) -> Result<TestRuntime> {
    Ok(Runtime::new(
        settings(seed)?,
        AtomicOwner::new(seed, change_on),
        FixedClock::new(instant(seed)?),
    )?)
}

/// Names the exact field and verifies a human-readable recovery instruction.
fn assert_swap(
    result: std::result::Result<rocco_runtime::runtime::TickReport, RuntimeError>,
    field: BindingField,
) {
    assert!(
        matches!(result, Err(RuntimeError::Binding(BindingError::Changed { field: got })) if got == field),
        "expected exact field {field:?}, got {result:?}"
    );
}

/// Trace identity contains actual recorded commands and events, in order.
fn trace(journal: &Journal) -> ContentDigest {
    let mut digest = ContentDigest::over(&[b"e65-trace"]);
    for entry in journal.entries() {
        let next = match *entry {
            JournalEntry::Command(ref command) => command.envelope_digest(),
            JournalEntry::Event(ref event) => event.payload_digest(),
            _ => continue,
        };
        digest = ContentDigest::over(&[digest.as_bytes(), next.as_bytes()]);
    }
    digest
}

/// A fault at each observation boundary must preserve the declared evidence.
fn swap_journey(seed: u64, change_on: u64) -> Result<ContentDigest> {
    let mut runtime = runtime(seed, change_on)?;
    let command = command(
        seed,
        1,
        &[Scope::FileRead],
        ReconciliationStrategy::Queryable,
    )?;
    let display = runtime.display(
        &command,
        holder(),
        &[grant(1, &[Scope::FileRead], 5, seed)?],
    )?;
    let shown = *display.binding();
    runtime.submit_displayed(command, display)?;
    assert_swap(runtime.tick(), swapped_field(seed));
    assert!(
        runtime.owner().executed.is_empty(),
        "swap must execute nothing, seed {seed}"
    );
    assert!(
        runtime.projection().receipts().is_empty(),
        "refusal cannot publish success"
    );
    assert_eq!(
        runtime.fence().offline_spent(),
        0,
        "refusal consumes no reach"
    );
    if change_on == 2 {
        assert!(
            runtime.journal().is_empty(),
            "admission swap must be atomic"
        );
        assert_eq!(
            runtime.remaining_budget(),
            10_000,
            "no reservation on admission refusal"
        );
    } else {
        assert_eq!(
            runtime
                .projection()
                .intents()
                .first()
                .and_then(EffectIntent::binding),
            Some(&shown),
            "the durable intent must retain exactly what was shown"
        );
        assert_eq!(
            runtime.remaining_budget(),
            9_997,
            "ambiguous reservation remains held"
        );
        runtime.tick()?;
        assert!(
            runtime.projection().receipts().is_empty(),
            "a status answer cannot resolve a never-sent effect"
        );
    }
    Ok(trace(runtime.journal()))
}

/// Resource/link/destination/body/adapter/schema/epoch swaps before admission.
#[test]
fn sim_e65_display_to_admission_swaps() -> Result {
    for seed in 0..SEEDS {
        assert_eq!(
            swap_journey(seed, 2)?,
            swap_journey(seed, 2)?,
            "seed {seed} replay"
        );
    }
    Ok(())
}

/// The durable reservation cannot authorize a changed dispatch target.
#[test]
fn sim_e65_admission_to_dispatch_swaps() -> Result {
    for seed in 0..SEEDS {
        assert_eq!(
            swap_journey(seed, 3)?,
            swap_journey(seed, 3)?,
            "seed {seed} replay"
        );
    }
    Ok(())
}

/// A race at the executing owner still refuses and preserves the sent witness.
#[test]
fn sim_e65_atomic_owner_refusal() -> Result {
    for seed in 0..SEEDS {
        assert_eq!(
            swap_journey(seed, 4)?,
            swap_journey(seed, 4)?,
            "seed {seed} replay"
        );
    }
    Ok(())
}

/// Shared resource identity does not imply shared tenant or approval identity.
fn success_journey(seed: u64) -> Result<ContentDigest> {
    let mut runtime = runtime(seed, 0)?;
    let mut displayed = Vec::new();
    for tenant in [1, 2] {
        let command = command(
            seed,
            tenant,
            &[Scope::FileRead],
            ReconciliationStrategy::Queryable,
        )?;
        let display = runtime.display(
            &command,
            holder(),
            &[grant(tenant, &[Scope::FileRead], 5, seed)?],
        )?;
        displayed.push(display.binding().digest());
        runtime.submit_displayed(command, display)?;
    }
    assert_ne!(
        displayed[0], displayed[1],
        "tenant is part of the display identity"
    );
    let report = runtime.tick()?;
    assert_eq!(
        report.receipts().len(),
        2,
        "both independently authorized tenants finish"
    );
    for &(tenant, operation, target) in &runtime.owner().executed {
        assert_eq!(
            target,
            resolved(seed, false)?,
            "only displayed target executes"
        );
        assert!(
            report
                .receipts()
                .iter()
                .any(|receipt| receipt.tenant() == tenant && receipt.operation() == operation),
            "receipt must stay within its owner scope"
        );
    }
    let expected = runtime.projection().intents().to_vec();
    let expected_trace = trace(runtime.journal());
    let mut restored: TestRuntime = restore(
        settings(seed)?,
        AtomicOwner::new(seed, 0),
        FixedClock::new(instant(seed)?),
        runtime.into_journal(),
    )?;
    assert_eq!(
        restored.projection().intents(),
        expected,
        "binding survives discarded process state"
    );
    assert!(
        replay_authorizations(restored.projection()).is_bit_identical(),
        "pinned authorization replays"
    );
    assert!(
        restored.tick()?.is_idle(),
        "terminal recovery must not redispatch"
    );
    assert!(
        restored.owner().executed.is_empty(),
        "recovery is not fresh consent"
    );
    assert_eq!(
        trace(restored.journal()),
        expected_trace,
        "replay must not rewrite history"
    );
    Ok(expected_trace)
}

/// Positive controls and exact seeded trace replay across two tenant identities.
#[test]
fn sim_e65_bound_execution_and_replay() -> Result {
    for seed in 0..SEEDS {
        assert_eq!(
            success_journey(seed)?,
            success_journey(seed)?,
            "seed {seed} trace"
        );
    }
    Ok(())
}

/// Lost replies and restart may not rebind an idempotent retry to another target.
#[test]
fn sim_e65_retry_after_restore_keeps_binding() -> Result {
    for seed in 0..SEEDS {
        let mut runtime: TestRuntime = Runtime::new(
            settings(seed)?,
            AtomicOwner {
                lose_reply: true,
                ..AtomicOwner::new(seed, 0)
            },
            FixedClock::new(instant(seed)?),
        )?;
        let command = command(
            seed,
            1,
            &[Scope::FileRead],
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        let display = runtime.display(
            &command,
            holder(),
            &[grant(1, &[Scope::FileRead], 3, seed)?],
        )?;
        runtime.submit_displayed(command, display)?;
        assert!(
            runtime.tick()?.receipts().is_empty(),
            "lost reply stays unknown"
        );
        let mut restored: TestRuntime = restore(
            settings(seed)?,
            AtomicOwner::new(seed, 1),
            FixedClock::new(instant(seed)?),
            runtime.into_journal(),
        )?;
        let before = trace(restored.journal());
        assert_swap(restored.tick(), swapped_field(seed));
        assert_eq!(
            trace(restored.journal()),
            before,
            "refused retry writes no invented outcome"
        );
        assert!(
            restored.owner().executed.is_empty(),
            "restoration must not swap targets"
        );
    }
    Ok(())
}

/// Separate grants must not contribute disjoint authority to one effect.
#[test]
fn sim_e65_overlapping_grants_never_union() -> Result {
    for seed in 0..SEEDS {
        let mut runtime = runtime(seed, 0)?;
        let command = command(
            seed,
            1,
            &[Scope::FileRead, Scope::FileWrite],
            ReconciliationStrategy::Queryable,
        )?;
        let split = [
            grant(1, &[Scope::FileRead, Scope::NetworkEgress], 10, seed)?,
            grant(1, &[Scope::FileWrite, Scope::NetworkEgress], 10, seed)?,
        ];
        assert!(
            matches!(
                runtime.display(&command, holder(), &split),
                Err(RuntimeError::Binding(BindingError::NoCoveringGrant))
            ),
            "disjoint scopes cannot stitch"
        );
        let cheap = grant(1, &[Scope::FileRead, Scope::FileWrite], 2, seed)?;
        assert!(
            matches!(
                runtime.display(&command, holder(), &[cheap.clone(), cheap]),
                Err(RuntimeError::Binding(BindingError::NoCoveringGrant))
            ),
            "budgets cannot add"
        );
        let foreign = grant(2, &[Scope::FileRead, Scope::FileWrite], 10, seed)?;
        assert!(
            matches!(
                runtime.display(&command, holder(), &[foreign]),
                Err(RuntimeError::Binding(BindingError::NoCoveringGrant))
            ),
            "tenant grants cannot cross"
        );
        let covering = grant(1, &[Scope::FileRead, Scope::FileWrite], 3, seed)?;
        let display = runtime.display(
            &command,
            holder(),
            &[split[0].clone(), covering, split[1].clone()],
        )?;
        runtime.submit_displayed(command, display)?;
        assert_eq!(
            runtime.tick()?.receipts().len(),
            1,
            "one sufficient grant must remain usable"
        );
        assert_eq!(
            runtime.projection().authorizations()[0]
                .pinned
                .budget
                .remaining,
            3,
            "selected ceiling is pinned"
        );
    }
    Ok(())
}

/// No display and copied displays fail before admission; the corrected journey succeeds.
#[test]
fn e65_missing_and_cross_command_display_refuse() -> Result {
    let mut runtime = runtime(9, 0)?;
    let first = command(1, 1, &[Scope::FileRead], ReconciliationStrategy::Queryable)?;
    runtime.submit(first.clone())?;
    assert!(
        matches!(
            runtime.tick(),
            Err(RuntimeError::Binding(BindingError::MissingDisplay))
        ),
        "display cannot be omitted"
    );
    assert!(runtime.journal().is_empty(), "missing display cannot admit");
    let display = runtime.display(&first, holder(), &[grant(1, &[Scope::FileRead], 5, 9)?])?;
    let foreign = command(1, 2, &[Scope::FileRead], ReconciliationStrategy::Queryable)?;
    assert!(
        matches!(
            runtime.submit_displayed(foreign, display.clone()),
            Err(RuntimeError::Binding(BindingError::Changed {
                field: BindingField::Command
            }))
        ),
        "display cannot move tenants"
    );
    runtime.submit_displayed(first.clone(), display)?;
    assert_eq!(
        runtime.tick()?.receipts().len(),
        1,
        "corrected command succeeds"
    );
    runtime.submit(first)?;
    assert_eq!(
        runtime.tick()?.duplicates().len(),
        1,
        "exact accepted retry needs no new display"
    );
    assert_eq!(
        runtime.owner().executed.len(),
        1,
        "duplicate cannot execute twice"
    );
    Ok(())
}

/// Queue saturation refuses without losing the caller's approval or prior work.
#[test]
fn e65_queue_saturation_and_recovery() -> Result {
    let mut runtime = runtime(0, 0)?;
    let grant = grant(1, &[Scope::FileRead], 3, 0)?;
    for number in 0..256 {
        let command = command(
            number,
            1,
            &[Scope::FileRead],
            ReconciliationStrategy::Queryable,
        )?;
        let display = runtime.display(&command, holder(), std::slice::from_ref(&grant))?;
        runtime.submit_displayed(command, display)?;
    }
    let overflow = command(
        256,
        1,
        &[Scope::FileRead],
        ReconciliationStrategy::Queryable,
    )?;
    let display = runtime.display(&overflow, holder(), &[grant])?;
    assert!(
        matches!(
            runtime.submit_displayed(overflow.clone(), display.clone()),
            Err(RuntimeError::ReadyWorkFull { capacity: 256 })
        ),
        "queue must backpressure"
    );
    assert_eq!(
        runtime.tick()?.receipts().len(),
        64,
        "bounded tick drains one batch"
    );
    runtime.submit_displayed(overflow, display)?;
    for _ in 0..4 {
        assert_eq!(
            runtime.fence_mut().authenticate(SecurityEpoch::INITIAL),
            SecurityEpoch::INITIAL,
            "fresh host authority"
        );
        runtime.tick()?;
    }
    assert_eq!(
        runtime.projection().receipts().len(),
        257,
        "all accepted work survives saturation"
    );
    Ok(())
}

/// Invalid sizes, grant bounds, schema drift and forged digest cache refuse explicitly.
#[test]
fn e65_bounds_epochs_and_integrity() -> Result {
    let too_large = vec![0; MAX_BOUND_FIELD_BYTES.saturating_add(1)];
    assert_eq!(
        EffectTarget::new(&too_large, b"", b"", b"", b"", b""),
        Err(BindingError::TooLarge {
            field: BindingField::Resource
        }),
        "bound precedes hashing"
    );
    let boundary = vec![0; MAX_BOUND_FIELD_BYTES];
    assert!(
        EffectTarget::new(&boundary, b"", b"", b"", b"", b"").is_ok(),
        "inclusive size bound"
    );
    let mut runtime = runtime(0, 0)?;
    let command = command(1, 1, &[Scope::FileRead], ReconciliationStrategy::Queryable)?;
    let grant = grant(1, &[Scope::FileRead], 3, 0)?;
    assert!(
        matches!(
            runtime.display(&command, holder(), &vec![grant.clone(); 65]),
            Err(RuntimeError::Binding(BindingError::TooManyGrants))
        ),
        "grant scan is bounded"
    );
    let display = runtime.display(&command, holder(), &[grant])?;
    let mut wire = lgwks_std::json::to_value(display.binding())?;
    wire["digest"] = lgwks_std::json::to_value(&ContentDigest::over(&[b"forged"]))?;
    let corrupt: rocco_contracts::binding::EffectBinding = lgwks_std::json::from_value(wire)?;
    assert_eq!(
        corrupt.verify_runtime(17, instant(0)?),
        Err(BindingError::Changed {
            field: BindingField::DisplayDigest
        }),
        "corrupt record must refuse"
    );
    assert_eq!(
        display.binding().verify_runtime(18, instant(0)?),
        Err(BindingError::Changed {
            field: BindingField::PolicyRevision
        }),
        "policy skew refuses"
    );
    assert_eq!(
        display.binding().verify_runtime(17, instant(1_000)?),
        Err(BindingError::Expired),
        "expiry is exclusive"
    );
    runtime.submit_displayed(command, display)?;
    assert_eq!(
        runtime.fence_mut().advance_epoch()?,
        SecurityEpoch::new(1),
        "host moves epoch"
    );
    assert_swap(runtime.tick(), BindingField::SecurityEpoch);
    assert!(
        runtime.journal().is_empty(),
        "stale display is refused before admission"
    );
    Ok(())
}

/// Losing the outcome write preserves the display in the authoritative intent.
#[test]
fn sim_e65_full_journal_restore_preserves_display() -> Result {
    for seed in 0..SEEDS {
        let mut limited = settings(seed)?;
        limited.journal_capacity = 6;
        let mut runtime: TestRuntime = Runtime::new(
            limited,
            AtomicOwner::new(seed, 0),
            FixedClock::new(instant(seed)?),
        )?;
        let command = command(
            seed,
            1,
            &[Scope::FileRead],
            ReconciliationStrategy::Queryable,
        )?;
        let display = runtime.display(
            &command,
            holder(),
            &[grant(1, &[Scope::FileRead], 3, seed)?],
        )?;
        let expected = *display.binding();
        runtime.submit_displayed(command, display)?;
        assert!(
            matches!(
                runtime.tick(),
                Err(RuntimeError::JournalFull { capacity: 6 })
            ),
            "fault must fall after reservation and before outcome, seed {seed}"
        );
        assert_eq!(
            runtime.owner().executed.len(),
            1,
            "effect happened before outcome loss"
        );
        let mut restored: TestRuntime = restore(
            settings(seed)?,
            AtomicOwner::new(seed, 1),
            FixedClock::new(instant(seed)?),
            runtime.into_journal(),
        )?;
        assert_eq!(
            restored.projection().intents()[0].binding(),
            Some(&expected),
            "display survives recovery"
        );
        assert_eq!(
            restored.tick()?.receipts().len(),
            1,
            "status of original operation remains queryable"
        );
        assert!(
            restored.owner().executed.is_empty(),
            "changed target cannot be dispatched by recovery"
        );
        assert!(
            replay_authorizations(restored.projection()).is_bit_identical(),
            "recovery preserves pinned authorization"
        );
    }
    Ok(())
}

/// Resolving alone is insufficient: an adapter must support atomic execution.
#[derive(Debug, Default)]
struct ResolverOnly {
    /// Detects an impermissible fall-through into legacy dispatch.
    legacy_calls: usize,
}

impl EffectOwner for ResolverOnly {
    fn resolve(
        &mut self,
        _intent: &EffectIntent,
    ) -> std::result::Result<Option<ResolvedEffect>, BindingError> {
        resolved(0, false).map(Some)
    }

    fn dispatch(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
        self.legacy_calls = self.legacy_calls.saturating_add(1);
        EffectOutcome::Succeeded {
            operation: intent.operation,
        }
    }

    fn reconcile(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
        EffectOutcome::Succeeded {
            operation: intent.operation,
        }
    }
}

/// Unsupported atomic execution cannot fall through or earn a success receipt.
#[test]
fn e65_unsupported_owner_and_malformed_display_refuse() -> Result {
    let mut runtime: Runtime<FoldProjection, ResolverOnly, FixedClock> = Runtime::new(
        settings(0)?,
        ResolverOnly::default(),
        FixedClock::new(instant(0)?),
    )?;
    let command = command(1, 1, &[Scope::FileRead], ReconciliationStrategy::Queryable)?;
    let grant = grant(1, &[Scope::FileRead], 3, 0)?;
    let malformed = Command::new(
        command.id(),
        command.tenant(),
        Vec::new(),
        CommandContent::new(CommandKind::Submit, Box::from(b"{".as_slice())),
    )?;
    assert!(
        matches!(
            runtime.display(&malformed, holder(), std::slice::from_ref(&grant)),
            Err(RuntimeError::Codec(_))
        ),
        "malformed bytes cannot create a display"
    );
    let oversized = Command::new(
        command.id(),
        command.tenant(),
        Vec::new(),
        CommandContent::new(
            CommandKind::Submit,
            vec![0; MAX_BOUND_FIELD_BYTES.saturating_add(1)].into_boxed_slice(),
        ),
    )?;
    assert!(
        matches!(
            runtime.display(&oversized, holder(), std::slice::from_ref(&grant)),
            Err(RuntimeError::Binding(BindingError::TooLarge {
                field: BindingField::Command
            }))
        ),
        "display input is bounded"
    );
    assert!(
        matches!(
            runtime.display(
                &command,
                WorkerId::from_bytes([99; 16]),
                std::slice::from_ref(&grant)
            ),
            Err(RuntimeError::Binding(BindingError::NoCoveringGrant))
        ),
        "holder identity cannot be substituted"
    );
    let display = runtime.display(&command, holder(), &[grant])?;
    let fields = display.binding().resolved().target().fields();
    for (index, field) in fields.iter().enumerate() {
        assert!(
            !fields[..index].contains(field),
            "identical bytes in different fields have separate domains"
        );
    }
    let json = lgwks_std::json::to_value(&display)?;
    assert_eq!(
        json["binding"]["digest"],
        lgwks_std::json::to_value(&display.binding().digest())?,
        "rendered identity is the retained identity"
    );
    runtime.submit_displayed(command, display)?;
    assert!(
        matches!(
            runtime.tick(),
            Err(RuntimeError::Binding(BindingError::UnsupportedOwner))
        ),
        "resolve-only owner must refuse atomic dispatch"
    );
    runtime.tick()?;
    assert_eq!(runtime.owner().legacy_calls, 0, "no legacy fallback");
    assert!(
        runtime.projection().receipts().is_empty(),
        "unsupported reach never earns success"
    );
    Ok(())
}
