//! The retry decision for an open effect (larry#68 row 5, larry#78).
//!
//! The rule that decides whether an effect may be sent again is lgwks_bot's
//! RQ-009, `lgwks_bot::kernel::retry::retry_admissible`: the budget, live authority,
//! an unchanged intent, and either proof that the effect did not happen or a
//! deduplication contract that rules out a second application. Larry keeps
//! only the strategy-to-action policy in `larry-contracts`. This module
//! supplies the facts that rule reads and owns no second copy of it.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use larry_contracts::ids::OperationId;
use larry_contracts::replay::ReplayRecord;
use lgwks_bot::kernel::journal::EffectEvidence;
use lgwks_bot::kernel::retry::{
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
            record.pinned.request.operation(),
            (
                record.pinned.request.observed_at(),
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
/// validity (E30) and is the retry deadline. Each input is a fact from its own
/// source (larry#197):
///
/// - `dispatched` is the key the last mutating reach warranted, or the one the
///   intent names at its recorded attempt after a restore, and `proposed` is the
///   digest of the intent a resend would carry, so a changed intent is refused;
/// - the deduplication contract is the owner's `window`, never the submitter's
///   strategy, and past it the late arrival is `Unspecified`;
/// - an effect no reach has sent carries the evidence that it did not happen.
///
/// `authority_live` is the caller's fence check. The rule reports every
/// conjunct it fails.
pub(super) fn decide(
    live: &LiveEffect,
    max_attempts: u8,
    at: SystemTime,
    authority_live: bool,
    window: Option<Duration>,
) -> Result<RetryDecision, RuntimeError> {
    let operation = live.intent.operation;
    let lease = span(live.admitted_at, live.valid_until);
    let elapsed = span(live.admitted_at, at);
    let dispatched = match live.dispatched {
        Some(key) => key,
        None => effect_key(&live.intent, live.dispatches)?,
    };
    let proposed = effect_key(&live.intent, live.dispatches.saturating_add(1))?.digest();
    let scope = match DedupScope::new(live.intent.tenant.to_string()) {
        Ok(scope) => scope,
        Err(_) => {
            return Err(RuntimeError::EffectIdentity {
                operation,
                field: "tenant",
            });
        }
    };
    let mut facts = RetryFacts::new(
        RetryPolicy::new(u32::from(max_attempts), Duration::ZERO, lease),
        u32::from(live.dispatches),
        lgwks_bot::error::DispatchCertainty::Unsettled.retry_class(),
        dispatched,
        proposed,
    )
    .with_elapsed(elapsed);
    if let Some(retention) = window {
        facts = facts.with_contract(DeduplicationContract::new(
            dispatched.action(),
            dispatched.digest(),
            scope,
            retention,
            LateArrival::Unspecified,
        ));
    }
    if !live.sent {
        facts = facts.with_evidence(EffectEvidence::NotApplied);
    }
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
    fn an_all_zero_operation_is_a_typed_refusal() {
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

    /// The reconciliation arm asks the estate rule, with the owner's window, and
    /// reaches the owner only when the rule permits or refuses for authority
    /// alone. The one attempt count left in `reconcile.rs` is the exit bound of
    /// `is_reconcilable`, which takes a spent effect out of reconciliation and
    /// never permits a resend (larry#197).
    #[test]
    fn larry_keeps_no_retry_rule_of_its_own() -> Result<(), Box<dyn std::error::Error>> {
        let source = include_str!("reconcile.rs");
        assert!(
            source.contains("retry::decide(") && source.contains("deduplication_window("),
            "the retry arm asks the estate rule with the owner's window"
        );
        assert!(
            source.contains("decision.permits_retry()"),
            "a resend waits on the estate rule's answer"
        );
        assert_eq!(
            source.matches("dispatches < max_attempts").count(),
            1,
            "the attempt count appears only as the exit bound"
        );
        let exit = source
            .find("fn is_reconcilable")
            .ok_or("reconcile.rs names is_reconcilable")?;
        let bound = source
            .find("dispatches < max_attempts")
            .ok_or("reconcile.rs names the exit bound")?;
        assert!(
            bound > exit,
            "the bound lives in is_reconcilable, not the retry arm"
        );
        Ok(())
    }
}

#[cfg(test)]
/// Seeded simulation of the retry decision against the attempt budget.
mod sim {
    use std::time::{Duration, SystemTime};

    use larry_contracts::authority::SecurityEpoch;
    use larry_contracts::effect::{EffectIntent, MAX_EFFECT_ATTEMPTS, ReconciliationStrategy};
    use larry_contracts::ids::{EffectId, OperationId, RunId, TenantId};
    use lgwks_bot::kernel::retry::RetryRefusal;

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

    /// Over 512 seeds, against a reference model of RQ-009: a retry is permitted
    /// exactly when the attempt budget is unspent, the lease has not run out,
    /// authority is live, the intent a resend carries is the one that was sent,
    /// and either no reach ever sent it or the owner declared a deduplication
    /// window still open (larry#197). The submitter's strategy alone never
    /// permits a resend.
    #[test]
    fn sim_retry_decision_matches_the_attempt_budget_for_every_seed()
    -> Result<(), Box<dyn std::error::Error>> {
        for seed in 0..512_u64 {
            let mut state = seed;
            let dispatches = u8::try_from(below(&mut state, 5))?;
            let lease_secs = below(&mut state, 10_000).saturating_add(1);
            let offset_secs = below(&mut state, lease_secs.saturating_mul(2));
            let authority = below(&mut state, 2) == 1;
            let sent = below(&mut state, 4) != 0;
            let window_secs = match below(&mut state, 3) {
                0 => None,
                _ => Some(below(&mut state, lease_secs.saturating_mul(2))),
            };
            let changed = below(&mut state, 5) == 0;
            let admitted_at = SystemTime::UNIX_EPOCH
                .checked_add(Duration::from_secs(1_000))
                .ok_or("fixture time overflow")?;
            let valid_until = admitted_at
                .checked_add(Duration::from_secs(lease_secs))
                .ok_or("fixture time overflow")?;
            let at = admitted_at
                .checked_add(Duration::from_secs(offset_secs))
                .ok_or("fixture time overflow")?;
            let intent = EffectIntent::new(
                EffectId::from_bytes([7; 16]),
                OperationId::from_bytes([8; 16]),
                RunId::from_bytes([9; 16]),
                TenantId::from_bytes([10; 16]),
                ReconciliationStrategy::IdempotentRetrySafe,
                SecurityEpoch::INITIAL,
            );
            // The key a reach sent: this intent's, or one another tenant's same
            // operation named, whose digest differs (a changed intent).
            let sent_intent = if changed {
                EffectIntent::new(
                    EffectId::from_bytes([7; 16]),
                    OperationId::from_bytes([8; 16]),
                    RunId::from_bytes([9; 16]),
                    TenantId::from_bytes([11; 16]),
                    ReconciliationStrategy::IdempotentRetrySafe,
                    SecurityEpoch::INITIAL,
                )
            } else {
                intent
            };
            let live = LiveEffect {
                proof: crate::gate::Proof::lapsed(),
                intent,
                sent,
                dispatched: Some(crate::fence::effect_key(&sent_intent, dispatches)?),
                dispatches,
                reconciliations: 0,
                dispatched_at_tick: 0,
                admitted_at,
                valid_until,
            };
            let window = window_secs.map(Duration::from_secs);
            let first = decide(&live, MAX_EFFECT_ATTEMPTS, at, authority, window)?;
            let second = decide(&live, MAX_EFFECT_ATTEMPTS, at, authority, window)?;
            assert_eq!(first, second, "seed {seed}: the decision is deterministic");
            let within_lease = offset_secs < lease_secs;
            let covered = !sent || window_secs.is_some_and(|secs| offset_secs < secs);
            let expected = dispatches < MAX_EFFECT_ATTEMPTS
                && within_lease
                && authority
                && !changed
                && covered;
            assert_eq!(
                first.permits_retry(),
                expected,
                "seed {seed}: permitted exactly when budget, lease, authority, an unchanged \
                 intent and the owner's window or an unsent effect all hold"
            );
            assert_eq!(
                first.refusals().contains(&RetryRefusal::IntentChanged),
                changed,
                "seed {seed}: a changed intent is named exactly when it changed"
            );
            assert_eq!(
                first.refusals().contains(&RetryRefusal::NotProvenUndone),
                !covered,
                "seed {seed}: a sent effect outside the owner's window is never proven safe"
            );
        }
        Ok(())
    }
}
