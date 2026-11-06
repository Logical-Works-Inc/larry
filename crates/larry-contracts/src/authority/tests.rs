use super::*;

fn tid() -> TenantId {
    TenantId::from_bytes([0x44; 16])
}

fn worker(byte: u8) -> WorkerId {
    WorkerId::from_bytes([byte; 16])
}

fn scopes(list: &[Scope]) -> BTreeSet<Scope> {
    list.iter().copied().collect()
}

use crate::fixture::instant as epoch_plus;

fn root_grant() -> Result<Grant, AuthorityError> {
    Grant::new(
        tid(),
        worker(1),
        scopes(&[Scope::FileRead, Scope::FileWrite, Scope::NetworkEgress]),
        Budget::new(100, BudgetUnit::Tokens),
        epoch_plus(1_000),
        0,
    )
}

fn full_policy() -> ResourcePolicy {
    ResourcePolicy::new(scopes(&[
        Scope::FileRead,
        Scope::FileWrite,
        Scope::NetworkEgress,
        Scope::ProcessSpawn,
        Scope::ModelInvoke,
    ]))
}

fn request(scopes: &[Scope], limit: u64, unit: BudgetUnit, expiry_secs: u64) -> AttenuationRequest {
    AttenuationRequest::new(
        worker(2),
        scopes.iter().copied().collect(),
        Budget::new(limit, unit),
        epoch_plus(expiry_secs),
    )
}

#[test]
fn attenuation_child_is_the_intersection_of_parent_policy_budget_expiry()
-> Result<(), AuthorityError> {
    let policy = full_policy();
    let child = attenuate(
        &root_grant()?,
        &policy,
        &request(
            &[Scope::FileRead, Scope::ProcessSpawn],
            40,
            BudgetUnit::Tokens,
            900,
        ),
    )?;
    // Scopes: requested ∩ parent ∩ policy — ProcessSpawn dies with the
    // parent, FileRead survives both.
    assert_eq!(child.scopes(), &scopes(&[Scope::FileRead]));
    assert!(child.scopes().contains(&Scope::FileRead));
    assert!(!child.scopes().contains(&Scope::ProcessSpawn));
    // Budget: min(parent, request) within one unit.
    assert_eq!(child.budget(), Budget::new(40, BudgetUnit::Tokens));
    // Expiry: the earlier of parent and request.
    assert_eq!(child.expiry(), epoch_plus(900));
    // Depth grows and stays within the bound.
    assert_eq!(child.delegation_depth(), 1);
    assert_eq!(child.holder(), worker(2));
    // Tenant is inherited, never re-declared.
    assert_eq!(child.tenant(), tid());
    Ok(())
}

#[test]
fn attenuation_caps_the_expiry_to_the_parent_when_the_request_outlives_it()
-> Result<(), AuthorityError> {
    let policy = full_policy();
    let child = attenuate(
        &root_grant()?,
        &policy,
        &request(&[Scope::FileRead], 10, BudgetUnit::Tokens, 5_000),
    )?;
    assert_eq!(
        child.expiry(),
        epoch_plus(1_000),
        "child expiry is capped by the parent"
    );
    Ok(())
}

#[test]
fn attenuation_refuses_delegation_beyond_depth_three() -> Result<(), AuthorityError> {
    let policy = full_policy();
    let level1 = attenuate(
        &root_grant()?,
        &policy,
        &request(&[Scope::FileRead], 10, BudgetUnit::Tokens, 900),
    )?;
    let level2 = attenuate(
        &level1,
        &policy,
        &request(&[Scope::FileRead], 5, BudgetUnit::Tokens, 900),
    )?;
    let level3 = attenuate(
        &level2,
        &policy,
        &request(&[Scope::FileRead], 5, BudgetUnit::Tokens, 900),
    )?;
    assert_eq!(
        level3.delegation_depth(),
        3,
        "depth 3 is still within the standing bound"
    );
    let level4 = attenuate(
        &level3,
        &policy,
        &request(&[Scope::FileRead], 5, BudgetUnit::Tokens, 900),
    );
    assert_eq!(
        level4,
        Err(AuthorityError::DelegationDepthExceeded {
            depth: 4,
            max: MAX_DELEGATION_DEPTH
        })
    );
    Ok(())
}

#[test]
fn attenuation_refuses_an_empty_intersection_instead_of_minting_empty_authority()
-> Result<(), AuthorityError> {
    let narrow_policy = ResourcePolicy::new(scopes(&[Scope::ModelInvoke]));
    let outcome = attenuate(
        &root_grant()?,
        &narrow_policy,
        &request(&[Scope::FileRead], 10, BudgetUnit::Tokens, 900),
    );
    assert_eq!(outcome, Err(AuthorityError::EmptyAttenuation));
    Ok(())
}

#[test]
fn budget_units_are_never_coerced_across_each_other() -> Result<(), AuthorityError> {
    let outcome = attenuate(
        &root_grant()?,
        &full_policy(),
        &request(&[Scope::FileRead], 10, BudgetUnit::MilliCents, 900),
    );
    assert_eq!(
        outcome,
        Err(AuthorityError::BudgetUnitMismatch {
            parent: BudgetUnit::Tokens,
            requested: BudgetUnit::MilliCents
        })
    );
    Ok(())
}

#[test]
fn cumulative_spend_is_bounded_by_the_ledger_not_the_ceiling() -> Result<(), AuthorityError> {
    let grant = root_grant()?;
    let policy = full_policy();
    // A 100-unit grant: one 60-unit effect fits, but a second 60-unit
    // effect against the ledger's remaining 40 must be refused, even
    // though each effect individually fits the ceiling. The ceiling-only
    // check admits both; the ledger-aware check bounds the sum.
    assert_eq!(
        validate_at_effect_boundary_with_ledger(
            &grant,
            Scope::FileRead,
            60,
            100,
            &policy,
            epoch_plus(500)
        ),
        Ok(())
    );
    assert_eq!(
        validate_at_effect_boundary(&grant, Scope::FileRead, 60, &policy, epoch_plus(500)),
        Ok(()),
        "the ceiling-only check is per-effect by contract and admits both effects; \
         cumulative bounding is the ledger-aware variant's job"
    );
    assert_eq!(
        validate_at_effect_boundary_with_ledger(
            &grant,
            Scope::FileRead,
            60,
            40,
            &policy,
            epoch_plus(500)
        ),
        Err(AuthorityError::BudgetExhausted {
            required: 60,
            available: 40,
            unit: grant.budget.unit,
        })
    );
    Ok(())
}

#[test]
fn e30_expired_approval_yields_a_typed_refusal_no_consent_from_timeout()
-> Result<(), AuthorityError> {
    let grant = root_grant()?;
    let policy = full_policy();
    // Before expiry the same scope and cost validate.
    assert_eq!(
        validate_at_effect_boundary(&grant, Scope::FileRead, 10, &policy, epoch_plus(999)),
        Ok(())
    );
    // At the exact expiry instant validity has ended.
    assert_eq!(
        validate_at_effect_boundary(&grant, Scope::FileRead, 10, &policy, epoch_plus(1_000)),
        Err(AuthorityError::GrantExpired {
            expiry: epoch_plus(1_000),
            now: epoch_plus(1_000)
        })
    );
    // After expiry: typed refusal, and expiry wins even over a
    // perfectly granted, policy-allowed, affordable scope.
    assert_eq!(
        validate_at_effect_boundary(&grant, Scope::FileRead, 10, &policy, epoch_plus(1_001)),
        Err(AuthorityError::GrantExpired {
            expiry: epoch_plus(1_000),
            now: epoch_plus(1_001)
        })
    );
    Ok(())
}

#[test]
fn the_boundary_revalidates_against_the_current_policy() -> Result<(), AuthorityError> {
    let grant = root_grant()?;
    // The policy has moved on since the grant was written: NetworkEgress
    // is no longer allowed even though it is still granted.
    let narrowed = ResourcePolicy::new(scopes(&[Scope::FileRead, Scope::FileWrite]));
    assert_eq!(
        validate_at_effect_boundary(&grant, Scope::NetworkEgress, 1, &narrowed, epoch_plus(500)),
        Err(AuthorityError::ScopeOutsidePolicy {
            scope: Scope::NetworkEgress
        })
    );
    Ok(())
}

#[test]
fn the_boundary_refuses_ungranted_scopes_and_overspend_with_typed_errors()
-> Result<(), AuthorityError> {
    let grant = root_grant()?;
    let policy = full_policy();
    assert_eq!(
        validate_at_effect_boundary(&grant, Scope::ProcessSpawn, 1, &policy, epoch_plus(500)),
        Err(AuthorityError::ScopeNotGranted {
            scope: Scope::ProcessSpawn
        })
    );
    assert_eq!(
        validate_at_effect_boundary(&grant, Scope::FileWrite, 101, &policy, epoch_plus(500)),
        Err(AuthorityError::BudgetExhausted {
            required: 101,
            available: 100,
            unit: BudgetUnit::Tokens
        })
    );
    // Exactly at the ceiling is still affordable.
    assert_eq!(
        validate_at_effect_boundary(&grant, Scope::FileWrite, 100, &policy, epoch_plus(500)),
        Ok(())
    );
    Ok(())
}

#[test]
fn e71_a_security_epoch_only_ever_moves_forward() -> Result<(), &'static str> {
    let start = SecurityEpoch::INITIAL;
    assert_eq!(start.value(), 0);
    let next = start
        .successor()
        .ok_or("initial epoch must have a successor")?;
    assert_eq!(next.value(), 1);
    assert!(next.is_later_than(start));
    assert!(!start.is_later_than(next));
    assert!(
        !start.is_later_than(start),
        "an epoch is not later than itself"
    );
    // There is no decrement and no subtraction: the only movement the
    // type offers goes up, which is what makes a rolled-back clock or a
    // replayed old snapshot unable to restore withdrawn authority.
    assert_eq!(start.later_of(next), next, "the later epoch wins");
    assert_eq!(next.later_of(start), next, "and an older one never wins");
    assert_eq!(next.later_of(next), next);
    Ok(())
}

#[test]
fn e71_epoch_exhaustion_is_typed_never_wrapped() {
    let last = SecurityEpoch::new(u64::MAX);
    assert_eq!(
        last.successor(),
        None,
        "a wrapped epoch would alias history"
    );
    assert_eq!(last.later_of(SecurityEpoch::new(0)), last);
}

#[test]
fn e71_a_replayed_older_epoch_is_absorbed_not_obeyed() {
    // The authority source has advanced to 9. An attacker replays the
    // device snapshot from generation 4. The fence's own value is the
    // later of the two, so the replay changes nothing.
    let current = SecurityEpoch::new(9);
    let replayed = SecurityEpoch::new(4);
    assert_eq!(current.later_of(replayed), current);
    // And the direction that does matter: a genuine advance revokes.
    let advance = SecurityEpoch::new(10);
    assert!(current.later_of(advance).is_later_than(current));
}
