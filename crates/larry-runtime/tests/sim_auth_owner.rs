//! Seeded owner-boundary simulations for larry#461 and larry#77.
//!
//! The runtime checks authority before the handoff. The owner must check the
//! same sealed proof again because its lease can lapse between those reads, and
//! the runtime must read it again at a retry because the retry rule decided on
//! an earlier instant.

#[path = "sim_kit/seed.rs"]
mod seed;
mod sim_kit;
mod support;

use std::time::{Duration, SystemTime};

use larry_contracts::authority::Scope;
use larry_contracts::binding::{BindingError, ResolvedEffect};
use larry_contracts::effect::{EffectIntent, EffectOutcome, ReconciliationStrategy};
use larry_contracts::ids::{OperationId, TenantId};
use larry_contracts::replay::DenialReason;
use larry_runtime::owner::EffectOwner;
use larry_runtime::runtime::Runtime;
use larry_runtime::{error::RuntimeError, projection::FoldProjection};
use lgwks_bot::kernel::cap::Auth;
use lgwks_bot::kernel::cap::Cap;
use lgwks_bot::kernel::clock::Clock;

use seed::step;
use sim_kit::read_work;
use support::{clock, command_id, settings, submit};

type Outcome<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// An owner that moves the shared clock at the two moments a lease can end behind
/// the runtime's back.
#[derive(Debug)]
struct Laps {
    clock: Clock,
    /// Passes inside a send, before the owner reads the proof.
    before_read: Duration,
    /// Passes while the owner answers for its deduplication window: the runtime
    /// asks after the tick has read its instant and before it reaches.
    while_asked: Duration,
    sent: Vec<(TenantId, OperationId)>,
}

impl Laps {
    fn new(clock: Clock, before_read: Duration, while_asked: Duration) -> Self {
        Self {
            clock,
            before_read,
            while_asked,
            sent: Vec::new(),
        }
    }

    /// Reads the proof after `before_read` has passed; a lapsed proof records
    /// nothing as sent.
    fn send(
        &mut self,
        request: &EffectIntent,
        proof: &Auth,
        required: &[Cap],
    ) -> Result<EffectOutcome, BindingError> {
        if self.clock.advance(self.before_read).is_err() {
            return Err(BindingError::Expired);
        }
        larry_runtime::owner::validate_proof(proof, required)?;
        self.sent.push((request.tenant, request.operation));
        Ok(EffectOutcome::Unknown {
            operation: request.operation,
        })
    }
}

impl EffectOwner for Laps {
    fn resolve(&mut self, _intent: &EffectIntent) -> Result<Option<ResolvedEffect>, BindingError> {
        Ok(None)
    }

    fn dispatch(
        &mut self,
        intent: &EffectIntent,
        credential: &Auth,
        required: &[Cap],
        _at: SystemTime,
    ) -> Result<EffectOutcome, BindingError> {
        self.send(intent, credential, required)
    }

    fn dispatch_bound(
        &mut self,
        request: &EffectIntent,
        credential: &Auth,
        required: &[Cap],
        _at: SystemTime,
    ) -> Result<EffectOutcome, BindingError> {
        self.send(request, credential, required)
    }

    fn reconcile(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
        EffectOutcome::Unknown {
            operation: intent.operation,
        }
    }

    fn deduplication_window(&self, _intent: &EffectIntent) -> Option<Duration> {
        if self.clock.advance(self.while_asked).is_err() {
            return None;
        }
        Some(Duration::MAX)
    }
}

/// A runtime over a [`Laps`] owner sharing its virtual clock.
fn lapsing(before_read: Duration, while_asked: Duration) -> Outcome<Runtime<FoldProjection, Laps>> {
    let clock = clock()?;
    let owner = Laps::new(clock.clone(), before_read, while_asked);
    Ok(Runtime::new(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        owner,
        clock,
    )?)
}

#[test]
fn sim_owner_refuses_lapsed_authority_before_send() -> Outcome {
    for seed in 0_u64..32 {
        let mut state = seed;
        let number = u8::try_from(step(&mut state, 0x9E37_79B9, 0x7F4A_7C15, 5) % 200)?;
        let mut runtime = lapsing(Duration::from_secs(3_601), Duration::ZERO)?;
        let body = read_work(
            number.saturating_add(1),
            1,
            ReconciliationStrategy::Queryable,
        );
        runtime.submit(submit(command_id(number.saturating_add(1)), &body)?)?;
        let report = runtime.tick()?;
        let refusal = report.refused().first().ok_or("owner refusal missing")?;
        assert!(
            matches!(refusal.1, RuntimeError::Binding(BindingError::Expired)),
            "seed {seed}: owner lapsed proof must be typed: {}",
            refusal.1
        );
        assert!(
            runtime.owner().sent.is_empty(),
            "seed {seed}: lapsed proof must record no sent effect"
        );
    }
    Ok(())
}

// Source: CWE-367, CWE-613 and larry#77 (a grant that lapsed since admission is not spent).
// The retry rule decides on the tick's instant, so it permits the resend; the proof is read
// again at the reach, on the live clock. A lease that ended in between is denied as expired
// and the owner is not sent to again. The same tick inside the lease resends once.
#[test]
fn sim_a_lease_that_ends_after_the_retry_decision_is_denied_at_the_reach() -> Outcome {
    for seed in 0_u64..32 {
        let mut state = seed;
        let drawn = step(&mut state, 0x9E37_79B9, 0x7F4A_7C15, 5);
        let number = u8::try_from(drawn % 200)?.saturating_add(1);
        let late = Duration::from_secs(3_601_u64.saturating_add(drawn % 97));
        let early = Duration::from_secs(3_599_u64.saturating_sub(drawn % 97));
        for (lapse, resends) in [(early, true), (late, false)] {
            let case = format!("seed {seed} lapse {lapse:?}");
            let mut runtime = lapsing(Duration::ZERO, lapse)?;
            let body = read_work(number, 1, ReconciliationStrategy::IdempotentRetrySafe);
            let operation = body.operation();
            runtime.submit(submit(command_id(number), &body)?)?;
            runtime.tick()?;
            let first = runtime.owner().sent.clone();
            assert_eq!(first, [(body.tenant(), operation)], "{case}: sent once");

            let report = runtime.tick()?;
            let expired: &[_] = if resends {
                &[]
            } else {
                &[(operation, DenialReason::AuthorityExpired)]
            };
            assert_eq!(report.denied(), expired, "{case}: denied");
            assert_eq!(
                runtime.owner().sent.len(),
                if resends { 2 } else { 1 },
                "{case}: a lapsed proof reaches no owner"
            );
            assert!(report.fenced().is_empty(), "{case}: not a fence refusal");
            assert!(report.refused().is_empty(), "{case}: no refusal");
        }
    }
    Ok(())
}
