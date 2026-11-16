//! Simulations of the bounded lanes (`sim_lanes`): the seed-ordered outputs for every lane
//! count and seed count, the lane cap that binds, the panic that is named instead of lost, and
//! the same trace hash whatever the lane count. No clock, no sleep: the work is a pure function
//! of its seed.

use std::error::Error;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::sync::atomic::{AtomicU64, Ordering};

use lgwks_std::seeded::Seeded;

use crate::sim_lanes::{LaneError, MAX_LANES, lanes, sweep, sweep_on};

/// What every test here returns.
type TestResult = Result<(), Box<dyn Error>>;

/// Seed counts that cover empty, one, fewer than the lanes, an exact multiple and a prime past them.
const COUNTS: [u64; 8] = [0, 1, 2, 7, 8, 9, 17, 101];

/// The one pure function of a seed every ordering test computes.
fn value_of(seed: u64) -> u64 {
    Seeded::from_seed(seed).next_u64()
}

/// A trace hash over `outputs`, in the order given.
fn trace_hash(outputs: &[u64]) -> u64 {
    let mut hasher = DefaultHasher::new();
    outputs.hash(&mut hasher);
    hasher.finish()
}

/// Every seed appears once, in seed order, whatever the lane count, and a seed's lane never
/// decreases as the seed grows (contiguous chunks).
#[test]
fn sim_lanes_sweep_returns_seed_order_for_every_lane_count() -> TestResult {
    for requested in 1..=9_u64 {
        for count in COUNTS {
            let pairs = sweep_on(requested, count, |lane, seed| (lane, seed))?;
            let seeds: Vec<u64> = pairs.iter().map(|&(_, seed)| seed).collect();
            assert_eq!(
                seeds,
                (0..count).collect::<Vec<u64>>(),
                "{requested} lanes, {count} seeds"
            );
            assert!(
                pairs.windows(2).all(|pair| pair[0].0 <= pair[1].0),
                "{requested} lanes, {count} seeds: lanes are contiguous chunks"
            );
            assert!(
                pairs.iter().all(|&(lane, _)| lane < requested),
                "{requested} lanes, {count} seeds: no lane past the request"
            );
        }
    }
    Ok(())
}

/// The lanes give what the serial loop gives, for seeded work, at every lane count.
#[test]
fn sim_lanes_sweep_matches_the_serial_loop_for_every_lane_count() -> TestResult {
    for count in COUNTS {
        let serial: Vec<u64> = (0..count).map(value_of).collect();
        for requested in 1..=9_u64 {
            assert_eq!(
                sweep_on(requested, count, |_lane, seed| value_of(seed))?,
                serial,
                "{requested} lanes, {count} seeds"
            );
        }
        assert_eq!(
            sweep(count, |_lane, seed| value_of(seed))?,
            serial,
            "the default lanes"
        );
    }
    let machine = lanes();
    assert!((1..=8).contains(&machine), "the machine's lanes: {machine}");
    Ok(())
}

/// A request for far more lanes than the cap runs on the cap and no more: no lane id reaches it
/// and no more than the cap run at once.
#[test]
fn sim_lanes_never_run_more_threads_than_the_lane_cap() -> TestResult {
    let live = AtomicU64::new(0);
    let peak = AtomicU64::new(0);
    let seen = sweep_on(1_000, 1_000, |lane, _seed| {
        let now = live.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        peak.fetch_max(now, Ordering::SeqCst);
        live.fetch_sub(1, Ordering::SeqCst);
        lane
    })?;
    assert_eq!(seen.len(), 1_000, "every seed ran");
    let highest = seen.iter().copied().max();
    assert_eq!(
        highest,
        MAX_LANES.checked_sub(1),
        "the cap binds: the last lane is the cap's"
    );
    let most = peak.load(Ordering::SeqCst);
    assert!(
        (1..=MAX_LANES).contains(&most),
        "{most} lanes ran at once, past the cap {MAX_LANES}"
    );
    Ok(())
}

/// The saboteur: one seed fails an assertion. The lane is named with the message, the other
/// lanes still finish, and a message carried as a fixed string survives as well.
#[test]
fn sim_lanes_sweep_names_a_panicked_lane() -> TestResult {
    let started = AtomicU64::new(0);
    let outcome = sweep_on(4, 100, |_lane, seed| {
        started.fetch_add(1, Ordering::SeqCst);
        assert_ne!(seed, 60, "saboteur seed {seed}");
        seed
    });
    match outcome {
        Err(LaneError::Panicked { lane, message }) => {
            assert_eq!(lane, 2, "seed 60 sits in the third chunk of 25");
            assert!(message.contains("saboteur seed 60"), "{message}");
        }
        other => return Err(format!("expected a panicked lane, got {other:?}").into()),
    }
    // Lanes 0, 1 and 3 ran all 75 of their seeds; lane 2 ran 50 through 60.
    assert_eq!(started.load(Ordering::SeqCst), 86, "the other lanes finish");
    let fixed = sweep_on(2, 4, |_lane, seed| {
        assert!(seed != 3, "fixed text");
        seed
    });
    assert_eq!(
        fixed,
        Err(LaneError::Panicked {
            lane: 1,
            message: String::from("fixed text")
        })
    );
    Ok(())
}

/// The same seeds hash the same at every lane count, and twice at one count.
#[test]
fn sim_lanes_same_seed_same_trace_hash_across_lane_counts() -> TestResult {
    let reference = trace_hash(&sweep_on(1, 200, |_lane, seed| value_of(seed))?);
    for requested in [1_u64, 2, 3, 8, 16] {
        let first = trace_hash(&sweep_on(requested, 200, |_lane, seed| value_of(seed))?);
        let second = trace_hash(&sweep_on(requested, 200, |_lane, seed| value_of(seed))?);
        assert_eq!(first, second, "{requested} lanes replay one trace");
        assert_eq!(first, reference, "{requested} lanes match the serial trace");
    }
    Ok(())
}

/// A sweep on no lane is refused with its variant; an empty range is an empty success.
#[test]
fn sim_lanes_refuse_zero_lanes() {
    assert_eq!(
        sweep_on(0, 5, |_lane, seed| seed),
        Err(LaneError::NoLanes),
        "no lane, no sweep"
    );
    assert_eq!(
        sweep(0, |_lane, seed| seed),
        Ok(Vec::new()),
        "nothing to run"
    );
}
