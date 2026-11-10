//! The learning budget: what a tenant's standing authority lets learning spend (larry#66 part 1,
//! ADR 0033).
//!
//! Learning has its own ceiling for rollouts, sessions and money, held beside the other pinned
//! standing-authority snapshots and counted per tenant across all of that tenant's plans. The money
//! ceiling reuses [`Budget`] in [`BudgetUnit::MilliCents`], so there is no float and no second money
//! type. [`LearningBudget::admit`] is the one pure rule; the runtime asks it before an effect is
//! reserved and the fold counts what was reserved, so a denial-of-wallet loop (#42) is refused at the
//! fence with nothing spent.
//!
//! What is charged. A recursion probe is one rollout. A first-attempt proposer or composer step and a
//! selector run are one session. A retry is the same session, so it adds no session. Every learning
//! step also charges its declared cost as money, and the cost must be in milli-cents: a learning step
//! priced in any other unit is refused ([`LearningError::Unit`]), never counted as free, so pricing a
//! loop in tokens cannot slip past the money ceiling.

use crate::authority::{Budget, BudgetUnit};
use crate::recursion::Stage;

/// A per-tenant learning ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LearningBudget {
    /// The most rollouts.
    rollouts: u64,
    /// The most sessions.
    sessions: u64,
    /// The most money, in milli-cents.
    money: Budget,
}

impl LearningBudget {
    /// A ceiling of `rollouts`, `sessions` and `milli_cents`.
    #[must_use]
    pub const fn new(rollouts: u64, sessions: u64, milli_cents: u64) -> Self {
        Self {
            rollouts,
            sessions,
            money: Budget::new(milli_cents, BudgetUnit::MilliCents),
        }
    }

    /// The rollout ceiling.
    #[must_use]
    pub const fn rollouts(&self) -> u64 {
        self.rollouts
    }

    /// The session ceiling.
    #[must_use]
    pub const fn sessions(&self) -> u64 {
        self.sessions
    }

    /// The money ceiling.
    #[must_use]
    pub const fn money(&self) -> Budget {
        self.money
    }

    /// Whether `charge` fits on top of `spent`.
    ///
    /// # Errors
    ///
    /// [`LearningError`] naming the first ceiling it would pass: rollouts, then sessions, then money.
    pub fn admit(&self, spent: &LearningSpent, charge: &Charge) -> Result<(), LearningError> {
        if !matches!(charge.unit, BudgetUnit::MilliCents) {
            return Err(LearningError::Unit { unit: charge.unit });
        }
        if spent.rollouts.saturating_add(charge.rollouts) > self.rollouts {
            return Err(LearningError::Rollouts {
                limit: self.rollouts,
                spent: spent.rollouts,
            });
        }
        if spent.sessions.saturating_add(charge.sessions) > self.sessions {
            return Err(LearningError::Sessions {
                limit: self.sessions,
                spent: spent.sessions,
            });
        }
        if spent.money.saturating_add(charge.money) > self.money.limit() {
            return Err(LearningError::Money {
                limit: self.money.limit(),
                spent: spent.money,
                requested: charge.money,
            });
        }
        Ok(())
    }
}

/// What one learning step costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Charge {
    /// Rollouts, 0 or 1.
    rollouts: u64,
    /// Sessions, 0 or 1.
    sessions: u64,
    /// Milli-cents when the unit is milli-cents.
    money: u64,
    /// The unit the cost was declared in.
    unit: BudgetUnit,
}

impl Charge {
    /// The charge of a recursion step: a probe is a rollout, a first-attempt proposer or composer is a
    /// session, a retry only costs money.
    #[must_use]
    pub const fn of_step(stage: Stage, attempt: u8, cost: u64, unit: BudgetUnit) -> Self {
        let probe = matches!(stage, Stage::Probe);
        Self {
            rollouts: one_if(probe),
            sessions: one_if(!probe && attempt == 0),
            money: cost,
            unit,
        }
    }

    /// The charge of a selector run: one session.
    #[must_use]
    pub const fn of_selection(cost: u64, unit: BudgetUnit) -> Self {
        Self {
            rollouts: 0,
            sessions: 1,
            money: cost,
            unit,
        }
    }
}

/// One for true, nothing for false.
const fn one_if(flag: bool) -> u64 {
    if flag { 1 } else { 0 }
}

/// What a tenant's learning has spent so far, summed over all its plans.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LearningSpent {
    /// Rollouts reserved.
    rollouts: u64,
    /// Sessions reserved.
    sessions: u64,
    /// Milli-cents reserved.
    money: u64,
}

impl LearningSpent {
    /// This spend with `charge` added.
    #[must_use]
    pub const fn plus(self, charge: &Charge) -> Self {
        Self {
            rollouts: self.rollouts.saturating_add(charge.rollouts),
            sessions: self.sessions.saturating_add(charge.sessions),
            money: self.money.saturating_add(charge.money),
        }
    }

    /// Rollouts spent.
    #[must_use]
    pub const fn rollouts(&self) -> u64 {
        self.rollouts
    }

    /// Sessions spent.
    #[must_use]
    pub const fn sessions(&self) -> u64 {
        self.sessions
    }

    /// Milli-cents spent.
    #[must_use]
    pub const fn money(&self) -> u64 {
        self.money
    }
}

impl LearningError {
    /// What the step asked for and what was left in the ceiling it passed, for the journalled denial.
    #[must_use]
    pub const fn required_and_remaining(&self, charge: &Charge) -> (u64, u64) {
        match *self {
            Self::Rollouts { limit, spent } => (charge.rollouts, limit.saturating_sub(spent)),
            Self::Sessions { limit, spent } => (charge.sessions, limit.saturating_sub(spent)),
            Self::Money {
                limit,
                spent,
                requested,
            } => (requested, limit.saturating_sub(spent)),
            Self::Unit { .. } => (charge.money, 0),
        }
    }
}

/// Which learning ceiling a step would pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LearningError {
    /// The rollout ceiling.
    #[error("learning has spent {spent} of {limit} rollouts")]
    Rollouts {
        /// The ceiling.
        limit: u64,
        /// Spent.
        spent: u64,
    },
    /// The session ceiling.
    #[error("learning has spent {spent} of {limit} sessions")]
    Sessions {
        /// The ceiling.
        limit: u64,
        /// Spent.
        spent: u64,
    },
    /// The step is priced outside milli-cents; learning money is milli-cents only, so it is refused.
    #[error("learning step priced in {unit:?}; learning money is milli-cents only")]
    Unit {
        /// The unit it was priced in.
        unit: BudgetUnit,
    },
    /// The money ceiling.
    #[error("learning has spent {spent} of {limit} milli-cents and asks {requested} more")]
    Money {
        /// The ceiling.
        limit: u64,
        /// Spent.
        spent: u64,
        /// The step's cost.
        requested: u64,
    },
}
