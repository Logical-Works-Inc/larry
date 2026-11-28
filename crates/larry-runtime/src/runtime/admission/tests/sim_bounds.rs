//! Cell 05, E28/E30/E65, D48, larry#75/#182/#209: the admission seams the
//! public surface cannot reach — the global pending bound, display evidence
//! on a non-submit command, question-book passthrough, dispatch without a
//! durable intent, zero-identity keys, reaches without a live entry, the
//! memory-journal seam and a lost file acknowledgement.

use std::time::{Duration, SystemTime};

use larry_contracts::authority::{BudgetUnit, Scope};
use larry_contracts::binding::BindingError;
use larry_contracts::command::{Command, CommandContent, CommandKind};
use larry_contracts::effect::{EffectIntent, EffectOutcome, ReconciliationStrategy};
use larry_contracts::ids::{CommandId, OperationId, RunId, TenantId};
use lgwks_bot::kernel::broker::Authority;
use lgwks_bot::kernel::cap::Cap;

use super::{
    Ack, Outcome, Rt, START_SECS, clock, command, ids, memory, seeded_submit, settings, work_body,
};
use crate::MAX_READY_WORK;
use crate::error::RuntimeError;
use crate::fence::{FenceRefusal, effect_key};
use crate::journal::Journal;
use crate::owner::fixture::ScriptedOwner;
use crate::projection::FoldProjection;
use crate::runtime::{Runtime, TickItem, TickReport, restore};

// Source: Cell 05 (one global bound beside the tenant shares) and CWE-400.
// The global bound fires only when no tenant is past its share: 257 tenants
// fill the set and the 257th is refused naming 256, writing nothing.
#[test]
fn sim_global_pending_set_refuses_past_its_bound() -> Outcome {
    let mut runtime = memory(ScriptedOwner::new())?;
    for seed in 0..256_u64 {
        let tenant = TenantId::from_bytes(ids(1, seed));
        let body = work_body(
            RunId::from_bytes(ids(2, seed)),
            tenant,
            OperationId::from_bytes(ids(3, seed)),
        );
        runtime.ack(command(
            CommandId::from_bytes(ids(4, seed)),
            CommandKind::Steer,
            &body,
        )?)?;
    }
    let before = runtime.journal().len();
    let tenant = TenantId::from_bytes(ids(1, 257));
    let body = work_body(
        RunId::from_bytes(ids(2, 257)),
        tenant,
        OperationId::from_bytes(ids(3, 257)),
    );
    let refused = runtime.ack(command(
        CommandId::from_bytes(ids(4, 257)),
        CommandKind::Steer,
        &body,
    )?);
    assert!(
        matches!(
            refused,
            Err(RuntimeError::ReadyWorkFull { capacity }) if capacity == MAX_READY_WORK
        ),
        "the 257th admission is refused: {refused:?}"
    );
    assert_eq!(
        runtime.journal().len(),
        before,
        "the refusal writes nothing"
    );
    Ok(())
}

// Source: E65 (a display is only ever made for a submission). Admitting a
// non-submit command with display evidence is refused before any journal
// write, naming the unsupported kind.
#[test]
fn sim_admit_with_display_on_non_submit_is_refused() -> Outcome {
    struct ResolvingOwner;
    impl crate::owner::EffectOwner for ResolvingOwner {
        fn resolve(
            &mut self,
            _intent: &larry_contracts::effect::EffectIntent,
        ) -> Result<
            Option<larry_contracts::binding::ResolvedEffect>,
            larry_contracts::binding::BindingError,
        > {
            let target =
                larry_contracts::binding::EffectTarget::new(b"a", b"b", b"c", b"d", b"e", b"f")?;
            Ok(Some(larry_contracts::binding::ResolvedEffect::new(
                target,
                1,
                larry_contracts::authority::SecurityEpoch::INITIAL,
            )))
        }
        fn reconcile(
            &mut self,
            intent: &larry_contracts::effect::EffectIntent,
            _at: SystemTime,
        ) -> EffectOutcome {
            EffectOutcome::Unknown {
                operation: intent.operation,
            }
        }
    }
    let tenant = TenantId::from_bytes([0xA1; 16]);
    let holder = larry_contracts::ids::WorkerId::from_bytes([0xB2; 16]);
    let mut runtime: Runtime<FoldProjection, ResolvingOwner> =
        Runtime::new(settings()?, ResolvingOwner, clock())?;
    let body = work_body(
        RunId::from_bytes([1; 16]),
        tenant,
        OperationId::from_bytes([1; 16]),
    );
    let submit = command(CommandId::from_bytes([2; 16]), CommandKind::Submit, &body)?;
    let grant = larry_contracts::authority::Grant::new(
        tenant,
        holder,
        [Scope::FileRead].into_iter().collect(),
        larry_contracts::authority::Budget::new(10, BudgetUnit::Requests),
        epoch_plus(3_600)?,
        0,
    )?;
    let shown = runtime.display(&submit, holder, &[grant])?;
    let cancel = command(CommandId::from_bytes([3; 16]), CommandKind::Cancel, &body)?;
    let before = runtime.journal().len();
    let refused = runtime.admit(cancel, Some(shown), None);
    assert!(
        matches!(
            refused,
            Err(RuntimeError::UnsupportedCommand(CommandKind::Cancel))
        ),
        "a display on a non-submit is refused: {refused:?}"
    );
    assert_eq!(
        runtime.journal().len(),
        before,
        "the refusal writes nothing"
    );
    Ok(())
}

// Source: E30 (questions enter only through Ask and Answer). Every other
// command kind carries no question: the check passes it through without
// touching the book or the journal.
#[test]
fn sim_check_question_ignores_non_question_commands() -> Outcome {
    let mut runtime = memory(ScriptedOwner::new())?;
    let tenant = TenantId::from_bytes([0xA3; 16]);
    for kind in [CommandKind::Submit, CommandKind::Steer, CommandKind::Cancel] {
        let body = work_body(
            RunId::from_bytes([4; 16]),
            tenant,
            OperationId::from_bytes([5; 16]),
        );
        let command = command(CommandId::from_bytes([6; 16]), kind, &body)?;
        assert!(
            runtime
                .check_question(&command, SystemTime::UNIX_EPOCH, None)
                .is_ok(),
            "{kind:?} carries no question"
        );
    }
    Ok(())
}

// Source: E30 (a pending question holds only its submit). Awaiting an answer
// is false for any command that is not a submit, and for a submit whose
// body does not decode: neither invents a wait.
#[test]
fn sim_awaits_answer_refuses_non_submit_and_garbage() -> Outcome {
    let runtime = memory(ScriptedOwner::new())?;
    let tenant = TenantId::from_bytes([0xA4; 16]);
    let body = work_body(
        RunId::from_bytes([7; 16]),
        tenant,
        OperationId::from_bytes([8; 16]),
    );
    let cancel = command(CommandId::from_bytes([9; 16]), CommandKind::Cancel, &body)?;
    assert!(
        !runtime.awaits_answer(RunId::from_bytes([7; 16]), &cancel, SystemTime::UNIX_EPOCH),
        "only a submit waits on an answer"
    );
    let garbage = Command::new(
        CommandId::from_bytes([10; 16]),
        tenant,
        Vec::new(),
        CommandContent::new(CommandKind::Submit, Box::from(&b"not json"[..])),
    )?;
    assert!(
        !runtime.awaits_answer(RunId::from_bytes([7; 16]), &garbage, SystemTime::UNIX_EPOCH),
        "an undecodable submit waits on nothing"
    );
    Ok(())
}

// Source: Cell 04 (reservation is persisted before dispatch). Dispatching an
// effect with no durable intent is refused naming the operation, before any
// fence check, owner reach or journal write.
#[test]
fn sim_dispatch_without_a_durable_intent_is_refused() -> Outcome {
    use crate::runtime::TickReport;
    let mut runtime = memory(ScriptedOwner::new())?;
    let operation = OperationId::from_bytes([0xB1; 16]);
    let intent = larry_contracts::effect::EffectIntent::new(
        larry_contracts::ids::EffectId::from_bytes([0xB2; 16]),
        operation,
        RunId::from_bytes([0xB3; 16]),
        TenantId::from_bytes([0xB4; 16]),
        ReconciliationStrategy::Queryable,
        larry_contracts::authority::SecurityEpoch::INITIAL,
    );
    let mut report = TickReport::default();
    let refused = runtime.dispatch(
        RunId::from_bytes([0xB3; 16]),
        intent,
        SystemTime::UNIX_EPOCH,
        &mut report,
    );
    assert!(
        matches!(
            refused,
            Err(RuntimeError::IntentNotDurable { operation: op }) if op == operation
        ),
        "dispatch without a reservation is refused: {refused:?}"
    );
    assert!(report.outcomes().is_empty(), "nothing was observed");
    assert!(
        runtime.owner().dispatched().is_empty(),
        "the owner is never reached without a reservation"
    );
    Ok(())
}

// Source: larry#182 (the dispatch budget is a fold) and CWE-20. A reserved
// intent whose effect key names no effect — an all-zero tenant, run or
// operation from a hostile generator — is refused typed at the key, before
// any liveness check or owner reach.
#[test]
fn sim_zero_identity_key_is_refused_before_the_live_check() -> Outcome {
    use crate::runtime::TickReport;
    for (field, tenant, run, operation) in [
        ("tenant", [0; 16], [0xC2; 16], [0xC3; 16]),
        ("run", [0xC1; 16], [0; 16], [0xC3; 16]),
        ("operation", [0xC1; 16], [0xC2; 16], [0; 16]),
    ] {
        let mut runtime = memory(ScriptedOwner::new())?;
        let intent = larry_contracts::effect::EffectIntent::new(
            larry_contracts::ids::EffectId::from_bytes([0xC0; 16]),
            OperationId::from_bytes(operation),
            RunId::from_bytes(run),
            TenantId::from_bytes(tenant),
            ReconciliationStrategy::Queryable,
            larry_contracts::authority::SecurityEpoch::INITIAL,
        );
        let record = crate::gate::authorize(&authorized_pinned(intent.operation)?)?;
        runtime.track_run(intent.run, TenantId::from_bytes(tenant))?;
        runtime.record_intent(intent.run, intent, record)?;
        let mut report = TickReport::default();
        let refused = runtime.dispatch(intent.run, intent, SystemTime::UNIX_EPOCH, &mut report);
        assert!(
            matches!(
                refused,
                Err(RuntimeError::EffectIdentity { field: got, .. }) if got == field
            ),
            "an all-zero {field} names no effect key: {refused:?}"
        );
        assert!(
            runtime.owner().dispatched().is_empty(),
            "the owner is never reached for a keyless effect"
        );
    }
    Ok(())
}

// Source: D48 (freshness is live state, never a journaled decision). A reach
// for an effect the live table does not hold is denied as expired authority
// without calling any owner: the journal cannot prove the reservation was
// ever dispatched.
#[test]
fn sim_reach_without_live_entry_is_denied() -> Outcome {
    use crate::runtime::TickReport;
    let mut runtime = memory(ScriptedOwner::new())?;
    let operation = OperationId::from_bytes([0xD1; 16]);
    let run = RunId::from_bytes([0xD2; 16]);
    let intent = larry_contracts::effect::EffectIntent::new(
        larry_contracts::ids::EffectId::from_bytes([0xD3; 16]),
        operation,
        run,
        TenantId::from_bytes([0xD4; 16]),
        ReconciliationStrategy::Queryable,
        larry_contracts::authority::SecurityEpoch::INITIAL,
    );
    let record = crate::gate::authorize(&authorized_pinned(operation)?)?;
    runtime.track_run(run, TenantId::from_bytes([0xD4; 16]))?;
    runtime.record_intent(run, intent, record)?;
    let mut report = TickReport::default();
    runtime.dispatch(run, intent, SystemTime::UNIX_EPOCH, &mut report)?;
    assert_eq!(report.denied().len(), 1, "exactly the denial is recorded");
    assert!(
        matches!(
            report.denied().first(),
            Some((
                op,
                larry_contracts::replay::DenialReason::AuthorityExpired
            )) if *op == operation
        ),
        "the reach is denied as expired authority: {:?}",
        report.denied()
    );
    assert!(
        runtime.owner().dispatched().is_empty(),
        "no owner is reached without a live witness"
    );
    Ok(())
}

// Source: D48 (observation resolves only what this runtime sent). A status
// query for an effect the live table does not hold touches no witness and
// still asks the endpoint, so a crash inside the dispatch window stays
// resolvable.
#[test]
fn sim_observe_without_live_entry_reconciles_nothing() -> Outcome {
    use crate::runtime::TickReport;
    use crate::runtime::live::OwnerReach;
    let mut runtime = memory(ScriptedOwner::scripted([EffectOutcome::Unknown {
        operation: OperationId::from_bytes([0xE1; 16]),
    }]))?;
    let intent = larry_contracts::effect::EffectIntent::new(
        larry_contracts::ids::EffectId::from_bytes([0xE2; 16]),
        OperationId::from_bytes([0xE1; 16]),
        RunId::from_bytes([0xE3; 16]),
        TenantId::from_bytes([0xE4; 16]),
        ReconciliationStrategy::Queryable,
        larry_contracts::authority::SecurityEpoch::INITIAL,
    );
    let mut report = TickReport::default();
    let outcome = runtime.reach_owner(
        intent,
        OwnerReach::Observe,
        SystemTime::UNIX_EPOCH,
        &mut report,
    )?;
    assert!(
        matches!(outcome, Some(EffectOutcome::Unknown { .. })),
        "the observation is asked and recorded: {outcome:?}"
    );
    Ok(())
}

// Source: Cell 05 (every bound refuses past its ceiling) and the test-only
// inspection seam. A memory-only journal stages nothing, so unsynced reads
// zero; the one-line assertion pins the seam for the durable tests.
#[test]
fn sim_memory_journal_stages_nothing() -> Outcome {
    let journal = Journal::new(4)?;
    let unsynced = std::hint::black_box(Journal::unsynced);
    assert_eq!(unsynced(&journal), 0, "a memory journal stages nothing");
    Ok(())
}

// Source: larry#75 (a refused file fails closed) and larry#209 (the latch).
// The store reports the lost acknowledgment of its own append: the bytes
// reached the file and the caller was told nothing was durable. The tick
// that would reach the owner on that entry fails typed instead, and the
// owner is never reached.
#[test]
fn sim_lost_acknowledgement_at_the_reach_fails_the_tick() -> Outcome {
    for seed in 0..8_u64 {
        let (_guard, path, _id, command) = seeded_submit(seed, "reachsync")?;
        let settings = settings()?;
        let mut journal = Journal::open(&path, settings.journal_capacity)?;
        journal.accept_command(command)?;
        let journal = journal.lost_acknowledgement();
        let mut runtime: Rt = restore(settings, ScriptedOwner::new(), clock(), journal)?;
        let tick = runtime.tick();
        assert!(
            matches!(tick, Err(RuntimeError::Durable(_))),
            "seed {seed}: a refused sync fails the tick: {tick:?}"
        );
        assert!(
            runtime.owner().dispatched().is_empty(),
            "seed {seed}: the owner is never reached past a refused sync"
        );
    }
    Ok(())
}

/// A runtime holding one seeded effect it has sent once and still counts as open, and the
/// warrant the runtime would mint for that effect's second attempt.
fn minted_after_one_send(
    seed: u64,
) -> Result<(Rt, EffectIntent, Authority), Box<dyn std::error::Error>> {
    let operation = OperationId::from_bytes(ids(3, seed));
    let body = work_body(
        RunId::from_bytes(ids(1, seed)),
        TenantId::from_bytes(ids(2, seed)),
        operation,
    );
    let mut runtime = memory(ScriptedOwner::new())?;
    runtime.submit(command(
        CommandId::from_bytes(ids(4, seed)),
        CommandKind::Submit,
        &body,
    )?)?;
    runtime.tick()?;
    assert_eq!(
        runtime.owner().dispatched(),
        [operation],
        "seed {seed}: sent"
    );
    let intent = runtime.live.get(&operation).ok_or("a live effect")?.intent;
    let warrant = runtime.fence.authorize(effect_key(&intent, 2)?, &intent)?;
    Ok((runtime, intent, warrant))
}

/// Withdraws the minted authority one of the two ways the fence can, and names the refusal.
fn withdraw(
    runtime: &mut Rt,
    intent: &EffectIntent,
    by_name: bool,
) -> Result<FenceRefusal, Box<dyn std::error::Error>> {
    if by_name {
        assert!(runtime.fence.revoke(intent.operation)?, "first withdrawal");
        return Ok(FenceRefusal::Revoked {
            operation: intent.operation,
        });
    }
    let current = runtime.fence.advance_epoch()?;
    Ok(FenceRefusal::StaleEpoch {
        admitted: intent.admission_epoch,
        current,
    })
}

// Source: CWE-367 (the check and the use are two moments), larry#76 and the INVARIANTS fence
// rule "re-checks it with revalidate immediately before the owner". The runtime holds the fence
// exclusively through one reach, so no public call can land between the mint and the handoff:
// each seed runs the real mint, withdraws, then runs the real handoff. A withdrawal stops it with
// the owner not called, no attempt and no offline unit spent, and the refusal reported fenced.
// The same handoff with no withdrawal reaches the owner once and counts one unit.
#[test]
fn sim_a_withdrawal_after_the_mint_refuses_the_handoff_and_counts_nothing() -> Outcome {
    for seed in 0..32_u64 {
        for withdrawn in [None, Some(true), Some(false)] {
            let case = format!("seed {seed} {withdrawn:?}");
            let (mut runtime, intent, warrant) = minted_after_one_send(seed)?;
            let expected = match withdrawn {
                Some(by_name) => Some(withdraw(&mut runtime, &intent, by_name)?),
                None => None,
            };
            let spent = runtime.fence.offline_spent()?;
            let live = runtime.live.get(&intent.operation).ok_or("live")?;
            let sends = live.dispatches;
            let auth = live
                .proof
                .checked_auth()
                .map_err(|reason| format!("{case}: the lease holds: {reason:?}"))?;
            let mut report = TickReport::default();
            let key = effect_key(&intent, 2)?;
            let at = epoch_plus(0)?;
            let outcome = runtime.hand_off(intent, key, &warrant, auth, at, &mut report)?;
            let reached = u8::from(expected.is_none());
            assert_eq!(outcome.is_some(), expected.is_none(), "{case}: outcome");
            assert_eq!(
                runtime.owner().dispatched().len(),
                usize::from(reached.saturating_add(1)),
                "{case}: the owner is called only with live authority"
            );
            let live = runtime.live.get(&intent.operation).ok_or("live")?;
            assert_eq!(
                live.dispatches.saturating_sub(sends),
                reached,
                "{case}: a refused handoff spends no attempt"
            );
            assert_eq!(
                runtime.fence.offline_spent()?,
                spent.saturating_add(u32::from(reached)),
                "{case}: a refused handoff counts nothing"
            );
            let fenced: Vec<_> = expected
                .into_iter()
                .map(|refusal| (intent.operation, refusal))
                .collect();
            assert_eq!(report.fenced(), fenced, "{case}: fenced");
            assert!(report.refused().is_empty(), "{case}: nothing refused");
        }
    }
    Ok(())
}

// Source: CWE-613 (authority that outlives its withdrawal) and the INVARIANTS retry rule "a
// spend refusal after the owner ran is the tick refusal Unspent, never fenced". The reach
// happened, so a count the fence refuses names the effect and the fence's own cause, and counts
// nothing.
#[test]
fn sim_a_count_refused_after_the_reach_is_unspent_and_never_fenced() -> Outcome {
    for seed in 0..32_u64 {
        let (mut runtime, intent, warrant) = minted_after_one_send(seed)?;
        let refusal = withdraw(&mut runtime, &intent, seed & 1 == 0)?;
        let spent = runtime.fence.offline_spent()?;
        let mut report = TickReport::default();
        runtime.count_reach(&intent, &warrant, &mut report);
        let &[
            (
                ref item,
                RuntimeError::Unspent {
                    ref operation,
                    refusal: ref cause,
                },
            ),
        ] = report.refused()
        else {
            return Err(format!("seed {seed}: {:?}", report.refused()).into());
        };
        assert_eq!(*item, TickItem::Effect(intent.operation), "seed {seed}");
        assert_eq!(*operation, intent.operation, "seed {seed}");
        assert_eq!(*cause, refusal, "seed {seed}: the fence's own cause");
        assert!(report.fenced().is_empty(), "seed {seed}: never fenced");
        assert_eq!(runtime.fence.offline_spent()?, spent, "seed {seed}");
    }
    Ok(())
}

// Source: CWE-285 (an effect that needs more than the proof covers), E65 and larry#197 ("a typed
// owner refusal ... spends none"). The owner refuses the handoff before any effect, so the live
// witness is what it was before the call: the same attempt count and the same dispatched key. No
// offline unit is counted and the refusal is the caller's typed error, never a fenced reach.
#[test]
fn sim_an_owner_refusal_at_the_handoff_restores_the_witness_and_counts_nothing() -> Outcome {
    for seed in 0..32_u64 {
        let (mut runtime, intent, warrant) = minted_after_one_send(seed)?;
        let spent = runtime.fence.offline_spent()?;
        let live = runtime.live.get(&intent.operation).ok_or("live")?;
        let before = (live.dispatches, live.dispatched, live.sent);
        let (auth, mut required) = live
            .proof
            .checked_auth()
            .map_err(|reason| format!("seed {seed}: the lease holds: {reason:?}"))?;
        required.push(Cap::new(format!("larry.missing.{seed}")));
        let mut report = TickReport::default();
        let key = effect_key(&intent, 2)?;
        let at = epoch_plus(0)?;
        let refused = runtime.hand_off(intent, key, &warrant, (auth, required), at, &mut report);
        assert!(
            matches!(
                refused,
                Err(RuntimeError::Binding(BindingError::NoCoveringGrant))
            ),
            "seed {seed}: the owner's typed refusal: {refused:?}"
        );
        let live = runtime.live.get(&intent.operation).ok_or("live")?;
        assert_eq!(
            (live.dispatches, live.dispatched, live.sent),
            before,
            "seed {seed}: the witness is restored"
        );
        assert_eq!(
            runtime.owner().dispatched(),
            [intent.operation],
            "seed {seed}: only the first send reached the owner"
        );
        assert_eq!(runtime.fence.offline_spent()?, spent, "seed {seed}");
        assert!(report.fenced().is_empty(), "seed {seed}: never fenced");
        assert!(report.refused().is_empty(), "seed {seed}: never unspent");
    }
    Ok(())
}

/// Pinned inputs that authorize one unit of `FileRead` for `operation`.
fn authorized_pinned(
    operation: OperationId,
) -> Result<larry_contracts::replay::PinnedInputs, Box<dyn std::error::Error>> {
    use larry_contracts::replay::{
        BudgetSnapshot, CapabilitySnapshot, EffectRequest, PinnedInputs, PolicySnapshot,
        ValiditySnapshot,
    };
    Ok(PinnedInputs::new(
        EffectRequest::new(
            operation,
            [Scope::FileRead].into_iter().collect(),
            1,
            BudgetUnit::Requests,
            SystemTime::UNIX_EPOCH
                .checked_add(Duration::from_secs(START_SECS))
                .ok_or("fixture time overflow")?,
        ),
        PolicySnapshot::new(1, [Scope::FileRead].into_iter().collect()),
        BudgetSnapshot::new(100, BudgetUnit::Requests),
        CapabilitySnapshot::new([Scope::FileRead].into_iter().collect()),
        ValiditySnapshot::new(
            SystemTime::UNIX_EPOCH
                .checked_add(Duration::from_secs(START_SECS + 3_600))
                .ok_or("fixture time overflow")?,
        ),
    ))
}

/// The validity end the settings pin, one hour after the start instant.
fn epoch_plus(secs: u64) -> Result<SystemTime, Box<dyn std::error::Error>> {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(secs.saturating_add(START_SECS)))
        .ok_or_else(|| "fixture time overflow".into())
}
