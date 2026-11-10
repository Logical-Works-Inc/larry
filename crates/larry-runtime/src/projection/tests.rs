//! The projection refuses before it is poisoned (larry#79): `admit` is the
//! non-mutating twin of `apply`, and a poisoned view recovers by `rebuild`.

use larry_contracts::event::{Event, EventKind};
use larry_contracts::ids::{RunId, TenantId};
use larry_contracts::state::{RunState, TransitionEvent};

use super::{FoldProjection, ProjectionBackend};
use crate::error::RuntimeError;
use crate::journal::Journal;
use crate::payload::TransitionPayload;

type Outcome = Result<(), Box<dyn std::error::Error>>;

/// A run id that is a pure function of the seed.
fn run_of(seed: u8) -> RunId {
    RunId::from_bytes([seed; 16])
}

/// A tenant id that is a pure function of the seed.
fn tenant_of(seed: u8) -> TenantId {
    TenantId::from_bytes([seed; 16])
}

/// A transition event for `run`, numbered by the journal it will join.
fn transition_event(
    journal: &Journal,
    run: RunId,
    from: RunState,
    to: RunState,
) -> Result<Event, Box<dyn std::error::Error>> {
    let payload = TransitionPayload {
        event: TransitionEvent::Admit,
        from,
        to,
    };
    Ok(Event::new(
        tenant_of(1),
        run,
        journal.next_sequence(run)?,
        Vec::new(),
        EventKind::StateTransition,
        lgwks_std::json::to_vec(&payload)?.into_boxed_slice(),
    )?)
}

/// A refused event: `Draft` plus `Admit` is `Admitted`, never `Completed`.
fn divergent(journal: &Journal, run: RunId) -> Result<Event, Box<dyn std::error::Error>> {
    transition_event(journal, run, RunState::Draft, RunState::Completed)
}

/// An admissible event: the algebra agrees with the recorded target.
fn admissible(journal: &Journal, run: RunId) -> Result<Event, Box<dyn std::error::Error>> {
    transition_event(journal, run, RunState::Draft, RunState::Admitted)
}

/// A refused `apply` poisons the raw view, and `rebuild` from the journal,
/// which never held the refused event, recovers a usable one.
#[test]
fn a_refused_apply_poisons_and_a_rebuild_recovers() -> Outcome {
    for seed in 1_u8..9 {
        let run = run_of(seed);
        let mut journal = Journal::new(16)?;
        journal.record(admissible(&journal, run)?)?;
        let mut view = FoldProjection::new(4);
        let bad = divergent(&journal, run)?;
        assert!(matches!(
            view.apply(&bad),
            Err(RuntimeError::FoldDivergence { .. })
        ));
        let good = admissible(&journal, run_of(seed.wrapping_add(100)))?;
        assert!(
            matches!(view.apply(&good), Err(RuntimeError::ProjectionPoisoned)),
            "a refused fold leaves the raw view poisoned"
        );
        view.rebuild(&journal)?;
        assert_eq!(view.run_state(run), Some(RunState::Admitted));
        assert_eq!(view.run_count(), 1, "the refused event left no row");
        view.apply(&good)?;
        assert_eq!(view.run_count(), 2, "the rebuilt view folds again");
    }
    Ok(())
}

/// `admit` reports what `apply` would refuse and changes nothing, so the view
/// is still usable afterwards.
#[test]
fn admit_refuses_without_poisoning_the_view() -> Outcome {
    for seed in 1_u8..9 {
        let journal = Journal::new(16)?;
        let mut view = FoldProjection::new(1);
        let first = run_of(seed);
        assert!(matches!(
            view.admit(&divergent(&journal, first)?),
            Err(RuntimeError::FoldDivergence { .. })
        ));
        assert_eq!(view.run_count(), 0, "a refusal spawns no row");
        view.admit(&admissible(&journal, first)?)?;
        view.apply(&admissible(&journal, first)?)?;
        assert!(
            matches!(
                view.admit(&admissible(&journal, run_of(seed.wrapping_add(100)))?),
                Err(RuntimeError::RunLimitReached { capacity: 1 })
            ),
            "a second run is past the capacity"
        );
        assert_eq!(view.run_count(), 1);
        assert_eq!(view.run_state(first), Some(RunState::Admitted));
    }
    Ok(())
}

/// The run view is the estate's `RecordProjection` folded by `RunFold`, and
/// Larry keeps no view of its own beside it (larry#68 row 6, larry#79).
#[test]
fn larry_keeps_no_projection_of_its_own() {
    let source = include_str!("../projection.rs");
    assert!(
        source.contains("RecordProjection<Event, RunFold>"),
        "the view is the estate projection"
    );
    assert!(
        source.contains("impl RecordFold<Event> for RunFold"),
        "the rules are a fold over it"
    );
    let own_view = ["BTreeMap<RunId, ", "RunView>"].concat();
    assert!(
        !source.contains(&own_view),
        "no run table of Larry's own survives beside the estate projection"
    );
}
