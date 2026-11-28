//! The pinned expiry against the declared clock-skew bound (E30, D36, larry#74, larry#212).
//! Sources: CWE-613 (insufficient session expiration) and
//! `lgwks_bot::kernel::skew::lease_holds`, which refuses once the observer's reading reaches
//! the expiry plus the bound.

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use larry_contracts::authority::{BudgetUnit, Scope};
use larry_contracts::ids::OperationId;
use larry_contracts::replay::{
    BudgetSnapshot, CapabilitySnapshot, Decision, DenialReason, EffectRequest, PolicySnapshot,
    ValiditySnapshot, decide,
};
use lgwks_std::seeded::Seeded;

/// What the test returns.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Nanoseconds in a second.
const SECOND: u64 = 1_000_000_000;
/// Nanoseconds in 365 days.
const YEAR: u64 = 31_536_000 * SECOND;

/// The refusal an expired lease gets.
const EXPIRED: Decision = Decision::Denied {
    reason: DenialReason::AuthorityExpired,
};

/// The decision for a request that passes every other check, observed `observed` nanoseconds
/// after the epoch against a lease ending at `expiry`, under `skew`. `None` when the host
/// cannot hold one of the instants.
fn judged(expiry: u64, observed: u64, skew: Duration) -> Option<Decision> {
    let at = |nanos: u64| SystemTime::UNIX_EPOCH.checked_add(Duration::from_nanos(nanos));
    let scopes: BTreeSet<Scope> = BTreeSet::from([Scope::FileRead]);
    Some(decide(
        &EffectRequest::new(
            OperationId::from_bytes([1; 16]),
            scopes.clone(),
            1,
            BudgetUnit::Tokens,
            at(observed)?,
        ),
        &PolicySnapshot::new(1, scopes.clone()),
        &BudgetSnapshot::new(10, BudgetUnit::Tokens),
        &CapabilitySnapshot::new(scopes),
        &ValiditySnapshot::with_skew(at(expiry)?, skew),
    ))
}

/// For 4,096 seeds an observation anywhere from a year before to a year after the expiry,
/// under any skew allowance, is authorized exactly when `observed < expiry + skew`, computed
/// here in wide integers. The named edges are the allowance itself, one nanosecond inside it,
/// an approval a year stale under a one millisecond bound, and the maximum bound.
#[test]
fn sim_an_expiry_past_the_skew_bound_is_denied_for_every_seed() -> Outcome {
    let millisecond = Duration::from_millis(1);
    let late = SECOND.saturating_add(YEAR);
    assert_eq!(judged(SECOND, late, millisecond), Some(EXPIRED));
    assert_eq!(judged(SECOND, 1_001_000_000, millisecond), Some(EXPIRED));
    assert_eq!(
        judged(SECOND, 1_000_999_999, millisecond),
        Some(Decision::Authorized)
    );
    assert_eq!(
        judged(SECOND, late, Duration::MAX),
        Some(Decision::Authorized)
    );
    assert_eq!(judged(SECOND, SECOND, Duration::ZERO), Some(EXPIRED));
    let (mut held, mut lapsed) = (0_u32, 0_u32);
    let window = SECOND.saturating_mul(6);
    for seed in 0..4096 {
        let mut rng = Seeded::from_seed(seed);
        let expiry = YEAR.saturating_mul(2).saturating_add(rng.below(YEAR)?);
        let skew = Duration::from_nanos(rng.below(SECOND.saturating_mul(3))?);
        // Draws cluster around the expiry and its allowance, where the rule turns.
        let observed = match rng.below(4)? {
            0 => expiry.saturating_sub(rng.below(YEAR)?),
            1 => expiry.saturating_add(rng.below(YEAR)?),
            2 => expiry.saturating_add(rng.below(window)?),
            _ => expiry
                .saturating_sub(SECOND.saturating_mul(3))
                .saturating_add(rng.below(window)?),
        };
        let ends = u128::from(expiry).saturating_add(skew.as_nanos());
        let want = if u128::from(observed) < ends {
            held = held.saturating_add(1);
            Decision::Authorized
        } else {
            lapsed = lapsed.saturating_add(1);
            EXPIRED
        };
        assert_eq!(
            judged(expiry, observed, skew),
            Some(want),
            "seed {seed}: expiry {expiry} observed {observed} skew {skew:?}"
        );
    }
    assert!(
        held > 500 && lapsed > 500,
        "both answers were drawn: {held} held, {lapsed} lapsed"
    );
    Ok(())
}
