//! larry#212: every bound is a setting, and the runtime answers for the one it
//! was built with.
//!
//! Deterministic simulations, each driving the real [`Runtime`] through its
//! public surface: a host that sets 4,096 runs fills 2,000 and restores all of
//! them after a simulated kill, a payload exactly at the byte ceiling is
//! admitted while one byte past it is refused, the protocol range and the skew
//! bound come from the settings, and every bad setting value, a question
//! ceiling past 256 included (larry#295), is refused by name before the runtime
//! is built. No wall clock and no unseeded
//! randomness: identities, instants and the lease all come from the fixture's
//! pinned epoch, so each trace replays identically.

mod support;

use std::collections::BTreeSet;
use std::time::Duration;

use larry_contracts::authority::{BudgetUnit, Scope};
use larry_contracts::binding::MAX_BOUND_FIELD_BYTES;
use larry_contracts::command::{Command, CommandAcceptance, CommandContent, CommandKind};
use larry_contracts::effect::ReconciliationStrategy;
use larry_contracts::ids::{CommandId, OperationId, RunId, TenantId};
use larry_contracts::state::RunState;
use larry_runtime::error::RuntimeError;
use larry_runtime::journal::Journal;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::payload::WorkProposal;
use larry_runtime::projection::{FoldProjection, ProjectionBackend};
use larry_runtime::protocol::{
    ProtocolCapability, ProtocolError, ProtocolOffer, ProtocolVersionRange,
};
use larry_runtime::runtime::{Runtime, RuntimeSettings, restore};

use support::{TestRuntime, clock, proposal, settings, submit, tenant};

/// The declared maxima the settings must respect, retyped here deliberately: a
/// change to one is a change to what this test holds the runtime to.
const DECLARED_MAX_GRANTS: usize = 64;
const DECLARED_MAX_SKEW: Duration = Duration::from_secs(60);
const DECLARED_MAX_LEASE: Duration = Duration::from_secs(3_600);
const DECLARED_MAX_QUESTIONS: usize = 256;
/// The stated ceilings the journal, run table and tenant count accept
/// (larry#333); retyped here for the same reason as the others.
const DECLARED_MAX_JOURNAL_CAPACITY: usize = 1_048_576;
const DECLARED_MAX_RUN_CAPACITY: usize = 65_536;
const DECLARED_MAX_TENANTS: usize = 65_536;

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// The run ceiling the restore simulation configures, and how many runs it
/// actually tracks: more than the default 1,024, fewer than the ceiling.
const RUN_CAPACITY: usize = 4_096;
const TRACKED_RUNS: u128 = 2_000;

/// The proposal of run `index`: one unit of read work, one identity per run.
fn indexed_proposal(index: u128) -> WorkProposal {
    proposal(
        RunId::from_bytes(index.to_be_bytes()),
        tenant(1),
        OperationId::from_bytes(index.to_be_bytes()),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    )
}

/// `settings` with one field changed to a value its meaning refuses.
fn refused(
    base: &RuntimeSettings,
    name: &'static str,
    mutate: impl FnOnce(&mut RuntimeSettings),
) -> Outcome {
    let mut bad = base.clone();
    mutate(&mut bad);
    match Runtime::<FoldProjection, ScriptedOwner>::new(bad, ScriptedOwner::new(), clock()?) {
        Err(RuntimeError::BadSetting { name: got, .. }) => {
            assert_eq!(got, name, "the refusal names the setting");
            Ok(())
        }
        Err(other) => Err(format!("expected BadSetting for {name}, got {other}").into()),
        Ok(_) => Err(format!("a runtime accepted a bad {name}").into()),
    }
}

/// A host set to 4,096 runs tracks 2,000 of them, is dropped (the process is
/// gone and only the journal survives), and restores every one of the 2,000.
///
/// The restore is the point: a projection rebuilt at its own default of 1,024
/// runs refuses the 1,025th run and the whole history with it, so this
/// simulation fails unless `restore` rebuilds at `settings.run_capacity`
/// (larry#212).
#[test]
fn sim_a_runtime_set_to_4096_runs_fills_2000_and_restores_them() -> Outcome {
    let mut capped = settings(&[Scope::FileRead], &[Scope::FileRead], 100_000)?;
    capped.run_capacity = RUN_CAPACITY;
    let mut runtime: TestRuntime = Runtime::new(capped.clone(), ScriptedOwner::new(), clock()?)?;
    for index in 1..=TRACKED_RUNS {
        let body = indexed_proposal(index);
        runtime.submit(submit(CommandId::from_bytes(index.to_be_bytes()), &body)?)?;
        runtime.tick()?;
    }
    assert_eq!(
        runtime.projection().run_count(),
        usize::try_from(TRACKED_RUNS)?,
        "every run the host set capacity for is tracked"
    );
    let entries = runtime.journal().len();
    // The kill: nothing of this process survives but the journal.
    let journal = runtime.into_journal();
    let restored: TestRuntime = restore(capped, ScriptedOwner::new(), clock()?, journal)?;
    assert_eq!(
        restored.settings().run_capacity,
        RUN_CAPACITY,
        "the restored runtime keeps the configured ceiling"
    );
    assert_eq!(
        restored.journal().len(),
        entries,
        "every acknowledged entry survives the kill"
    );
    assert_eq!(
        restored.projection().run_count(),
        usize::try_from(TRACKED_RUNS)?,
        "the projection rebuilder ran at the configured capacity, not its default"
    );
    assert_eq!(
        restored
            .projection()
            .run_state(RunId::from_bytes(1_u128.to_be_bytes())),
        Some(RunState::Reconciling),
        "a restored run keeps the state the journal recorded"
    );
    Ok(())
}

/// A `Steer` command whose payload is exactly `payload_len` bytes.
///
/// The proposal encodes as a JSON object, so the padding field is sized
/// against the object's own length: the caller gets the exact byte count the
/// ceiling is measured in, never an approximation of it.
fn padded_steer(
    identity: u128,
    scope: TenantId,
    payload_len: usize,
) -> Result<Command, Box<dyn std::error::Error>> {
    let body = WorkProposal::new(
        RunId::from_bytes(identity.to_be_bytes()),
        scope,
        OperationId::from_bytes(identity.to_be_bytes()),
        BTreeSet::new(),
        0,
        BudgetUnit::Requests,
        ReconciliationStrategy::Queryable,
    );
    let mut value = lgwks_std::json::to_value(&body)?;
    value
        .as_object_mut()
        .ok_or("a proposal encodes as a JSON object")?
        .insert("padding".to_owned(), String::new().into());
    let base = lgwks_std::json::to_vec(&value)?.len();
    let pad = payload_len
        .checked_sub(base)
        .ok_or("the ceiling is below the proposal's own encoded size")?;
    value
        .as_object_mut()
        .ok_or("a proposal encodes as a JSON object")?
        .insert("padding".to_owned(), "x".repeat(pad).into());
    let bytes = lgwks_std::json::to_vec(&value)?.into_boxed_slice();
    assert_eq!(
        bytes.len(),
        payload_len,
        "the payload is the exact size asked"
    );
    Ok(Command::new(
        CommandId::from_bytes(identity.to_be_bytes()),
        scope,
        Vec::new(),
        CommandContent::new(CommandKind::Steer, bytes),
    )?)
}

/// The protocol offer the ingress accepts: one tenant, one version, the
/// command capability, headless.
fn offer(scope: TenantId) -> ProtocolOffer {
    ProtocolOffer::new(
        scope,
        ProtocolVersionRange::new(1, 1),
        BTreeSet::from([ProtocolCapability::SubmitCommand]),
        true,
        false,
    )
}

/// A payload at the configured ceiling is admitted; one byte past it is
/// refused with its own size and the ceiling, and nothing is written.
#[test]
fn sim_a_payload_one_byte_past_the_ceiling_is_refused_and_one_at_it_is_admitted() -> Outcome {
    const CEILING: usize = 512;
    let mut capped = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
    capped.max_payload_bytes = CEILING;
    let mut runtime: TestRuntime = Runtime::new(capped, ScriptedOwner::new(), clock()?)?;
    let scope = tenant(1);
    let offer = offer(scope);

    let before = runtime.journal().len();
    let over = padded_steer(3, scope, CEILING + 1)?;
    match runtime.submit_protocol(&offer, over, None) {
        Err(RuntimeError::PayloadTooLarge { bytes, ceiling }) => {
            assert_eq!(bytes, CEILING + 1, "the refusal names the payload's size");
            assert_eq!(ceiling, CEILING, "the refusal names the configured ceiling");
        }
        other => {
            return Err(format!("one byte past the ceiling must be refused: {other:?}").into());
        }
    }
    assert_eq!(
        runtime.journal().len(),
        before,
        "a refused payload leaves nothing recorded"
    );

    let at = padded_steer(4, scope, CEILING)?;
    let (acceptance, version) = runtime.submit_protocol(&offer, at, None)?;
    assert_eq!(
        acceptance,
        CommandAcceptance::Initial,
        "the at-limit payload is admitted"
    );
    assert_eq!(version, 1, "the negotiated version is the configured one");
    assert_eq!(
        runtime.journal().len(),
        before + 1,
        "the admitted payload is durable"
    );
    Ok(())
}

/// The protocol range is the runtime's setting, not a compiled constant: each
/// runtime negotiates against the range it was built with, and a refusal names
/// that range.
#[test]
fn sim_the_protocol_range_comes_from_the_settings() -> Outcome {
    let scope = tenant(1);
    let mut narrow = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
    narrow.min_protocol_version = 1;
    narrow.max_protocol_version = 1;
    let mut runtime: TestRuntime = Runtime::new(narrow, ScriptedOwner::new(), clock()?)?;
    let newer = ProtocolOffer::new(
        scope,
        ProtocolVersionRange::new(2, 2),
        BTreeSet::from([ProtocolCapability::SubmitCommand]),
        true,
        false,
    );
    match runtime.submit_protocol(&newer, padded_steer(5, scope, 512)?, None) {
        Err(RuntimeError::Protocol(ProtocolError::UnsupportedVersion {
            supported_min,
            supported_max,
            ..
        })) => {
            assert_eq!(
                (supported_min, supported_max),
                (1, 1),
                "the refusal names the range this runtime supports"
            );
        }
        other => {
            return Err(format!("version 2 must be refused by a 1..=1 runtime: {other:?}").into());
        }
    }

    let mut wide = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
    wide.min_protocol_version = 1;
    wide.max_protocol_version = 2;
    let mut upgraded: TestRuntime = Runtime::new(wide, ScriptedOwner::new(), clock()?)?;
    let (acceptance, version) =
        upgraded.submit_protocol(&newer, padded_steer(6, scope, 512)?, None)?;
    assert_eq!(acceptance, CommandAcceptance::Initial);
    assert_eq!(version, 2, "the configured range admits the newer version");
    Ok(())
}

/// The skew bound is pinned into the validity snapshot every decision carries,
/// so one knob decides the allowance and a replayed decision re-derives it.
#[test]
fn sim_the_skew_bound_is_pinned_into_every_decisions_validity() -> Outcome {
    let skew = Duration::from_secs(30);
    let mut honest = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
    honest.skew_bound = skew;
    let runtime: TestRuntime = Runtime::new(honest, ScriptedOwner::new(), clock()?)?;
    assert_eq!(
        runtime.settings().validity.skew_bound,
        skew,
        "the runtime pins the configured skew into its validity snapshot"
    );
    Ok(())
}

/// One bad setting: the name its refusal must carry, and the change that makes
/// the value bad.
type BadCase = (&'static str, fn(&mut RuntimeSettings));

/// Every setting whose value its meaning refuses is refused by name, before a
/// runtime exists to enforce it: zero capacities, a journal or run ceiling past
/// its stated maximum (larry#333), a tenant count outside its range, a question
/// bound above its declared ceiling (larry#295), a grant bound above the
/// declared work bound, a payload ceiling above the hashed-field bound, a skew
/// or lease past its maximum, and a protocol range that is not a range.
#[test]
fn sim_every_bad_setting_value_is_refused_by_name() -> Outcome {
    let good = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
    let mut at_ceiling = good.clone();
    at_ceiling.max_pending_questions = DECLARED_MAX_QUESTIONS;
    at_ceiling.journal_capacity = DECLARED_MAX_JOURNAL_CAPACITY;
    at_ceiling.run_capacity = DECLARED_MAX_RUN_CAPACITY;
    at_ceiling.max_tenants = DECLARED_MAX_TENANTS;
    Runtime::<FoldProjection, ScriptedOwner>::new(at_ceiling, ScriptedOwner::new(), clock()?)?;
    let cases: [BadCase; 18] = [
        ("journal_capacity", |settings| settings.journal_capacity = 0),
        ("journal_capacity", |settings| {
            settings.journal_capacity = DECLARED_MAX_JOURNAL_CAPACITY + 1;
        }),
        ("run_capacity", |settings| settings.run_capacity = 0),
        ("run_capacity", |settings| {
            settings.run_capacity = DECLARED_MAX_RUN_CAPACITY + 1;
        }),
        ("max_tenants", |settings| settings.max_tenants = 0),
        ("max_tenants", |settings| {
            settings.max_tenants = DECLARED_MAX_TENANTS + 1;
        }),
        ("max_pending_questions", |settings| {
            settings.max_pending_questions = 0;
        }),
        ("max_pending_questions", |settings| {
            settings.max_pending_questions = DECLARED_MAX_QUESTIONS + 1;
        }),
        ("max_grants", |settings| settings.max_grants = 0),
        ("max_grants", |settings| {
            settings.max_grants = DECLARED_MAX_GRANTS + 1;
        }),
        ("max_payload_bytes", |settings| {
            settings.max_payload_bytes = 0;
        }),
        ("max_payload_bytes", |settings| {
            settings.max_payload_bytes = MAX_BOUND_FIELD_BYTES + 1;
        }),
        ("max_effect_attempts", |settings| {
            settings.max_effect_attempts = 0;
        }),
        ("skew_bound_secs", |settings| {
            settings.skew_bound = DECLARED_MAX_SKEW + Duration::from_secs(1);
        }),
        ("lease_secs", |settings| settings.lease = Duration::ZERO),
        ("lease_secs", |settings| {
            settings.lease = DECLARED_MAX_LEASE + Duration::from_secs(1);
        }),
        ("min_protocol_version", |settings| {
            settings.min_protocol_version = 0;
        }),
        ("max_protocol_version", |settings| {
            settings.max_protocol_version = 0;
        }),
    ];
    for (name, mutate) in cases {
        refused(&good, name, mutate)?;
    }
    // A restore validates the same way: a bad ceiling is refused before the
    // recorded history is read, not after it.
    let mut bad_questions = good.clone();
    bad_questions.max_pending_questions = usize::MAX;
    match restore::<FoldProjection, ScriptedOwner>(
        bad_questions,
        ScriptedOwner::new(),
        clock()?,
        Journal::new(4)?,
    ) {
        Err(RuntimeError::BadSetting { name, .. }) => {
            assert_eq!(name, "max_pending_questions", "restore caps the questions");
        }
        Err(other) => return Err(format!("expected BadSetting from restore, got {other}").into()),
        Ok(_) => return Err("restore accepted an unbounded question ceiling".into()),
    }
    let mut bad = good;
    bad.run_capacity = 0;
    match restore::<FoldProjection, ScriptedOwner>(
        bad,
        ScriptedOwner::new(),
        clock()?,
        Journal::new(4)?,
    ) {
        Err(RuntimeError::BadSetting { name, .. }) => {
            assert_eq!(name, "run_capacity", "restore names the setting too");
            Ok(())
        }
        Err(other) => Err(format!("expected BadSetting from restore, got {other}").into()),
        Ok(_) => Err("restore accepted a zero run capacity".into()),
    }
}
