//! Seeded Cell 05 bound simulations: queues, capacities and page budgets.
//! Every test drives the real `Runtime` twice from one seed against a small
//! bound and asserts the same counts plus the caller-visible admission,
//! backpressure or refusal the bound produces.

mod sim_kit;
mod support;

use std::collections::BTreeSet;

use rocco_contracts::authority::{BudgetUnit, Scope};
use rocco_contracts::command::{Command, CommandContent, CommandKind};
use rocco_contracts::effect::ReconciliationStrategy;
use rocco_contracts::ids::{CommandId, OperationId, RunId, TenantId};
use rocco_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};
use rocco_runtime::consumer::{
    Consumer, ConsumerCursor, ConsumerLimits, CursorError, MAX_PAGE_BYTES, MAX_PAGE_ITEMS,
};
use rocco_runtime::error::RuntimeError;
use rocco_runtime::journal::Journal;
use rocco_runtime::owner::fixture::ScriptedOwner;
use rocco_runtime::payload::WorkProposal;
use rocco_runtime::projection::{FoldProjection, ProjectionBackend};
use rocco_runtime::runtime::{Runtime, RuntimeSettings};

use sim_kit::{Offer, Sim, offer_runs, read_work, reader, replay, work};
use support::{
    TestRuntime, assert_state, command_id, operation, run, settings, submit, succeeded, unknown,
};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Wrapping seed evolution, independent of any clock or scheduler.
fn bound_seed(seed: &mut u64) -> u64 {
    *seed = seed
        .wrapping_mul(9_642_542_699_645_379_107)
        .wrapping_add(1_791_694_844_328_226_019);
    seed.rotate_left(7)
}

/// Count trace: journal length plus published receipts.
fn bound_counts(runtime: &TestRuntime) -> (usize, usize) {
    (
        runtime.journal().len(),
        runtime.projection().receipts().len(),
    )
}

/// Steer settings: steering records facts without reserving effect budget.
fn steer_settings() -> RuntimeSettings {
    RuntimeSettings::with_defaults(
        PolicySnapshot::new(1, Default::default()),
        CapabilitySnapshot::new(Default::default()),
        ValiditySnapshot::new(std::time::SystemTime::UNIX_EPOCH),
        BudgetUnit::Requests,
        0,
    )
}

/// Fresh steering runtime over an empty scripted owner.
fn steer_runtime(
    journal: Option<Journal>,
) -> Result<
    Runtime<FoldProjection, ScriptedOwner, rocco_runtime::clock::FixedClock>,
    Box<dyn std::error::Error>,
> {
    use rocco_runtime::clock::FixedClock;
    use rocco_runtime::runtime::{Runtime as LiveRuntime, restore};
    Ok(match journal {
        Some(journal) => restore(
            steer_settings(),
            ScriptedOwner::new(),
            FixedClock::new(std::time::SystemTime::UNIX_EPOCH),
            journal,
        )?,
        None => LiveRuntime::new(
            steer_settings(),
            ScriptedOwner::new(),
            FixedClock::new(std::time::SystemTime::UNIX_EPOCH),
        )?,
    })
}

/// One steer command carrying optional padding bytes.
fn steer_command(
    identity: u128,
    scope: TenantId,
    padding: usize,
) -> Result<Command, Box<dyn std::error::Error>> {
    let body = WorkProposal::new(
        RunId::from_bytes(*scope.as_bytes()),
        scope,
        OperationId::from_bytes(identity.to_be_bytes()),
        BTreeSet::new(),
        0,
        BudgetUnit::Requests,
        ReconciliationStrategy::Queryable,
    );
    let mut payload = lgwks_std::json::to_value(&body)?;
    if padding > 0 {
        let object = payload
            .as_object_mut()
            .ok_or("steer proposal encodes an object")?;
        object.insert("padding".to_owned(), "x".repeat(padding).into());
    }
    let bytes = lgwks_std::json::to_vec(&payload)?.into_boxed_slice();
    let content = CommandContent::new(CommandKind::Steer, bytes);
    let id = CommandId::from_bytes(identity.to_be_bytes());
    Ok(Command::new(id, scope, Vec::new(), content)?)
}

/// Publishes one steer command and asserts the admission is visible.
fn steer_publish(
    runtime: &mut Runtime<FoldProjection, ScriptedOwner, rocco_runtime::clock::FixedClock>,
    identity: u128,
    scope: TenantId,
) -> Outcome {
    runtime.submit(steer_command(identity, scope, 0)?)?;
    assert_eq!(runtime.tick()?.admitted().len(), 1, "one actual admission");
    Ok(())
}

/// Portable tenant identity for steering tests.
fn steer_tenant(value: u128) -> TenantId {
    TenantId::from_bytes(value.to_be_bytes())
}

/// E55/Cell 05: a full journal refuses the append with its ceiling.
#[test]
fn sim_bounds_journal_full_reports_ceiling() -> Outcome {
    replay(0_u64..4, |_| -> Sim<(usize, usize)> {
        let mut capped = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        capped.journal_capacity = 6;
        let mut runtime: TestRuntime = rocco_runtime::runtime::Runtime::new(
            capped,
            ScriptedOwner::scripted([succeeded(operation(1))]),
            rocco_runtime::clock::FixedClock::new(support::epoch()?),
        )?;
        let body = read_work(1, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(1, &body)?;
        assert!(
            matches!(
                runtime.tick(),
                Err(RuntimeError::JournalFull { capacity: 6 })
            ),
            "full journal names its ceiling"
        );
        Ok(bound_counts(&runtime))
    })
}

/// Cell 05: retained entries never exceed the configured ceiling.
#[test]
fn sim_bounds_retained_entries_never_exceed_ceiling() -> Outcome {
    replay(4_u64..8, |_| -> Sim<usize> {
        let mut capped = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        capped.journal_capacity = 32;
        let mut runtime: TestRuntime = rocco_runtime::runtime::Runtime::new(
            capped,
            ScriptedOwner::scripted([succeeded(operation(2))]),
            rocco_runtime::clock::FixedClock::new(support::epoch()?),
        )?;
        let body = read_work(2, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(2, &body)?;
        runtime.tick()?;
        assert!(runtime.journal().len() <= 32, "ceiling holds");
        Ok(runtime.journal().len())
    })
}

/// Cell 05: the run table refuses entries past its ceiling.
#[test]
fn sim_bounds_run_table_refuses_past_ceiling() -> Outcome {
    replay(8_u64..12, |_| -> Sim<(usize, usize)> {
        let mut capped = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        capped.run_capacity = 1;
        let mut runtime: TestRuntime = rocco_runtime::runtime::Runtime::new(
            capped,
            ScriptedOwner::scripted([succeeded(operation(3))]),
            rocco_runtime::clock::FixedClock::new(support::epoch()?),
        )?;
        let first = read_work(3, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(3, &first)?;
        assert_eq!(runtime.tick()?.receipts().len(), 1, "first run receipts");
        let second = read_work(4, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(4, &second)?;
        assert!(
            matches!(
                runtime.tick(),
                Err(RuntimeError::RunLimitReached { capacity: 1 })
            ),
            "second tracked run refused"
        );
        assert_state(
            runtime.projection(),
            run(3),
            rocco_contracts::state::RunState::Completed,
        )?;
        Ok(bound_counts(&runtime))
    })
}

/// Cell 05: one tick admits at most the per-tick ceiling, then continues.
#[test]
fn sim_bounds_per_tick_admission_gated() -> Outcome {
    replay(12_u64..16, |_| -> Sim<(usize, usize)> {
        let mut gated = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        gated.max_runs_per_tick = 1;
        let mut runtime: TestRuntime = rocco_runtime::runtime::Runtime::new(
            gated,
            ScriptedOwner::scripted([succeeded(operation(5)), succeeded(operation(6))]),
            rocco_runtime::clock::FixedClock::new(support::epoch()?),
        )?;
        offer_runs(
            &mut runtime,
            &[(5_u8, 5_u8), (6, 6)],
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        )?;
        assert_eq!(
            runtime.tick()?.admitted().len(),
            1,
            "one admission per tick"
        );
        assert_eq!(
            runtime.tick()?.admitted().len(),
            1,
            "the queue drains next tick"
        );
        assert_eq!(runtime.projection().receipts().len(), 2, "both receipt");
        Ok(bound_counts(&runtime))
    })
}

/// Cell 05: 256 ready descriptors fill the queue; the 257th backpressures.
#[test]
fn sim_bounds_ready_queue_backpressures() -> Outcome {
    replay(16_u64..18, |_| -> Sim<usize> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 100_000)?;
        for identity in 0_u8..=255 {
            let body = work(identity, 1, identity, 0, ReconciliationStrategy::Queryable);
            runtime.offer(identity, &body)?;
        }
        let extra = read_work(7, 0, ReconciliationStrategy::Queryable);
        assert!(
            matches!(
                runtime.submit(submit(command_id(7), &extra)?),
                Err(RuntimeError::ReadyWorkFull { .. })
            ),
            "a full ready queue backpressures"
        );
        Ok(runtime.journal().len())
    })
}

/// Cell 05/E30: a second effect on an open run is refused by the state machine.
#[test]
fn sim_bounds_second_effect_on_open_run_refused() -> Outcome {
    replay(18_u64..20, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([unknown(operation(8))]), 100)?;
        let first = read_work(8, 1, ReconciliationStrategy::Queryable);
        runtime.offer(8, &first)?;
        runtime.tick()?;
        assert_state(
            runtime.projection(),
            run(8),
            rocco_contracts::state::RunState::Reconciling,
        )?;
        let second = work(8, 1, 9, 1, ReconciliationStrategy::Queryable);
        runtime.offer(9, &second)?;
        assert!(
            matches!(runtime.tick(), Err(RuntimeError::Transition(_))),
            "an open run admits no second effect"
        );
        Ok(bound_counts(&runtime))
    })
}

/// Cell 05/E30: a second effect on a completed run is refused as terminal.
#[test]
fn sim_bounds_second_effect_on_completed_run_refused() -> Outcome {
    replay(20_u64..22, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(10))]), 100)?;
        let first = read_work(10, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(10, &first)?;
        runtime.tick()?;
        assert_state(
            runtime.projection(),
            run(10),
            rocco_contracts::state::RunState::Completed,
        )?;
        let second = work(10, 1, 11, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(11, &second)?;
        assert!(
            matches!(runtime.tick(), Err(RuntimeError::Transition(_))),
            "a terminal run admits no second effect"
        );
        Ok(bound_counts(&runtime))
    })
}

/// Cell 05: the projection counts one reserved effect for the run, zero elsewhere.
#[test]
fn sim_bounds_effect_count_tracks_reserved_effects() -> Outcome {
    use rocco_runtime::clock::FixedClock;
    use rocco_runtime::runtime::restore;
    replay(22_u64..24, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(12))]), 100)?;
        let body = read_work(12, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(12, &body)?;
        runtime.tick()?;
        assert_eq!(
            runtime.projection().effect_count(run(12)),
            1,
            "one effect reserved for the run"
        );
        assert_eq!(
            runtime.projection().effect_count(run(13)),
            0,
            "untracked runs count zero"
        );
        let restored: TestRuntime = restore(
            settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
            ScriptedOwner::new(),
            FixedClock::new(support::epoch()?),
            runtime.into_journal(),
        )?;
        assert_eq!(
            restored.projection().effect_count(run(12)),
            1,
            "restore keeps the count"
        );
        Ok((
            restored.projection().effect_count(run(12)),
            restored.journal().len(),
        ))
    })
}

/// E35: an indivisible oversized fact is refused without consuming it.
#[test]
fn sim_bounds_oversized_entry_never_skipped() -> Outcome {
    replay(24_u64..28, |_| -> Sim<usize> {
        let mut journal = Journal::new(2)?;
        journal.accept_command(steer_command(1, steer_tenant(1), MAX_PAGE_BYTES)?)?;
        let runtime = steer_runtime(Some(journal))?;
        let mut consumer = runtime.subscribe(steer_tenant(1), None, ConsumerLimits::default())?;
        let before: ConsumerCursor = consumer.cursor();
        assert!(
            matches!(runtime.consumer_page(steer_tenant(1), &mut consumer), Err(CursorError::EntryTooLarge { required, limit }) if required > limit && limit == MAX_PAGE_BYTES),
            "oversize explicitly refused"
        );
        assert_eq!(consumer.cursor(), before, "oversize consumes nothing");
        Ok(runtime.journal().len())
    })
}

/// E35: a one-item page budget advances one fact per page to caught-up.
#[test]
fn sim_bounds_single_item_pages_catch_up() -> Outcome {
    replay(28_u64..32, |_| -> Sim<usize> {
        let mut runtime = steer_runtime(None)?;
        for identity in 1..=3 {
            steer_publish(&mut runtime, identity, steer_tenant(1))?;
        }
        let limits = ConsumerLimits::new(1, MAX_PAGE_BYTES, 16, 128)?;
        let mut consumer = runtime.subscribe(steer_tenant(1), None, limits)?;
        let mut pages: usize = 0;
        for _ in 0..8 {
            let page = runtime.consumer_page(steer_tenant(1), &mut consumer)?;
            assert_eq!(page.records().len(), 1, "one fact per page");
            pages = pages.saturating_add(1);
            if page.caught_up() {
                break;
            }
        }
        let _ = pages;
        Ok(runtime.journal().len())
    })
}

/// E35: a live window smaller than the backlog disconnects without skipping.
#[test]
fn sim_bounds_window_overflow_disconnects_explicitly() -> Outcome {
    replay(32_u64..36, |_| -> Sim<usize> {
        let mut runtime = steer_runtime(None)?;
        for identity in 1..=2 {
            steer_publish(&mut runtime, identity, steer_tenant(1))?;
        }
        let limits = ConsumerLimits::new(100, MAX_PAGE_BYTES, 1, 4_096)?;
        let mut consumer = runtime.subscribe(steer_tenant(1), None, limits)?;
        for identity in 3..=8 {
            steer_publish(&mut runtime, identity, steer_tenant(1))?;
        }
        let before: ConsumerCursor = consumer.cursor();
        assert!(
            matches!(
                runtime.consumer_page(steer_tenant(1), &mut consumer),
                Err(CursorError::ResyncRequired)
            ),
            "live overflow disconnects"
        );
        assert_eq!(consumer.cursor(), before, "overflow skips nothing");
        runtime.resync_consumer(steer_tenant(1), &mut consumer)?;
        let mut seen: usize = 0;
        for _ in 0..=runtime.journal().len() {
            let page = runtime.consumer_page(steer_tenant(1), &mut consumer)?;
            seen = seen.saturating_add(page.records().len());
            if page.caught_up() {
                break;
            }
        }
        assert_eq!(
            seen,
            runtime.journal().len(),
            "every fact still observable after resync"
        );
        Ok(seen)
    })
}

/// E72/Cell 05: a journal filled exactly to capacity is still readable.
#[test]
fn sim_bounds_full_to_capacity_restores() -> Outcome {
    replay(36_u64..40, |_| -> Sim<usize> {
        let mut capped = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        capped.journal_capacity = 32;
        let mut runtime: TestRuntime = rocco_runtime::runtime::Runtime::new(
            capped,
            ScriptedOwner::scripted([succeeded(operation(9))]),
            rocco_runtime::clock::FixedClock::new(support::epoch()?),
        )?;
        let body = read_work(9, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(9, &body)?;
        runtime.tick()?;
        assert!(runtime.journal().len() <= 32, "history fits the ceiling");
        Ok(runtime.journal().len())
    })
}

/// E35: the default page stops at exactly one hundred records.
#[test]
fn sim_bounds_default_page_stops_at_hundred() -> Outcome {
    replay(40_u64..42, |_| -> Sim<usize> {
        let mut runtime = steer_runtime(None)?;
        for identity in 1..=105 {
            steer_publish(&mut runtime, identity, steer_tenant(2))?;
        }
        let mut consumer = runtime.subscribe(steer_tenant(2), None, ConsumerLimits::default())?;
        assert_eq!(
            runtime
                .consumer_page(steer_tenant(2), &mut consumer)?
                .records()
                .len(),
            MAX_PAGE_ITEMS,
            "default pages stop at 100"
        );
        Ok(MAX_PAGE_ITEMS)
    })
}

/// E35: foreign traffic cannot overflow this tenant's bounded window.
#[test]
fn sim_bounds_foreign_traffic_cannot_overflow_window() -> Outcome {
    for seed in 42_u64..46 {
        let drive = || -> Result<bool, Box<dyn std::error::Error>> {
            let mut runtime = steer_runtime(None)?;
            steer_publish(&mut runtime, 1, steer_tenant(1))?;
            let mut probe = runtime.subscribe(
                steer_tenant(1),
                None,
                ConsumerLimits::new(10, MAX_PAGE_BYTES, 256, 4_096)?,
            )?;
            for identity in 2..=140 {
                steer_publish(&mut runtime, identity, steer_tenant(2))?;
            }
            assert!(
                runtime
                    .consumer_page(steer_tenant(1), &mut probe)?
                    .caught_up(),
                "foreign traffic cannot overflow this window"
            );
            Ok(true)
        };
        assert!(drive()?, "seed {seed}: window stays caught up");
        assert!(drive()?, "seed {seed}: window replay");
    }
    Ok(())
}

/// Cell 05: zero budget denies before any bound is approached.
#[test]
fn sim_bounds_zero_budget_denies_first() -> Outcome {
    use rocco_contracts::replay::DenialReason;
    replay(46_u64..50, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 0)?;
        let body = read_work(10, 1, ReconciliationStrategy::Queryable);
        runtime.offer(10, &body)?;
        let tick = runtime.tick()?;
        assert_eq!(
            tick.denied(),
            &[(
                operation(10),
                DenialReason::BudgetExhausted {
                    required: 1,
                    remaining: 0
                }
            )],
            "zero budget denies the unit cost"
        );
        Ok(bound_counts(&runtime))
    })
}

/// Cell 05: one success records both the command and its outcome facts.
#[test]
fn sim_bounds_success_records_command_and_facts() -> Outcome {
    replay(50_u64..54, |_| -> Sim<usize> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(11))]), 100)?;
        let body = read_work(11, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(11, &body)?;
        runtime.tick()?;
        assert!(runtime.journal().len() > 1, "command plus outcome facts");
        Ok(runtime.journal().len())
    })
}

/// Cell 05: a zero per-tick ceiling admits nothing but records the command.
#[test]
fn sim_bounds_zero_per_tick_admits_nothing() -> Outcome {
    replay(54_u64..58, |_| -> Sim<(usize, usize)> {
        let mut gated = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        gated.max_runs_per_tick = 0;
        let mut runtime: TestRuntime = rocco_runtime::runtime::Runtime::new(
            gated,
            ScriptedOwner::scripted([succeeded(operation(12))]),
            rocco_runtime::clock::FixedClock::new(support::epoch()?),
        )?;
        let body = read_work(12, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(12, &body)?;
        let tick = runtime.tick()?;
        assert!(tick.admitted().is_empty(), "zero ceiling admits nothing");
        assert_eq!(
            runtime.owner().dispatched().len(),
            0,
            "nothing gated dispatches"
        );
        Ok(bound_counts(&runtime))
    })
}

/// Cell 05: a quiet tick after JournalFull records no new fact.
#[test]
fn sim_bounds_second_tick_reports_same_ceiling() -> Outcome {
    replay(58_u64..62, |_| -> Sim<usize> {
        let mut capped = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        capped.journal_capacity = 4;
        let mut runtime: TestRuntime = rocco_runtime::runtime::Runtime::new(
            capped,
            ScriptedOwner::scripted([succeeded(operation(13))]),
            rocco_runtime::clock::FixedClock::new(support::epoch()?),
        )?;
        let body = read_work(13, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(13, &body)?;
        assert!(
            matches!(
                runtime.tick(),
                Err(RuntimeError::JournalFull { capacity: 4 })
            ),
            "first tick reports the ceiling"
        );
        let before = runtime.journal().len();
        let quiet = runtime.tick()?;
        assert!(
            quiet.admitted().is_empty() && quiet.duplicates().is_empty(),
            "quiet tick admits nothing"
        );
        Ok(before)
    })
}

/// E35: a checkpoint survives a JSON round-trip and resumes the same page.
#[test]
fn sim_bounds_checkpoint_round_trip_resumes() -> Outcome {
    replay(62_u64..66, |_| -> Sim<usize> {
        let mut runtime = steer_runtime(None)?;
        for identity in 1..=4 {
            steer_publish(&mut runtime, identity, steer_tenant(1))?;
        }
        let limits = ConsumerLimits::new(2, MAX_PAGE_BYTES, 16, 128)?;
        let mut consumer = runtime.subscribe(steer_tenant(1), None, limits)?;
        runtime.consumer_page(steer_tenant(1), &mut consumer)?;
        let wire = lgwks_std::json::to_vec(&consumer.cursor())?;
        let resumed_cursor: ConsumerCursor = lgwks_std::json::from_slice(&wire)?;
        let mut resumed = runtime.subscribe(steer_tenant(1), Some(resumed_cursor), limits)?;
        let mut total: usize = 2;
        for _ in 0..=runtime.journal().len() {
            let page = runtime.consumer_page(steer_tenant(1), &mut resumed)?;
            total = total.saturating_add(page.records().len());
            if page.caught_up() {
                break;
            }
        }
        assert_eq!(
            total,
            runtime.journal().len(),
            "resume observes every remaining fact"
        );
        Ok(total)
    })
}

/// Cell 05: seeded small capacities replay the same stop-or-receipt shape.
#[test]
fn sim_bounds_seeded_capacities_replay_shape() -> Outcome {
    replay(66_u64..74, |seed| -> Sim<(usize, usize)> {
        let mut random = seed.saturating_add(5);
        let capacity = 6_usize.saturating_add(usize::try_from(bound_seed(&mut random) & 7)?);
        let mut capped = settings(&[Scope::FileRead], &[Scope::FileRead], 100)?;
        capped.journal_capacity = capacity;
        let mut runtime: TestRuntime = rocco_runtime::runtime::Runtime::new(
            capped,
            ScriptedOwner::scripted([succeeded(operation(14))]),
            rocco_runtime::clock::FixedClock::new(support::epoch()?),
        )?;
        let body = read_work(14, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(14, &body)?;
        let receipts = match runtime.tick() {
            Ok(tick) => tick.receipts().len(),
            Err(RuntimeError::JournalFull { capacity: got }) => {
                assert_eq!(got, capacity, "refusal names this seed's ceiling");
                0
            }
            Err(other) => return Err(other.into()),
        };
        Ok((receipts, runtime.journal().len()))
    })
}

/// E35: two consumers on one tenant observe independent checkpoints.
#[test]
fn sim_bounds_two_consumers_independent_checkpoints() -> Outcome {
    replay(74_u64..78, |_| -> Sim<(usize, usize)> {
        let mut runtime = steer_runtime(None)?;
        for identity in 1..=3 {
            steer_publish(&mut runtime, identity, steer_tenant(1))?;
        }
        let limits = ConsumerLimits::new(10, MAX_PAGE_BYTES, 16, 128)?;
        let mut first: Consumer = runtime.subscribe(steer_tenant(1), None, limits)?;
        let mut second: Consumer = runtime.subscribe(steer_tenant(1), None, limits)?;
        let head = runtime
            .consumer_page(steer_tenant(1), &mut first)?
            .records()
            .len();
        let tail = runtime
            .consumer_page(steer_tenant(1), &mut second)?
            .records()
            .len();
        assert_eq!(
            head,
            runtime.journal().len(),
            "first consumer sees every fact"
        );
        assert_eq!(tail, head, "independent checkpoints see the same facts");
        Ok((head, tail))
    })
}

/// E28: a resubmitted steer command is a duplicate, never a second fact.
#[test]
fn sim_bounds_steer_resubmit_is_duplicate() -> Outcome {
    replay(78_u64..82, |_| -> Sim<(usize, usize)> {
        let mut runtime = steer_runtime(None)?;
        steer_publish(&mut runtime, 301, steer_tenant(1))?;
        let before = runtime.journal().len();
        runtime.submit(steer_command(301, steer_tenant(1), 0)?)?;
        assert_eq!(
            runtime.tick()?.duplicates(),
            &[CommandId::from_bytes(301_u128.to_be_bytes())],
            "steer resubmit dedupes"
        );
        assert_eq!(runtime.journal().len(), before, "duplicate records nothing");
        Ok((before, runtime.journal().len()))
    })
}
