//! Authority grants, attenuation and the effect-boundary re-validation
//! (spec Cell 04 authority equation, D07; work package C02).
//!
//! The authority equation is a design obligation (P1):
//! `child grants = parent grants ∩ resource policy ∩ budget ∩ expiry`,
//! validated again at the effect boundary. [`attenuate`] implements the
//! intersection over scopes, budget and expiry; [`validate_at_effect_boundary`]
//! re-checks a grant against the *current* resource policy, the required
//! scope, the cost, and authority-supplied time.
//!
//! P1 is a proof obligation, not a claim: this module provides the typed
//! design surface only. Nothing here proves revocation propagation,
//! lease fencing or offline semantics. Declared, unproved:
//!
//! - A worker can never extend a lease with its own observed timestamp:
//!   `now` is a parameter of the boundary check, supplied by the
//!   authority layer, never by the worker.
//! - Immediate remote revocation while disconnected is impossible; this
//!   module offers no API that would suggest otherwise. Available local
//!   authority can renew only within its own standing delegated limits.
//! - Delegation depth is bounded at [`MAX_DELEGATION_DEPTH`] (Cell 05:
//!   depth <= 3); an attenuated grant with no surviving scopes is refused
//!   rather than minted empty.

use core::fmt;
use std::collections::BTreeSet;
use std::time::SystemTime;

use crate::error::AuthorityError;
use crate::ids::{TenantId, WorkerId};

#[cfg(feature = "serde")]
use lgwks_std::json::serde::{Deserialize, Serialize};

/// Maximum delegation depth (Cell 05: depth <= 3).
pub const MAX_DELEGATION_DEPTH: u8 = 3;

/// A monotone generation of the authority state (R87, D48, E71).
///
/// The epoch is revocation's **linearization point**. Everything admitted
/// while the authority stood at epoch `N` belongs to generation `N`; moving
/// the fence to a later epoch withdraws all of it at one total-order point,
/// with no per-effect reconciliation and no window in which two effects
/// admitted either side of the change can be compared as if they were the
/// same decision.
///
/// The type is deliberately *monotone*. It has no decrement, no arithmetic
/// and no constructor that lowers a fence — the only movement this type
/// offers is [`SecurityEpoch::successor`] and [`SecurityEpoch::later_of`],
/// both of which only ever go up. That is the whole anti-replay rule for
/// the epoch: a replayed old device, grant or policy snapshot reports an
/// epoch the fence has already passed, and is absorbed rather than obeyed,
/// so stale evidence can never independently authorize execution (E71).
///
/// The number is not a version and not a clock. It carries no unit, it moves
/// only by explicit host action, and it never moves because time passed —
/// which is precisely why a rolled-back clock extends nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub struct SecurityEpoch(u64);

impl SecurityEpoch {
    /// The generation an authority source starts at.
    pub const INITIAL: Self = Self(0);

    /// The generation a raw value names.
    ///
    /// Constructing an epoch is not a claim that it is current: only the
    /// authority source and the dispatch fence decide that.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The raw generation number.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    /// The next generation, or `None` when the space is exhausted.
    ///
    /// `None` rather than a wrap: a wrapped epoch would alias a withdrawn
    /// generation with a live one, which is the exact confusion the type
    /// exists to prevent.
    #[must_use]
    pub const fn successor(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(next) => Some(Self(next)),
            None => None,
        }
    }

    /// The later of two generations.
    ///
    /// This is the anti-replay rule in one function. Raising a fence to
    /// whatever the authority source reports, and never lowering it to what
    /// an older snapshot reports, is what makes a replayed snapshot a
    /// non-event instead of an authorization.
    #[must_use]
    pub const fn later_of(self, other: Self) -> Self {
        if self.0 >= other.0 { self } else { other }
    }

    /// Whether this generation is strictly later than `floor`.
    #[must_use]
    pub const fn is_later_than(self, floor: Self) -> bool {
        self.0 > floor.0
    }
}

impl fmt::Display for SecurityEpoch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "epoch {}", self.0)
    }
}

/// One capability the estate can grant.
///
/// The taxonomy is a design placeholder pending the Secure-Authority
/// schema (D07: RBAC for administration plus resource/action/context
/// capabilities for execution). It is an exhaustive enum, not a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub enum Scope {
    /// Read workspace content.
    FileRead,
    /// Write or mutate workspace content.
    FileWrite,
    /// Send network traffic through the egress boundary.
    NetworkEgress,
    /// Spawn or signal processes.
    ProcessSpawn,
    /// Invoke a model provider with budgeted tokens.
    ModelInvoke,
}

/// The unit a budget is denominated in.
///
/// Budgets are only comparable within one unit; mismatched units are a
/// typed error and are never silently coerced (min across units would
/// fabricate an amount).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub enum BudgetUnit {
    /// Count of authorized requests.
    Requests,
    /// Count of model tokens.
    Tokens,
    /// Thousandths of a currency cent.
    MilliCents,
}

/// A spend ceiling in one unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub struct Budget {
    /// Maximum spend in the declared unit.
    limit: u64,
    /// Denomination of the ceiling.
    unit: BudgetUnit,
}

impl Budget {
    /// Builds a budget ceiling.
    #[must_use]
    pub const fn new(limit: u64, unit: BudgetUnit) -> Self {
        Self { limit, unit }
    }

    /// The ceiling amount.
    #[must_use]
    pub const fn limit(&self) -> u64 {
        self.limit
    }

    /// The unit the ceiling is denominated in.
    #[must_use]
    pub const fn unit(&self) -> BudgetUnit {
        self.unit
    }

    /// Caps this budget to the parent's: `min(limit)` within one unit.
    ///
    /// # Errors
    ///
    /// Returns [`AuthorityError::BudgetUnitMismatch`] when the units
    /// differ; units are never coerced.
    pub fn cap_to(self, parent: Budget) -> Result<Budget, AuthorityError> {
        if self.unit != parent.unit {
            return Err(AuthorityError::BudgetUnitMismatch {
                parent: parent.unit,
                requested: self.unit,
            });
        }
        Ok(Budget {
            limit: if self.limit < parent.limit {
                self.limit
            } else {
                parent.limit
            },
            unit: self.unit,
        })
    }
}

/// The current resource policy a grant must intersect with (D07).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub struct ResourcePolicy {
    /// Scopes admitted by this policy snapshot.
    allowed: BTreeSet<Scope>,
}

impl ResourcePolicy {
    /// Builds a policy from the scopes it currently allows.
    #[must_use]
    pub fn new(allowed: BTreeSet<Scope>) -> Self {
        Self { allowed }
    }

    /// The currently allowed scopes.
    #[must_use]
    pub fn allowed(&self) -> &BTreeSet<Scope> {
        &self.allowed
    }

    /// Whether the policy currently allows `scope`.
    #[must_use]
    pub fn allows(&self, scope: Scope) -> bool {
        self.allowed.contains(&scope)
    }
}

/// A standing authority grant held by a worker (D07).
///
/// All fields are private; grants are created by [`Grant::new`] and by
/// [`attenuate`], and every consumer reads them through accessors. A grant
/// is immutable once constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub struct Grant {
    /// Tenant inherited by every attenuated child.
    tenant: TenantId,
    /// Worker entitled to exercise this grant.
    holder: WorkerId,
    /// Bounded number of attenuation hops from the root.
    delegation_depth: u8,
    /// Scopes carried by the grant; attenuation rejects empty intersections.
    scopes: BTreeSet<Scope>,
    /// Per-grant spending ceiling, not a live ledger.
    budget: Budget,
    /// Exclusive end of authority validity.
    expiry: SystemTime,
}

impl Grant {
    /// Establishes a root or inherited grant.
    ///
    /// # Errors
    ///
    /// Returns [`AuthorityError::DelegationDepthExceeded`] when
    /// `delegation_depth` exceeds [`MAX_DELEGATION_DEPTH`].
    pub fn new(
        tenant: TenantId,
        holder: WorkerId,
        scopes: BTreeSet<Scope>,
        budget: Budget,
        expiry: SystemTime,
        delegation_depth: u8,
    ) -> Result<Self, AuthorityError> {
        if delegation_depth > MAX_DELEGATION_DEPTH {
            return Err(AuthorityError::DelegationDepthExceeded {
                depth: delegation_depth,
                max: MAX_DELEGATION_DEPTH,
            });
        }
        Ok(Self {
            tenant,
            holder,
            scopes,
            budget,
            expiry,
            delegation_depth,
        })
    }

    /// The tenant the grant is scoped to.
    #[must_use]
    pub const fn tenant(&self) -> TenantId {
        self.tenant
    }

    /// The worker holding the grant.
    #[must_use]
    pub const fn holder(&self) -> WorkerId {
        self.holder
    }

    /// The granted scopes.
    #[must_use]
    pub fn scopes(&self) -> &BTreeSet<Scope> {
        &self.scopes
    }

    /// The spend ceiling.
    #[must_use]
    pub const fn budget(&self) -> Budget {
        self.budget
    }

    /// The instant validity ends.
    #[must_use]
    pub const fn expiry(&self) -> SystemTime {
        self.expiry
    }

    /// The delegation depth this grant sits at (0 for a root grant).
    #[must_use]
    pub const fn delegation_depth(&self) -> u8 {
        self.delegation_depth
    }
}

/// The requested shape of a child grant, before intersection.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[non_exhaustive]
pub struct AttenuationRequest {
    /// The worker the child grant would be held by.
    holder: WorkerId,
    /// The scopes requested; they survive only inside parent-and-policy.
    scopes: BTreeSet<Scope>,
    /// The spend ceiling requested; it is capped to the parent's.
    budget: Budget,
    /// The expiry requested; it is capped to the parent's.
    expiry: SystemTime,
}

impl AttenuationRequest {
    /// Describes the desired shape of one attenuated child grant.
    #[must_use]
    pub fn new(
        holder: WorkerId,
        scopes: BTreeSet<Scope>,
        budget: Budget,
        expiry: SystemTime,
    ) -> Self {
        Self {
            holder,
            scopes,
            budget,
            expiry,
        }
    }

    /// Returns the prospective child holder.
    #[must_use]
    pub const fn holder(&self) -> WorkerId {
        self.holder
    }

    /// Returns the requested scopes without allowing mutation.
    #[must_use]
    pub fn scopes(&self) -> &BTreeSet<Scope> {
        &self.scopes
    }

    /// Returns the requested spending ceiling.
    #[must_use]
    pub const fn budget(&self) -> Budget {
        self.budget
    }

    /// Returns the requested expiry.
    #[must_use]
    pub const fn expiry(&self) -> SystemTime {
        self.expiry
    }
}

/// The authority equation (Cell 04): derive a child grant from a parent.
///
/// The child's scopes are the requested scopes intersected with the
/// parent's scopes and the current resource policy; the budget is capped
/// to the parent's within one unit; the expiry is the earlier of parent
/// and request; the delegation depth grows by one and is bounded.
///
/// P1 is a proof obligation and is NOT proved here — this is the typed
/// design surface the proof will attach to.
///
/// # Errors
///
/// Returns [`AuthorityError::DelegationDepthExceeded`] at the depth bound,
/// [`AuthorityError::BudgetUnitMismatch`] on unit coercion attempts, and
/// [`AuthorityError::EmptyAttenuation`] when no requested scope survives
/// the intersection.
pub fn attenuate(
    parent: &Grant,
    policy: &ResourcePolicy,
    request: &AttenuationRequest,
) -> Result<Grant, AuthorityError> {
    let child_depth = parent.delegation_depth.saturating_add(1);
    if child_depth > MAX_DELEGATION_DEPTH {
        return Err(AuthorityError::DelegationDepthExceeded {
            depth: child_depth,
            max: MAX_DELEGATION_DEPTH,
        });
    }
    let surviving: BTreeSet<Scope> = request
        .scopes()
        .intersection(&parent.scopes)
        .copied()
        .filter(|scope| policy.allows(*scope))
        .collect();
    if surviving.is_empty() {
        return Err(AuthorityError::EmptyAttenuation);
    }
    let budget = request.budget().cap_to(parent.budget)?;
    let expiry = if parent.expiry <= request.expiry() {
        parent.expiry
    } else {
        request.expiry()
    };
    Grant::new(
        parent.tenant,
        request.holder(),
        surviving,
        budget,
        expiry,
        child_depth,
    )
}

/// Re-validates a grant at the effect boundary, against the *current*
/// resource policy and authority-supplied time (Cell 04: "validate again
/// at the effect boundary").
///
/// Expiry never consents (E30): an expired grant yields
/// [`AuthorityError::GrantExpired`] even when scope and budget would
/// otherwise pass. The observation instant `now` is supplied by the
/// authority layer — a worker's own timestamp can never extend a lease.
///
/// This function enforces only that a *single* effect's cost fits inside
/// the grant ceiling. It never enforces cumulative spend: this crate is
/// stateless and owns no ledger (D39 places the budget ledger with the
/// owner scheduler). Calling this per effect does not bound total spend —
/// use [`validate_at_effect_boundary_with_ledger`] with the remaining
/// balance from the owner ledger to bound cumulative consumption.
///
/// # Errors
///
/// Returns [`AuthorityError::GrantExpired`] when the grant's validity has
/// ended, [`AuthorityError::ScopeNotGranted`] when the scope was never
/// granted, [`AuthorityError::ScopeOutsidePolicy`] when the current policy
/// refuses the scope, and [`AuthorityError::BudgetExhausted`] when the
/// cost exceeds the granted ceiling.
pub fn validate_at_effect_boundary(
    grant: &Grant,
    required: Scope,
    cost: u64,
    policy: &ResourcePolicy,
    now: SystemTime,
) -> Result<(), AuthorityError> {
    if now >= grant.expiry {
        return Err(AuthorityError::GrantExpired {
            expiry: grant.expiry,
            now,
        });
    }
    if !grant.scopes.contains(&required) {
        return Err(AuthorityError::ScopeNotGranted { scope: required });
    }
    if !policy.allows(required) {
        return Err(AuthorityError::ScopeOutsidePolicy { scope: required });
    }
    if cost > grant.budget.limit {
        return Err(AuthorityError::BudgetExhausted {
            required: cost,
            available: grant.budget.limit,
            unit: grant.budget.unit,
        });
    }
    Ok(())
}

/// Re-validates a grant at the effect boundary against the remaining
/// balance reported by the owner budget ledger, bounding *cumulative*
/// spend across effects and not just each effect against the ceiling.
///
/// `remaining` is the ledger's authoritative remaining balance for this
/// grant at validation time; the effect is admitted only when its cost
/// fits in `remaining` and inside the grant ceiling. Reservation of the
/// spend is the caller's atomic compare-and-set against the same ledger
/// (Cell 04 ordering); this check and the reservation must share one
/// ledger observation or the bound is advisory.
///
/// # Errors
///
/// Same conditions as [`validate_at_effect_boundary`], plus
/// [`AuthorityError::BudgetExhausted`] when `cost` exceeds `remaining`
/// even though it may fit the ceiling.
pub fn validate_at_effect_boundary_with_ledger(
    grant: &Grant,
    required: Scope,
    cost: u64,
    remaining: u64,
    policy: &ResourcePolicy,
    now: SystemTime,
) -> Result<(), AuthorityError> {
    validate_at_effect_boundary(grant, required, cost, policy, now)?;
    if cost > remaining {
        return Err(AuthorityError::BudgetExhausted {
            required: cost,
            available: remaining,
            unit: grant.budget.unit,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
