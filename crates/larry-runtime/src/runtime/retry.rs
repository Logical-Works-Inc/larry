//! The retry decision for an open effect (larry#68 row 5, larry#78).
//!
//! The rule that decides whether an effect may be sent again is lgwks_bot's
//! RQ-009, `lgwks_bot::retry::retry_admissible`: the budget, live authority,
//! an unchanged intent, and either proof that the effect did not happen or a
//! deduplication contract that rules out a second application. Larry keeps
//! only the strategy-to-action policy in `larry-contracts`. This module
//! supplies the facts that rule reads and owns no second copy of it.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use larry_contracts::ids::OperationId;
use larry_contracts::replay::ReplayRecord;
use lgwks_bot::retry::{
    DedupScope, DeduplicationContract, LateArrival, RetryDecision, RetryFacts, retry_admissible,
};
use lgwks_std::retry::RetryPolicy;

use super::LiveEffect;
use crate::error::RuntimeError;
use crate::fence::effect_key;

/// The authority lease each recorded authorization pinned, keyed by operation.
///
/// An intent with no recorded authorization has no lease here, and
/// `leased` gives it an expired one, so it can never be retried: restore fails
/// closed when the journal and the reservation disagree.
pub(super) fn leases<'a>(
    records: impl IntoIterator<Item = &'a ReplayRecord>,
) -> BTreeMap<OperationId, (SystemTime, SystemTime)> {
    let mut leases = BTreeMap::new();
    for record in records {
        leases.insert(
            record.pinned.request.operation,
            (
                record.pinned.request.observed_at,
                record.pinned.validity.valid_until,
            ),
        );
    }
    leases
}

/// The lease an intent restores with: its recorded one, or an expired one.
pub(super) fn leased(
    leases: &BTreeMap<OperationId, (SystemTime, SystemTime)>,
    operation: OperationId,
) -> (SystemTime, SystemTime) {
    match leases.get(&operation).copied() {
        Some(lease) => lease,
        None => (SystemTime::UNIX_EPOCH, SystemTime::UNIX_EPOCH),
    }
}

/// Time from `from` to `to`, zero when the clock ran backwards.
fn span(from: SystemTime, to: SystemTime) -> Duration {
    match to.duration_since(from) {
        Ok(elapsed) => elapsed,
        Err(_) => Duration::ZERO,
    }
}

/// Whether an open effect may be sent again now, by lgwks_bot's RQ-009 rule.
///
/// The authority lease runs from the pinned observation to the end of
/// validity (E30). It is both the retry deadline and the window the endpoint's
/// deduplication has to cover, because Larry attests no retention beyond it.
/// `IdempotentRetrySafe` declares that the endpoint deduplicates on the stable
/// operation ID, so past the lease the contract's late arrival is
/// `Unspecified`. `authority_live` is the caller's fence check. The rule
/// reports every conjunct it fails.
pub(super) fn decide(
    live: &LiveEffect,
    max_attempts: u8,
    at: SystemTime,
    authority_live: bool,
) -> Result<RetryDecision, RuntimeError> {
    let operation = live.intent.operation;
    let lease = span(live.admitted_at, live.valid_until);
    let elapsed = span(live.admitted_at, at);
    let key = effect_key(&live.intent, live.dispatches)?;
    let scope = match DedupScope::new(live.intent.tenant.to_string()) {
        Ok(scope) => scope,
        Err(_) => {
            return Err(RuntimeError::EffectIdentity {
                operation,
                field: "tenant",
            });
        }
    };
    let contract = DeduplicationContract::new(
        key.action(),
        key.digest(),
        scope,
        lease,
        LateArrival::Unspecified,
    );
    let mut facts = RetryFacts::new(
        RetryPolicy::new(u32::from(max_attempts), Duration::ZERO, lease),
        u32::from(live.dispatches),
        lgwks_bot::DispatchCertainty::Unsettled.retry_class(),
        key,
        key.digest(),
    )
    .with_elapsed(elapsed)
    .with_contract(contract);
    if authority_live {
        facts = facts.with_live_authority();
    }
    Ok(retry_admissible(&facts))
}

#[cfg(test)]
/// Adversarial identity and wiring checks for the retry decision.
mod tests {
    use larry_contracts::authority::SecurityEpoch;
    use larry_contracts::effect::{EffectIntent, ReconciliationStrategy};
    use larry_contracts::ids::{EffectId, OperationId, RunId, TenantId};

    use crate::error::RuntimeError;
    use crate::fence::effect_key;

    /// An intent for `operation`, `run` and `tenant` under the retry-safe strategy.
    fn intent(operation: OperationId, run: RunId, tenant: TenantId) -> EffectIntent {
        EffectIntent::new(
            EffectId::from_bytes([1; 16]),
            operation,
            run,
            tenant,
            ReconciliationStrategy::IdempotentRetrySafe,
            SecurityEpoch::INITIAL,
        )
    }

    /// Each all-zero Larry identifier is refused with a typed error naming it.
    #[test]
    fn an_all_zero_operation_is_a_typed_refusal() -> Result<(), Box<dyn std::error::Error>> {
        let zero_operation = intent(
            OperationId::from_bytes([0; 16]),
            RunId::from_bytes([2; 16]),
            TenantId::from_bytes([3; 16]),
        );
        assert!(
            matches!(
                effect_key(&zero_operation, 1),
                Err(RuntimeError::EffectIdentity {
                    field: "operation",
                    ..
                })
            ),
            "an all-zero operation names no effect key"
        );
        let zero_run = intent(
            OperationId::from_bytes([4; 16]),
            RunId::from_bytes([0; 16]),
            TenantId::from_bytes([3; 16]),
        );
        assert!(
            matches!(
                effect_key(&zero_run, 1),
                Err(RuntimeError::EffectIdentity { field: "run", .. })
            ),
            "an all-zero run names no effect key"
        );
        let zero_tenant = intent(
            OperationId::from_bytes([4; 16]),
            RunId::from_bytes([2; 16]),
            TenantId::from_bytes([0; 16]),
        );
        assert!(
            matches!(
                effect_key(&zero_tenant, 1),
                Err(RuntimeError::EffectIdentity {
                    field: "tenant",
                    ..
                })
            ),
            "an all-zero tenant names no effect key"
        );
        Ok(())
    }

    /// Two tenants reusing one operation and run never share a key or digest.
    #[test]
    fn two_tenants_on_one_operation_get_distinct_effect_keys()
    -> Result<(), Box<dyn std::error::Error>> {
        let first_key = effect_key(
            &intent(
                OperationId::from_bytes([5; 16]),
                RunId::from_bytes([6; 16]),
                TenantId::from_bytes([1; 16]),
            ),
            1,
        )?;
        let second_key = effect_key(
            &intent(
                OperationId::from_bytes([5; 16]),
                RunId::from_bytes([6; 16]),
                TenantId::from_bytes([2; 16]),
            ),
            1,
        )?;
        assert_ne!(first_key, second_key, "tenants never share an effect key");
        assert_ne!(
            first_key.digest(),
            second_key.digest(),
            "tenants never share an action digest"
        );
        Ok(())
    }

    /// The reconciliation arm asks the estate rule and keeps no inline guard.
    #[test]
    fn larry_keeps_no_retry_rule_of_its_own() {
        let source = include_str!("reconcile.rs");
        assert!(
            source.contains("retry::decide("),
            "the retry arm asks the estate rule"
        );
        assert!(
            !source.contains("if live.dispatches < max_attempts =>"),
            "no inline retry guard survives beside the estate rule"
        );
    }
}

#[cfg(test)]
/// Seeded simulation of the retry decision against the attempt budget.
mod sim {
    use std::time::{Duration, SystemTime};

    use larry_contracts::authority::SecurityEpoch;
    use larry_contracts::effect::{EffectIntent, MAX_EFFECT_ATTEMPTS, ReconciliationStrategy};
    use larry_contracts::ids::{EffectId, OperationId, RunId, TenantId};

    use super::{LiveEffect, decide};

    /// One splitmix-style step over `state`, a pure function of the seed.
    fn next(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        state.wrapping_mul(0xBF58_476D_1CE4_E5B9).rotate_left(31) ^ *state
    }

    /// A draw in `0..bound`, for a non-zero bound.
    fn below(state: &mut u64, bound: u64) -> u64 {
        let draw = next(state);
        match draw.checked_rem(bound) {
            Some(value) => value,
            None => draw,
        }
    }

    /// Over 512 seeds, the decision permits a retry exactly when the attempt
    /// budget is unspent, the lease has not run out and authority is live, and
    /// matches the old `dispatches < 3` rule inside the lease.
    #[test]
    fn sim_retry_decision_matches_the_attempt_budget_for_every_seed()
    -> Result<(), Box<dyn std::error::Error>> {
        for seed in 0..512_u64 {
            let mut state = seed;
            let dispatches = u8::try_from(below(&mut state, 5))?;
            let lease_secs = below(&mut state, 10_000).saturating_add(1);
            let offset_secs = below(&mut state, lease_secs.saturating_mul(2));
            let authority = below(&mut state, 2) == 1;
            let admitted_at = SystemTime::UNIX_EPOCH
                .checked_add(Duration::from_secs(1_000))
                .ok_or("fixture time overflow")?;
            let valid_until = admitted_at
                .checked_add(Duration::from_secs(lease_secs))
                .ok_or("fixture time overflow")?;
            let at = admitted_at
                .checked_add(Duration::from_secs(offset_secs))
                .ok_or("fixture time overflow")?;
            let live = LiveEffect {
                proof: crate::gate::Proof::lapsed(),
                intent: EffectIntent::new(
                    EffectId::from_bytes([7; 16]),
                    OperationId::from_bytes([8; 16]),
                    RunId::from_bytes([9; 16]),
                    TenantId::from_bytes([10; 16]),
                    ReconciliationStrategy::IdempotentRetrySafe,
                    SecurityEpoch::INITIAL,
                ),
                sent: true,
                dispatches,
                reconciliations: 0,
                dispatched_at_tick: 0,
                admitted_at,
                valid_until,
            };
            let first = decide(&live, MAX_EFFECT_ATTEMPTS, at, authority)?;
            let second = decide(&live, MAX_EFFECT_ATTEMPTS, at, authority)?;
            assert_eq!(first, second, "seed {seed}: the decision is deterministic");
            let within_lease = offset_secs < lease_secs;
            let expected = dispatches < MAX_EFFECT_ATTEMPTS && within_lease && authority;
            assert_eq!(
                first.permits_retry(),
                expected,
                "seed {seed}: permitted exactly when budget, lease and authority all hold"
            );
            if within_lease && authority {
                assert_eq!(
                    first.permits_retry(),
                    dispatches < MAX_EFFECT_ATTEMPTS,
                    "seed {seed}: inside the lease the rule is the attempt budget"
                );
            }
        }
        Ok(())
    }
}
