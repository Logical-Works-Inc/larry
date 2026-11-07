//! Falsifiers for E71 after the fact: restore, tenant attribution, the reservation a fenced effect keeps,
//! answers about effects never sent, the zero bound and the host's control surface (R87, D48, E63).
//!
//! Every assertion inspects a real value; the owner is the scripted fixture double.

mod fence_kit;
mod support;

use larry_contracts::authority::{Scope, SecurityEpoch};
use larry_contracts::command::{Command, CommandContent, CommandKind};
use larry_contracts::effect::{EffectOutcome, MAX_EFFECT_ATTEMPTS, ReconciliationStrategy};
use larry_contracts::ids::{RunId, TenantId};
use larry_contracts::state::RunState;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::projection::{EffectPhase, ProjectionBackend};
use larry_runtime::runtime::restore;
use larry_runtime::{FenceError, FenceRefusal, RevocationFence};

use fence_kit::{open_retry_safe_effect, runtime_at, settings_at, work, work_in};
use support::{
    TestRuntime, assert_state, command_id, operation, proposal, run, runtime, runtime_with,
    settings, succeeded, tenant,
};

/// The tenant a run belongs to is the one the runtime recorded for that run,
/// never the one the proposal body declares, so a hostile generator cannot
/// record an effect — or re-attribute a run after restore — to a tenant it
/// does not own (D42, E63).
///
/// This is the same rule as the admission epoch below: the body's `tenant`
/// field is a declaration like any other, and the runtime consults its own
/// run table instead. The envelope tenant and the body's claim are different
/// here on purpose, which is the only way the two sources can be told apart.
#[test]
fn e63_a_proposal_cannot_declare_its_own_tenant() -> Result<(), Box<dyn std::error::Error>> {
    // The envelope says tenant 1. The body claims tenant 2.
    let body = proposal(
        run(1),
        tenant(2),
        operation(1),
        &[Scope::FileRead],
        5,
        ReconciliationStrategy::Queryable,
    );
    let payload = lgwks_std::json::to_vec(&body)?;
    let envelope = Command::new(
        command_id(1),
        tenant(1),
        Vec::new(),
        CommandContent::new(CommandKind::Submit, payload.into_boxed_slice()),
    )?;

    let mut rt = runtime_with(
        ScriptedOwner::scripted([succeeded(operation(1))]),
        &[Scope::FileRead],
        &[Scope::FileRead],
        100,
    )?;
    rt.submit(envelope)?;
    let tick = rt.tick()?;
    assert_eq!(tick.authorized(), &[operation(1)]);

    assert_eq!(
        rt.projection()
            .intents()
            .first()
            .map(|intent| intent.tenant),
        Some(tenant(1)),
        "the durable intent carries the run's recorded tenant, not the body's claim"
    );
    let receipts = rt.projection().receipts();
    assert_eq!(receipts.len(), 1);
    assert_eq!(
        receipts
            .first()
            .map(larry_contracts::effect::TerminalReceipt::tenant),
        Some(tenant(1))
    );

    // A restart must not re-attribute the run to the declared tenant: the
    // restored run table is the same table the journal recorded.
    let journal = rt.into_journal();
    let restored: TestRuntime = restore(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        ScriptedOwner::new(),
        crate::support::clock()?,
        journal,
    )?;
    let receipt = larry_contracts::effect::TerminalReceipt::publish(
        operation(3),
        run(1),
        restored
            .projection()
            .run_tenant(run(1))
            .ok_or("the restored run must carry a tenant")?,
        larry_contracts::effect::TerminalDisposition::Succeeded,
    );
    assert_eq!(
        receipt.tenant(),
        tenant(1),
        "the restored run table attributes the run to the tenant the journal recorded"
    );
    Ok(())
}

/// A restored runtime brings each effect's admission epoch back with it, so a
/// host whose authority has moved on recovers with those effects already fenced
/// instead of re-dispatching them on a decision that no longer stands
/// (D48, E71). Restoring at the *same* generation is the differential: the
/// epoch fences what it withdrew, not everything.
#[test]
fn e71_restore_fences_only_the_effects_the_new_generation_left_behind()
-> Result<(), Box<dyn std::error::Error>> {
    let reopened = |at_epoch: SecurityEpoch| -> Result<TestRuntime, Box<dyn std::error::Error>> {
        let (original, _) = open_retry_safe_effect(64)?;
        Ok(restore(
            settings_at(at_epoch, 64)?,
            ScriptedOwner::new(),
            crate::support::clock()?,
            original.into_journal(),
        )?)
    };

    let mut advanced = reopened(SecurityEpoch::new(1))?;
    assert_eq!(advanced.fence().epoch(), SecurityEpoch::new(1));
    let tick = advanced.tick()?;
    assert_eq!(
        tick.fenced(),
        &[(
            operation(1),
            FenceRefusal::StaleEpoch {
                admitted: SecurityEpoch::INITIAL,
                current: SecurityEpoch::new(1)
            }
        )],
        "the recovered effect is fenced, not re-dispatched"
    );
    assert_eq!(advanced.owner().dispatched(), &[]);

    let mut same = reopened(SecurityEpoch::INITIAL)?;
    let unrestored_tick = same.tick()?;
    assert!(
        unrestored_tick.fenced().is_empty(),
        "the same generation does not fence an effect admitted under it"
    );
    assert_eq!(
        same.owner().dispatched(),
        &[operation(1)],
        "the retry proceeds, because nothing withdrew its authority"
    );
    Ok(())
}

/// A fenced effect has already reserved its budget, and this slice releases
/// nothing. The direction is deliberate — a reservation that is not released
/// fails closed — and it is pinned here so a later work package has to change
/// this test to change the behaviour.
///
/// It also pins the consequence that follows from the closed event vocabulary:
/// the journal cannot record "reserved but never dispatched", so a fenced
/// first dispatch is *indistinguishable from a crash in the dispatch window*
/// and the runtime treats it as such — it queries, and never re-dispatches an
/// effect whose strategy is not retry-safe.
#[test]
fn e71_a_fenced_effect_keeps_its_reservation_and_that_is_a_known_limit()
-> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime()?;
    assert_eq!(rt.remaining_budget(), 100);
    rt.submit(work(1, operation(1), ReconciliationStrategy::Queryable)?)?;
    assert!(rt.fence_mut().revoke(operation(1)));
    let tick = rt.tick()?;

    assert_eq!(tick.fenced().len(), 1);
    assert_eq!(
        rt.remaining_budget(),
        95,
        "budget is reserved at admission, before the fence runs, so a fenced \
         effect holds its reservation; releasing it would need a journal fact \
         the closed vocabulary does not have"
    );
    assert_eq!(
        rt.projection().effect_phase(operation(1)),
        Some(EffectPhase::Reserved),
        "the reservation is conservative, not lost: the intent is durable"
    );

    // The ambiguity is resolved the only way the vocabulary allows: ask. A
    // query cannot cause the change the fence refused, so containment holds
    // whether or not the effect had actually reached the endpoint.
    let next = rt.tick()?;
    assert_eq!(
        next.reconciled(),
        &[operation(1)],
        "the reserved effect is resolved by querying, and the tick says so"
    );
    assert!(
        next.fenced().is_empty(),
        "an observation is not a reach the fence gates"
    );
    assert_eq!(
        rt.owner().reconciled(),
        &[operation(1)],
        "the reserved effect is queried, not re-dispatched"
    );
    assert_eq!(
        rt.owner().dispatched(),
        &[],
        "the revoked reach is never taken"
    );
    assert_eq!(
        rt.projection().effect_phase(operation(1)),
        Some(EffectPhase::Observed {
            outcome: EffectOutcome::Unknown {
                operation: operation(1)
            }
        })
    );
    Ok(())
}

/// An endpoint's answer is evidence about an effect this runtime actually
/// sent, and about nothing else. An effect whose only reach was refused never
/// left the process, so the status answer that follows — `Succeeded` here,
/// which is the worst case — cannot resolve it, cannot publish a receipt and
/// cannot carry the run to `Completed`.
///
/// The journal cannot tell a refused reach from a crash inside the dispatch
/// window, and this test exists because the *runtime* can: a refused reach
/// calls no owner, so the live effect table still reads "never sent", while a
/// restored reservation is restored as possibly-sent and keeps its full
/// treatment. Reading the endpoint's answer as truth here would manufacture
/// success for an effect that provably never happened (D42, R88).
#[test]
fn e71_an_observation_cannot_resolve_an_effect_that_was_never_sent()
-> Result<(), Box<dyn std::error::Error>> {
    // The offline bound is spent, so the only reach this effect ever gets is
    // the refused one: `dispatch` is never called on the owner.
    // The endpoint answers "succeeded" to the status query.
    let mut rt = runtime_at(
        SecurityEpoch::INITIAL,
        0,
        ScriptedOwner::scripted([succeeded(operation(1))]),
    )?;
    rt.submit(work(1, operation(1), ReconciliationStrategy::Queryable)?)?;

    let refused = rt.tick()?;
    assert_eq!(
        refused.fenced(),
        &[(
            operation(1),
            FenceRefusal::OfflineExposureExhausted { bound: 0 }
        )]
    );
    assert_eq!(
        rt.owner().dispatched(),
        &[],
        "the mutating reach was refused, so the effect never left the process"
    );
    assert_eq!(
        rt.projection().effect_phase(operation(1)),
        Some(EffectPhase::Reserved)
    );

    // The next tick queries the endpoint. The query itself is the
    // already-dispatched treatment and stays open; what it may not do is
    // become the effect's disposition.
    let next = rt.tick()?;
    assert_eq!(next.reconciled(), &[operation(1)]);
    assert_eq!(
        rt.owner().reconciled(),
        &[operation(1)],
        "the status query is still sent, so the refusal stays visible"
    );
    assert_eq!(
        rt.owner().dispatched(),
        &[],
        "and the dispatch still never happens"
    );
    assert_eq!(next.receipts(), &[], "no reach earned a receipt");
    assert_eq!(
        next.receipts().len(),
        0,
        "an effect that never left this process is not receipted as a success"
    );
    assert_state(rt.projection(), run(1), RunState::Reconciling)?;
    assert_eq!(
        rt.projection().effect_phase(operation(1)),
        Some(EffectPhase::Observed {
            outcome: succeeded(operation(1))
        }),
        "the answer is recorded as what the owner said, and resolves nothing"
    );

    // It stays open and reported for as long as the reconciliation budget
    // allows, so a host that leaves the fence shut watches the effect stay
    // unresolved rather than lose sight of it.
    for rest in 1..MAX_EFFECT_ATTEMPTS {
        let tick = rt.tick()?;
        assert_eq!(tick.reconciled(), &[operation(1)], "query {rest}");
        assert_eq!(tick.receipts(), &[]);
    }
    assert_eq!(
        rt.owner().reconciled().len(),
        usize::from(MAX_EFFECT_ATTEMPTS),
        "the query budget is spent and bounded, not unbounded"
    );

    // Budget spent: quiet, and still unresolved rather than resolved wrongly.
    let quiet = rt.tick()?;
    assert_eq!(quiet.reconciled(), &[]);
    assert_eq!(quiet.receipts(), &[]);
    assert_eq!(
        rt.projection().receipts().len(),
        0,
        "no tick in the effect's lifetime publishes a receipt for it"
    );
    assert_state(rt.projection(), run(1), RunState::Reconciling)?;
    Ok(())
}

/// A zero bound is a kill switch: no reach is permitted, and authentication
/// does not lift it, because the bound is a ceiling on reaches between
/// authentications and not a credit that authentication hands out.
#[test]
fn e71_a_zero_bound_stops_every_effect_permanently() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime_at(SecurityEpoch::INITIAL, 0, ScriptedOwner::new())?;
    rt.submit(work(1, operation(1), ReconciliationStrategy::Queryable)?)?;
    let refused = rt.tick()?;
    assert_eq!(
        refused.fenced(),
        &[(
            operation(1),
            FenceRefusal::OfflineExposureExhausted { bound: 0 }
        )]
    );

    assert_eq!(
        rt.fence_mut().authenticate(SecurityEpoch::INITIAL),
        SecurityEpoch::INITIAL
    );
    assert_eq!(
        rt.fence().offline_spent(),
        0,
        "authentication cleared the count"
    );
    rt.submit(work_in(
        2,
        2,
        operation(2),
        ReconciliationStrategy::Queryable,
    )?)?;
    let again = rt.tick()?;
    assert_eq!(
        again.fenced(),
        &[(
            operation(2),
            FenceRefusal::OfflineExposureExhausted { bound: 0 }
        )],
        "and the reach is still refused: zero is a ceiling, not a credit"
    );
    assert_eq!(rt.owner().dispatched(), &[]);
    Ok(())
}

/// The fence can only ever refuse. An effect the reducer denied is denied
/// whether the fence is open or closed, so the two checks cannot be traded
/// against each other — the freshness check is not a second way in.
#[test]
fn the_fence_cannot_authorize_what_the_reducer_denied() -> Result<(), Box<dyn std::error::Error>> {
    // The policy allows nothing, so the gate denies before the fence is
    // consulted at all, even though the fence is wide open.
    let mut rt = runtime_with(ScriptedOwner::new(), &[], &[Scope::FileRead], 100)?;
    rt.submit(work(1, operation(1), ReconciliationStrategy::Queryable)?)?;
    let tick = rt.tick()?;
    assert_eq!(tick.denied().len(), 1);
    assert!(
        tick.fenced().is_empty(),
        "a denied effect never reaches the fence"
    );
    assert_eq!(rt.owner().dispatched(), &[]);
    Ok(())
}

/// A run whose effect was fenced neither silently completes nor silently fails.
#[test]
fn a_fenced_effect_leaves_the_run_visibly_unfinished() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime()?;
    rt.submit(work(
        1,
        operation(1),
        ReconciliationStrategy::RequiresHuman,
    )?)?;
    assert!(rt.fence_mut().revoke(operation(1)));
    rt.tick()?;
    let state = rt.projection().run_state(run(1));
    assert_eq!(state, Some(RunState::Reconciling));
    assert!(!state.is_some_and(RunState::is_terminal));
    assert!(rt.projection().receipts().is_empty());
    Ok(())
}

/// Epoch exhaustion refuses rather than wrapping, because a wrapped epoch would
/// alias a withdrawn generation with a live one.
#[test]
fn e71_epoch_exhaustion_refuses_at_the_runtime_boundary() -> Result<(), Box<dyn std::error::Error>>
{
    let mut fence = RevocationFence::new(SecurityEpoch::new(u64::MAX), 64);
    assert_eq!(fence.advance_epoch(), Err(FenceError::EpochExhausted));
    assert_eq!(fence.epoch(), SecurityEpoch::new(u64::MAX));
    Ok(())
}

/// The host surface is the withdrawal and the refresh; the runtime's own
/// mutating operations on the fence are crate-internal, so a caller cannot
/// pre-clear a reach or assert that a dispatch happened.
#[test]
fn the_host_control_surface_is_withdrawal_refresh_and_reading()
-> Result<(), Box<dyn std::error::Error>> {
    let mut rt = runtime()?;
    let fence: &mut RevocationFence = rt.fence_mut();
    assert!(!fence.is_revoked(operation(9)));
    assert!(fence.revoke(operation(9)));
    assert!(fence.is_revoked(operation(9)));
    assert_eq!(fence.offline_bound(), larry_runtime::MAX_OFFLINE_DISPATCHES);
    assert_eq!(fence.offline_spent(), 0);
    assert_eq!(fence.epoch(), SecurityEpoch::INITIAL);
    assert_eq!(
        fence.authenticate(SecurityEpoch::new(2)),
        SecurityEpoch::new(2)
    );
    assert_eq!(fence.advance_epoch()?, SecurityEpoch::new(3));
    // Refusals and epoch failures render as sentences, not as Debug output.
    assert_eq!(
        FenceRefusal::Revoked {
            operation: operation(9)
        }
        .to_string(),
        format!("operation {} was revoked by name", operation(9))
    );
    assert_eq!(
        FenceError::EpochExhausted.to_string(),
        "security epoch space is exhausted; it never wraps"
    );
    // The tenant and run of the frozen effect are untouched by any of this.
    assert_eq!(TenantId::from_bytes([1; 16]), tenant(1));
    assert_eq!(RunId::from_bytes([1; 16]), run(1));
    Ok(())
}
