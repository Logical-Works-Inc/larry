//! The owner boundary the executor presents: the task body is the one external
//! effect. This is the runtime side of the admission gate's second job —
//! `super` admits the run and seals the report, while this module is the owner
//! the bound reach calls: the resolved target identity, the tenant check and
//! the one gated body, run at most once. An unbound dispatch or a
//! reconciliation reports unknown, a fact the proposal's `RequiresHuman`
//! strategy records as such rather than a success it never observed.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::SystemTime;

use larry_contracts::authority::SecurityEpoch;
use larry_contracts::binding::{BindingError, EffectTarget, ResolvedEffect};
use larry_contracts::effect::{EffectIntent, EffectOutcome};
use larry_contracts::ids::TenantId;
use larry_runtime::owner::{EffectOwner, validate_proof};
use lgwks_bot::kernel::cap::{Auth, Cap};

use super::super::{ExecError, Report};

/// The schema version of the executor's own adapter contract, which a resolved
/// effect records.
const ADAPTER_SCHEMA: u64 = 1;

/// The owner boundary the executor presents: the task body is the one external
/// effect. `run` is the gated body, held until the bound reach runs it at most
/// once; its outcome is the effect's outcome. The executor cannot be queried
/// after an unbound dispatch, so both unbound dispatch and reconciliation
/// report unknown — a fact the proposal's `RequiresHuman` strategy records as
/// such rather than a success it never observed.
pub(super) struct TaskOwner<'a, F>
where
    F: FnOnce() -> Result<Report, ExecError>,
{
    /// The repository path the task binds to.
    pub(super) repo: &'a std::path::Path,
    /// The tenant the run belongs to; one state directory is one tenant.
    pub(super) tenant: TenantId,
    /// The security generation the runtime's fence starts at: the epoch a
    /// resolved effect names, read from the runtime's own settings rather than
    /// a constant (larry#212).
    pub(super) epoch: SecurityEpoch,
    /// The task document bytes the effect's payload binds to.
    pub(super) task_bytes: &'a [u8],
    /// The gated body, held until the bound reach runs it once.
    pub(super) run: Option<F>,
    /// The body's outcome, shared with the gate through one slot.
    pub(super) slot: Rc<RefCell<Option<Result<Report, ExecError>>>>,
}

impl<F> EffectOwner for TaskOwner<'_, F>
where
    F: FnOnce() -> Result<Report, ExecError>,
{
    fn resolve(&mut self, _intent: &EffectIntent) -> Result<Option<ResolvedEffect>, BindingError> {
        let raw = self.repo.as_os_str().as_encoded_bytes();
        // The resolved link identity is the canonical repository path. A path
        // that cannot be canonicalized binds to its raw bytes instead, which
        // resolves identically on every call for a path that does not move.
        let canonical = match std::fs::canonicalize(self.repo) {
            Ok(path) => path.as_os_str().as_encoded_bytes().to_vec(),
            Err(_) => raw.to_vec(),
        };
        let target = EffectTarget::new(
            raw,
            &canonical,
            b"",
            self.tenant.as_bytes(),
            self.task_bytes,
            b"larry.executor/v1",
        )?;
        // The schema version of the executor's own adapter contract, and the
        // generation the runtime's fence was built at.
        Ok(Some(ResolvedEffect::new(
            target,
            ADAPTER_SCHEMA,
            self.epoch,
        )))
    }

    fn dispatch_bound(
        &mut self,
        intent: &EffectIntent,
        proof: &Auth,
        required: &[Cap],
        _at: SystemTime,
    ) -> Result<EffectOutcome, BindingError> {
        validate_proof(proof, required)?;
        if intent.tenant != self.tenant {
            return Err(BindingError::Changed {
                field: larry_contracts::binding::BindingField::Command,
            });
        }
        if self.slot.borrow().is_some() {
            // The body ran once; a second reach never re-runs it.
            return Ok(EffectOutcome::Unknown {
                operation: intent.operation,
            });
        }
        let Some(run) = self.run.take() else {
            return Ok(EffectOutcome::Unknown {
                operation: intent.operation,
            });
        };
        let finished = run();
        let outcome = match finished.is_ok() {
            true => EffectOutcome::Succeeded {
                operation: intent.operation,
            },
            false => EffectOutcome::Failed {
                operation: intent.operation,
            },
        };
        *self.slot.borrow_mut() = Some(finished);
        Ok(outcome)
    }

    fn reconcile(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
        EffectOutcome::Unknown {
            operation: intent.operation,
        }
    }
}
