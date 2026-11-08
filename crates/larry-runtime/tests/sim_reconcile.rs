//! Seeded reconciliation-strategy simulations (E56, R72).
//! Every test drives the real `Runtime` twice from one seed and asserts tallies.

#[path = "sim_kit/seed.rs"]
mod seed;
mod sim_kit;
mod support;

use std::time::Duration;

use larry_contracts::authority::Scope;
use larry_contracts::effect::{EffectOutcome, ReconciliationStrategy, TerminalDisposition};
use larry_contracts::state::RunState;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::projection::ProjectionBackend;

use seed::step;
use sim_kit::{Offer, Sim, read_work, reader, replay, work};
use support::{TestRuntime, assert_state, operation, run, succeeded, unknown};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Seeded tally: terminal disposition plus the witness counts.
type SeededTally = (TerminalDisposition, (usize, usize, usize));

/// Wrapping seed evolution, independent of any clock or scheduler.
fn spin(seed: &mut u64) -> u64 {
    step(seed, 13_888_259_582_513_372_209, 7_225_534_248_253_489, 13)
}

/// Witness tally: receipts folded, owner dispatches and reconciliations seen.
fn witness_tally(runtime: &TestRuntime) -> (usize, usize, usize) {
    let work = runtime.projection();
    (
        work.receipts().len(),
        runtime.owner().dispatched().len(),
        runtime.owner().reconciled().len(),
    )
}

/// Runtime with default FileRead scopes and budget 100 over `owner`.
fn mk_rec(owner: ScriptedOwner) -> Result<TestRuntime, Box<dyn std::error::Error>> {
    reader(owner, 100)
}

/// E56/Queryable: an unknown outcome reconciles to success on the next tick.
#[test]
fn sim_reconcile_queryable_unknown_then_success() -> Outcome {
    replay(0_u64..8, |_| -> Sim<(usize, usize, usize)> {
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([
            unknown(operation(1)),
            succeeded(operation(1)),
        ]))?;
        let work = read_work(1, 1, ReconciliationStrategy::Queryable);
        rt.offer(1, &work)?;
        rt.tick()?;
        assert_state(rt.projection(), run(1), RunState::Reconciling)?;
        let tick = rt.tick()?;
        assert_eq!(tick.reconciled(), &[operation(1)], "one effect reconciled");
        assert_eq!(
            tick.receipts()[0].disposition(),
            TerminalDisposition::Succeeded,
            "reconcile succeeds"
        );
        assert_state(rt.projection(), run(1), RunState::Completed)?;
        Ok(witness_tally(&rt))
    })
}

/// E56/Queryable: a definitive failure receipts Failed without retry.
#[test]
fn sim_reconcile_queryable_failed_is_terminal() -> Outcome {
    replay(8_u64..16, |_| -> Sim<(usize, usize, usize)> {
        let failed = EffectOutcome::Failed {
            operation: operation(2),
        };
        let mut rt: TestRuntime =
            mk_rec(ScriptedOwner::scripted([failed, succeeded(operation(2))]))?;
        let work = read_work(2, 1, ReconciliationStrategy::Queryable);
        rt.offer(2, &work)?;
        let tick = rt.tick()?;
        assert_eq!(
            tick.receipts()[0].disposition(),
            TerminalDisposition::Failed,
            "failure is terminal"
        );
        assert_state(rt.projection(), run(2), RunState::Failed)?;
        assert_eq!(rt.owner().dispatched().len(), 1, "failure is never retried");
        Ok(witness_tally(&rt))
    })
}

/// E56/Queryable: a first-try success never touches the reconcile path.
#[test]
fn sim_reconcile_queryable_success_skips_reconcile() -> Outcome {
    replay(16_u64..24, |_| -> Sim<(usize, usize, usize)> {
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([succeeded(operation(3))]))?;
        let work = read_work(3, 1, ReconciliationStrategy::Queryable);
        rt.offer(3, &work)?;
        let tick = rt.tick()?;
        assert!(tick.reconciled().is_empty(), "no reconcile needed");
        assert!(
            rt.owner().reconciled().is_empty(),
            "owner never status-queried"
        );
        assert_eq!(
            tick.receipts()[0].disposition(),
            TerminalDisposition::Succeeded,
            "success receipts at once"
        );
        Ok(witness_tally(&rt))
    })
}

/// E56/Cell 05: an idempotent retry succeeds on the third reach.
#[test]
fn sim_reconcile_retry_succeeds_third_reach() -> Outcome {
    replay(24_u64..32, |_| -> Sim<(usize, usize, usize)> {
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([
            unknown(operation(4)),
            unknown(operation(4)),
            succeeded(operation(4)),
        ]))?;
        let work = read_work(4, 1, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(4, &work)?;
        rt.tick()?;
        rt.tick()?;
        assert_state(rt.projection(), run(4), RunState::Reconciling)?;
        assert_eq!(
            rt.tick()?.receipts()[0].disposition(),
            TerminalDisposition::Succeeded,
            "third reach succeeds"
        );
        assert_state(rt.projection(), run(4), RunState::Completed)?;
        Ok(witness_tally(&rt))
    })
}

/// E56/Cell 05: three unknown reaches exhaust the budget and stay open.
#[test]
fn sim_reconcile_three_unknowns_terminate_once() -> Outcome {
    replay(32_u64..40, |seed| -> Sim<(usize, usize, usize)> {
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([
            unknown(operation(5)),
            unknown(operation(5)),
            unknown(operation(5)),
            unknown(operation(5)),
        ]))?;
        let work = read_work(5, 1, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(5, &work)?;
        for _ in 0..6 {
            rt.tick()?;
        }
        assert_eq!(
            rt.projection().receipts().len(),
            0,
            "exhausted retries invent no receipt"
        );
        assert_state(rt.projection(), run(5), RunState::Reconciling)?;
        assert_eq!(
            rt.owner().dispatched().len(),
            3,
            "retries stop at the three-attempt bound"
        );
        let (fr, fd, fq) = witness_tally(&rt);
        assert_eq!(
            (fr, fd, fq),
            (0, 3, 0),
            "seed {seed}: three attempts, no receipt"
        );
        assert_eq!((fr, fq), (fr, fq), "seed {seed}: reconcile replay");
        Ok((fr, fd, fq))
    })
}

/// E56: a RequiresHuman success succeeds without any reconcile call.
#[test]
fn sim_reconcile_human_success_needs_no_reconcile() -> Outcome {
    replay(40_u64..48, |_| -> Sim<(usize, usize, usize)> {
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([succeeded(operation(6))]))?;
        let work = read_work(6, 1, ReconciliationStrategy::RequiresHuman);
        rt.offer(6, &work)?;
        let tick = rt.tick()?;
        assert_eq!(
            tick.receipts()[0].disposition(),
            TerminalDisposition::Succeeded,
            "success succeeds"
        );
        assert!(
            rt.owner().reconciled().is_empty(),
            "success needs no reconcile"
        );
        Ok(witness_tally(&rt))
    })
}

/// E56: a RequiresHuman failure fails without any reconcile call.
#[test]
fn sim_reconcile_human_failed_is_terminal() -> Outcome {
    replay(48_u64..56, |_| -> Sim<(usize, usize, usize)> {
        let failed = EffectOutcome::Failed {
            operation: operation(7),
        };
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([failed]))?;
        let work = read_work(7, 1, ReconciliationStrategy::RequiresHuman);
        rt.offer(7, &work)?;
        let tick = rt.tick()?;
        assert_eq!(
            tick.receipts()[0].disposition(),
            TerminalDisposition::Failed,
            "failure fails"
        );
        assert_state(rt.projection(), run(7), RunState::Failed)?;
        Ok(witness_tally(&rt))
    })
}

/// E56: an AlwaysCompensate success succeeds without compensation.
#[test]
fn sim_reconcile_compensate_success_needs_no_compensation() -> Outcome {
    replay(56_u64..64, |_| -> Sim<(usize, usize, usize)> {
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([succeeded(operation(8))]))?;
        let work = read_work(8, 1, ReconciliationStrategy::AlwaysCompensate);
        rt.offer(8, &work)?;
        let tick = rt.tick()?;
        assert_eq!(
            tick.receipts()[0].disposition(),
            TerminalDisposition::Succeeded,
            "success succeeds"
        );
        assert_state(rt.projection(), run(8), RunState::Completed)?;
        Ok(witness_tally(&rt))
    })
}

/// E56: an AlwaysCompensate failure records Failed.
#[test]
fn sim_reconcile_compensate_failed_is_terminal() -> Outcome {
    replay(64_u64..72, |_| -> Sim<(usize, usize, usize)> {
        let failed = EffectOutcome::Failed {
            operation: operation(9),
        };
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([failed]))?;
        let work = read_work(9, 1, ReconciliationStrategy::AlwaysCompensate);
        rt.offer(9, &work)?;
        assert_eq!(
            rt.tick()?.receipts()[0].disposition(),
            TerminalDisposition::Failed,
            "failure fails under any strategy"
        );
        Ok(witness_tally(&rt))
    })
}

/// E56: an unknown under AlwaysCompensate closes UnknownPermanent.
#[test]
fn sim_reconcile_compensate_unknown_is_permanent() -> Outcome {
    replay(72_u64..80, |_| -> Sim<(usize, usize, usize)> {
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([unknown(operation(10))]))?;
        let work = read_work(10, 3, ReconciliationStrategy::AlwaysCompensate);
        rt.offer(10, &work)?;
        let tick = rt.tick()?;
        assert_eq!(
            tick.receipts()[0].disposition(),
            TerminalDisposition::UnknownPermanent,
            "unknown closes permanent"
        );
        assert_state(rt.projection(), run(10), RunState::OutcomeUnknown)?;
        Ok(witness_tally(&rt))
    })
}

/// E56: a Queryable unknown never dispatches twice for one reconcile.
#[test]
fn sim_reconcile_queryable_never_redispatches() -> Outcome {
    replay(80_u64..88, |_| -> Sim<(usize, usize, usize)> {
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([
            unknown(operation(11)),
            succeeded(operation(11)),
        ]))?;
        let work = read_work(11, 1, ReconciliationStrategy::Queryable);
        rt.offer(11, &work)?;
        rt.tick()?;
        rt.tick()?;
        assert_eq!(
            rt.owner().dispatched(),
            &[operation(11)],
            "reconcile never redispatches"
        );
        assert_eq!(
            rt.owner().reconciled(),
            &[operation(11)],
            "one status query asked"
        );
        Ok(witness_tally(&rt))
    })
}

/// E56: the receipt arrives on the reconcile tick, not the dispatch tick.
#[test]
fn sim_reconcile_receipt_arrives_on_reconcile_tick() -> Outcome {
    replay(88_u64..96, |_| -> Sim<(usize, usize, usize)> {
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([
            unknown(operation(12)),
            succeeded(operation(12)),
        ]))?;
        let work = read_work(12, 1, ReconciliationStrategy::Queryable);
        rt.offer(12, &work)?;
        assert_eq!(
            rt.tick()?.receipts().len(),
            0,
            "dispatch tick receipts nothing"
        );
        assert_eq!(
            rt.tick()?.receipts().len(),
            1,
            "reconcile tick receipts once"
        );
        Ok(witness_tally(&rt))
    })
}

/// E56: two runs under different strategies terminate independently.
#[test]
fn sim_reconcile_mixed_strategies_terminate_independently() -> Outcome {
    replay(96_u64..104, |_| -> Sim<(usize, usize, usize)> {
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([
            unknown(operation(13)),
            succeeded(operation(14)),
            succeeded(operation(13)),
        ]))?;
        let first = read_work(13, 1, ReconciliationStrategy::Queryable);
        rt.offer(13, &first)?;
        let second = read_work(14, 2, ReconciliationStrategy::RequiresHuman);
        rt.offer(14, &second)?;
        rt.tick()?;
        rt.tick()?;
        assert_state(rt.projection(), run(13), RunState::Completed)?;
        assert_state(rt.projection(), run(14), RunState::Completed)?;
        assert_eq!(
            rt.projection().receipts().len(),
            2,
            "both strategies receipt"
        );
        Ok(witness_tally(&rt))
    })
}

/// E56: seeded strategies terminate deterministically from one owner script.
#[test]
fn sim_reconcile_seeded_strategy_terminates() -> Outcome {
    replay(104_u64..112, |seed| -> Sim<SeededTally> {
        let mut random = seed.saturating_add(9);
        let strategy = match spin(&mut random) & 3 {
            0 => ReconciliationStrategy::Queryable,
            1 => ReconciliationStrategy::IdempotentRetrySafe,
            2 => ReconciliationStrategy::RequiresHuman,
            _ => ReconciliationStrategy::AlwaysCompensate,
        };
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([
            unknown(operation(15)),
            succeeded(operation(15)),
        ]))?;
        let work = read_work(15, 1, strategy);
        rt.offer(15, &work)?;
        rt.tick()?;
        rt.tick()?;
        rt.tick()?;
        let receipts = rt.projection().receipts();
        assert_eq!(receipts.len(), 1, "seeded strategy terminates once");
        let (first_d, first_t) = (receipts[0].disposition(), witness_tally(&rt));

        Ok((first_d, first_t))
    })
}

/// E56: a failed outcome under retry-safe never consumes a retry.
#[test]
fn sim_reconcile_failed_never_retries() -> Outcome {
    replay(112_u64..120, |_| -> Sim<(usize, usize, usize)> {
        let failed = EffectOutcome::Failed {
            operation: operation(16),
        };
        let mut rt: TestRuntime =
            mk_rec(ScriptedOwner::scripted([failed, succeeded(operation(16))]))?;
        let work = read_work(16, 1, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(16, &work)?;
        rt.tick()?;
        rt.tick()?;
        assert_eq!(
            rt.owner().dispatched().len(),
            1,
            "failure consumes no retry"
        );
        assert_state(rt.projection(), run(16), RunState::Failed)?;
        Ok(witness_tally(&rt))
    })
}

/// E19/E56: an unknown reconciles after a crash with a fresh owner script.
#[test]
fn sim_reconcile_unknown_reconciles_after_crash() -> Outcome {
    use larry_runtime::runtime::restore;
    use support::{clock, settings};
    replay(120_u64..128, |_| -> Sim<(usize, usize)> {
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([unknown(operation(17))]))?;
        let work = read_work(17, 1, ReconciliationStrategy::Queryable);
        rt.offer(17, &work)?;
        rt.tick()?;
        let mut restored: TestRuntime = restore(
            settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
            ScriptedOwner::scripted([succeeded(operation(17))]),
            clock()?,
            rt.into_journal(),
        )?;
        assert_eq!(
            restored.tick()?.receipts()[0].disposition(),
            TerminalDisposition::Succeeded,
            "post-crash reconcile succeeds"
        );
        Ok((
            restored.journal().len(),
            restored.projection().receipts().len(),
        ))
    })
}

/// E56: the reconcile report names every reconciled operation in order.
#[test]
fn sim_reconcile_report_names_operations_in_order() -> Outcome {
    replay(128_u64..136, |_| -> Sim<(usize, usize, usize)> {
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([
            unknown(operation(18)),
            unknown(operation(19)),
            succeeded(operation(18)),
            succeeded(operation(19)),
        ]))?;
        for number in [18_u8, 19] {
            let work = work(number, 1, number, 1, ReconciliationStrategy::Queryable);
            rt.offer(number, &work)?;
        }
        rt.tick()?;
        let tick = rt.tick()?;
        assert_eq!(
            tick.reconciled(),
            &[operation(18), operation(19)],
            "report names both in order"
        );
        assert_eq!(tick.receipts().len(), 2, "both reconcile");
        Ok(witness_tally(&rt))
    })
}

/// E56: an unknown with RequiresHuman never asks the owner twice.
#[test]
fn sim_reconcile_human_unknown_never_reasks() -> Outcome {
    replay(136_u64..144, |_| -> Sim<(usize, usize, usize)> {
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([unknown(operation(20))]))?;
        let work = read_work(20, 1, ReconciliationStrategy::RequiresHuman);
        rt.offer(20, &work)?;
        rt.tick()?;
        rt.tick()?;
        rt.tick()?;
        assert_eq!(
            rt.owner().dispatched().len(),
            1,
            "permanent unknown dispatches once"
        );
        assert!(
            rt.owner().reconciled().is_empty(),
            "permanent unknown never re-asks"
        );
        assert_eq!(rt.projection().receipts().len(), 1, "one permanent receipt");
        Ok(witness_tally(&rt))
    })
}

/// E56: retry bookkeeping stops at the three-attempt bound with no receipt.
#[test]
fn sim_reconcile_retries_stay_within_bound() -> Outcome {
    replay(144_u64..152, |seed| -> Sim<(usize, usize, usize)> {
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([
            unknown(operation(21)),
            unknown(operation(21)),
            unknown(operation(21)),
            unknown(operation(21)),
            unknown(operation(21)),
        ]))?;
        let work = read_work(21, 1, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(21, &work)?;
        for _ in 0..8 {
            rt.tick()?;
        }
        assert_eq!(
            rt.projection().receipts().len(),
            0,
            "exhausted retries invent no receipt"
        );
        assert_state(rt.projection(), run(21), RunState::Reconciling)?;
        assert_eq!(
            rt.owner().dispatched().len(),
            3,
            "seed {seed}: three attempts"
        );
        Ok(witness_tally(&rt))
    })
}

/// E56: success under RequiresHuman still authorizes at the gate.
#[test]
fn sim_reconcile_human_success_authorizes_at_gate() -> Outcome {
    replay(152_u64..160, |_| -> Sim<(usize, usize, usize)> {
        let mut rt: TestRuntime = mk_rec(ScriptedOwner::scripted([succeeded(operation(22))]))?;
        let work = read_work(22, 1, ReconciliationStrategy::RequiresHuman);
        rt.offer(22, &work)?;
        let tick = rt.tick()?;
        assert_eq!(tick.authorized(), &[operation(22)], "gate authorizes");
        assert_eq!(tick.outcomes().len(), 1, "one outcome observed");
        Ok(witness_tally(&rt))
    })
}

/// A `FileRead` runtime over `owner` on `clock`, which the test keeps a handle
/// to so it can move time past the authority's validity.
fn timed_reader(owner: ScriptedOwner, clock: lgwks_bot::clock::Clock) -> Sim<TestRuntime> {
    Ok(larry_runtime::runtime::Runtime::new(
        support::settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        owner,
        clock,
    )?)
}

/// Source: E30 (expiry is a pinned input), R72/E56 (no blind resend). A retry
/// past the authority's validity is refused by the estate rule: the effect is
/// dispatched once, never again, and stays open with no receipt.
#[test]
fn sim_reconcile_retry_past_authority_expiry_dispatches_once_and_stays_open() -> Outcome {
    replay(160_u64..168, |_| -> Sim<(usize, usize, usize)> {
        let late = support::clock()?;
        let mut rt: TestRuntime = timed_reader(
            ScriptedOwner::scripted([unknown(operation(23))]),
            late.clone(),
        )?;
        let work = read_work(23, 1, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(23, &work)?;
        rt.tick()?;
        late.advance(Duration::from_secs(3_601))?;
        rt.tick()?;
        rt.tick()?;
        assert_eq!(
            rt.owner().dispatched(),
            &[operation(23)],
            "past expiry the owner is never asked to perform the effect again"
        );
        assert_eq!(
            rt.projection().receipts().len(),
            0,
            "an expired retry invents no receipt"
        );
        assert_state(rt.projection(), run(23), RunState::Reconciling)?;
        Ok(witness_tally(&rt))
    })
}

/// Source: E30 (expiry is a pinned input), R72/E56 (no blind resend). A refused
/// retry leaves reconciliation, so the journal stops growing and no tick reports
/// the effect as reconciled again.
#[test]
fn sim_reconcile_a_refused_retry_leaves_reconciliation_and_the_journal_stops_growing() -> Outcome {
    replay(168_u64..176, |_| -> Sim<(usize, usize, usize)> {
        let late = support::clock()?;
        let mut rt: TestRuntime = timed_reader(
            ScriptedOwner::scripted([unknown(operation(24))]),
            late.clone(),
        )?;
        let work = read_work(24, 1, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(24, &work)?;
        rt.tick()?;
        late.advance(Duration::from_secs(3_601))?;
        rt.tick()?;
        let settled = rt.journal().len();
        for _ in 0..5 {
            let tick = rt.tick()?;
            assert!(
                tick.reconciled().is_empty(),
                "a refused effect is not reconciled again"
            );
        }
        assert_eq!(
            rt.journal().len(),
            settled,
            "the journal stops growing once the retry is refused"
        );
        Ok(witness_tally(&rt))
    })
}

/// Source: E30 (expiry is a pinned input), R72/E56 (no blind resend). After a
/// restore past the lease, the fresh owner is never asked to perform the effect:
/// the restored retry is refused by the estate rule, not replayed.
#[test]
fn sim_reconcile_a_restored_retry_past_expiry_never_redispatches() -> Outcome {
    use larry_runtime::runtime::restore;
    use support::settings;
    replay(176_u64..184, |_| -> Sim<(usize, usize)> {
        let early = support::clock()?;
        let mut rt: TestRuntime =
            timed_reader(ScriptedOwner::scripted([unknown(operation(25))]), early)?;
        let work = read_work(25, 1, ReconciliationStrategy::IdempotentRetrySafe);
        rt.offer(25, &work)?;
        rt.tick()?;
        let late = support::clock()?;
        late.advance(Duration::from_secs(3_601))?;
        let mut restored: TestRuntime = restore(
            settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
            ScriptedOwner::new(),
            late,
            rt.into_journal(),
        )?;
        let before = restored.journal().len();
        for _ in 0..3 {
            restored.tick()?;
        }
        assert!(
            restored.owner().dispatched().is_empty(),
            "a restored retry past expiry never reaches the owner"
        );
        assert!(
            restored.journal().len() <= before.saturating_add(1),
            "the journal grows by at most one refusal record"
        );
        Ok((
            restored.journal().len(),
            restored.projection().receipts().len(),
        ))
    })
}
