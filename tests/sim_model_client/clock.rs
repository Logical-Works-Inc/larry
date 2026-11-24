//! Seeded simulations of the model client's retry budget on an injected clock (larry#436, SD-4).
//!
//! Requirement proved: the retry deadline is read from the clock the caller injected, never from
//! the wall, so one seed on a virtual clock replays one attempt count, one spent time and one
//! trace hash on any host, loaded or idle. Source: RFC 9110 15.6.4 (503 Service Unavailable) and
//! the estate's `RetryPolicy` (`lgwks_std::retry`): attempts, doubling backoff and a total
//! deadline the caller measures on its own clock. The oracle is that policy's own rule worked
//! out beside the client, never the client's output.

use std::error::Error;
use std::io;
use std::num::TryFromIntError;
use std::time::Duration;

use larry_demo::model::{Completion, Message, ModelClient, ModelConfig, ModelError, Role};
use lgwks_bot::kernel::clock::Clock;
use lgwks_std::hash::Hasher;
use lgwks_std::retry::RetryPolicy;
use lgwks_std::seeded::{DrawError, Seeded};

use crate::mock_model::{self, Config, Fault};

/// What every test here returns.
type TestResult = Result<(), Box<dyn Error>>;

/// What one run against the refusing mock observed: the client's outcome and the requests served.
type Refused = (Result<Completion, ModelError>, u64);

/// Why one simulation could not be driven to its assertions.
#[derive(Debug, lgwks_macros::Error)]
enum ClockSimError {
    /// The seeded stream refused a draw.
    #[error("seeded draw refused: {0}")]
    Draw(#[from] DrawError),
    /// A drawn number does not fit the type its use needs.
    #[error("drawn number out of range: {0}")]
    Range(#[from] TryFromIntError),
    /// The mock endpoint could not start.
    #[error("the mock endpoint failed: {0}")]
    Mock(#[from] io::Error),
    /// The drawn settings were refused as a config.
    #[error("the drawn config was refused: {0}")]
    Config(#[from] ModelError),
}

/// Seeds each simulation sweeps.
const SEEDS: u64 = 64;

/// Most attempts a policy may carry: the config's own cap, and the length of the refusal script.
const MAX_ATTEMPTS: u64 = 16;

/// One drawn policy, as the `LARRY_MODEL_*` knobs carry it.
struct Drawn {
    /// Tries including the first.
    attempts: u32,
    /// Backoff after the first failure, in milliseconds.
    backoff_ms: u64,
    /// Total budget across every try, in milliseconds.
    deadline_ms: u64,
}

impl Drawn {
    /// The estate policy these knobs build, the oracle's own copy of the rule.
    fn policy(&self) -> RetryPolicy {
        RetryPolicy::new(
            self.attempts,
            Duration::from_millis(self.backoff_ms),
            Duration::from_millis(self.deadline_ms),
        )
    }

    /// The config for a gateway at `base` under these knobs; a timeout is never the stop.
    fn config(&self, base: &str) -> Result<ModelConfig, ModelError> {
        let table = [
            ("LARRY_MODEL_URL", base.to_owned()),
            ("LARRY_MODEL", String::from("mock-model")),
            ("LARRY_MODEL_KEY", String::from("test-key")),
            ("LARRY_MODEL_TIMEOUT_MS", String::from("5000")),
            ("LARRY_MODEL_ATTEMPTS", self.attempts.to_string()),
            ("LARRY_MODEL_BACKOFF_MS", self.backoff_ms.to_string()),
            ("LARRY_MODEL_DEADLINE_MS", self.deadline_ms.to_string()),
        ];
        ModelConfig::from_env(move |name| {
            table
                .iter()
                .find(|entry| entry.0 == name)
                .map(|entry| entry.1.clone())
        })
    }
}

/// A policy drawn from `seeded`: one to sixteen tries, up to 4 s of backoff, a 1 ms to 30 s budget.
fn drawn(seeded: &mut Seeded) -> Result<Drawn, ClockSimError> {
    let attempts = u32::try_from(seeded.below(MAX_ATTEMPTS)?)?.saturating_add(1);
    let backoff_ms = seeded.below(4_000)?;
    let deadline_ms = seeded.below(30_000)?.saturating_add(1);
    Ok(Drawn {
        attempts,
        backoff_ms,
        deadline_ms,
    })
}

/// The tries sent and the virtual time spent against a gateway that always refuses, from the
/// policy alone: a try fails, the rule is asked with the time spent, and the wait is the backoff
/// cut to what the deadline leaves. It ends: `should_retry` refuses at the attempt count.
fn reference(policy: &RetryPolicy) -> (u32, Duration) {
    let mut sent = 0_u32;
    let mut spent = Duration::ZERO;
    loop {
        sent = sent.saturating_add(1);
        if !policy.should_retry(sent, spent) {
            return (sent, spent);
        }
        let remaining = policy.deadline.saturating_sub(spent);
        let wait = policy.delay(sent.saturating_sub(1), 0).min(remaining);
        spent = spent.saturating_add(wait);
    }
}

/// A mock whose first sixteen requests all answer 503, so no policy outlasts the script.
fn refusing(seed: u64) -> Config {
    Config {
        script: vec![Some(Fault::ServerError { status: 503 }); 16],
        ..Config::seeded(seed)
    }
}

/// One run of `draw` against the refusing mock, each wait the client asks for passed to `pause`.
fn run_against_503(
    seed: u64,
    draw: &Drawn,
    clock: &Clock,
    pause: impl Fn(Duration) -> bool + Send + Sync + 'static,
) -> Result<Refused, ClockSimError> {
    mock_model::run(refusing(seed), |handle| {
        let base = format!("http://{}/v1", handle.addr());
        let client = ModelClient::new(draw.config(&base)?, clock.clone(), pause);
        let outcome = client.complete(&[Message::new(Role::User, "refused")]);
        Ok::<_, ClockSimError>((outcome, handle.served()))
    })
}

/// BLAKE3 of a trace, as 64 hex characters: one seed must give one trace hash.
fn trace_hash(trace: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(trace.as_bytes());
    hasher.finalize().to_hex()
}

/// One seed's run on a virtual clock the pause advances by each wait it is given, asserted
/// against the policy's own rule and returned as a trace.
fn budget_trace(seed: u64) -> Result<String, ClockSimError> {
    let mut seeded = Seeded::from_seed(seed.wrapping_add(11 << 40));
    let draw = drawn(&mut seeded)?;
    let (sent, spent) = reference(&draw.policy());
    let clock = Clock::virtual_at(Duration::ZERO);
    let driver = clock.clone();
    let (outcome, served) = run_against_503(seed, &draw, &clock, move |wait| {
        driver.advance(wait).is_ok()
    })?;
    assert!(
        matches!(outcome, Err(ModelError::Server { status: 503, .. })),
        "seed {seed}: the last refusal is the answer, got {outcome:?}"
    );
    assert_eq!(
        served,
        u64::from(sent),
        "seed {seed}: {} tries, {} ms backoff, {} ms budget stop where the rule stops",
        draw.attempts,
        draw.backoff_ms,
        draw.deadline_ms
    );
    assert_eq!(
        clock.now(),
        spent,
        "seed {seed}: the virtual time spent is the waits the rule asked for, no more"
    );
    Ok(format!(
        "attempts={} backoff_ms={} deadline_ms={} served={served} spent_ns={}",
        draw.attempts,
        draw.backoff_ms,
        draw.deadline_ms,
        clock.now().as_nanos()
    ))
}

/// The retry budget stops on the injected clock: for every seed the client sends exactly the
/// tries the policy allows when each wait moves a virtual clock, spends exactly the waits the
/// rule asked for, and replays one trace hash. A client that kept the wall clock would stop on
/// real elapsed time, which a virtual run never spends (larry#436).
#[test]
fn sim_the_retry_budget_stops_on_the_injected_clock() -> TestResult {
    for seed in 0..SEEDS {
        let first = trace_hash(&budget_trace(seed)?);
        let second = trace_hash(&budget_trace(seed)?);
        assert_eq!(first, second, "seed {seed}: one seed gives one trace hash");
    }
    Ok(())
}

/// A clock nobody moves never spends a budget: with a 1 ms deadline and a pause that waits for
/// nothing, every seed still sends all its tries, though real time passes between them. This is
/// the discriminating case: a budget read from the wall would be spent by the first round trip.
#[test]
fn sim_a_clock_that_never_moves_never_spends_the_deadline() -> TestResult {
    for seed in 0..SEEDS {
        let mut seeded = Seeded::from_seed(seed.wrapping_add(13 << 40));
        // Two to sixteen tries: one try never retries, so it proves nothing about the budget.
        let attempts = u32::try_from(seeded.below(15)?)?.saturating_add(2);
        let draw = Drawn {
            attempts,
            backoff_ms: 0,
            deadline_ms: 1,
        };
        let clock = Clock::virtual_at(Duration::ZERO);
        let (outcome, served) = run_against_503(seed, &draw, &clock, |_wait| true)?;
        assert!(
            matches!(outcome, Err(ModelError::Server { status: 503, .. })),
            "seed {seed}: the last refusal is the answer, got {outcome:?}"
        );
        assert_eq!(
            served,
            u64::from(attempts),
            "seed {seed}: a frozen clock leaves the 1 ms budget whole for all {attempts} tries"
        );
        assert_eq!(clock.now(), Duration::ZERO, "seed {seed}: nothing moved it");
    }
    Ok(())
}
