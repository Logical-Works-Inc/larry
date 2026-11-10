//! Milli-cent variants of the shared fixtures, for the suites that run learning steps: a learning step
//! priced in any other unit is refused (larry#66). Included with `#[path]` only by those suites.

use std::collections::BTreeSet;

use larry_contracts::authority::{BudgetUnit, Scope};
use larry_contracts::effect::ReconciliationStrategy;
use larry_contracts::ids::{OperationId, RunId, TenantId};
use larry_contracts::replay::{CapabilitySnapshot, PolicySnapshot};
use larry_runtime::payload::WorkProposal;
use larry_runtime::runtime::RuntimeSettings;

use crate::support::validity;

/// As `support::settings`, denominated in milli-cents.
///
/// # Errors
///
/// As `support::validity`.
pub fn settings_milli(
    allowed: &[Scope],
    capable: &[Scope],
    budget: u64,
) -> Result<RuntimeSettings, Box<dyn std::error::Error>> {
    Ok(RuntimeSettings::with_defaults(
        PolicySnapshot::new(1, allowed.iter().copied().collect()),
        CapabilitySnapshot::new(capable.iter().copied().collect()),
        validity()?,
        BudgetUnit::MilliCents,
        budget,
    ))
}

/// As `support::proposal`, priced in milli-cents.
pub fn proposal_milli(
    run: RunId,
    tenant: TenantId,
    operation: OperationId,
    scopes: &[Scope],
    cost: u64,
    strategy: ReconciliationStrategy,
) -> WorkProposal {
    WorkProposal::new(
        run,
        tenant,
        operation,
        scopes.iter().copied().collect::<BTreeSet<_>>(),
        cost,
        BudgetUnit::MilliCents,
        strategy,
    )
}
