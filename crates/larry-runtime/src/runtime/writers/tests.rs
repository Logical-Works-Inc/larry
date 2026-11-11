//! The write path asks the projection before it journals (larry#79): a fact
//! the fold would refuse is returned as a refusal, never written, and leaves
//! the view usable.

use std::time::{Duration, SystemTime};

use larry_contracts::authority::{BudgetUnit, Scope};
use larry_contracts::event::EventKind;
use larry_contracts::ids::{RunId, TenantId};
use larry_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};
use larry_contracts::state::{RunState, TransitionEvent};
use lgwks_bot::clock::Clock;

use crate::error::RuntimeError;
use crate::owner::fixture::ScriptedOwner;
use crate::payload::TransitionPayload;
use crate::projection::{FoldProjection, ProjectionBackend};
use crate::runtime::{Runtime, RuntimeSettings};

type Outcome = Result<(), Box<dyn std::error::Error>>;
type Rt = Runtime<FoldProjection, ScriptedOwner>;

fn settings() -> Result<RuntimeSettings, Box<dyn std::error::Error>> {
    let until = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(3_600))
        .ok_or("fixture time overflow")?;
    Ok(RuntimeSettings::with_defaults(
        PolicySnapshot::new(1, [Scope::FileRead].into_iter().collect()),
        CapabilitySnapshot::new([Scope::FileRead].into_iter().collect()),
        ValiditySnapshot::new(until),
        BudgetUnit::Requests,
        100,
    ))
}

/// A runtime whose projection holds at most `capacity` runs.
fn runtime(capacity: usize) -> Result<Rt, Box<dyn std::error::Error>> {
    let mut runtime = Runtime::new(
        settings()?,
        ScriptedOwner::new(),
        Clock::virtual_at(Duration::ZERO),
    )?;
    runtime.projection = FoldProjection::new(capacity);
    Ok(runtime)
}

/// The bytes of a `Draft` + `Admit` transition recorded as leading to `to`.
fn admit_to(to: RunState) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let payload = TransitionPayload {
        event: TransitionEvent::Admit,
        from: RunState::Draft,
        to,
    };
    Ok(lgwks_std::json::to_vec(&payload)?)
}

/// A transition the algebra disagrees with is refused before the journal sees
/// it, and the run's next valid fact still folds.
#[test]
fn a_diverging_transition_is_refused_before_the_journal_write() -> Outcome {
    for seed in 1_u8..9 {
        let (run, tenant) = (
            RunId::from_bytes([seed; 16]),
            TenantId::from_bytes([seed; 16]),
        );
        let mut runtime = runtime(4)?;
        let refused = runtime.record_event(
            tenant,
            run,
            EventKind::StateTransition,
            admit_to(RunState::Completed)?,
        );
        assert!(matches!(refused, Err(RuntimeError::FoldDivergence { .. })));
        assert_eq!(runtime.journal.len(), 0, "a refused fact is not journaled");
        assert_eq!(runtime.projection.run_count(), 0, "no ghost row");
        runtime.record_event(
            tenant,
            run,
            EventKind::StateTransition,
            admit_to(RunState::Admitted)?,
        )?;
        assert_eq!(runtime.journal.len(), 1);
        assert_eq!(runtime.projection.run_state(run), Some(RunState::Admitted));
    }
    Ok(())
}

/// A new run past the projection's capacity is refused before the journal
/// write; the runs it holds keep folding.
#[test]
fn a_run_past_the_projection_capacity_is_refused_before_the_journal_write() -> Outcome {
    for seed in 1_u8..9 {
        let (first, second) = (
            RunId::from_bytes([seed; 16]),
            RunId::from_bytes([seed.wrapping_add(100); 16]),
        );
        let tenant = TenantId::from_bytes([seed; 16]);
        let mut runtime = runtime(1)?;
        runtime.record_event(
            tenant,
            first,
            EventKind::StateTransition,
            admit_to(RunState::Admitted)?,
        )?;
        let refused = runtime.record_event(
            tenant,
            second,
            EventKind::StateTransition,
            admit_to(RunState::Admitted)?,
        );
        assert!(matches!(
            refused,
            Err(RuntimeError::RunLimitReached { capacity: 1 })
        ));
        assert_eq!(
            runtime.journal.len(),
            1,
            "the refused fact is not journaled"
        );
        assert_eq!(runtime.projection.run_count(), 1);
        assert_eq!(runtime.projection.run_state(second), None);
        let next = TransitionPayload {
            event: TransitionEvent::Start,
            from: RunState::Admitted,
            to: RunState::Running,
        };
        runtime.record_event(
            tenant,
            first,
            EventKind::StateTransition,
            lgwks_std::json::to_vec(&next)?,
        )?;
        assert_eq!(runtime.projection.run_state(first), Some(RunState::Running));
    }
    Ok(())
}
