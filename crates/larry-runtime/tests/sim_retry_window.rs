//! A retry-safe effect is resent only under its owner's deduplication window (larry#197,
//! larry#182 defect 4). Source: lgwks_bot RQ-009 (`retry_admissible`): a resend of an effect
//! that may have landed needs a contract that rules out a second application. The strategy
//! is what the submitter declared. Only the owner knows whether its endpoint deduplicates.

#[path = "sim_kit/seed.rs"]
mod seed;
mod sim_kit;
mod support;

use std::time::SystemTime;

use larry_contracts::authority::{Scope, SecurityEpoch};
use larry_contracts::binding::{BindingError, ResolvedEffect};
use larry_contracts::effect::{
    EffectIntent, EffectOutcome, MAX_EFFECT_ATTEMPTS, ReconciliationStrategy,
};
use larry_runtime::FenceRefusal;
use larry_runtime::error::RuntimeError;
use larry_runtime::owner::EffectOwner;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::projection::{FoldProjection, ProjectionBackend};
use larry_runtime::runtime::Runtime;
use lgwks_bot::kernel::cap::Auth;

use seed::step;
use sim_kit::read_work;
use support::{clock, command_id, operation, settings, submit, tenant, unknown};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The scripted double with the trait's default window: an endpoint that never said it
/// refuses a second application of an operation id.
struct Undeclared(ScriptedOwner);

impl EffectOwner for Undeclared {
    fn resolve(&mut self, intent: &EffectIntent) -> Result<Option<ResolvedEffect>, BindingError> {
        self.0.resolve(intent)
    }

    fn dispatch(
        &mut self,
        request: &EffectIntent,
        credential: &Auth,
        observed_at: SystemTime,
    ) -> Result<EffectOutcome, BindingError> {
        self.0.dispatch(request, credential, observed_at)
    }

    fn reconcile(&mut self, intent: &EffectIntent, at: SystemTime) -> EffectOutcome {
        self.0.reconcile(intent, at)
    }
}

/// Offers one retry-safe `FileRead` effect for operation `n`, ticks six times, and returns
/// the runtime once every tick refused nothing.
fn drive<O: EffectOwner>(owner: O, n: u8, seed: u64) -> Outcome<Runtime<FoldProjection, O>> {
    let allowed = [Scope::FileRead];
    let mut rt: Runtime<FoldProjection, O> =
        Runtime::new(settings(&allowed, &allowed, 100)?, owner, clock()?)?;
    let body = read_work(n, 1, ReconciliationStrategy::IdempotentRetrySafe);
    rt.submit(submit(command_id(n), &body)?)?;
    for _ in 0..6 {
        let tick = rt.tick()?;
        assert!(
            tick.refused().is_empty(),
            "seed {seed}: the tick refuses nothing"
        );
    }
    assert_eq!(
        rt.projection().receipts().len(),
        0,
        "seed {seed}: an effect nobody saw land publishes no receipt"
    );
    Ok(rt)
}

/// For every seed, one retry-safe effect whose every reach is answered unknown. Under an owner
/// that declares no deduplication window it reaches the owner once and is never resent, and it
/// stays open with no receipt. Under an owner that declares one it is resent up to the attempt
/// budget. The tick refuses nothing in either case.
#[test]
fn sim_a_retry_needs_the_owners_window_or_proof_it_never_left() -> Outcome {
    for seed in 0_u64..64 {
        let mut state = seed;
        let byte = u8::try_from(step(&mut state, 0x9E37_79B9, 0x7F4A_7C15, 7) % 200)?;
        let n = byte.saturating_add(1);
        let answers = [
            unknown(operation(n)),
            unknown(operation(n)),
            unknown(operation(n)),
        ];
        let undeclared = Undeclared(ScriptedOwner::scripted(answers));
        let intent = read_work(n, 1, ReconciliationStrategy::IdempotentRetrySafe)
            .intent(tenant(1), SecurityEpoch::INITIAL);
        assert_eq!(
            undeclared.deduplication_window(&intent),
            None,
            "seed {seed}: an owner that declares nothing has no window, not a zero one"
        );
        // A reach the broker then refused to count is a tick refusal that says the owner
        // was reached, never a fenced reach that says nothing was sent.
        let unspent = RuntimeError::Unspent {
            operation: operation(n),
            refusal: FenceRefusal::Closed,
        }
        .to_string();
        assert!(
            unspent.contains("reached its owner"),
            "seed {seed}: {unspent}"
        );
        let bare = drive(undeclared, n, seed)?;
        assert_eq!(
            bare.owner().0.dispatched().len(),
            1,
            "seed {seed}: no window, so the effect that may have landed is never resent"
        );
        let windowed = drive(ScriptedOwner::scripted(answers), n, seed)?;
        assert_eq!(
            windowed.owner().dispatched().len(),
            usize::from(MAX_EFFECT_ATTEMPTS),
            "seed {seed}: under the owner's window it is resent up to the budget"
        );
    }
    Ok(())
}
