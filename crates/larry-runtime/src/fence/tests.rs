//! The fence on `lgwks_bot::broker` (larry#76): E71's rules, the handoff
//! re-check, the broker's bounds, and the epoch mapping Larry keeps.

use super::*;
use larry_contracts::ids::{EffectId, RunId, TenantId};

type Outcome = Result<(), Box<dyn std::error::Error>>;

fn operation(byte: u8) -> OperationId {
    OperationId::from_bytes([byte; 16])
}

fn admitted_at(operation: OperationId, epoch: SecurityEpoch) -> EffectIntent {
    EffectIntent::new(
        EffectId::from_bytes(*operation.as_bytes()),
        operation,
        RunId::from_bytes([0x11; 16]),
        TenantId::from_bytes([0x22; 16]),
        ReconciliationStrategy::Queryable,
        epoch,
    )
}

fn fence(bound: u32) -> Result<RevocationFence, FenceError> {
    RevocationFence::new(SecurityEpoch::INITIAL, bound)
}

/// The mint-time answer for `intent`.
fn check(fence: &RevocationFence, intent: &EffectIntent) -> Result<(), FenceRefusal> {
    let Ok(key) = effect_key(intent, 1) else {
        return Err(FenceRefusal::Closed);
    };
    fence.authorize(key, intent).map(drop)
}

/// Mints, hands off and counts one reach for `intent`, as `reach_owner` does.
fn reach(fence: &mut RevocationFence, intent: &EffectIntent) -> Result<(), FenceRefusal> {
    let Ok(key) = effect_key(intent, 1) else {
        return Err(FenceRefusal::Closed);
    };
    let warrant = fence.authorize(key, intent)?;
    let before = fence.offline_spent();
    fence.revalidate(&warrant, intent)?;
    assert_eq!(fence.offline_spent(), before, "revalidation counts nothing");
    fence.spend(&warrant, intent)
}

#[test]
fn e71_a_fresh_fence_at_the_admission_epoch_admits_the_reach() -> Outcome {
    let fence = fence(4)?;
    assert_eq!(
        check(&fence, &admitted_at(operation(1), SecurityEpoch::INITIAL)),
        Ok(())
    );
    assert_eq!(fence.epoch()?, SecurityEpoch::INITIAL);
    assert_eq!(fence.offline_spent()?, 0);
    assert_eq!(fence.offline_bound()?, 4);
    assert!(!fence.is_revoked(operation(1)));
    Ok(())
}

#[test]
fn e71_revocation_by_name_is_one_way_and_idempotent() -> Outcome {
    let mut fence = fence(4)?;
    let intent = admitted_at(operation(7), SecurityEpoch::INITIAL);
    assert!(fence.revoke(operation(7))?, "the first call withdraws");
    assert!(!fence.revoke(operation(7))?, "the second call is a no-op");
    assert_eq!(
        check(&fence, &intent),
        Err(FenceRefusal::Revoked {
            operation: operation(7)
        })
    );
    // A different operation is untouched: revocation is not a blanket.
    assert_eq!(
        check(&fence, &admitted_at(operation(8), SecurityEpoch::INITIAL)),
        Ok(())
    );
    assert!(!fence.is_revoked(operation(8)));
    // No unrevoke: the withdrawal survives an advance and an authentication.
    assert_eq!(fence.advance_epoch()?, SecurityEpoch::new(1));
    assert_eq!(
        fence.authenticate(SecurityEpoch::new(0))?,
        SecurityEpoch::new(1),
        "authentication never lowers the epoch, so the replay is absorbed"
    );
    assert!(fence.is_revoked(operation(7)));
    assert_eq!(fence.revoke(operation(0)), Err(FenceError::ZeroOperation));
    Ok(())
}

#[test]
fn e71_an_advanced_epoch_fences_everything_admitted_below_it() -> Outcome {
    let mut fence = fence(4)?;
    let stale = admitted_at(operation(1), SecurityEpoch::INITIAL);
    assert_eq!(fence.advance_epoch()?, SecurityEpoch::new(1));
    assert_eq!(
        check(&fence, &stale),
        Err(FenceRefusal::StaleEpoch {
            admitted: SecurityEpoch::INITIAL,
            current: SecurityEpoch::new(1)
        })
    );
    assert_eq!(
        check(&fence, &admitted_at(operation(2), SecurityEpoch::new(1))),
        Ok(())
    );
    Ok(())
}

#[test]
fn e71_authentication_raises_to_the_reported_epoch_and_absorbs_an_older_one() -> Outcome {
    let mut fence = fence(4)?;
    reach(
        &mut fence,
        &admitted_at(operation(1), SecurityEpoch::INITIAL),
    )?;
    assert_eq!(fence.offline_spent()?, 1);
    assert_eq!(
        fence.authenticate(SecurityEpoch::new(9))?,
        SecurityEpoch::new(9)
    );
    assert_eq!(fence.offline_spent()?, 0, "reaching the source refreshes");
    assert_eq!(
        check(&fence, &admitted_at(operation(1), SecurityEpoch::new(8))),
        Err(FenceRefusal::StaleEpoch {
            admitted: SecurityEpoch::new(8),
            current: SecurityEpoch::new(9)
        })
    );
    assert_eq!(
        fence.authenticate(SecurityEpoch::new(2))?,
        SecurityEpoch::new(9)
    );
    assert_eq!(fence.epoch()?, SecurityEpoch::new(9));
    Ok(())
}

/// One reach is permitted, the next refused, and only the authority source
/// being reached recharges it. The fence takes no time, so no elapsed or
/// rolled-back time can do the same.
fn assert_one_reach_then_recharge(mut fence: RevocationFence) -> Outcome {
    let at = fence.epoch()?;
    let intent = admitted_at(operation(1), at);
    reach(&mut fence, &intent)?;
    assert_eq!(
        reach(&mut fence, &intent),
        Err(FenceRefusal::OfflineExposureExhausted { bound: 1 })
    );
    assert_eq!(
        fence.offline_spent()?,
        1,
        "a refused handoff counts nothing"
    );
    assert_eq!(fence.authenticate(at)?, at);
    assert_eq!(reach(&mut fence, &intent), Ok(()));
    Ok(())
}

#[test]
fn e71_time_passing_does_not_authenticate_and_a_rolled_back_clock_is_irrelevant() -> Outcome {
    assert_one_reach_then_recharge(RevocationFence::new(SecurityEpoch::new(5), 1)?)
}

#[test]
fn e71_a_one_bound_permits_exactly_one_reach_between_authentications() -> Outcome {
    assert_one_reach_then_recharge(fence(1)?)
}

#[test]
fn e71_a_zero_bound_refuses_every_reach_even_a_freshly_authenticated_one() -> Outcome {
    let mut fence = fence(0)?;
    let intent = admitted_at(operation(1), SecurityEpoch::INITIAL);
    let exhausted = Err(FenceRefusal::OfflineExposureExhausted { bound: 0 });
    assert_eq!(check(&fence, &intent), exhausted);
    assert_eq!(
        fence.authenticate(SecurityEpoch::INITIAL)?,
        SecurityEpoch::INITIAL
    );
    assert_eq!(
        fence.offline_spent()?,
        0,
        "authentication cleared the count"
    );
    assert_eq!(
        check(&fence, &intent),
        exhausted,
        "zero is a ceiling, not a credit"
    );
    Ok(())
}

#[test]
fn e71_refusals_are_ordered_revoked_then_stale_then_offline() -> Outcome {
    let mut fence = fence(0)?;
    assert!(fence.revoke(operation(3))?);
    assert_eq!(fence.advance_epoch()?, SecurityEpoch::new(1));
    assert_eq!(
        check(&fence, &admitted_at(operation(3), SecurityEpoch::INITIAL)),
        Err(FenceRefusal::Revoked {
            operation: operation(3)
        })
    );
    assert_eq!(
        check(&fence, &admitted_at(operation(4), SecurityEpoch::INITIAL)),
        Err(FenceRefusal::StaleEpoch {
            admitted: SecurityEpoch::INITIAL,
            current: SecurityEpoch::new(1)
        })
    );
    assert_eq!(
        check(&fence, &admitted_at(operation(5), SecurityEpoch::new(1))),
        Err(FenceRefusal::OfflineExposureExhausted { bound: 0 })
    );
    Ok(())
}

/// Mints a warrant for operation 6, withdraws it with `withdraw`, and asserts
/// the handoff and the count both refuse with `expected` and spend nothing.
fn withdrawn_before_the_handoff(
    withdraw: fn(&mut RevocationFence) -> Result<(), FenceError>,
    expected: FenceRefusal,
) -> Outcome {
    let mut fence = fence(4)?;
    let intent = admitted_at(operation(6), SecurityEpoch::INITIAL);
    let key = effect_key(&intent, 1)?;
    let warrant = fence.authorize(key, &intent)?;
    withdraw(&mut fence)?;
    assert_eq!(fence.revalidate(&warrant, &intent), Err(expected));
    assert_eq!(fence.spend(&warrant, &intent), Err(expected));
    assert_eq!(
        fence.offline_spent()?,
        0,
        "a refused handoff spends nothing"
    );
    Ok(())
}

#[test]
fn a_withdrawal_between_the_mint_and_the_handoff_refuses_the_handoff() -> Outcome {
    withdrawn_before_the_handoff(
        |fence| fence.revoke(operation(6)).map(drop),
        FenceRefusal::Revoked {
            operation: operation(6),
        },
    )?;
    withdrawn_before_the_handoff(
        |fence| fence.advance_epoch().map(drop),
        FenceRefusal::StaleEpoch {
            admitted: SecurityEpoch::INITIAL,
            current: SecurityEpoch::new(1),
        },
    )
}

#[test]
fn an_epoch_the_fence_never_reached_is_never_issued_not_stale() -> Outcome {
    let fence = fence(4)?;
    assert_eq!(
        check(&fence, &admitted_at(operation(2), SecurityEpoch::new(3))),
        Err(FenceRefusal::NeverIssued {
            admitted: SecurityEpoch::new(3),
            current: SecurityEpoch::INITIAL
        })
    );
    Ok(())
}

#[test]
fn e71_epoch_exhaustion_refuses_rather_than_wrapping() -> Outcome {
    assert_eq!(
        RevocationFence::new(SecurityEpoch::new(u64::MAX), 1).map(drop),
        Err(FenceError::EpochExhausted),
        "u64::MAX has no generation"
    );
    let mut fence = RevocationFence::new(SecurityEpoch::new(u64::MAX - 1), 1)?;
    assert_eq!(fence.advance_epoch(), Err(FenceError::EpochExhausted));
    assert_eq!(fence.epoch()?, SecurityEpoch::new(u64::MAX - 1));
    Ok(())
}

#[test]
fn a_report_past_the_last_epoch_closes_the_fence_for_good() -> Outcome {
    let mut fence = fence(4)?;
    let intent = admitted_at(operation(1), SecurityEpoch::INITIAL);
    assert_eq!(
        fence.authenticate(SecurityEpoch::new(u64::MAX)),
        Err(FenceError::EpochExhausted)
    );
    assert_eq!(check(&fence, &intent), Err(FenceRefusal::Closed));
    assert_eq!(
        fence.authenticate(SecurityEpoch::INITIAL),
        Err(FenceError::Closed),
        "nothing reopens it"
    );
    assert_eq!(fence.advance_epoch(), Err(FenceError::Closed));
    assert!(fence.revoke(operation(1))?, "a late withdrawal still lands");
    Ok(())
}

#[test]
fn revocations_saturate_at_the_broker_bound_and_the_epoch_still_fences() -> Outcome {
    let mut fence = fence(4)?;
    let limit = 65_536_u128;
    for n in 1..=limit {
        assert!(fence.revoke(OperationId::from_bytes(n.to_be_bytes()))?);
    }
    let overflow = OperationId::from_bytes(limit.saturating_add(1).to_be_bytes());
    assert_eq!(
        fence.revoke(overflow),
        Err(FenceError::RevocationsFull { limit: 65_536 })
    );
    assert!(
        !fence.is_revoked(overflow),
        "a refused withdrawal is not one"
    );
    assert!(
        !fence.revoke(OperationId::from_bytes(1_u128.to_be_bytes()))?,
        "an existing withdrawal still answers when full"
    );
    let intent = admitted_at(overflow, SecurityEpoch::INITIAL);
    assert_eq!(check(&fence, &intent), Ok(()));
    assert_eq!(fence.advance_epoch()?, SecurityEpoch::new(1));
    assert!(
        matches!(check(&fence, &intent), Err(FenceRefusal::StaleEpoch { .. })),
        "the wider withdrawal still fences what the set could not hold"
    );
    Ok(())
}

#[test]
fn neighbouring_top_epochs_never_share_a_key_generation() -> Outcome {
    // The old mapping saturated, so epochs u64::MAX - 1 and u64::MAX keyed to
    // one generation and a reservation one epoch stale could pass the fence.
    let below = effect_key(
        &admitted_at(operation(9), SecurityEpoch::new(u64::MAX - 2)),
        1,
    )?;
    let top = effect_key(
        &admitted_at(operation(9), SecurityEpoch::new(u64::MAX - 1)),
        1,
    )?;
    assert_ne!(below.epoch(), top.epoch());
    assert!(matches!(
        effect_key(&admitted_at(operation(9), SecurityEpoch::new(u64::MAX)), 1),
        Err(RuntimeError::EffectIdentity {
            field: "admission_epoch",
            ..
        })
    ));
    Ok(())
}

#[test]
fn larry_keeps_no_fence_rule_of_its_own() {
    let source = include_str!("../fence.rs");
    for owned in [
        ".is_later_than(",
        "BTreeSet",
        "offline_spent:",
        ">= self.offline",
    ] {
        assert!(
            !source.contains(owned),
            "the broker owns `{owned}`; the fence keeps no copy"
        );
    }
    assert!(source.contains("broker: Broker"), "the fence is a broker");
}

#[test]
fn an_epoch_serializes_as_a_bare_number() {
    // Persisted inside a recorded intent, so the exact string is the property.
    assert_eq!(
        lgwks_std::json::to_string(&SecurityEpoch::new(7)).map_err(|error| error.to_string()),
        Ok("7".to_owned())
    );
    assert_eq!(
        lgwks_std::json::from_str::<SecurityEpoch>("7").map_err(|error| error.to_string()),
        Ok(SecurityEpoch::new(7))
    );
}
