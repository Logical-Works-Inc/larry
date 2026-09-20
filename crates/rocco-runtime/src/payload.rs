//! The typed payloads carried by recorded events.
//!
//! `contracts::event::EventKind` is a closed five-variant vocabulary; the
//! specification owns it and this crate may not extend it. Each payload here
//! is the body of exactly one of those kinds, encoded as JSON through the
//! `lgwks_std/json` edge (the same serde re-export `rocco-contracts` uses).
//!
//! The payloads are chosen so that the journal is *self-checking*: a
//! [`TransitionPayload`] carries both the event offered and the state the
//! reducer claimed, so a fold re-derives the transition from the pure algebra
//! and surfaces a mismatch instead of trusting the record.
//! [`IntentPayload`] carries the [`ReplayRecord`] — the pinned authorization
//! inputs and the decision they produced — so replaying the journal re-derives
//! every authorization bit-identically (R71, D36, E55).

use std::collections::BTreeSet;

use lgwks_std::json::serde::{Deserialize, Serialize};
use rocco_contracts::authority::{BudgetUnit, Scope, SecurityEpoch};
use rocco_contracts::command::CommandKind;
use rocco_contracts::effect::{
    EffectIntent, EffectOutcome, ReconciliationStrategy, TerminalReceipt,
};
use rocco_contracts::ids::{CommandId, EffectId, OperationId, RunId, TenantId};
use rocco_contracts::replay::ReplayRecord;
use rocco_contracts::state::{RunState, TransitionEvent};

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
    pub required_scopes: BTreeSet<Scope>,
    /// The declared cost.
    pub cost: u64,
    /// The unit the cost is denominated in.
    pub unit: BudgetUnit,
    /// The declared reconciliation strategy (R72).
    pub strategy: ReconciliationStrategy,
}

impl WorkProposal {
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
        EffectIntent {
            id: EffectId::from_bytes(*self.operation.as_bytes()),
            operation: self.operation,
            run: self.run,
            tenant,
            strategy: self.strategy,
            admission_epoch,
        }
    }
}

/// The body of an [`EventKind::CommandAccepted`](rocco_contracts::event::EventKind::CommandAccepted)
/// record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
pub struct CommandAcceptedPayload {
    /// The accepted command identity.
    pub command: CommandId,
    /// The run the command targets.
    pub run: RunId,
    /// The command class.
    pub kind: CommandKind,
}

/// The body of an [`EventKind::StateTransition`](rocco_contracts::event::EventKind::StateTransition)
/// record.
///
/// `from` and `to` are the reducer's claim; a fold re-derives `to` from
/// `from` and `event` through `state::transition` and refuses a mismatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
pub struct TransitionPayload {
    /// The event offered to the algebra.
    pub event: TransitionEvent,
    /// The state the run was in.
    pub from: RunState,
    /// The state the algebra produced.
    pub to: RunState,
}

/// The body of an [`EventKind::EffectIntentRecorded`](rocco_contracts::event::EventKind::EffectIntentRecorded)
/// record: the durable reservation, written before dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
pub struct IntentPayload {
    /// The reserved intent, including its stable operation ID and declared
    /// reconciliation strategy.
    pub intent: EffectIntent,
    /// The pinned authorization inputs and the decision they produced. This
    /// is what makes the journal replayable (E55).
    pub authorization: ReplayRecord,
}

/// The body of an [`EventKind::EffectOutcomeRecorded`](rocco_contracts::event::EventKind::EffectOutcomeRecorded)
/// record: what the owner reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
pub struct OutcomePayload {
    /// The intent the outcome belongs to, carried so the fold can resolve the
    /// disposition without a separate lookup table.
    pub intent: EffectIntent,
    /// The observed outcome.
    pub outcome: EffectOutcome,
}

/// The body of an [`EventKind::TerminalReceiptPublished`](rocco_contracts::event::EventKind::TerminalReceiptPublished)
/// record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde")]
pub struct ReceiptPayload {
    /// The immutable terminal receipt.
    pub receipt: TerminalReceipt,
}
