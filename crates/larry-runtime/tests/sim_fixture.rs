//! Seeded simulations of the scripted owner double (larry#68 row 8, ADR-R04).
//!
//! The double reaches these tests only through the crate's `fixture` feature,
//! which its own dev-dependency turns on; a shipped build never carries it.
//! Each seed draws a script on `lgwks_std::seeded`, drives the double through
//! the public `EffectOwner` trait twice and refuses any divergence, so a seed
//! that fails replays exactly.

use std::collections::BTreeSet;
use std::time::SystemTime;

use larry_contracts::authority::SecurityEpoch;
use larry_contracts::effect::{EffectIntent, EffectOutcome, ReconciliationStrategy};
use larry_contracts::ids::{EffectId, OperationId, RunId, TenantId};
use larry_runtime::owner::EffectOwner;
use larry_runtime::owner::fixture::ScriptedOwner;
use lgwks_std::seeded::{DrawError, Seeded};

/// Seeds per simulation; each is driven twice.
const SEEDS: u64 = 1_024;

/// What one owner call answered: (succeeded?, operation byte).
type Seen = (bool, u8);

/// An intent for operation byte `byte`, otherwise fixed.
fn intent(byte: u8) -> EffectIntent {
    EffectIntent::new(
        EffectId::from_bytes([0x33; 16]),
        OperationId::from_bytes([byte; 16]),
        RunId::from_bytes([0x11; 16]),
        TenantId::from_bytes([0x22; 16]),
        ReconciliationStrategy::Queryable,
        SecurityEpoch::INITIAL,
    )
}

/// The script a seed draws: up to eight outcomes, each a success or an
/// unknown for a drawn operation byte.
fn script(stream: &mut Seeded) -> Result<Vec<EffectOutcome>, DrawError> {
    let length = stream.below(9)?;
    let mut outcomes = Vec::new();
    for _ in 0..length {
        let operation = OperationId::from_bytes([stream.below(256)?.to_le_bytes()[0]; 16]);
        outcomes.push(if stream.below(2)? == 0 {
            EffectOutcome::Succeeded { operation }
        } else {
            EffectOutcome::Unknown { operation }
        });
    }
    Ok(outcomes)
}

/// Folds one answer into what the test compares.
fn seen(outcome: EffectOutcome) -> Seen {
    let succeeded = matches!(outcome, EffectOutcome::Succeeded { .. });
    (succeeded, outcome.operation().as_bytes()[0])
}

/// One seeded world through `dispatch`: the answers for the script, then
/// three answers past its end.
fn dispatch_world(seed: u64) -> Result<(Vec<Seen>, [Seen; 3]), DrawError> {
    let mut stream = Seeded::from_seed(seed);
    let outcomes = script(&mut stream)?;
    let calls = outcomes.len();
    let mut owner = ScriptedOwner::scripted(outcomes);
    let asked = intent(0xee);
    let at = SystemTime::UNIX_EPOCH;
    let scripted = (0..calls)
        .map(|_| seen(owner.dispatch(&asked, at)))
        .collect();
    let past = [(); 3].map(|()| seen(owner.dispatch(&asked, at)));
    Ok((scripted, past))
}

// Source: ADR-R04 and runtime invariant 2 (an owner that cannot say what
// happened answers unknown, so no receipt is published for an effect nobody
// observed); AGENTS.md "Tests and evidence" (same seed, same trace).
#[test]
fn sim_fixture_dispatch_replays_its_script_then_answers_unknown() -> Result<(), DrawError> {
    let mut worlds = BTreeSet::new();
    for seed in 0..SEEDS {
        let world = dispatch_world(seed)?;
        assert_eq!(world, dispatch_world(seed)?, "seed {seed} must replay");
        for &(succeeded, byte) in &world.1 {
            assert!(
                !succeeded,
                "seed {seed}: an exhausted script never succeeds"
            );
            assert_eq!(byte, 0xee, "seed {seed}: unknown names the asked operation");
        }
        worlds.insert(world);
    }
    assert!(worlds.len() > 64, "distinct seeds draw distinct worlds");
    Ok(())
}

/// One seeded world through interleaved `dispatch` and `reconcile`: the
/// operation bytes each call path recorded, in order.
fn witness_world(seed: u64) -> Result<(Vec<u8>, Vec<u8>), DrawError> {
    let mut stream = Seeded::from_seed(seed);
    let mut owner = ScriptedOwner::scripted(script(&mut stream)?);
    let at = SystemTime::UNIX_EPOCH;
    for _ in 0..stream.below(17)? {
        let asked = intent(stream.below(256)?.to_le_bytes()[0]);
        let _ = if stream.below(2)? == 0 {
            owner.dispatch(&asked, at)
        } else {
            owner.reconcile(&asked, at)
        };
    }
    let bytes = |ops: &[OperationId]| ops.iter().map(|op| op.as_bytes()[0]).collect();
    Ok((bytes(owner.dispatched()), bytes(owner.reconciled())))
}

// Source: specification E63 (a status answer resolves only an effect the
// runtime actually sent): the double's witness lists are what the forged-claim
// suites read, so each call path must record exactly its own operations.
#[test]
fn sim_fixture_records_each_call_path_in_order() -> Result<(), DrawError> {
    for seed in 0..SEEDS {
        let (dispatched, reconciled) = witness_world(seed)?;
        assert_eq!(
            (dispatched.clone(), reconciled.clone()),
            witness_world(seed)?,
            "seed {seed} must replay"
        );
        let mut stream = Seeded::from_seed(seed);
        script(&mut stream)?;
        let (mut want_dispatched, mut want_reconciled) = (Vec::new(), Vec::new());
        for _ in 0..stream.below(17)? {
            let byte = stream.below(256)?.to_le_bytes()[0];
            if stream.below(2)? == 0 {
                want_dispatched.push(byte);
            } else {
                want_reconciled.push(byte);
            }
        }
        assert_eq!(dispatched, want_dispatched, "seed {seed}: dispatch witness");
        assert_eq!(
            reconciled, want_reconciled,
            "seed {seed}: reconcile witness"
        );
    }
    Ok(())
}
