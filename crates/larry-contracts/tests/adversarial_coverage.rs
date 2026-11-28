//! Adversarial coverage for the contract constructors, getters, displays and
//! bindings that the happy-path falsifiers never call out of line.
//!
//! The pure algebra here is already falsified elsewhere; these simulations
//! execute the definition lines themselves. Tiny `const fn` constructors are
//! called through function pointers so cross-crate inlining cannot attribute
//! their bodies to the caller: the assertions below verify the exact values
//! the real constructor produced. Sources: CWE-190 (integer boundary costs),
//! CWE-20 (hostile identity bytes), RFC 3339 pre-epoch instants, larry#197
//! (tenant isolation), E65 (display binding), Cell 04 (run-state algebra).

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use larry_contracts::authority::{
    AttenuationRequest, Budget, BudgetUnit, Grant, ResourcePolicy, Scope, SecurityEpoch,
};
use larry_contracts::binding::{
    BindingError, BindingField, EffectBinding, EffectTarget, ResolvedEffect,
};
use larry_contracts::error::AuthorityError;
use larry_contracts::ids::{
    CommandId, ContentDigest, OperationId, RunId, TenantId, WorkerId, WorkspaceId,
};
use larry_contracts::replay::{
    BudgetSnapshot, CapabilitySnapshot, Decision, DenialReason, EffectRequest, PinnedInputs,
    PolicySnapshot, ReplayRecord, ValiditySnapshot, capability, decide, replay_log,
};
use larry_contracts::state::RunState;
use larry_contracts::workspace::{
    ContinueReceipt, ContinueRequest, MissingMooAdapter, WorkBoard, WorkItem, WorkVersion,
    WorkspaceError, WorkspaceScope,
};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Calls a tiny `const` constructor or getter through an opaque function
/// pointer so its body executes out of line in its own crate. A direct call
/// lets cross-crate inlining attribute the body to this test binary, whose
/// paths coverage excludes, and the definition lines stay uncovered even
/// though they ran. Every value below is still asserted exactly.
fn outlined<F>(func: F) -> F {
    std::hint::black_box(func)
}

/// One scope set from a list.
fn scopes(list: &[Scope]) -> BTreeSet<Scope> {
    list.iter().copied().collect()
}

/// A pinned instant `secs` after the Unix epoch.
fn instant(secs: u64) -> Result<SystemTime, Box<dyn std::error::Error>> {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(secs))
        .ok_or_else(|| "fixture instant overflow".into())
}

/// The scope a workspace view is bound to.
fn scope(tenant: u8, workspace: u8) -> WorkspaceScope {
    WorkspaceScope::new(
        TenantId::from_bytes([tenant; 16]),
        WorkspaceId::from_bytes([workspace; 16]),
    )
}

// Source: larry#197 (one tenant's rows never answer another tenant) and the
// workspace projection invariant (INVARIANTS.md, workspace isolation). A
// descriptor's scope and version bindings must read back exactly, for two
// tenants holding the same run identity at once.
#[test]
fn sim_workspace_scope_and_version_roundtrip_assert_exact_bindings() -> Outcome {
    let version = WorkVersion::new(u64::MAX, ContentDigest::over(&[b"baseline"]), u64::MAX);
    // Boundary binding: the maximum revision and fence the projection orders.
    version.verify_current(version)?;
    assert!(
        version
            .verify_current(WorkVersion::new(
                0,
                ContentDigest::over(&[b"baseline"]),
                u64::MAX
            ))
            .is_err(),
        "a changed revision is a stale selection"
    );
    for tenant in [1_u8, 2] {
        let board_scope = scope(tenant, 9);
        let board = WorkBoard::new(board_scope);
        // Called through an opaque function pointer so the tiny const
        // getters execute out of line: cross-crate inlining would otherwise
        // attribute their bodies to this test binary, which coverage
        // excludes, and leave their definition lines uncovered.
        let scope_of: fn(&WorkBoard) -> WorkspaceScope = std::hint::black_box(WorkBoard::scope);
        assert_eq!(scope_of(&board), board_scope, "the board keeps its scope");
        let run = RunId::from_bytes([tenant; 16]);
        let item = WorkItem::new(board_scope, run, "resume", version, RunState::Admitted)?;
        let version_of: fn(&WorkItem) -> WorkVersion = std::hint::black_box(WorkItem::version);
        assert_eq!(version_of(&item), version, "the version reads back exactly");
        assert_eq!(item.run(), run, "the identity reads back exactly");
        let mut board = board;
        board.upsert(item)?;
        let stored = board
            .get(run)
            .ok_or("a stored descriptor must be found in its own scope")?;
        assert_eq!(version_of(stored), version);
    }
    // One selection through the disconnected boundary stays awaiting
    // dependency: no owner, no fabricated success.
    let board = WorkBoard::new(scope(1, 1));
    let mut missing = MissingMooAdapter::default();
    let missing_outcome = {
        let mut board = board;
        board.upsert(WorkItem::new(
            scope(1, 1),
            RunId::from_bytes([7; 16]),
            "held",
            version,
            RunState::Admitted,
        )?)?;
        let mut controller = larry_contracts::workspace::ContinueCoordinator::new(board);
        controller.continue_work(
            &mut missing,
            ContinueRequest::new(
                CommandId::from_bytes([7; 16]),
                scope(1, 1),
                RunId::from_bytes([7; 16]),
                u64::MAX,
            ),
        )?
    };
    assert_eq!(
        missing_outcome,
        larry_contracts::workspace::ContinueOutcome::AwaitingDependency
    );
    // A receipt binds the exact request, version and operation it names.
    let request = ContinueRequest::new(
        CommandId::from_bytes([3; 16]),
        scope(1, 1),
        RunId::from_bytes([3; 16]),
        4,
    );
    let receipt = ContinueReceipt::new(request, version, OperationId::from_bytes([5; 16]));
    assert_eq!(receipt.request, request);
    assert_eq!(receipt.version, version);
    assert_eq!(
        receipt.operation,
        OperationId::from_bytes([5; 16]),
        "the operation identity is the one named"
    );
    assert_eq!(
        WorkspaceError::ScopeMismatch.to_string(),
        "work belongs to another tenant or workspace"
    );
    Ok(())
}

// Source: the coverage algebra (every constructor executes out of line) and
// CWE-190 (a cost of u64::MAX must not wrap or coerce). Each constructor is
// called through a function pointer and its exact value asserted, including
// hostile inputs: empty scope sets, the maximum cost, and a pre-epoch
// observation instant.
#[test]
fn sim_contract_constructors_execute_out_of_line_with_exact_values() -> Outcome {
    // PolicySnapshot::new, revision and allowed_scopes.
    let new_policy: fn(u64, BTreeSet<Scope>) -> PolicySnapshot = outlined(PolicySnapshot::new);
    let policy = new_policy(u64::MAX, scopes(&[]));
    let revision_of: fn(&PolicySnapshot) -> u64 = outlined(PolicySnapshot::revision);
    assert_eq!(
        revision_of(&policy),
        u64::MAX,
        "the revision is kept exactly"
    );
    let scopes_of: fn(&PolicySnapshot) -> &BTreeSet<Scope> =
        outlined(PolicySnapshot::allowed_scopes);
    assert!(scopes_of(&policy).is_empty(), "no scope is admitted");
    let open = new_policy(7, scopes(&[Scope::FileRead, Scope::ModelCommand]));
    assert_eq!(revision_of(&open), 7);
    assert!(scopes_of(&open).contains(&Scope::ModelCommand));

    // BudgetSnapshot::new.
    let new_budget: fn(u64, BudgetUnit) -> BudgetSnapshot = outlined(BudgetSnapshot::new);
    let budget = new_budget(u64::MAX, BudgetUnit::MilliCents);
    assert_eq!(budget.remaining, u64::MAX, "the maximum cost is kept");
    assert_eq!(budget.unit, BudgetUnit::MilliCents);

    // CapabilitySnapshot::new and capabilities.
    let new_caps: fn(BTreeSet<Scope>) -> CapabilitySnapshot = outlined(CapabilitySnapshot::new);
    let caps = new_caps(scopes(&[]));
    let held_of: fn(&CapabilitySnapshot) -> &BTreeSet<Scope> =
        outlined(CapabilitySnapshot::capabilities);
    assert!(held_of(&caps).is_empty(), "no capability is held");
    let caps = new_caps(scopes(&[Scope::NetworkEgress]));
    assert!(held_of(&caps).contains(&Scope::NetworkEgress));

    // EffectRequest::new and every getter, at hostile values.
    let new_request: fn(
        OperationId,
        BTreeSet<Scope>,
        u64,
        BudgetUnit,
        SystemTime,
    ) -> EffectRequest = outlined(EffectRequest::new);
    let operation = OperationId::from_bytes([0xA5; 16]);
    let before_epoch = SystemTime::UNIX_EPOCH
        .checked_sub(Duration::from_secs(1))
        .ok_or("the platform must hold one second before the epoch")?;
    let request = new_request(
        operation,
        scopes(&[]),
        u64::MAX,
        BudgetUnit::Tokens,
        before_epoch,
    );
    let operation_of: fn(&EffectRequest) -> OperationId = outlined(EffectRequest::operation);
    assert_eq!(operation_of(&request), operation);
    let required_of: fn(&EffectRequest) -> &BTreeSet<Scope> =
        outlined(EffectRequest::required_scopes);
    assert!(required_of(&request).is_empty());
    let cost_of: fn(&EffectRequest) -> u64 = outlined(EffectRequest::cost);
    assert_eq!(cost_of(&request), u64::MAX, "the maximum cost is kept");
    let unit_of: fn(&EffectRequest) -> BudgetUnit = outlined(EffectRequest::unit);
    assert_eq!(unit_of(&request), BudgetUnit::Tokens);
    let observed_of: fn(&EffectRequest) -> SystemTime = outlined(EffectRequest::observed_at);
    assert_eq!(observed_of(&request), before_epoch);

    // PinnedInputs::new and decide through it.
    let new_pinned: fn(
        EffectRequest,
        PolicySnapshot,
        BudgetSnapshot,
        CapabilitySnapshot,
        ValiditySnapshot,
    ) -> PinnedInputs = outlined(PinnedInputs::new);
    let validity = ValiditySnapshot::with_skew(instant(1_000)?, Duration::ZERO);
    let pinned = new_pinned(
        new_request(
            operation,
            scopes(&[Scope::FileRead]),
            10,
            BudgetUnit::Requests,
            instant(500)?,
        ),
        new_policy(1, scopes(&[Scope::FileRead])),
        new_budget(50, BudgetUnit::Requests),
        new_caps(scopes(&[Scope::FileRead])),
        validity,
    );
    assert_eq!(pinned.decide(), Decision::Authorized);

    // ReplayRecord::new and a replayed log over it.
    let new_record: fn(PinnedInputs, Decision) -> ReplayRecord = outlined(ReplayRecord::new);
    let record = new_record(pinned, Decision::Authorized);
    let report = replay_log(std::slice::from_ref(&record));
    assert!(report.is_bit_identical(), "a recorded decision replays");
    assert_eq!(report.divergences(), 0);
    assert_eq!(report.findings().len(), 1);
    Ok(())
}

// Source: larry#93 (a model command needs its own capability) and CWE-284
// (improper access control). `Scope::ModelCommand` must map to exactly
// `larry.model.command`, never to a sibling capability, and a policy that
// refuses it names it deterministically.
#[test]
fn sim_model_command_scope_maps_to_its_own_capability() -> Outcome {
    assert_eq!(
        capability(Scope::ModelCommand).as_str(),
        "larry.model.command",
        "a model command exercises only its own capability"
    );
    for scope in [
        Scope::FileRead,
        Scope::FileWrite,
        Scope::NetworkEgress,
        Scope::ProcessSpawn,
        Scope::ModelInvoke,
        Scope::ModelCommand,
    ] {
        let cap = capability(scope);
        let name = cap.as_str();
        assert!(
            name.starts_with("larry."),
            "{scope:?} stays inside the larry capability namespace: {name}"
        );
    }
    assert_ne!(
        capability(Scope::ModelCommand).as_str(),
        capability(Scope::ModelInvoke).as_str(),
        "a model command is not a model invocation"
    );
    // A policy that refuses only ModelCommand denies exactly it, first.
    let request = EffectRequest::new(
        OperationId::from_bytes([9; 16]),
        scopes(&[Scope::FileRead, Scope::ModelCommand]),
        1,
        BudgetUnit::Requests,
        instant(100)?,
    );
    let policy = PolicySnapshot::new(1, scopes(&[Scope::FileRead]));
    let decision = decide(
        &request,
        &policy,
        &BudgetSnapshot::new(10, BudgetUnit::Requests),
        &CapabilitySnapshot::new(scopes(&[Scope::FileRead, Scope::ModelCommand])),
        &ValiditySnapshot::new(instant(1_000)?),
    );
    assert_eq!(
        decision,
        Decision::Denied {
            reason: DenialReason::PolicyRefuses {
                scope: Scope::ModelCommand
            }
        },
        "the first refused scope in sorted order is named"
    );
    Ok(())
}

// Source: RFC 3339 pre-epoch instants and E65 (expiry is never consent). A
// display whose validity ends before the Unix epoch is refused as expired,
// and the fixed-width encoding keeps pre-epoch instants distinct instead of
// wrapping them onto the epoch.
#[test]
fn sim_display_digest_rejects_pre_epoch_expiry() -> Outcome {
    let target = EffectTarget::new(b"res", b"", b"rcpt", b"acct", b"pay", b"ad")?;
    let new_resolved: fn(EffectTarget, u64, SecurityEpoch) -> ResolvedEffect =
        outlined(ResolvedEffect::new);
    let resolved = new_resolved(target, 1, SecurityEpoch::INITIAL);
    let approval = ContentDigest::over(&[b"approval"]);
    let before_epoch = SystemTime::UNIX_EPOCH
        .checked_sub(Duration::from_secs(1))
        .ok_or("the platform must hold one second before the epoch")?;
    let binding = EffectBinding::new(resolved, approval, 3, before_epoch);
    let digest_of: fn(&EffectBinding) -> ContentDigest = outlined(EffectBinding::digest);
    // The canonical digest is stable: two displays of the same approval bind
    // the same identity, and a different approval binds a different one.
    let again = EffectBinding::new(resolved, approval, 3, before_epoch);
    assert_eq!(digest_of(&binding), digest_of(&again));
    let other = EffectBinding::new(resolved, ContentDigest::over(&[b"other"]), 3, before_epoch);
    assert_ne!(
        digest_of(&binding),
        digest_of(&other),
        "a different approval binds a different display"
    );
    assert_eq!(
        binding.verify_runtime(3, SystemTime::UNIX_EPOCH),
        Err(BindingError::Expired),
        "a pre-epoch expiry is never consent, even at the epoch"
    );
    assert_eq!(
        binding.verify_runtime(3, before_epoch),
        Err(BindingError::Expired),
        "an instant at expiry is already invalid"
    );
    // A policy revision swap after display is a changed binding, not consent.
    let current = EffectBinding::new(resolved, approval, 3, instant(1_000)?);
    assert_eq!(
        current.verify_runtime(4, instant(100)?),
        Err(BindingError::Changed {
            field: BindingField::PolicyRevision
        }),
        "a revision swap after display refuses the reach"
    );
    assert_eq!(current.verify_runtime(3, instant(100)?), Ok(()));
    Ok(())
}

/// The six bound fields the target matrix covers, in specification order.
const TARGET_FIELDS: [BindingField; 6] = [
    BindingField::Resource,
    BindingField::Symlink,
    BindingField::Recipient,
    BindingField::Account,
    BindingField::Payload,
    BindingField::Adapter,
];

// Source: E65 (canonical owner-resolved identity) and CWE-20 (an attacker
// swaps one resolved identity). `verify` compares every field in fixed
// specification order and names exactly the first swap; each field, the
// schema and the policy epoch is falsified alone.
#[test]
fn sim_resolved_effect_verify_names_each_changed_field() -> Outcome {
    let target = EffectTarget::new(b"r", b"s", b"c", b"a", b"p", b"d")?;
    let epoch = SecurityEpoch::new(11);
    let new_resolved: fn(EffectTarget, u64, SecurityEpoch) -> ResolvedEffect =
        outlined(ResolvedEffect::new);
    let base = new_resolved(target, 1, epoch);
    let target_of: fn(&ResolvedEffect) -> &EffectTarget = outlined(ResolvedEffect::target);
    assert_eq!(target_of(&base), &target);
    base.verify(&base)?;
    // Each bound field oversized past 65,536 bytes is refused by name.
    let big = vec![b'x'; 65_537];
    for (index, field) in TARGET_FIELDS.into_iter().enumerate() {
        let mut parts: [&[u8]; 6] = [b"r", b"s", b"c", b"a", b"p", b"d"];
        let Some(slot) = parts.get_mut(index) else {
            return Err("the field matrix covers every bound field".into());
        };
        *slot = &big;
        assert_eq!(
            EffectTarget::new(parts[0], parts[1], parts[2], parts[3], parts[4], parts[5]),
            Err(BindingError::TooLarge { field }),
            "an oversized field is refused by name, never hashed"
        );
    }
    // Exactly the swapped field is named, each alone.
    let changed_target = |index: usize| {
        let mut parts: [&[u8]; 6] = [b"r", b"s", b"c", b"a", b"p", b"d"];
        let slot = parts.get_mut(index).ok_or(BindingError::TooLarge {
            field: BindingField::Resource,
        })?;
        *slot = b"X";
        EffectTarget::new(parts[0], parts[1], parts[2], parts[3], parts[4], parts[5])
    };
    for (index, field) in TARGET_FIELDS.into_iter().enumerate() {
        let swapped = new_resolved(changed_target(index)?, 1, epoch);
        assert_eq!(
            base.verify(&swapped),
            Err(BindingError::Changed { field }),
            "only the swapped field is named"
        );
    }
    assert_eq!(
        base.verify(&new_resolved(target, 2, epoch)),
        Err(BindingError::Changed {
            field: BindingField::Schema
        })
    );
    assert_eq!(
        base.verify(&new_resolved(target, 1, SecurityEpoch::new(12))),
        Err(BindingError::Changed {
            field: BindingField::PolicyEpoch
        })
    );
    Ok(())
}

// Source: Cell 04 (the finite run-state algebra is closed) and CWE-436
// (interpretation conflict). Every state's display name is exact, so a log
// line, a receipt and the algebra can never disagree on what a state means.
#[test]
fn sim_run_state_display_names_every_state() {
    let names = [
        (RunState::Draft, "Draft"),
        (RunState::Admitted, "Admitted"),
        (RunState::AwaitingAuthority, "AwaitingAuthority"),
        (RunState::AwaitingDependency, "AwaitingDependency"),
        (RunState::Running, "Running"),
        (RunState::Reconciling, "Reconciling"),
        (RunState::Verifying, "Verifying"),
        (RunState::Finalizing, "Finalizing"),
        (RunState::Completed, "Completed"),
        (RunState::Conflict, "Conflict"),
        (RunState::Cancelling, "Cancelling"),
        (RunState::Failed, "Failed"),
        (RunState::RolledBack, "RolledBack"),
        (RunState::Cancelled, "Cancelled"),
        (RunState::OutcomeUnknown, "OutcomeUnknown"),
    ];
    assert_eq!(RunState::ALL.len(), names.len(), "all fifteen states");
    for (state, name) in names {
        assert_eq!(state.to_string(), name, "the display name is exact");
    }
}

// Source: Cell 05 (delegation depth at most 3), D07 (attenuated grants) and
// the authority equation. The epoch display is exact at both ends of its
// range, the budget ceiling reads back, an empty policy allows nothing, and
// a grant past the delegation bound is refused naming the bound.
#[test]
fn sim_security_epoch_display_grant_depth_and_policy_accessors() -> Outcome {
    assert_eq!(SecurityEpoch::INITIAL.to_string(), "epoch 0");
    // The last generation displays exactly: one step past the largest
    // generation the space is exhausted, never wrapped, and the fence closes
    // for good (INVARIANTS.md, the dispatch fence).
    let Some(last) = SecurityEpoch::new(u64::MAX - 1).successor() else {
        return Err("the penultimate generation has a successor".into());
    };
    assert_eq!(
        last.to_string(),
        format!("epoch {}", u64::MAX),
        "the last generation displays exactly"
    );
    assert_eq!(last.successor(), None, "no generation succeeds the last");
    assert_eq!(
        Budget::new(u64::MAX, BudgetUnit::MilliCents).limit(),
        u64::MAX
    );
    assert_eq!(
        Budget::new(40, BudgetUnit::Tokens).unit(),
        BudgetUnit::Tokens
    );
    let empty = ResourcePolicy::new(scopes(&[]));
    assert!(
        !empty.allows(Scope::FileRead),
        "an empty policy allows nothing"
    );
    assert!(empty.allowed().is_empty());
    let full = ResourcePolicy::new(scopes(&[Scope::FileRead, Scope::ModelCommand]));
    assert!(full.allows(Scope::ModelCommand));
    assert_eq!(full.allowed().len(), 2);
    // A grant past the delegation bound is refused naming the bound, however
    // it is constructed: directly and through attenuation.
    let tenant = TenantId::from_bytes([1; 16]);
    let holder = WorkerId::from_bytes([2; 16]);
    assert_eq!(
        Grant::new(
            tenant,
            holder,
            scopes(&[Scope::FileRead]),
            Budget::new(10, BudgetUnit::Tokens),
            instant(1_000)?,
            4,
        ),
        Err(AuthorityError::DelegationDepthExceeded { depth: 4, max: 3 }),
        "depth four is refused naming the bound"
    );
    let root = Grant::new(
        tenant,
        holder,
        scopes(&[Scope::FileRead]),
        Budget::new(10, BudgetUnit::Tokens),
        instant(1_000)?,
        3,
    )?;
    assert_eq!(root.delegation_depth(), 3, "depth three is the last rung");
    let request = AttenuationRequest::new(
        WorkerId::from_bytes([9; 16]),
        scopes(&[Scope::FileRead]),
        Budget::new(10, BudgetUnit::Tokens),
        instant(900)?,
    );
    assert_eq!(
        larry_contracts::authority::attenuate(&root, &full, &request),
        Err(AuthorityError::DelegationDepthExceeded { depth: 4, max: 3 }),
        "attenuation past the bound is refused the same way"
    );
    // Mismatched budget units never coerce: the refusal names both units.
    assert_eq!(
        Budget::new(10, BudgetUnit::Requests).cap_to(Budget::new(5, BudgetUnit::Tokens)),
        Err(AuthorityError::BudgetUnitMismatch {
            parent: BudgetUnit::Tokens,
            requested: BudgetUnit::Requests,
        })
    );
    // An empty attenuation mints nothing: no scope survives, no grant exists.
    let bare = Grant::new(
        tenant,
        holder,
        scopes(&[Scope::FileRead]),
        Budget::new(10, BudgetUnit::Tokens),
        instant(1_000)?,
        0,
    )?;
    assert_eq!(
        larry_contracts::authority::attenuate(&bare, &ResourcePolicy::new(scopes(&[])), &request,),
        Err(AuthorityError::EmptyAttenuation)
    );
    Ok(())
}
