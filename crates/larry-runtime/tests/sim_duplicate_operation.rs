//! A reused operation id is refused before its run moves (larry#198, larry#182 defect 8).
//! Source: D42 and E63 (a command cannot claim what another tenant owns) and larry#371 (a
//! refusal beside a durable acceptance is itself durable). The runtime keys its live table,
//! its journal fold and the fence's revocation by operation id, so one id names one effect of
//! one tenant. A second submit that reuses the id is refused before `Admit`, `Start` or
//! `EnterReconciling` is journaled, and the refusal is recorded so a restart reads it.

#[path = "sim_kit/seed.rs"]
mod seed;
mod sim_kit;
mod support;

use larry_contracts::authority::Scope;
use larry_contracts::effect::ReconciliationStrategy;
use larry_contracts::state::RunState;
use larry_runtime::error::RuntimeError;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::payload::CommandRefusal;
use larry_runtime::projection::ProjectionBackend;
use larry_runtime::runtime::TickItem;

use seed::step;
use sim_kit::{reader, reopen};
use support::{command_id, operation, proposal, run, submit, succeeded, tenant};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// For every seed, run 1 of tenant 1 submits operation `n`, and in the same tick a second
/// command reuses `n`: from another tenant's run, from the same tenant's other run, or from
/// run 1 itself. The second command is refused with `DuplicateOperation` and recorded, its
/// run never reaches `Reconciling`, and run 1 still completes with one dispatch. A restore
/// keeps the refusal and dispatches nothing more.
#[test]
fn sim_a_reused_operation_id_is_refused_before_its_run_moves() -> Outcome {
    for seed in 0_u64..96 {
        let mut state = seed;
        let n =
            u8::try_from(step(&mut state, 0x9E37_79B9, 0x7F4A_7C15, 7) % 200)?.saturating_add(3);
        let (second_tenant, second_run) = match seed % 3 {
            0 => (tenant(2), run(2)),
            1 => (tenant(1), run(2)),
            _ => (tenant(1), run(1)),
        };
        let op = operation(n);
        let scopes = [Scope::FileRead];
        let strategy = ReconciliationStrategy::Queryable;
        let first = proposal(run(1), tenant(1), op, &scopes, 1, strategy);
        let reuse = proposal(second_run, second_tenant, op, &scopes, 1, strategy);
        let mut rt = reader(ScriptedOwner::scripted([succeeded(op)]), 100)?;
        rt.submit(submit(command_id(1), &first)?)?;
        rt.submit(submit(command_id(2), &reuse)?)?;
        let tick = rt.tick()?;
        assert!(
            tick.refused().iter().any(|&(item, ref error)| {
                item == TickItem::Command(command_id(2))
                    && matches!(*error, RuntimeError::DuplicateOperation { operation } if operation == op)
            }),
            "seed {seed}: the reuse is refused as a duplicate: {:?}",
            tick.refused()
        );
        assert_eq!(
            rt.projection().run_state(run(1)),
            Some(RunState::Completed),
            "seed {seed}: the first run's tick is whole"
        );
        assert_eq!(
            rt.owner().dispatched(),
            [op],
            "seed {seed}: one effect, one dispatch"
        );
        if second_run != run(1) {
            assert!(
                matches!(
                    rt.projection().run_state(second_run),
                    None | Some(RunState::Draft)
                ),
                "seed {seed}: the refused run never moved: {:?}",
                rt.projection().run_state(second_run)
            );
        }
        assert!(
            rt.projection()
                .refusals()
                .contains(&(command_id(2), CommandRefusal::DuplicateOperation)),
            "seed {seed}: the refusal is recorded beside the acceptance"
        );
        let mut restored = reopen(rt.into_journal(), 100)?;
        assert!(
            restored
                .projection()
                .refusals()
                .contains(&(command_id(2), CommandRefusal::DuplicateOperation)),
            "seed {seed}: a restart reads why the command never ran"
        );
        let after = restored.tick()?;
        assert!(
            after.refused().is_empty() && restored.owner().dispatched().is_empty(),
            "seed {seed}: the restored runtime reduces nothing again: {:?}",
            after.refused()
        );
    }
    Ok(())
}
