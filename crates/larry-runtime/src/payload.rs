//! The typed payloads carried by recorded events.
//!
//! `contracts::event::EventKind` is a closed five-variant vocabulary; the
//! specification owns it and this crate may not extend it. Each payload here
//! is the body of exactly one of those kinds, encoded as JSON through the
//! `lgwks_std/json` edge (the same serde re-export `larry-contracts` uses).
//!
//! The payloads are chosen so that the journal is *self-checking*: a
//! [`TransitionPayload`] carries both the event offered and the state the
//! reducer claimed, so a fold re-derives the transition from the pure algebra
//! and surfaces a mismatch instead of trusting the record.
//! [`IntentPayload`] carries the [`ReplayRecord`] — the pinned authorization
//! inputs and the decision they produced — so replaying the journal re-derives
//! every authorization bit-identically (R71, D36, E55).

use std::collections::BTreeSet;

use larry_contracts::authority::{BudgetUnit, Scope, SecurityEpoch};
use larry_contracts::command::CommandKind;
use larry_contracts::effect::{
    EffectIntent, EffectOutcome, ReconciliationStrategy, TerminalReceipt,
};
use larry_contracts::ids::{CommandId, EffectId, OperationId, RunId, TenantId};
use larry_contracts::replay::ReplayRecord;
use larry_contracts::state::{RunState, TransitionEvent};
use lgwks_std::json::serde::{Deserialize, Serialize};

/// The body of a `Submit` or `Cancel` command.
///
/// A command envelope carries identity, tenant and causal parents but no run,
/// so the run a command targets is part of its content. Decoding this body is
/// what tells the runtime which run the command steers.
///
/// Nothing here is an authority claim: `cost` and `required_scopes` are what
/// the work *declares*, and the gate is what decides whether the declaration
/// is allowed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
#[non_exhaustive]
pub struct WorkProposal {
    /// The run the work belongs to.
    pub run: RunId,
    /// The tenant the body claims the run belongs to.
    ///
    /// A declaration, and one the runtime does not consult: the run's tenant
    /// is read from the runtime's own run table, exactly as the admission
    /// epoch is read from the dispatch fence, so a hostile generator cannot
    /// record an effect — or re-attribute a run after a restart — to a tenant
    /// it does not own (D42, E63). The field stays because the body is the
    /// caller's declaration of its own intent, not because the runtime trusts
    /// it.
    pub tenant: TenantId,
    /// The stable operation identity presented to the endpoint.
    pub operation: OperationId,
    /// The scopes the effect requires; every one must pass the gate.
    required_scopes: BTreeSet<Scope>,
    /// The declared cost.
    pub cost: u64,
    /// The unit the cost is denominated in.
    pub unit: BudgetUnit,
    /// The declared reconciliation strategy (R72).
    pub strategy: ReconciliationStrategy,
}

impl WorkProposal {
    /// Declares work; the runtime still supplies and validates its authority.
    #[must_use]
    pub const fn new(
        run: RunId,
        tenant: TenantId,
        operation: OperationId,
        required_scopes: BTreeSet<Scope>,
        cost: u64,
        unit: BudgetUnit,
        strategy: ReconciliationStrategy,
    ) -> Self {
        Self {
            run,
            tenant,
            operation,
            required_scopes,
            cost,
            unit,
            strategy,
        }
    }

    /// Returns the declared scope requirements without allowing mutation.
    #[must_use]
    pub fn required_scopes(&self) -> &BTreeSet<Scope> {
        &self.required_scopes
    }

    /// The intent this proposal reserves, for `tenant`, admitted under
    /// `admission_epoch`.
    ///
    /// The local effect identity is derived from the stable operation
    /// identity, so the two are never accidentally swapped and a resumed run
    /// reconstructs the same local identity it recorded.
    ///
    /// `tenant` and `admission_epoch` are arguments rather than fields of this
    /// type on purpose, and for the same reason. Every other value here is
    /// *declared* by the work and then checked by the gate; neither the
    /// security generation nor the run's tenant is declarable at all. Both
    /// come from protected runtime state — the dispatch fence and the run
    /// table — so a hostile generator composing this payload cannot choose the
    /// security generation it is admitted under, and cannot name a generation
    /// that would survive a revocation it should not survive, or a tenant it
    /// does not own (D42, E63).
    #[must_use]
    pub fn intent(&self, tenant: TenantId, admission_epoch: SecurityEpoch) -> EffectIntent {
        EffectIntent::new(
            EffectId::from_bytes(*self.operation.as_bytes()),
            self.operation,
            self.run,
            tenant,
            self.strategy,
            admission_epoch,
        )
    }
}

/// The body of an [`EventKind::CommandAccepted`](larry_contracts::event::EventKind::CommandAccepted)
/// record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
#[non_exhaustive]
pub struct CommandAcceptedPayload {
    /// The accepted command identity.
    pub command: CommandId,
    /// The run the command targets.
    pub run: RunId,
    /// The command class.
    pub kind: CommandKind,
}

/// The body of an [`EventKind::StateTransition`](larry_contracts::event::EventKind::StateTransition)
/// record.
///
/// `from` and `to` are the reducer's claim; a fold re-derives `to` from
/// `from` and `event` through `state::transition` and refuses a mismatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
#[non_exhaustive]
pub struct TransitionPayload {
    /// The event offered to the algebra.
    pub event: TransitionEvent,
    /// The state the run was in.
    pub from: RunState,
    /// The state the algebra produced.
    pub to: RunState,
}

/// The body of an [`EventKind::EffectIntentRecorded`](larry_contracts::event::EventKind::EffectIntentRecorded)
/// record: the durable reservation, written before dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
#[non_exhaustive]
pub struct IntentPayload {
    /// The reserved intent, including its stable operation ID and declared
    /// reconciliation strategy.
    pub intent: EffectIntent,
    /// The pinned authorization inputs and the decision they produced. This
    /// is what makes the journal replayable (E55).
    pub authorization: ReplayRecord,
}

/// The body of an [`EventKind::EffectOutcomeRecorded`](larry_contracts::event::EventKind::EffectOutcomeRecorded)
/// record: what the owner reported.
///
/// `dispatches` is how many times the owner had been asked to perform the
/// effect when it reported this outcome, and it is what makes the dispatch
/// budget a fold over the journal rather than live state a restart recharges
/// (larry#182). A record that does not carry it — one written before larry#182
/// — restores as if the whole budget were spent, because the journal cannot
/// prove the effect was dispatched fewer times than allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
#[non_exhaustive]
pub struct OutcomePayload {
    /// The intent the outcome belongs to, carried so the fold can resolve the
    /// disposition without a separate lookup table.
    pub intent: EffectIntent,
    /// The observed outcome.
    pub outcome: EffectOutcome,
    /// How many dispatches this runtime had made when the outcome was recorded.
    #[serde(default = "spent_budget")]
    pub dispatches: u8,
}

/// The dispatch count an old record with no attempt number restores as: the
/// whole budget spent. The journal cannot prove the effect was dispatched
/// fewer times, so the fail-closed answer is that it may have used every
/// attempt (larry#182).
const fn spent_budget() -> u8 {
    larry_contracts::effect::MAX_EFFECT_ATTEMPTS
}

/// The body of an [`EventKind::TerminalReceiptPublished`](larry_contracts::event::EventKind::TerminalReceiptPublished)
/// record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
#[non_exhaustive]
pub struct ReceiptPayload {
    /// The immutable terminal receipt.
    pub receipt: TerminalReceipt,
}

impl CommandAcceptedPayload {
    /// Records a command's accepted target and kind.
    #[must_use]
    pub const fn new(command: CommandId, run: RunId, kind: CommandKind) -> Self {
        Self { command, run, kind }
    }
}

impl TransitionPayload {
    /// Records a transition and the states independently checked by replay.
    #[must_use]
    pub const fn new(event: TransitionEvent, from: RunState, to: RunState) -> Self {
        Self { event, from, to }
    }
}

impl IntentPayload {
    /// Records a reservation and the authorization inputs that justified it.
    #[must_use]
    pub const fn new(intent: EffectIntent, authorization: ReplayRecord) -> Self {
        Self {
            intent,
            authorization,
        }
    }
}

impl OutcomePayload {
    /// Records an owner's observation for the reserved intent, made after
    /// `dispatches` reaches.
    #[must_use]
    pub const fn new(intent: EffectIntent, outcome: EffectOutcome, dispatches: u8) -> Self {
        Self {
            intent,
            outcome,
            dispatches,
        }
    }
}

impl ReceiptPayload {
    /// Wraps an immutable terminal receipt for journal encoding.
    #[must_use]
    pub const fn new(receipt: TerminalReceipt) -> Self {
        Self { receipt }
    }
}
