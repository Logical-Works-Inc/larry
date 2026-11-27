//! The operator's host fence action and how its outcome reads (larry#215, ADR 0121).
//!
//! `LARRY_FENCE_CONTROL` names one authority action — revoke, advance or
//! authenticate — applied after the admitted task is durable and before
//! `Runtime::tick` can reach the owner. A reach the fence or the journal
//! refuses is sealed with its cause, never with a bare denial, so the denial
//! line stays the operator's whole evidence for what held the dispatch.

use larry_contracts::authority::SecurityEpoch;
use larry_contracts::ids::OperationId;
use larry_contracts::replay::DenialReason;
use larry_runtime::error::RuntimeError;
use larry_runtime::owner::EffectOwner;
use larry_runtime::projection::FoldProjection;
use larry_runtime::runtime::{Runtime, TickReport};

use super::ExecError;
use super::gate::scope_name;

/// One operator-controlled fence action applied after durable admission and
/// before the executor's owner reach.
#[derive(Clone, Copy)]
pub(crate) enum FenceControl {
    /// Withdraw the operation of the task being admitted.
    Revoke,
    /// Move the fence to its next security epoch.
    Advance,
    /// Re-authenticate at the authority source's reported epoch.
    Authenticate(SecurityEpoch),
}

/// Reads the operator's one host fence action. The action is applied only
/// after this task is durably admitted and names no test-only bypass.
pub(super) fn read(
    read: &impl Fn(&str) -> Option<String>,
) -> Result<Option<FenceControl>, ExecError> {
    let Some(value) = read("LARRY_FENCE_CONTROL") else {
        return Ok(None);
    };
    let Some((kind, argument)) = value.split_once(':') else {
        return match value.as_str() {
            "revoke" => Ok(Some(FenceControl::Revoke)),
            "advance" => Ok(Some(FenceControl::Advance)),
            _ => Err(ExecError::BadFenceControl {
                value,
                cause: String::from("expected revoke, advance or authenticate:<epoch>"),
            }),
        };
    };
    match (kind, argument) {
        ("revoke", "") => Ok(Some(FenceControl::Revoke)),
        ("advance", "") => Ok(Some(FenceControl::Advance)),
        ("authenticate", epoch) => epoch
            .parse::<u64>()
            .map(|value| Some(FenceControl::Authenticate(SecurityEpoch::new(value))))
            .map_err(|cause| ExecError::BadFenceControl {
                value,
                cause: cause.to_string(),
            }),
        _ => Err(ExecError::BadFenceControl {
            value,
            cause: String::from("expected revoke, advance or authenticate:<epoch>"),
        }),
    }
}

/// Applies one operator authority action after durable admission and before
/// `Runtime::tick` reaches the external owner.
pub(super) fn apply<O: EffectOwner>(
    runtime: &mut Runtime<FoldProjection, O>,
    control: FenceControl,
    operation: OperationId,
) -> Result<(), RuntimeError> {
    match control {
        FenceControl::Revoke => runtime
            .fence_mut()
            .revoke(operation)
            .map(|_| ())
            .map_err(RuntimeError::from),
        FenceControl::Advance => runtime
            .fence_mut()
            .advance_epoch()
            .map(|_| ())
            .map_err(RuntimeError::from),
        FenceControl::Authenticate(epoch) => runtime
            .fence_mut()
            .authenticate(epoch)
            .map(|_| ())
            .map_err(RuntimeError::from),
    }
}

/// The sealed denial line for a tick that ran nothing: the first recorded
/// reason, else a fenced or refused reach with its cause, else the bare
/// denial when the tick recorded nothing at all.
pub(super) fn report_line(tick: &TickReport) -> String {
    match tick.denied().first() {
        Some(pair) => denial_line(&pair.1),
        None => match tick.fenced().first() {
            Some(&(_operation, ref refusal)) => format!("gate: fence: {refusal}"),
            None => match tick.refused().first().map(|pair| &pair.1) {
                Some(refusal) => format!("gate: refusal: {refusal}"),
                None => String::from("gate: denied"),
            },
        },
    }
}

/// The one denial line a recorded authorization reason stands for, naming the
/// refused scope with its dotted name.
fn denial_line(reason: &DenialReason) -> String {
    match *reason {
        DenialReason::PolicyRefuses { scope } => {
            format!("gate: policy_refuses: {}", scope_name(scope))
        }
        DenialReason::CapabilityMissing { scope } => {
            format!("gate: capability_missing: {}", scope_name(scope))
        }
        DenialReason::AuthorityExpired => String::from("gate: authority_expired"),
        DenialReason::BudgetExhausted { .. } => String::from("gate: budget_exhausted"),
        DenialReason::UnitMismatch { .. } => String::from("gate: unit_mismatch"),
        DenialReason::NotApproved => String::from("gate: not_approved"),
        _ => String::from("gate: denied"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::ExecError;
    use super::{FenceControl, read, report_line};

    /// `LARRY_FENCE_CONTROL` reads one typed host action: revoke, advance or
    /// authenticate at the named epoch, and nothing else (larry#215, ADR 0121).
    #[test]
    fn fence_control_reads_revoke_advance_and_authenticate() {
        let control = |value: &str| {
            let owned = String::from(value);
            move |name: &str| (name == "LARRY_FENCE_CONTROL").then(|| owned.clone())
        };
        assert!(matches!(
            read(&control("revoke")),
            Ok(Some(FenceControl::Revoke))
        ));
        assert!(matches!(
            read(&control("advance")),
            Ok(Some(FenceControl::Advance))
        ));
        let epoch = read(&control("authenticate:7"));
        assert!(matches!(
            epoch,
            Ok(Some(FenceControl::Authenticate(at))) if at.value() == 7
        ));
        let absent = |_: &str| None;
        assert!(matches!(read(&absent), Ok(None)));
    }

    /// A word the host does not know is a typed refusal naming the value, so
    /// a typo fails closed before any admission (larry#215, ADR 0121).
    #[test]
    fn fence_control_refuses_an_unknown_word_typed() {
        let control =
            |name: &str| (name == "LARRY_FENCE_CONTROL").then(|| String::from("explode:now"));
        assert!(matches!(
            read(&control),
            Err(ExecError::BadFenceControl { .. })
        ));
    }

    /// A tick that ran nothing still seals a line: the tests pin the fence
    /// arm here so the binary journey below can rely on its wording.
    #[test]
    fn report_line_names_a_bare_denial_last() {
        let report = larry_runtime::runtime::TickReport::default();
        assert_eq!(report_line(&report), "gate: denied");
    }
}
