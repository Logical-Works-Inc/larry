//! Seeded owner-boundary simulations for larry#461.
//!
//! The runtime checks authority before the handoff. The owner must check the
//! same sealed proof again because its lease can lapse between those reads.

#[path = "sim_kit/seed.rs"]
mod seed;
mod sim_kit;
mod support;

use std::time::{Duration, SystemTime};

use larry_contracts::binding::{BindingError, ResolvedEffect};
use larry_contracts::effect::{EffectIntent, EffectOutcome};
use larry_contracts::ids::{OperationId, TenantId};
use larry_runtime::owner::EffectOwner;
use larry_runtime::runtime::Runtime;
use larry_runtime::{error::RuntimeError, projection::FoldProjection};
use lgwks_bot::kernel::cap::Auth;
use lgwks_bot::kernel::clock::Clock;

use seed::step;
use sim_kit::read_work;
use support::{clock, command_id, settings, submit};

type Outcome<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[derive(Debug)]
struct LapsBeforeRead {
    clock: Clock,
    sent: Vec<(TenantId, OperationId)>,
}

impl LapsBeforeRead {
    fn new(clock: Clock) -> Self {
        Self {
            clock,
            sent: Vec::new(),
        }
    }

    fn read(&self, proof: &Auth) -> Result<(), BindingError> {
        if proof.is_expired() {
            Err(BindingError::Expired)
        } else {
            Ok(())
        }
    }
}

impl EffectOwner for LapsBeforeRead {
    fn resolve(&mut self, _intent: &EffectIntent) -> Result<Option<ResolvedEffect>, BindingError> {
        Ok(None)
    }

    fn dispatch(
        &mut self,
        request: &EffectIntent,
        credential: &Auth,
        _at: SystemTime,
    ) -> Result<EffectOutcome, BindingError> {
        if self.clock.advance(Duration::from_secs(3_601)).is_err() {
            return Err(BindingError::Expired);
        }
        self.read(credential)?;
        self.sent.push((request.tenant, request.operation));
        Ok(EffectOutcome::Unknown {
            operation: request.operation,
        })
    }

    fn dispatch_bound(
        &mut self,
        request: &EffectIntent,
        credential: &Auth,
        at: SystemTime,
    ) -> Result<EffectOutcome, BindingError> {
        self.dispatch(request, credential, at)
    }

    fn reconcile(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
        EffectOutcome::Unknown {
            operation: intent.operation,
        }
    }
}

#[test]
fn sim_owner_refuses_lapsed_authority_before_send() -> Outcome {
    for seed in 0_u64..32 {
        let mut state = seed;
        let number = u8::try_from(step(&mut state, 0x9E37_79B9, 0x7F4A_7C15, 5) % 200)?;
        let clock = clock()?;
        let owner = LapsBeforeRead::new(clock.clone());
        let mut runtime: Runtime<FoldProjection, LapsBeforeRead> = Runtime::new(
            settings(
                &[larry_contracts::authority::Scope::FileRead],
                &[larry_contracts::authority::Scope::FileRead],
                100,
            )?,
            owner,
            clock,
        )?;
        let body = read_work(
            number.saturating_add(1),
            1,
            larry_contracts::effect::ReconciliationStrategy::Queryable,
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
