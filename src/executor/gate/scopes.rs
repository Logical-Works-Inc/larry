//! The scope vocabulary the gate admits tasks against: what the executor is
//! capable of, the default bundle an unset `LARRY_POLICY` names, and the scope
//! sets a terminal session runs under. Admission and driving live in the parent
//! module; this one only names the sets (larry#164, larry#203).

use std::collections::BTreeSet;

use larry_contracts::authority::Scope;

/// The scopes the executor is capable of. The capability snapshot pins them,
/// so the policy alone decides what a task may do.
pub(super) const EXECUTOR_CAPABILITIES: [Scope; 6] = [
    Scope::FileRead,
    Scope::FileWrite,
    Scope::ProcessSpawn,
    Scope::ModelInvoke,
    Scope::ModelCommand,
    Scope::NetworkEgress,
];

/// The policy when `LARRY_POLICY` is unset: the five scopes a model task uses. A model
/// command (larry#93) is never in it: only an operator who isolates the commands
/// outside Larry, as a Harbor task container does, names `model.command`. A terminal
/// session that confines its own commands to a workspace copy (larry#243, ADR 0046)
/// needs no such scope and runs under this policy as it is.
const DEFAULT_POLICY: [Scope; 5] = [
    Scope::FileRead,
    Scope::FileWrite,
    Scope::ProcessSpawn,
    Scope::ModelInvoke,
    Scope::NetworkEgress,
];

/// The policy when `LARRY_POLICY` is unset.
pub(crate) fn default_policy() -> BTreeSet<Scope> {
    DEFAULT_POLICY.iter().copied().collect()
}

/// The scopes a terminal command session needs (larry#93): command execution and
/// model-command selection. A model brief adds network egress through
/// `required_scopes`, so an ordinary confined session stays local by default.
pub(crate) fn session_scopes() -> BTreeSet<Scope> {
    let mut scopes: BTreeSet<Scope> = EXECUTOR_CAPABILITIES.iter().copied().collect();
    scopes.remove(&Scope::NetworkEgress);
    scopes
}

/// The scopes a terminal session needs when Larry itself confines each command to a
/// workspace copy (larry#243): a briefed task's scopes without `model.command`, which
/// names a command run on the host with no confinement of Larry's own.
pub(crate) fn isolated_scopes() -> BTreeSet<Scope> {
    let mut scopes = session_scopes();
    scopes.remove(&Scope::ModelCommand);
    scopes
}
