//! A recorded authorization replayed after one pinned input was changed (R71, D36, E55;
//! larry#576). Sources: CWE-345 (insufficient verification of data authenticity) and CWE-863
//! (incorrect authorization). Requirement: a changed input is an explicit divergence naming
//! its exact reason, never a silent pass.

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use larry_contracts::authority::{BudgetUnit, Scope};
use larry_contracts::ids::OperationId;
use larry_contracts::replay::{
    BudgetSnapshot, CapabilitySnapshot, Decision, DenialReason, EffectRequest, PinnedInputs,
    PolicySnapshot, ReplayFinding, ReplayRecord, ValiditySnapshot, replay_log,
};
use lgwks_std::seeded::Seeded;

/// What the test returns.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Seeds the simulation runs.
const SEEDS: u64 = 2048;

/// The scopes a request draws from.
const SCOPES: [Scope; 4] = [
    Scope::FileRead,
    Scope::FileWrite,
    Scope::ProcessSpawn,
    Scope::ModelInvoke,
];

/// For every seed an authorized bundle built through the public constructors replays as one
/// exact match. The same log with one pinned input changed (budget one short, another unit, a
/// capability or a policy scope withdrawn, the lease ended) replays as one divergence at that
/// record with the reason for exactly that change, while the untouched record still matches.
#[test]
fn sim_a_mutated_pinned_input_diverges_with_its_exact_reason() -> Outcome {
    let mut drawn = [0_u32; 5];
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let mut scopes = BTreeSet::new();
        scopes.insert(*SCOPES.get(rng.index(SCOPES.len())?).ok_or("scope")?);
        for scope in SCOPES {
            if rng.below(2)? == 0 {
                scopes.insert(scope);
            }
        }
        let first = *scopes.first().ok_or("one scope is always drawn")?;
        let mut rest = scopes.clone();
        rest.remove(&first);

        let revision = rng.below(1 << 40)?;
        let cost = rng.below(1000)?.saturating_add(1);
        let remaining = cost.saturating_add(rng.below(1000)?);
        let observed = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(rng.below(1 << 30)?.saturating_add(1)))
            .ok_or("observed instant")?;
        let lease = observed
            .checked_add(Duration::from_secs(rng.below(1000)?.saturating_add(1)))
            .ok_or("expiry instant")?;
        let policy = PolicySnapshot::new(revision, scopes.clone());
        assert_eq!(policy.revision(), revision, "seed {seed}");
        let pinned = PinnedInputs::new(
            EffectRequest::new(
                OperationId::from_bytes([7; 16]),
                scopes.clone(),
                cost,
                BudgetUnit::Tokens,
                observed,
            ),
            policy,
            BudgetSnapshot::new(remaining, BudgetUnit::Tokens),
            CapabilitySnapshot::new(scopes.clone()),
            ValiditySnapshot::new(lease),
        );
        let recorded = pinned.decide();
        assert_eq!(recorded, Decision::Authorized, "seed {seed}");

        let mut changed = pinned.clone();
        let kind = rng.index(drawn.len())?;
        let reason = match kind {
            0 => {
                let short = cost.saturating_sub(1);
                changed.budget = BudgetSnapshot::new(short, BudgetUnit::Tokens);
                DenialReason::BudgetExhausted {
                    required: cost,
                    remaining: short,
                }
            }
            1 => {
                changed.budget = BudgetSnapshot::new(remaining, BudgetUnit::Requests);
                DenialReason::UnitMismatch {
                    requested: BudgetUnit::Tokens,
                    budget: BudgetUnit::Requests,
                }
            }
            2 => {
                changed.capabilities = CapabilitySnapshot::new(rest);
                DenialReason::CapabilityMissing { scope: first }
            }
            3 => {
                changed.policy = PolicySnapshot::new(revision, rest);
                DenialReason::PolicyRefuses { scope: first }
            }
            _ => {
                changed.validity = ValiditySnapshot::new(observed);
                DenialReason::AuthorityExpired
            }
        };
        let seen = drawn.get_mut(kind).ok_or("kind")?;
        *seen = seen.saturating_add(1);

        let report = replay_log(&[
            ReplayRecord::new(pinned, recorded),
            ReplayRecord::new(changed, recorded),
        ]);
        let want = [
            ReplayFinding::Match {
                index: 0,
                decision: Decision::Authorized,
            },
            ReplayFinding::Diverged {
                index: 1,
                recorded: Decision::Authorized,
                rederived: Decision::Denied { reason },
            },
        ];
        assert_eq!(report.findings(), want.as_slice(), "seed {seed}");
        assert_eq!(report.divergences(), 1, "seed {seed}");
        assert!(!report.is_bit_identical(), "seed {seed}");
    }
    assert!(
        drawn.iter().all(|&count| count > 100),
        "every change was drawn: {drawn:?}"
    );
    Ok(())
}
