//! The atomic-compare-and-dispatch owner double shared by the display and
//! protocol ingress falsifiers. Nothing here reaches a network; the owner
//! models an atomic effect boundary over deterministic seeded identity.

use std::time::SystemTime;

use larry_contracts::authority::SecurityEpoch;
use larry_contracts::binding::{BindingError, BindingField, EffectTarget, ResolvedEffect};
use larry_contracts::effect::{EffectIntent, EffectOutcome};
use larry_contracts::ids::{OperationId, TenantId, WorkerId};
use larry_runtime::owner::EffectOwner;

/// Host-authenticated holder the display journeys share: the identity every
/// grant names, without sharing the grants themselves across tenants.
#[must_use]
pub fn holder() -> WorkerId {
    WorkerId::from_bytes([42; 16])
}

/// Seed-selected field with a separate oracle from the digest implementation.
#[must_use]
pub fn swapped_field(seed: u64) -> BindingField {
    match seed & 7 {
        0 => BindingField::Resource,
        1 => BindingField::Symlink,
        2 => BindingField::Recipient,
        3 => BindingField::Account,
        4 => BindingField::Payload,
        5 => BindingField::Adapter,
        6 => BindingField::Schema,
        _ => BindingField::PolicyEpoch,
    }
}

/// Opaque fixture identities vary by seed; exactly one field changes.
pub fn resolved(seed: u64, changed: bool) -> Result<ResolvedEffect, BindingError> {
    let original = seed.to_be_bytes();
    let replacement = seed.saturating_add(1).to_be_bytes();
    let field = seed & 7;
    let bytes = |index| {
        if changed && field == index {
            replacement.as_slice()
        } else {
            original.as_slice()
        }
    };
    let target = EffectTarget::new(bytes(0), bytes(1), bytes(2), bytes(3), bytes(4), bytes(5))?;
    Ok(ResolvedEffect::new(
        target,
        if changed && field == 6 { 2 } else { 1 },
        SecurityEpoch::new(u64::from(changed && field == 7)),
    ))
}

/// A deterministic owner whose fourth observation is the atomic effect boundary.
#[derive(Debug, Clone)]
pub struct AtomicOwner {
    /// Single seed controls bytes, identity, time and fault location.
    pub seed: u64,
    /// Observation number that swaps one field; zero never swaps.
    pub change_on: u64,
    /// Observations already made.
    pub observations: u64,
    /// Whether the effect's response is deliberately lost.
    pub lose_reply: bool,
    /// Bounded by the runtime queue/workload in these tests; read through `executed`.
    executed: Vec<(TenantId, OperationId, ResolvedEffect)>,
}

impl AtomicOwner {
    /// Starts an isolated owner without an ambient clock or network.
    #[must_use]
    pub fn new(seed: u64, change_on: u64) -> Self {
        Self {
            seed,
            change_on,
            observations: 0,
            lose_reply: false,
            executed: Vec::new(),
        }
    }

    /// Every bound effect this owner executed, in dispatch order: tenant, operation, target.
    #[must_use]
    pub fn executed(&self) -> &[(TenantId, OperationId, ResolvedEffect)] {
        &self.executed
    }

    /// Atomically observes the same scripted state used by execution.
    fn observe(&mut self) -> Result<ResolvedEffect, BindingError> {
        self.observations = self.observations.saturating_add(1);
        resolved(
            self.seed,
            self.change_on != 0 && self.observations >= self.change_on,
        )
    }
}

impl EffectOwner for AtomicOwner {
    fn resolve(&mut self, _intent: &EffectIntent) -> Result<Option<ResolvedEffect>, BindingError> {
        self.observe().map(Some)
    }

    fn dispatch_bound(
        &mut self,
        intent: &EffectIntent,
        _at: SystemTime,
    ) -> Result<EffectOutcome, BindingError> {
        let current = self.observe()?;
        intent
            .binding()
            .ok_or(BindingError::MissingDisplay)?
            .resolved()
            .verify(&current)?;
        // The validated resolved value itself is executed, never a second lookup.
        self.executed
            .push((intent.tenant, intent.operation, current));
        Ok(if self.lose_reply {
            EffectOutcome::Unknown {
                operation: intent.operation,
            }
        } else {
            EffectOutcome::Succeeded {
                operation: intent.operation,
            }
        })
    }

    fn dispatch(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
        // Any legacy dispatch is an observable test failure, not a fallback.
        self.observations = u64::MAX;
        EffectOutcome::Unknown {
            operation: intent.operation,
        }
    }

    fn reconcile(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
        EffectOutcome::Succeeded {
            operation: intent.operation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AtomicOwner, holder, resolved, swapped_field};
    use larry_contracts::authority::{BudgetUnit, Scope, SecurityEpoch};
    use larry_contracts::binding::BindingError;
    use larry_contracts::effect::ReconciliationStrategy;
    use larry_contracts::ids::{OperationId, RunId, TenantId, WorkerId};
    use larry_runtime::owner::EffectOwner;
    use larry_runtime::payload::WorkProposal;
    use std::time::{Duration, SystemTime};

    /// The atomic fixtures build what they name: a holder, a field oracle, a
    /// stable resolved target and an owner that refuses an unbound dispatch.
    #[test]
    fn atomic_owner_fixtures_build_what_they_name() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(holder(), WorkerId::from_bytes([42; 16]));
        assert_ne!(
            swapped_field(0),
            swapped_field(1),
            "different seeds name different fields"
        );
        let kept = resolved(5, false)?;
        assert_eq!(resolved(5, false)?, kept, "one seed resolves identically");
        assert_ne!(
            resolved(5, true)?,
            kept,
            "the swap changes the resolved identity"
        );
        let operation = OperationId::from_bytes([1; 16]);
        let body = WorkProposal::new(
            RunId::from_bytes([1; 16]),
            TenantId::from_bytes([1; 16]),
            operation,
            [Scope::FileRead].into_iter().collect(),
            1,
            BudgetUnit::Requests,
            ReconciliationStrategy::Queryable,
        );
        let intent = body.intent(TenantId::from_bytes([1; 16]), SecurityEpoch::INITIAL);
        let mut owner = AtomicOwner::new(5, 0);
        let at = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(1_700_000_000))
            .ok_or("fixture instant overflow")?;
        assert!(
            owner.resolve(&intent)?.is_some(),
            "the atomic owner resolves its target"
        );
        assert!(
            matches!(
                owner.dispatch_bound(&intent, at),
                Err(BindingError::MissingDisplay)
            ),
            "an unbound intent is refused, never dispatched"
        );
        assert!(owner.executed().is_empty(), "nothing was executed");
        Ok(())
    }
}
