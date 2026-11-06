//! Deterministic E35 fault schedules over the public runtime API.

use super::{TestResult, command, drain, expected, publish, read, subject, tenant};
use larry_contracts::ids::ContentDigest;
use larry_runtime::consumer::{ConsumerLimits, CursorError, MAX_PAGE_BYTES};
use std::collections::BTreeMap;

/// Seed evolution uses explicit wrapping, independent of wall-clock scheduling.
fn advance(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    seed.rotate_right(32)
}

/// A complete catch-up/replay/overflow/restore schedule and stable trace hash.
fn simulation(initial: u64, fault: u8) -> TestResult<ContentDigest> {
    let mut random = initial;
    let mut runtime = subject(None)?;
    let scope = tenant(1);
    let foreign = tenant(2);
    let items = usize::try_from(advance(&mut random) & 7)?.saturating_add(1);
    let limits = ConsumerLimits::new(items, MAX_PAGE_BYTES, 16, 128)?;
    for identity in 1..=8 {
        publish(&mut runtime, identity, scope)?;
    }
    let mut consumer = runtime.subscribe(scope, None, limits)?;
    let mut seen = BTreeMap::new();
    for identity in 9..=28 {
        let checkpoint = consumer.cursor();
        let target = if advance(&mut random) & 1 == 0 {
            scope
        } else {
            foreign
        };
        publish(&mut runtime, identity, target)?;
        if fault == 1 && advance(&mut random) & 3 == 0 {
            runtime.submit(command(identity, target, 0)?)?;
            assert_eq!(
                runtime.tick()?.duplicates().len(),
                1,
                "duplicate publish is not a new fact"
            );
        }
        match read(&runtime, scope, &mut consumer, &mut seen) {
            Ok(_) => {}
            Err(error)
                if error.downcast_ref::<CursorError>() == Some(&CursorError::ResyncRequired) =>
            {
                assert_eq!(
                    consumer.cursor(),
                    checkpoint,
                    "overflow never skips unread history"
                );
                runtime.resync_consumer(scope, &mut consumer)?;
            }
            Err(error) => return Err(error),
        }
        if fault == 2 && advance(&mut random) & 3 == 0 {
            // Lost application acknowledgement: restart at the prior checkpoint.
            consumer = runtime.subscribe(scope, Some(checkpoint), limits)?;
        }
        if fault == 3 && advance(&mut random) & 3 == 0 {
            let saved = consumer.cursor();
            runtime = subject(Some(runtime.into_journal()))?;
            let encoded = lgwks_std::json::to_vec(&saved)?;
            consumer =
                runtime.subscribe(scope, Some(lgwks_std::json::from_slice(&encoded)?), limits)?;
        }
    }
    if fault == 4 {
        let saved = consumer.cursor();
        for identity in 29..=45 {
            publish(&mut runtime, identity, scope)?;
        }
        assert!(
            matches!(
                runtime.consumer_page(scope, &mut consumer),
                Err(CursorError::ResyncRequired)
            ),
            "overflow must disconnect"
        );
        assert!(
            matches!(
                runtime.consumer_page(scope, &mut consumer),
                Err(CursorError::ResyncRequired)
            ),
            "overflow stays explicit"
        );
        assert_eq!(consumer.cursor(), saved, "overflow preserves checkpoint");
        runtime.resync_consumer(scope, &mut consumer)?;
    }
    runtime.resync_consumer(scope, &mut consumer)?;
    drain(&runtime, scope, &mut consumer, &mut seen)?;
    assert_eq!(
        seen,
        expected(&runtime, scope),
        "seed {initial}: no silent gaps or tenant contamination"
    );
    let replay = subject(Some(runtime.into_journal()))?;
    let mut resumed = replay.subscribe(scope, Some(consumer.cursor()), limits)?;
    assert!(
        replay
            .consumer_page(scope, &mut resumed)?
            .records()
            .is_empty(),
        "checkpoint survives index reconstruction"
    );
    let bytes = lgwks_std::json::to_vec(&resumed.cursor())?;
    Ok(ContentDigest::over(&[&bytes]))
}

/// Each run repeats exact seeds and compares traces, not a success boolean.
fn sweep(start: u64, fault: u8) -> TestResult {
    for seed in start..start.saturating_add(1_000) {
        assert_eq!(
            simulation(seed, fault)?,
            simulation(seed, fault)?,
            "seed {seed}, fault {fault}: replay hash"
        );
    }
    Ok(())
}

/// Publishing during catch-up has no missing scoped sequence.
#[test]
fn sim_e35_publish_during_catchup() -> TestResult {
    sweep(0, 0)
}
/// Duplicate publications never manufacture consumer facts.
#[test]
fn sim_e35_duplicate_publish() -> TestResult {
    sweep(1_000, 1)
}
/// Lost acknowledgements replay identically and deduplicate safely.
#[test]
fn sim_e35_lost_page_acknowledgement() -> TestResult {
    sweep(2_000, 2)
}
/// Reconstructed runtime/index preserves checkpoint semantics.
#[test]
fn sim_e35_restore_while_catching_up() -> TestResult {
    sweep(3_000, 3)
}
/// Overflow disconnects explicitly and resync preserves every unread fact.
#[test]
fn sim_e35_overflow_resync() -> TestResult {
    sweep(4_000, 4)
}
