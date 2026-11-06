//! Seeded lifecycle simulations over the public runtime API (E55, D04).
//! Every test drives the real `Runtime` twice from one seed and asserts the
//! same journal fingerprint plus the caller-visible report values.

#[path = "sim_kit/seed.rs"]
mod seed;
mod sim_kit;
mod support;

use rocco_contracts::authority::{Grant, Scope};
use rocco_contracts::effect::{EffectOutcome, ReconciliationStrategy, TerminalDisposition};
use rocco_contracts::event::EventKind;
use rocco_contracts::ids::{ContentDigest, RunId, WorkerId};
use rocco_contracts::replay::DenialReason;
use rocco_contracts::state::RunState;
use rocco_runtime::clock::FixedClock;
use rocco_runtime::error::RuntimeError;
use rocco_runtime::journal::{Journal, JournalEntry};
use rocco_runtime::owner::fixture::ScriptedOwner;
use rocco_runtime::projection::ProjectionBackend;

use rocco_runtime::runtime::{Runtime, TickReport};

use seed::step;
use sim_kit::{Offer, Sim, read_work, reader, reopen, replay, work};
use support::{
    TestRuntime, assert_state, cancel, command_id, epoch, operation, proposal, run, runtime_with,
    settings, submit, succeeded, tenant, unknown,
};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;
/// Stable identity list for one journal.
type Trace = Vec<(RunId, u64, EventKind, ContentDigest)>;

/// Wrapping seed evolution, independent of any clock or scheduler.
fn evolve(seed: &mut u64) -> u64 {
    step(
        seed,
        6_364_136_223_846_793_005,
        1_442_695_038_195_672_373,
        29,
    )
}

/// Stable identity list of recorded events.
fn trace_of(journal: &Journal) -> Trace {
    let mut events = Trace::new();
    for entry in journal.entries() {
        if let JournalEntry::Event(ref event) = *entry {
            let sequence = event.sequence().value();
            events.push((event.run(), sequence, event.kind(), event.payload_digest()));
        }
    }
    events
}

/// Submits one unit of work and ticks once, returning the tick report.
fn fire(
    runtime: &mut TestRuntime,
    id: u8,
    scopes: &[Scope],
    cost: u64,
    strategy: ReconciliationStrategy,
) -> Result<TickReport, Box<dyn std::error::Error>> {
    let body = proposal(run(id), tenant(1), operation(id), scopes, cost, strategy);
    runtime.offer(id, &body)?;
    Ok(runtime.tick()?)
}

/// Standard FileRead/FileRead/100 runtime over `owner`.
fn mk_life(owner: ScriptedOwner) -> Result<TestRuntime, Box<dyn std::error::Error>> {
    reader(owner, 100)
}

/// One seeded success journey and its trace plus remaining budget (E55).
fn success_journey(seed: u64) -> Result<(Trace, u64), Box<dyn std::error::Error>> {
    let mut random = seed;
    let cost = (evolve(&mut random) & 7).saturating_add(1);
    let mut runtime = mk_life(ScriptedOwner::scripted([succeeded(operation(1))]))?;
    let tick = fire(
        &mut runtime,
        1,
        &[Scope::FileRead],
        cost,
        ReconciliationStrategy::IdempotentRetrySafe,
    )?;
    assert_eq!(tick.admitted(), &[command_id(1)], "seed admits the command");
    assert_eq!(
        tick.authorized(),
        &[operation(1)],
        "gate authorizes the effect"
    );
    assert_eq!(
        tick.outcomes(),
        &[(operation(1), succeeded(operation(1)))],
        "owner success is observed"
    );
    assert_eq!(tick.receipts().len(), 1, "one effect publishes one receipt");
    assert_eq!(
        tick.receipts()[0].disposition(),
        TerminalDisposition::Succeeded,
        "the receipt records success"
    );
    assert_state(runtime.projection(), run(1), RunState::Completed)?;
    let remaining = runtime.remaining_budget();
    assert_eq!(
        remaining,
        100_u64.saturating_sub(cost),
        "reservation spends exactly the declared cost"
    );
    Ok((trace_of(runtime.journal()), remaining))
}

/// E55/D04: one success publishes exactly one success receipt.
#[test]
fn sim_lifecycle_success_receipt_and_budget() -> Outcome {
    for seed in 0_u64..8 {
        let (first, first_budget) = success_journey(seed)?;
        let (second, second_budget) = success_journey(seed)?;
        assert_eq!(first, second, "seed {seed}: journal replay");
        assert_eq!(first_budget, second_budget, "seed {seed}: budget replay");
        assert!(!first.is_empty(), "seed {seed}: history recorded");
    }
    Ok(())
}

/// E55: a definitive endpoint failure ends the run Failed with Failed.
#[test]
fn sim_lifecycle_failed_outcome_is_terminal() -> Outcome {
    for seed in 8_u64..16 {
        let drive = |seed: u64| -> Result<Trace, Box<dyn std::error::Error>> {
            let mut rt = mk_life(ScriptedOwner::scripted([EffectOutcome::Failed {
                operation: operation(2),
            }]))?;
            let tick = fire(
                &mut rt,
                2,
                &[Scope::FileRead],
                1,
                ReconciliationStrategy::Queryable,
            )?;
            assert_eq!(
                tick.receipts()[0].disposition(),
                TerminalDisposition::Failed,
                "seed {seed}: failure recorded"
            );
            assert_state(rt.projection(), run(2), RunState::Failed)?;
            assert!(
                rt.owner().dispatched().contains(&operation(2)),
                "seed {seed}: owner saw it"
            );
            let _ = seed;
            Ok(trace_of(rt.journal()))
        };
        assert_eq!(drive(seed)?, drive(seed)?, "seed {seed}: journal replay");
    }
    Ok(())
}

/// E55/R72: unknown under Queryable stays Reconciling, then reconciles.
#[test]
fn sim_lifecycle_unknown_queryable_reconciles() -> Outcome {
    replay(16_u64..24, |_| -> Sim<Trace> {
        let mut rt = mk_life(ScriptedOwner::scripted([
            unknown(operation(3)),
            succeeded(operation(3)),
        ]))?;
        let body = read_work(3, 1, ReconciliationStrategy::Queryable);
        rt.offer(3, &body)?;
        assert_eq!(
            rt.tick()?.receipts().len(),
            0,
            "open unknown publishes nothing"
        );
        assert_state(rt.projection(), run(3), RunState::Reconciling)?;
        let second = rt.tick()?;
        assert_eq!(second.reconciled(), &[operation(3)], "reconcile closes it");
        assert_eq!(
            second.receipts()[0].disposition(),
            TerminalDisposition::Succeeded,
            "reconciled outcome succeeds"
        );
        assert_state(rt.projection(), run(3), RunState::Completed)?;
        Ok(trace_of(rt.journal()))
    })
}

/// E55/E56: unknown under RequiresHuman is terminal UnknownPermanent.
#[test]
fn sim_lifecycle_unknown_requires_human_permanent() -> Outcome {
    replay(24_u64..32, |_| -> Sim<Trace> {
        let mut rt = mk_life(ScriptedOwner::scripted([unknown(operation(4))]))?;
        let tick = fire(
            &mut rt,
            4,
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::RequiresHuman,
        )?;
        assert_eq!(tick.receipts().len(), 1, "permanent unknown still receipts");
        assert_eq!(
            tick.receipts()[0].disposition(),
            TerminalDisposition::UnknownPermanent,
            "unknown stays unknown"
        );
        assert_state(rt.projection(), run(4), RunState::OutcomeUnknown)?;
        Ok(trace_of(rt.journal()))
    })
}

/// E55/E56: unknown under AlwaysCompensate is terminal UnknownPermanent.
#[test]
fn sim_lifecycle_unknown_compensate_permanent() -> Outcome {
    replay(32_u64..40, |_| -> Sim<Trace> {
        let mut rt = mk_life(ScriptedOwner::scripted([unknown(operation(5))]))?;
        let tick = fire(
            &mut rt,
            5,
            &[Scope::FileRead],
            2,
            ReconciliationStrategy::AlwaysCompensate,
        )?;
        assert_eq!(
            tick.receipts()[0].disposition(),
            TerminalDisposition::UnknownPermanent,
            "compensate closes unknown"
        );
        assert_state(rt.projection(), run(5), RunState::OutcomeUnknown)?;
        Ok(trace_of(rt.journal()))
    })
}

/// E55/Cell 05: an idempotent-safe retry succeeds on the second reach.
#[test]
fn sim_lifecycle_idempotent_retry_succeeds_second_reach() -> Outcome {
    replay(40_u64..48, |_| -> Sim<Trace> {
        let mut rt = mk_life(ScriptedOwner::scripted([
            unknown(operation(6)),
            succeeded(operation(6)),
        ]))?;
        let body = read_work(6, 3, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(6, &body)?;
        rt.tick()?;
        assert_state(rt.projection(), run(6), RunState::Reconciling)?;
        let second = rt.tick()?;
        assert_eq!(second.receipts().len(), 1, "retry publishes one receipt");
        assert_eq!(
            second.receipts()[0].disposition(),
            TerminalDisposition::Succeeded,
            "retry succeeds"
        );
        Ok(trace_of(rt.journal()))
    })
}

/// E55: two effects admitted in one tick both authorize and receipt.
#[test]
fn sim_lifecycle_two_effects_single_tick() -> Outcome {
    replay(48_u64..56, |_| -> Sim<Trace> {
        let mut rt = mk_life(ScriptedOwner::scripted([
            succeeded(operation(7)),
            succeeded(operation(8)),
        ]))?;
        for number in [7_u8, 8] {
            let body = work(
                number,
                1,
                number,
                3,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            rt.offer(number, &body)?;
        }
        let tick = rt.tick()?;
        assert_eq!(
            tick.authorized(),
            &[operation(7), operation(8)],
            "both authorize in order"
        );
        assert_eq!(tick.receipts().len(), 2, "both effects receipt");
        assert_eq!(rt.remaining_budget(), 94, "both reservations held");
        assert_state(rt.projection(), run(7), RunState::Completed)?;
        assert_state(rt.projection(), run(8), RunState::Completed)?;
        Ok(trace_of(rt.journal()))
    })
}

/// E55: receipts accumulate across sequential ticks, one run per tick.
#[test]
fn sim_lifecycle_sequential_ticks_accumulate() -> Outcome {
    replay(56_u64..64, |_| -> Sim<Trace> {
        let mut rt = mk_life(ScriptedOwner::scripted([
            succeeded(operation(9)),
            succeeded(operation(10)),
        ]))?;
        for number in [9_u8, 10] {
            let tick = fire(
                &mut rt,
                number,
                &[Scope::FileRead],
                4,
                ReconciliationStrategy::IdempotentRetrySafe,
            )?;
            assert_eq!(tick.receipts().len(), 1, "each tick receipts once");
        }
        assert_eq!(rt.projection().receipts().len(), 2, "receipts accumulate");
        assert_eq!(rt.remaining_budget(), 92, "both costs reserved");
        Ok(trace_of(rt.journal()))
    })
}

/// E65/D21: the display surface refuses a disconnected fixture owner.
#[test]
fn sim_lifecycle_display_refused_for_fixture_owner() -> Outcome {
    replay(64_u64..72, |_| -> Sim<usize> {
        let mut rt: TestRuntime = Runtime::new(
            settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
            ScriptedOwner::new(),
            FixedClock::new(epoch()?),
        )?;
        let body = read_work(11, 1, ReconciliationStrategy::Queryable);
        let holder = WorkerId::from_bytes([7; 16]);
        let grants: &[Grant] = &[];
        assert!(
            matches!(
                rt.display(&submit(command_id(11), &body)?, holder, grants),
                Err(RuntimeError::Binding(_))
            ),
            "fixture owner has no resolvable target"
        );
        assert!(
            rt.owner().dispatched().is_empty(),
            "refused display dispatches nothing"
        );
        Ok(rt.journal().len())
    })
}

/// E55: an empty tick is idle and records nothing on both runs.
#[test]
fn sim_lifecycle_empty_tick_is_idle() -> Outcome {
    replay(72_u64..80, |seed| -> Sim<usize> {
        let mut rt = mk_life(ScriptedOwner::new())?;
        assert!(rt.tick()?.is_idle(), "seed {seed}: nothing to do");
        assert!(rt.journal().is_empty(), "idle tick records nothing");
        Ok(rt.journal().len())
    })
}

/// E55: the logical tick counter advances once per tick on both runs.
#[test]
fn sim_lifecycle_tick_count_advances() -> Outcome {
    replay(80_u64..88, |seed| -> Sim<(u64, usize)> {
        let mut rt = mk_life(ScriptedOwner::scripted([succeeded(operation(12))]))?;
        let body = read_work(12, 1, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(12, &body)?;
        rt.tick()?;
        rt.tick()?;
        rt.tick()?;
        let (first_count, first_len) = (rt.tick_count(), trace_of(rt.journal()).len());
        assert_eq!(first_count, 3, "seed {seed}: three ticks");
        Ok((first_count, first_len))
    })
}

/// E55: a budget denial reserves nothing and records the same history twice.
#[test]
fn sim_lifecycle_budget_denial_holds_no_reservation() -> Outcome {
    replay(88_u64..96, |_| -> Sim<(u64, Trace)> {
        let mut rt = reader(ScriptedOwner::new(), 10)?;
        let tick = fire(
            &mut rt,
            13,
            &[Scope::FileRead],
            11,
            ReconciliationStrategy::Queryable,
        )?;
        assert_eq!(tick.authorized().len(), 0, "over-budget authorizes nothing");
        assert_eq!(tick.denied().len(), 1, "over-budget denies once");
        assert_state(rt.projection(), run(13), RunState::Failed)?;
        let (first_budget, first_fp) = (rt.remaining_budget(), trace_of(rt.journal()));
        assert_eq!(first_budget, 10, "denial reserves nothing");
        Ok((first_budget, first_fp))
    })
}

/// E55/E30: cancelling a run the journal never named completes cancelled.
#[test]
fn sim_lifecycle_cancel_of_untracked_run() -> Outcome {
    replay(96_u64..104, |_| -> Sim<Trace> {
        let mut rt = mk_life(ScriptedOwner::new())?;
        let body = proposal(
            run(14),
            tenant(1),
            operation(14),
            &[],
            0,
            ReconciliationStrategy::RequiresHuman,
        );
        rt.submit(cancel(command_id(14), &body)?)?;
        assert_eq!(
            rt.tick()?.receipts().len(),
            0,
            "cancel fabricates no receipt"
        );
        assert_state(rt.projection(), run(14), RunState::Cancelled)?;
        Ok(trace_of(rt.journal()))
    })
}

/// E55: a failed run still folds to Failed after restore with the same print.
#[test]
fn sim_lifecycle_failed_run_survives_restore() -> Outcome {
    replay(104_u64..112, |_| -> Sim<Trace> {
        let mut rt = mk_life(ScriptedOwner::scripted([EffectOutcome::Failed {
            operation: operation(15),
        }]))?;
        let body = read_work(15, 1, ReconciliationStrategy::Queryable);
        rt.offer(15, &body)?;
        rt.tick()?;
        let journal = rt.into_journal();
        let before = trace_of(&journal);
        let restored = reopen(journal, 100)?;
        assert_state(restored.projection(), run(15), RunState::Failed)?;
        assert_eq!(
            trace_of(restored.journal()),
            before,
            "restore keeps the recorded history"
        );
        Ok(before)
    })
}

/// E55: a completed run still folds to Completed after restore.
#[test]
fn sim_lifecycle_completed_run_survives_restore() -> Outcome {
    replay(112_u64..120, |_| -> Sim<Trace> {
        let mut rt = mk_life(ScriptedOwner::scripted([succeeded(operation(16))]))?;
        let body = read_work(16, 6, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(16, &body)?;
        rt.tick()?;
        let journal = rt.into_journal();
        let before = trace_of(&journal);
        let restored = reopen(journal, 100)?;
        assert_state(restored.projection(), run(16), RunState::Completed)?;
        assert_eq!(
            restored.remaining_budget(),
            94,
            "reservation survives restore"
        );
        Ok(before)
    })
}

/// E55: the owner witness order matches the authorized order for two effects.
#[test]
fn sim_lifecycle_owner_witness_matches_authorized() -> Outcome {
    replay(120_u64..128, |_| -> Sim<Trace> {
        let mut rt = mk_life(ScriptedOwner::scripted([
            succeeded(operation(17)),
            succeeded(operation(18)),
        ]))?;
        for number in [17_u8, 18] {
            let body = work(
                number,
                1,
                number,
                1,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            rt.offer(number, &body)?;
        }
        let tick = rt.tick()?;
        assert_eq!(
            rt.owner().dispatched(),
            tick.authorized(),
            "witness matches authorization"
        );
        assert_eq!(
            tick.authorized(),
            &[operation(17), operation(18)],
            "both authorize in order"
        );
        Ok(trace_of(rt.journal()))
    })
}

/// E55/R72: the reconcile witness matches the reconciled report exactly.
#[test]
fn sim_lifecycle_reconcile_witness_matches_report() -> Outcome {
    replay(128_u64..136, |_| -> Sim<Trace> {
        let mut rt = mk_life(ScriptedOwner::scripted([
            unknown(operation(19)),
            succeeded(operation(19)),
        ]))?;
        let body = read_work(19, 1, ReconciliationStrategy::Queryable);
        rt.offer(19, &body)?;
        rt.tick()?;
        assert!(
            rt.owner().reconciled().is_empty(),
            "dispatch is not a reconciliation"
        );
        let second = rt.tick()?;
        assert_eq!(
            rt.owner().reconciled(),
            second.reconciled(),
            "witness matches report"
        );
        assert_eq!(
            second.reconciled(),
            &[operation(19)],
            "one effect reconciled"
        );
        Ok(trace_of(rt.journal()))
    })
}

/// E55: a second run after a terminal first run completes independently.
#[test]
fn sim_lifecycle_second_run_after_terminal_first() -> Outcome {
    replay(136_u64..144, |_| -> Sim<Trace> {
        let mut rt = mk_life(ScriptedOwner::scripted([
            succeeded(operation(20)),
            succeeded(operation(21)),
        ]))?;
        let first_body = read_work(20, 5, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(20, &first_body)?;
        rt.tick()?;
        assert_state(rt.projection(), run(20), RunState::Completed)?;
        let second_body = read_work(21, 5, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(21, &second_body)?;
        assert_eq!(rt.tick()?.receipts().len(), 1, "second run receipts once");
        assert_state(rt.projection(), run(21), RunState::Completed)?;
        assert_eq!(
            rt.projection().receipts().len(),
            2,
            "receipts accumulate across runs"
        );
        Ok(trace_of(rt.journal()))
    })
}

/// E55: policy denial names the exact refused scope on both runs.
#[test]
fn sim_lifecycle_policy_denial_names_scope() -> Outcome {
    replay(144_u64..152, |_| -> Sim<Trace> {
        let mut rt = mk_life(ScriptedOwner::new())?;
        let tick = fire(
            &mut rt,
            22,
            &[Scope::NetworkEgress],
            1,
            ReconciliationStrategy::Queryable,
        )?;
        assert_eq!(
            tick.denied(),
            &[(
                operation(22),
                DenialReason::PolicyRefuses {
                    scope: Scope::NetworkEgress,
                }
            )],
            "denial names the refused scope"
        );
        assert_state(rt.projection(), run(22), RunState::Failed)?;
        assert!(
            rt.owner().dispatched().is_empty(),
            "denial reaches no owner"
        );
        Ok(trace_of(rt.journal()))
    })
}

/// E55: capability denial follows policy in the fixed check order.
#[test]
fn sim_lifecycle_capability_denial_follows_policy() -> Outcome {
    replay(152_u64..160, |_| -> Sim<Trace> {
        let mut rt = runtime_with(
            ScriptedOwner::new(),
            &[Scope::FileRead, Scope::FileWrite],
            &[Scope::FileRead],
            100,
        )?;
        let tick = fire(
            &mut rt,
            23,
            &[Scope::FileWrite],
            1,
            ReconciliationStrategy::Queryable,
        )?;
        assert_eq!(
            tick.denied(),
            &[(
                operation(23),
                DenialReason::CapabilityMissing {
                    scope: Scope::FileWrite,
                }
            )],
            "capability is checked after policy"
        );
        Ok(trace_of(rt.journal()))
    })
}
