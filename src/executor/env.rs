//! The operator's declared environment as typed values (larry#215, ADR 0121).
//!
//! Everything here is read from the process environment or derived from what
//! it names: the policy, the one host fence action, and the runtime settings
//! one task runs under. The gate (`super::gate`) decides and drives; this
//! module only reads what the operator declared, so a second caller of a
//! setting finds one owner and one parsing rule.

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use larry_contracts::authority::{BudgetUnit, Scope};
use larry_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};
use larry_runtime::error::RuntimeError;
use larry_runtime::runtime::RuntimeSettings;

use super::ExecError;

/// The prefix every policy revision digest carries, so a revision computed here
/// can never collide with a digest computed for another purpose.
const POLICY_REVISION_TAG: &[u8] = b"larry/executor/policy-revision/v1\0";

/// The request each task costs against its one-run budget.
pub(super) const TASK_COST: u64 = 1;

/// The scopes the executor is capable of. The capability snapshot pins them,
/// so the policy alone decides what a task may do.
pub(super) const EXECUTOR_CAPABILITIES: [Scope; 5] = [
    Scope::FileRead,
    Scope::FileWrite,
    Scope::ProcessSpawn,
    Scope::ModelInvoke,
    Scope::ModelCommand,
];

/// The default policy excludes unconstrained model commands (larry#93).
const DEFAULT_POLICY: [Scope; 4] = [
    Scope::FileRead,
    Scope::FileWrite,
    Scope::ProcessSpawn,
    Scope::ModelInvoke,
];

/// The policy when `LARRY_POLICY` is unset.
pub(super) fn default_policy() -> BTreeSet<Scope> {
    DEFAULT_POLICY.iter().copied().collect()
}

/// The prefix the estate capability names carry and a policy name omits.
const CAPABILITY_PREFIX: &str = "larry.";

/// The tenants one executor state directory serves: its own, because the
/// tenant is derived from the directory (larry#182). Every shared runtime cap
/// is whole to that one tenant, and the shipped path names the count rather
/// than leaving a default nothing reached (larry#333, SD-1).
const SERVED_TENANTS: usize = 1;

/// The runtime settings one task runs under: the revision of the policy record
/// it runs under, the executor's capabilities, one request, and the runtime's
/// own default lease.
///
/// The revision is derived from the record itself, so the same `LARRY_POLICY`
/// always names the same revision and a changed policy names a different one,
/// rather than a constant only this build knows (larry#212).
pub(super) fn task_settings(policy: &BTreeSet<Scope>) -> RuntimeSettings {
    let mut settings = RuntimeSettings::with_defaults(
        PolicySnapshot::new(policy_revision(policy), policy.clone()),
        CapabilitySnapshot::new(EXECUTOR_CAPABILITIES.iter().copied().collect()),
        ValiditySnapshot::new(SystemTime::UNIX_EPOCH),
        BudgetUnit::Requests,
        TASK_COST,
    );
    settings.max_tenants = SERVED_TENANTS;
    settings
}

/// The revision of one policy record: a digest over the policy's own scope
/// names, in `Scope` order, so the same admitted set always yields the same
/// revision and a decision is only ever comparable inside one (larry#212).
fn policy_revision(policy: &BTreeSet<Scope>) -> u64 {
    let mut record = String::new();
    for scope in policy {
        record.push_str(&scope_name(*scope));
        record.push('\n');
    }
    let digest =
        larry_contracts::ids::ContentDigest::over(&[POLICY_REVISION_TAG, record.as_bytes()]);
    let mut revision = 0u64;
    for byte in digest.as_bytes().iter().take(8) {
        revision = (revision << 8) | u64::from(*byte);
    }
    revision
}

/// `settings` with its validity pinned to the admission lease that starts at
/// `now`: the lease length is the runtime's own setting, so the executor keeps
/// no clock constant of its own (larry#212).
pub(super) fn with_expiry(
    settings: RuntimeSettings,
    now: SystemTime,
) -> Result<RuntimeSettings, RuntimeError> {
    let mut settings = settings;
    settings.validity = ValiditySnapshot::new(lease_end(now, settings.lease)?);
    Ok(settings)
}

/// `now + lease` as an instant, or the typed refusal when the platform cannot
/// hold the reading.
fn lease_end(now: SystemTime, lease: Duration) -> Result<SystemTime, RuntimeError> {
    let elapsed = match now.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(elapsed) => elapsed,
        Err(_) => Duration::ZERO,
    };
    let total = elapsed.saturating_add(lease);
    SystemTime::UNIX_EPOCH
        .checked_add(total)
        .ok_or(RuntimeError::ClockOutOfRange { elapsed: total })
}

/// The dotted name a scope carries in `LARRY_POLICY` and in a denial line: the
/// estate capability name `replay::capability` gives it (larry#77), without
/// its `larry.` prefix, so the scope-to-name mapping has one owner.
pub(super) fn scope_name(scope: Scope) -> String {
    let capability = larry_contracts::replay::capability(scope);
    let name = capability.as_str();
    match name.strip_prefix(CAPABILITY_PREFIX) {
        Some(short) => short.to_owned(),
        None => name.to_owned(),
    }
}

/// The scope a policy name admits, or `None` for a name that is not one: the
/// executor's own scopes plus network egress, which a policy may name though
/// no executor task needs it.
fn named_scope(name: &str) -> Option<Scope> {
    EXECUTOR_CAPABILITIES
        .iter()
        .copied()
        .chain(core::iter::once(Scope::NetworkEgress))
        .find(|scope| scope_name(*scope) == name)
}

/// The `LARRY_POLICY` scopes `read` yields: unset is the four scopes the
/// executor uses, the empty string is no scope, names are trimmed and
/// comma-separated, and an unknown name is a typed refusal.
pub(super) fn policy(read: &impl Fn(&str) -> Option<String>) -> Result<BTreeSet<Scope>, ExecError> {
    let Some(text) = read("LARRY_POLICY") else {
        return Ok(default_policy());
    };
    let mut scopes = BTreeSet::new();
    for name in text.split(',') {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        match named_scope(name) {
            Some(scope) => {
                scopes.insert(scope);
            }
            None => {
                return Err(ExecError::BadPolicy {
                    value: name.to_owned(),
                });
            }
        }
    }
    Ok(scopes)
}

/// Reads the operator's one host fence action. The action is applied only
/// after this task is durably admitted and names no test-only bypass.
pub(super) fn fence_control(
    read: &impl Fn(&str) -> Option<String>,
) -> Result<Option<super::FenceControl>, ExecError> {
    let Some(value) = read("LARRY_FENCE_CONTROL") else {
        return Ok(None);
    };
    let Some((kind, argument)) = value.split_once(':') else {
        return match value.as_str() {
            "revoke" => Ok(Some(super::FenceControl::Revoke)),
            "advance" => Ok(Some(super::FenceControl::Advance)),
            _ => Err(ExecError::BadFenceControl {
                value,
                cause: String::from("expected revoke, advance or authenticate:<epoch>"),
            }),
        };
    };
    match (kind, argument) {
        ("revoke", "") => Ok(Some(super::FenceControl::Revoke)),
        ("advance", "") => Ok(Some(super::FenceControl::Advance)),
        ("authenticate", epoch) => epoch
            .parse::<u64>()
            .map(|value| {
                Some(super::FenceControl::Authenticate(
                    larry_contracts::authority::SecurityEpoch::new(value),
                ))
            })
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use larry_contracts::authority::Scope;

    use super::super::{ExecError, FenceControl};
    use super::{
        SERVED_TENANTS, default_policy, fence_control, policy_revision, task_settings, with_expiry,
    };

    /// The executor's lease and the runtime's tick are one timeline: the live
    /// clock the executor passes reads after 2023, not 1970, and the validity
    /// the runtime is built with sits exactly one configured lease after that
    /// reading (larry#182, larry#212).
    #[test]
    fn the_executors_lease_and_the_runtimes_tick_share_one_timeline()
    -> Result<(), Box<dyn std::error::Error>> {
        let clock = larry_runtime::live_clock()?;
        let tick = SystemTime::UNIX_EPOCH
            .checked_add(clock.now())
            .ok_or("tick instant overflow")?;
        let anchored = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(1_700_000_000))
            .ok_or("anchor instant overflow")?;
        assert!(
            tick > anchored,
            "the live clock is anchored after 2023, not at 1970: {tick:?}"
        );
        let run_settings = with_expiry(task_settings(&default_policy()), tick)?;
        let gap = run_settings.validity.valid_until.duration_since(tick)?;
        assert_eq!(
            gap, run_settings.lease,
            "the validity sits one configured lease after the tick"
        );
        Ok(())
    }

    /// The executor's settings name the tenants one state directory serves, so
    /// the runtime's shared caps divide by a value the shipped task path sets,
    /// not by a default nothing reached (larry#333, SD-1).
    #[test]
    fn task_settings_name_the_tenants_the_executor_serves() -> Result<(), Box<dyn std::error::Error>>
    {
        let settings = task_settings(&default_policy());
        assert_eq!(
            settings.max_tenants, SERVED_TENANTS,
            "the task settings carry the tenant count"
        );
        assert_eq!(SERVED_TENANTS, 1, "one state directory is one tenant");
        let tick = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(1_700_000_000))
            .ok_or("tick instant overflow")?;
        let carried = with_expiry(settings, tick)?;
        assert_eq!(
            carried.max_tenants, 1,
            "the settings a task runs under carry it"
        );
        Ok(())
    }

    /// The policy revision is a fact about the policy record, not a constant:
    /// the same admitted set yields the same revision, and one more scope
    /// yields a different one (larry#212).
    #[test]
    fn task_settings_derive_the_revision_from_the_policy_record() {
        let first = task_settings(&default_policy());
        let again = task_settings(&default_policy());
        assert_eq!(
            first.policy.revision(),
            again.policy.revision(),
            "the same policy record yields the same revision"
        );

        let mut widened = default_policy();
        widened.insert(Scope::NetworkEgress);
        assert_ne!(
            policy_revision(&default_policy()),
            policy_revision(&widened)
        );
    }

    /// `LARRY_FENCE_CONTROL` reads one typed host action: revoke, advance or
    /// authenticate at the named epoch, and nothing else (larry#215, ADR 0121).
    #[test]
    fn fence_control_reads_revoke_advance_and_authenticate() {
        let control = |value: &str| {
            let owned = String::from(value);
            move |name: &str| (name == "LARRY_FENCE_CONTROL").then(|| owned.clone())
        };
        assert!(matches!(
            fence_control(&control("revoke")),
            Ok(Some(FenceControl::Revoke))
        ));
        assert!(matches!(
            fence_control(&control("advance")),
            Ok(Some(FenceControl::Advance))
        ));
        let epoch = fence_control(&control("authenticate:7"));
        assert!(matches!(
            epoch,
            Ok(Some(FenceControl::Authenticate(at))) if at.value() == 7
        ));
        let absent = |_: &str| None;
        assert!(matches!(fence_control(&absent), Ok(None)));
    }

    /// A word the host does not know is a typed refusal naming the value, so
    /// a typo fails closed before any admission (larry#215, ADR 0121).
    #[test]
    fn fence_control_refuses_an_unknown_word_typed() {
        let control =
            |name: &str| (name == "LARRY_FENCE_CONTROL").then(|| String::from("explode:now"));
        assert!(matches!(
            fence_control(&control),
            Err(ExecError::BadFenceControl { .. })
        ));
    }
}
