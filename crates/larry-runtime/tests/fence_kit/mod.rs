//! Fixtures shared by the E71 fence falsifiers (`fence.rs`, `fence_recovery.rs`): a clock the test moves by
//! hand, fence settings and the work items every probe submits.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use larry_contracts::authority::{Scope, SecurityEpoch};
use larry_contracts::command::Command;
use larry_contracts::effect::{EffectOutcome, ReconciliationStrategy};
use larry_contracts::ids::OperationId;
use larry_contracts::replay::ValiditySnapshot;
use larry_runtime::clock::{Clock, FixedClock};
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::projection::{EffectPhase, FoldProjection, ProjectionBackend};
use larry_runtime::runtime::{Runtime, RuntimeSettings};

use crate::support::{
    TestRuntime, command_id, epoch, operation, proposal, run, settings, submit, tenant,
};

/// A clock the test moves by hand, so that "time passed" and "the clock ran
/// backwards" are asserted as input changes instead of waited for.
#[derive(Clone)]
pub struct ManualClock(Rc<Cell<SystemTime>>);

impl ManualClock {
    pub fn new(at: SystemTime) -> Self {
        Self(Rc::new(Cell::new(at)))
    }

    pub fn set(&self, at: SystemTime) {
        self.0.set(at);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> SystemTime {
        self.0.get()
    }
}

/// The runtime under test with a clock the test controls.
pub type ManualRuntime = Runtime<FoldProjection, ScriptedOwner, ManualClock>;

/// Settings whose fence permits `bound` mutating reaches between
/// authentications and stands at `at_epoch`.
pub fn settings_at(
    at_epoch: SecurityEpoch,
    bound: u32,
) -> Result<RuntimeSettings, Box<dyn std::error::Error>> {
    let mut base = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
    base.security_epoch = at_epoch;
    base.offline_dispatch_bound = bound;
    Ok(base)
}

/// [`settings_at`] with authority validity far enough out that the E30 expiry
/// check cannot confound a probe that deliberately moves the clock by a year.
pub fn settings_that_outlive_a_year(
    at_epoch: SecurityEpoch,
    bound: u32,
) -> Result<RuntimeSettings, Box<dyn std::error::Error>> {
    let mut base = settings_at(at_epoch, bound)?;
    base.validity = ValiditySnapshot::new(
        epoch()?
            .checked_add(Duration::from_secs(10 * 365 * 24 * 3_600))
            .ok_or("fixture validity overflow")?,
    );
    Ok(base)
}

/// A `Submit` for one file-read effect, in `run_byte`'s run.
///
/// Each run admits one effect and walks its own lifecycle, so a test that
/// needs several effects open at once gives each its own run.
///
/// # Errors
///
/// Returns the contract's ordering error for an over-long causal-parent list,
/// which cannot happen here but is not silently discarded.
pub fn work_in(
    run_byte: u8,
    id: u8,
    op: OperationId,
    strategy: ReconciliationStrategy,
) -> Result<Command, Box<dyn std::error::Error>> {
    let body = proposal(
        run(run_byte),
        tenant(run_byte),
        op,
        &[Scope::FileRead],
        5,
        strategy,
    );
    submit(command_id(id), &body)
}

/// [`work_in`] for run 1.
///
/// # Errors
///
/// As [`work_in`].
pub fn work(
    id: u8,
    op: OperationId,
    strategy: ReconciliationStrategy,
) -> Result<Command, Box<dyn std::error::Error>> {
    work_in(1, id, op, strategy)
}

/// A retry-safe effect with an ambiguous outcome stays open, which is the state
/// the retry falsifiers below start from.
///
/// Returns the runtime and the journal-independent fact that the effect is
/// open, so a caller cannot accidentally assert on a resolved effect.
pub fn open_retry_safe_effect(
    bound: u32,
) -> Result<(TestRuntime, OperationId), Box<dyn std::error::Error>> {
    let mut rt = runtime_at(SecurityEpoch::INITIAL, bound, ScriptedOwner::new())?;
    rt.submit(work(
        1,
        operation(1),
        ReconciliationStrategy::IdempotentRetrySafe,
    )?)?;
    let tick = rt.tick()?;
    assert_eq!(
        tick.authorized(),
        &[operation(1)],
        "the fixture must reserve the intended operation"
    );
    assert_eq!(
        rt.projection().effect_phase(operation(1)),
        Some(EffectPhase::Observed {
            outcome: EffectOutcome::Unknown {
                operation: operation(1)
            }
        }),
        "an ambiguous outcome under a retry-safe strategy must stay open"
    );
    Ok((rt, operation(1)))
}

/// A runtime over `owner` whose fence stands at `at_epoch` and permits `bound` reaches.
pub fn runtime_at(
    at_epoch: SecurityEpoch,
    bound: u32,
    owner: ScriptedOwner,
) -> Result<TestRuntime, Box<dyn std::error::Error>> {
    Ok(Runtime::new(
        settings_at(at_epoch, bound)?,
        owner,
        FixedClock::new(epoch()?),
    )?)
}

/// Every fixture here builds what it names, in both test binaries that share it.
#[test]
fn fence_kit_fixtures_build_what_they_name() -> Result<(), Box<dyn std::error::Error>> {
    let clock = ManualClock::new(epoch()?);
    let later = epoch()?
        .checked_add(Duration::from_secs(60))
        .ok_or("fixture time overflow")?;
    clock.set(later);
    assert_eq!(clock.now(), later);
    let manual = ManualRuntime::new(
        settings_that_outlive_a_year(SecurityEpoch::new(2), 3)?,
        ScriptedOwner::new(),
        clock,
    )?;
    assert_eq!(manual.fence().epoch(), SecurityEpoch::new(2));
    assert_eq!(manual.fence().offline_bound(), 3);
    assert_eq!(
        work_in(2, 2, operation(2), ReconciliationStrategy::Queryable)?.id(),
        command_id(2)
    );
    let (open, op) = open_retry_safe_effect(64)?;
    assert_eq!(op, operation(1));
    assert_eq!(open.owner().dispatched(), &[operation(1)]);
    assert_eq!(
        runtime_at(SecurityEpoch::new(5), 0, ScriptedOwner::new())?
            .fence()
            .epoch(),
        SecurityEpoch::new(5)
    );
    Ok(())
}
