//! Adversarial fault injection at the gate (larry#575): every spelling of the host's fence
//! action, every denial reason a real tick records, and the task owner's refusal to run its body
//! twice. Each answer is asserted as its exact typed value or line.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use larry_contracts::authority::{BudgetUnit, Scope, SecurityEpoch};
use larry_contracts::binding::{BindingError, BindingField, EffectTarget, ResolvedEffect};
use larry_contracts::command::{Command, CommandContent, CommandKind};
use larry_contracts::effect::{EffectIntent, EffectOutcome, ReconciliationStrategy};
use larry_contracts::error::OrderingError;
use larry_contracts::ids::{CommandId, EffectId, OperationId, RunId, TenantId, WorkerId};
use larry_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};
use larry_runtime::error::RuntimeError;
use larry_runtime::owner::EffectOwner;
use larry_runtime::payload::WorkProposal;
use larry_runtime::projection::FoldProjection;
use larry_runtime::protocol::{ProtocolCapability, ProtocolOffer};
use larry_runtime::runtime::{Runtime, RuntimeSettings};
use lgwks_bot::kernel::cap::{Auth, Cap};
use lgwks_bot::kernel::clock::Clock;
use lgwks_bot::kernel::gate::GrantSet;
use lgwks_macros::Error;
use lgwks_std::seeded::Seeded;

use super::super::owner::TaskOwner;
use crate::executor::fence_control::{FenceControl, read, report_line};
use crate::executor::receipt::seeded_key;
use crate::executor::{ExecError, Report};

/// What every simulation here returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The instant the scenarios run at, in seconds since 1970.
const NOW: u64 = 1_700_000_000;

/// What the host's fence action reads as, spelled for comparison.
fn spelled(value: Option<&str>) -> String {
    let owned = value.map(str::to_owned);
    let environment = move |name: &str| owned.clone().filter(|_| name == "LARRY_FENCE_CONTROL");
    match read(&environment) {
        Ok(None) => "none".to_owned(),
        Ok(Some(FenceControl::Revoke)) => "revoke".to_owned(),
        Ok(Some(FenceControl::Advance)) => "advance".to_owned(),
        Ok(Some(FenceControl::Authenticate(epoch))) => format!("authenticate {}", epoch.value()),
        Err(ExecError::BadFenceControl { value, cause }) => format!("refused {value:?}: {cause}"),
        Err(other) => format!("another error: {other}"),
    }
}

/// Every spelling the host may set: the empty argument of revoke and advance is accepted, and
/// anything else fails closed naming the value and the cause (CWE-20, larry#215, ADR 0121).
#[test]
fn sim_every_spelling_of_the_fence_action_reads_or_fails_closed() {
    const EXPECTED: &str = "expected revoke, advance or authenticate:<epoch>";
    for (value, want) in [
        ("revoke", "revoke".to_owned()),
        ("advance", "advance".to_owned()),
        ("revoke:", "revoke".to_owned()),
        ("advance:", "advance".to_owned()),
        ("authenticate:0", "authenticate 0".to_owned()),
        ("", format!("refused \"\": {EXPECTED}")),
        ("Revoke", format!("refused \"Revoke\": {EXPECTED}")),
        ("revoke:now", format!("refused \"revoke:now\": {EXPECTED}")),
        ("pause:1", format!("refused \"pause:1\": {EXPECTED}")),
        (
            "authenticate:-1",
            "refused \"authenticate:-1\": invalid digit found in string".to_owned(),
        ),
        (
            "authenticate:",
            "refused \"authenticate:\": cannot parse integer from empty string".to_owned(),
        ),
        (
            "authenticate:18446744073709551616",
            "refused \"authenticate:18446744073709551616\": number too large to fit in target type"
                .to_owned(),
        ),
    ] {
        assert_eq!(spelled(Some(value)), want, "{value:?}");
    }
    assert_eq!(spelled(None), "none", "an unset action applies nothing");
}

/// One admission the gate must deny: what the policy and the capabilities cover, how long the
/// authority lasts, how much budget is left and in what unit the proposal asks.
struct Denial {
    /// Scopes the policy allows.
    policy: &'static [Scope],
    /// Scopes a capability covers.
    capabilities: &'static [Scope],
    /// The instant the authority ends, in seconds since 1970.
    valid_until: u64,
    /// Requests left in the budget.
    budget: u64,
    /// The unit the proposal asks in.
    unit: BudgetUnit,
}

/// Why one denial scenario could not be driven to its tick.
#[derive(Debug, Error)]
enum DenialSimError {
    /// A fixture instant left the range an instant holds.
    #[error("a fixture instant overflowed")]
    Overflow,
    /// The proposal could not be encoded.
    #[error("the proposal was not encoded: {0}")]
    Encode(#[from] lgwks_std::json::Error),
    /// The command was refused before submission.
    #[error("the command was refused: {0}")]
    Command(#[from] OrderingError),
    /// The runtime refused a step.
    #[error("the runtime refused: {0}")]
    Runtime(#[from] RuntimeError),
}

/// The denial line a real tick over `case` records.
fn denied_line(case: &Denial) -> Result<String, DenialSimError> {
    let scopes = |list: &[Scope]| -> BTreeSet<Scope> { list.iter().copied().collect() };
    let valid_until = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(case.valid_until))
        .ok_or(DenialSimError::Overflow)?;
    let settings = RuntimeSettings::with_defaults(
        PolicySnapshot::new(1, scopes(case.policy)),
        CapabilitySnapshot::new(scopes(case.capabilities)),
        ValiditySnapshot::new(valid_until),
        BudgetUnit::Requests,
        case.budget,
    );
    let tenant = TenantId::from_bytes([9; 16]);
    let clock = Clock::virtual_at(Duration::from_secs(NOW));
    let mut runtime: Runtime<FoldProjection, _> = Runtime::new(settings, Unseen, clock)?;
    let proposal = WorkProposal::new(
        RunId::from_bytes([9; 16]),
        tenant,
        OperationId::from_bytes([9; 16]),
        scopes(&[Scope::FileWrite]),
        1,
        case.unit,
        ReconciliationStrategy::RequiresHuman,
    );
    let command = Command::new(
        CommandId::from_bytes([9; 16]),
        tenant,
        Vec::new(),
        CommandContent::new(
            CommandKind::Submit,
            lgwks_std::json::to_vec(&proposal)?.into_boxed_slice(),
        ),
    )?;
    let offer = ProtocolOffer::new(
        tenant,
        runtime.settings().protocol_range(),
        BTreeSet::from([ProtocolCapability::SubmitCommand]),
        true,
        false,
    )
    .authenticated(WorkerId::from_bytes([7; 16]));
    runtime.submit_protocol(&offer, command, None)?;
    let tick = runtime.tick()?;
    assert!(tick.authorized().is_empty(), "a denied command ran");
    Ok(report_line(&tick))
}

/// An owner that binds no target, so a command needs no display, and that a denied command
/// never reaches: every reach is refused.
struct Unseen;

impl EffectOwner for Unseen {
    fn resolve(&mut self, _intent: &EffectIntent) -> Result<Option<ResolvedEffect>, BindingError> {
        Ok(None)
    }

    fn dispatch(
        &mut self,
        _intent: &EffectIntent,
        _authority: &Auth,
        _required: &[Cap],
        _at: SystemTime,
    ) -> Result<EffectOutcome, BindingError> {
        Err(BindingError::UnsupportedOwner)
    }

    fn reconcile(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
        EffectOutcome::Unknown {
            operation: intent.operation,
        }
    }
}

/// Every reason a tick records for running nothing is sealed as its own line naming the scope,
/// never a bare denial (CWE-778, larry#215, ADR 0121).
#[test]
fn sim_every_recorded_denial_reason_is_one_named_line() -> TestResult {
    const BOTH: &[Scope] = &[Scope::FileRead, Scope::FileWrite];
    const READ: &[Scope] = &[Scope::FileRead];
    let allowed = || Denial {
        policy: BOTH,
        capabilities: BOTH,
        valid_until: NOW + 3_600,
        budget: 10,
        unit: BudgetUnit::Requests,
    };
    for (case, want) in [
        (
            Denial {
                policy: READ,
                ..allowed()
            },
            "gate: policy_refuses: file.write",
        ),
        (
            Denial {
                capabilities: READ,
                ..allowed()
            },
            "gate: capability_missing: file.write",
        ),
        (
            Denial {
                valid_until: NOW - 3_600,
                ..allowed()
            },
            "gate: authority_expired",
        ),
        (
            Denial {
                budget: 0,
                ..allowed()
            },
            "gate: budget_exhausted",
        ),
        (
            Denial {
                unit: BudgetUnit::Tokens,
                ..allowed()
            },
            "gate: unit_mismatch",
        ),
    ] {
        assert_eq!(denied_line(&case)?, want);
    }
    Ok(())
}

/// An intent the task owner is asked about, its identity drawn from one byte.
fn intent(byte: u8, tenant: TenantId) -> EffectIntent {
    EffectIntent::new(
        EffectId::from_bytes([byte; 16]),
        OperationId::from_bytes([byte; 16]),
        RunId::from_bytes([byte; 16]),
        tenant,
        ReconciliationStrategy::RequiresHuman,
        SecurityEpoch::INITIAL,
    )
}

/// A task body that counts its runs and succeeds or fails as asked. One closure type serves
/// every owner here, so one instantiation of the owner is driven through every arm.
fn body(succeeds: bool, runs: Rc<Cell<u32>>) -> impl FnOnce() -> Result<Report, ExecError> {
    move || {
        runs.set(runs.get().saturating_add(1));
        if !succeeds {
            return Err(ExecError::PathUnset);
        }
        let key = seeded_key(&mut Seeded::from_seed(0x5EED_0000_0000_0061));
        Ok(Report::sealed(lgwks_std::json::json!({}), &key, 1))
    }
}

/// The task body runs at most once and only for its own tenant: a foreign tenant is refused
/// with nothing run, the first reach records success or failure as the body answered, and a
/// second reach, a reach with no body left and a reconciliation each answer unknown, never a
/// second run or an invented success (CWE-639, CWE-837, larry#76).
#[test]
fn sim_a_task_body_never_runs_twice_and_unseen_reaches_are_unknown() -> TestResult {
    let tenant = TenantId::from_bytes([4; 16]);
    let granted = Cap::new(String::from("larry.owner"));
    let proof = GrantSet::empty()
        .grant(granted.clone())
        .issue(std::slice::from_ref(&granted))?;
    let required = [granted];
    let at = SystemTime::UNIX_EPOCH;
    for succeeds in [true, false] {
        let runs = Rc::new(Cell::new(0_u32));
        let slot = Rc::new(RefCell::new(None));
        let mut owner = TaskOwner {
            repo: Path::new("."),
            tenant,
            epoch: SecurityEpoch::INITIAL,
            task_bytes: b"task",
            run: Some(body(succeeds, Rc::clone(&runs))),
            slot: Rc::clone(&slot),
        };
        let asked = intent(4, tenant);
        let operation = asked.operation;
        let unknown = EffectOutcome::Unknown { operation };
        assert_eq!(
            owner.dispatch_bound(
                &intent(4, TenantId::from_bytes([5; 16])),
                &proof,
                &required,
                at
            ),
            Err(BindingError::Changed {
                field: BindingField::Command
            })
        );
        assert_eq!(runs.get(), 0, "a foreign tenant runs nothing");
        let finished = if succeeds {
            EffectOutcome::Succeeded { operation }
        } else {
            EffectOutcome::Failed { operation }
        };
        assert_eq!(
            owner.dispatch_bound(&asked, &proof, &required, at),
            Ok(finished)
        );
        assert_eq!(
            slot.borrow().as_ref().map(Result::is_ok),
            Some(succeeds),
            "the body's own answer is kept"
        );
        assert_eq!(
            owner.dispatch_bound(&asked, &proof, &required, at),
            Ok(unknown)
        );
        slot.borrow_mut().take();
        assert_eq!(
            owner.dispatch_bound(&asked, &proof, &required, at),
            Ok(unknown),
            "with no body left there is nothing to run"
        );
        assert_eq!(owner.reconcile(&asked, at), unknown);
        assert!(owner.resolve(&asked)?.is_some(), "a real folder binds");
        assert_eq!(runs.get(), 1, "the body ran once");
        assert!(slot.borrow().is_none(), "no second result was recorded");
    }
    Ok(())
}

/// A repository path that cannot be canonicalized binds to its raw bytes, the same on every
/// call, so the effect's identity never changes between admission and reach (CWE-367, ADR 0019).
#[test]
fn sim_a_repository_that_cannot_be_resolved_binds_to_its_own_bytes() -> TestResult {
    let tenant = TenantId::from_bytes([6; 16]);
    let repo = Path::new("/larry/no/such/repository");
    let raw = repo.as_os_str().as_encoded_bytes();
    let mut owner = TaskOwner {
        repo,
        tenant,
        epoch: SecurityEpoch::new(3),
        task_bytes: b"task",
        run: Some(body(false, Rc::new(Cell::new(0)))),
        slot: Rc::new(RefCell::new(None)),
    };
    let want = ResolvedEffect::new(
        EffectTarget::new(
            raw,
            raw,
            b"",
            tenant.as_bytes(),
            b"task",
            b"larry.executor/v1",
        )?,
        1,
        SecurityEpoch::new(3),
    );
    let asked = intent(6, tenant);
    assert_eq!(owner.resolve(&asked), Ok(Some(want)));
    assert_eq!(owner.resolve(&asked), Ok(Some(want)));
    Ok(())
}
