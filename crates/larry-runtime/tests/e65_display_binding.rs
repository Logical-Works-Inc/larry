//! E65 public-runtime journeys. The runtime is real; the deterministic owner
//! models an atomic compare-and-dispatch, not an integrated external service.

/// Seeded simulations use the required dedicated simulation source tree.
#[path = "sim/e65_display_binding.rs"]
mod simulation;

/// The atomic-compare-and-dispatch owner double and its helpers.
#[path = "support/atomic.rs"]
mod atomic;

use std::time::{Duration, SystemTime};

use atomic::{AtomicOwner, holder, resolved, swapped_field};
use larry_contracts::authority::{Budget, BudgetUnit, Grant, Scope, SecurityEpoch};
use larry_contracts::binding::{
    BindingError, BindingField, EffectTarget, MAX_BOUND_FIELD_BYTES, ResolvedEffect,
};
use larry_contracts::command::{Command, CommandContent, CommandKind};
use larry_contracts::effect::{EffectIntent, EffectOutcome, ReconciliationStrategy};
use larry_contracts::ids::{CommandId, ContentDigest, OperationId, RunId, TenantId, WorkerId};
use larry_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};
use larry_runtime::error::RuntimeError;
use larry_runtime::journal::{Journal, JournalEntry};
use larry_runtime::owner::EffectOwner;
use larry_runtime::payload::WorkProposal;
use larry_runtime::projection::{FoldProjection, ProjectionBackend, replay_authorizations};
use larry_runtime::runtime::{Runtime, RuntimeSettings, restore};
use lgwks_bot::clock::Clock;

/// Each simulation family exhausts this deterministic seed interval.
const SEEDS: u64 = 1_024;
/// Public runtime under the replacement owner contract.
type TestRuntime = Runtime<FoldProjection, AtomicOwner>;
/// Fallible public journeys avoid panic-based fixture construction.
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Seed-controlled time remains within portable SystemTime ranges.
fn instant(seed: u64) -> Result<SystemTime> {
    Ok(SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(seed.saturating_add(100)))
        .ok_or("fixture instant overflow")?)
}

/// The estate's virtual clock placed at `instant(seed)`, read as elapsed time since the Unix
/// epoch (larry#74).
fn clock_at(seed: u64) -> Result<Clock> {
    Ok(Clock::virtual_at(
        instant(seed)?.duration_since(SystemTime::UNIX_EPOCH)?,
    ))
}

/// Deterministic identity bytes, disjoint across tenants and commands.
fn identity(number: u64, tenant: u8) -> [u8; 16] {
    let mut bytes = [tenant; 16];
    bytes[..8].copy_from_slice(&number.to_be_bytes());
    bytes
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
        clock_at(seed)?,
    )?)
}

/// Names the exact field and verifies a human-readable recovery instruction.
fn assert_swap(
    result: std::result::Result<larry_runtime::runtime::TickReport, RuntimeError>,
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
        runtime.owner().executed().is_empty(),
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
    for &(tenant, operation, target) in runtime.owner().executed() {
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
        clock_at(seed)?,
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
        restored.owner().executed().is_empty(),
        "recovery is not fresh consent"
    );
    assert_eq!(
        trace(restored.journal()),
        expected_trace,
        "replay must not rewrite history"
    );
    Ok(expected_trace)
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
        runtime.owner().executed().len(),
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
    let members = wire
        .as_object_mut()
        .ok_or("serialized binding is not an object")?;
    members.insert(
        "digest",
        lgwks_std::json::to_value(&ContentDigest::over(&[b"forged"]))?,
    );
    let corrupt: larry_contracts::binding::EffectBinding = lgwks_std::json::from_value(wire)?;
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
    let mut runtime: Runtime<FoldProjection, ResolverOnly> =
        Runtime::new(settings(0)?, ResolverOnly::default(), clock_at(0)?)?;
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
