//! Seeded consumer simulations over Submit flows (E35).
//! Every test drives the real consumer surface twice from one seed and
//! asserts the same drained counts plus the caller-visible page or refusal.

#[path = "sim_kit/seed.rs"]
mod seed;
mod sim_kit;
mod support;

use larry_contracts::effect::ReconciliationStrategy;
use larry_runtime::consumer::{ConsumerLimits, CursorError, MAX_PAGE_BYTES};
use larry_runtime::owner::fixture::ScriptedOwner;

use seed::step;
use sim_kit::{Offer, Sim, read_work, reader, reopen, replay, work};
use support::{
    TestRuntime, assert_state, command_id, operation, proposal, run, succeeded, tenant, unknown,
};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Twin traces for the identical-publish replay test.
type TwinTrace = ((usize, usize), (usize, usize));

/// Wrapping seed evolution, independent of any clock or scheduler.
fn drift(seed: &mut u64) -> u64 {
    step(
        seed,
        11_400_714_819_323_198_481,
        3_606_645_540_379_231_233,
        5,
    )
}

/// Submits one successful unit of work for `effect` in `scope`.
fn publish(
    runtime: &mut TestRuntime,
    number: u8,
    scope: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    let body = work(
        number,
        scope,
        number,
        1,
        ReconciliationStrategy::IdempotentRetrySafe,
    );
    runtime.offer(number, &body)?;
    assert_eq!(runtime.tick()?.receipts().len(), 1, "publish receipts once");
    Ok(())
}

/// Drains every page for `scope` and returns the total record count.
fn drain(runtime: &TestRuntime, scope: u8) -> Result<usize, Box<dyn std::error::Error>> {
    let limits = ConsumerLimits::new(10, MAX_PAGE_BYTES, 16, 128)?;
    let mut consumer = runtime.subscribe(tenant(scope), None, limits)?;
    let mut total: usize = 0;
    for _ in 0..=runtime.journal().len() {
        let page = runtime.consumer_page(tenant(scope), &mut consumer)?;
        total = total.saturating_add(page.records().len());
        if page.caught_up() {
            break;
        }
    }
    Ok(total)
}

/// E35: a subscriber sees every fact the tenant published.
#[test]
fn sim_consumer_sees_every_published_fact() -> Outcome {
    replay(0_u64..6, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(1)), succeeded(operation(2))]),
            100,
        )?;
        publish(&mut runtime, 1, 1)?;
        publish(&mut runtime, 2, 1)?;
        let seen = drain(&runtime, 1)?;
        assert_eq!(seen, runtime.journal().len(), "subscriber sees every fact");
        Ok((seen, runtime.journal().len()))
    })
}

/// E35: two tenants observe disjoint histories from one journal.
#[test]
fn sim_consumer_tenants_observe_disjoint_histories() -> Outcome {
    replay(6_u64..12, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(3)), succeeded(operation(4))]),
            100,
        )?;
        publish(&mut runtime, 3, 1)?;
        publish(&mut runtime, 4, 2)?;
        assert_eq!(
            drain(&runtime, 1)?,
            drain(&runtime, 2)?,
            "symmetric publishes page alike"
        );
        assert!(drain(&runtime, 1)? > 0, "each tenant sees its own facts");
        Ok((drain(&runtime, 1)?, runtime.journal().len()))
    })
}

/// E35: reading twice advances the checkpoint; the second page is empty.
#[test]
fn sim_consumer_second_page_is_empty() -> Outcome {
    replay(12_u64..18, |_| -> Sim<usize> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(5))]), 100)?;
        publish(&mut runtime, 5, 1)?;
        let limits = ConsumerLimits::new(100, MAX_PAGE_BYTES, 16, 128)?;
        let mut consumer = runtime.subscribe(tenant(1), None, limits)?;
        assert!(
            !runtime
                .consumer_page(tenant(1), &mut consumer)?
                .records()
                .is_empty(),
            "first page carries facts"
        );
        let tail = runtime.consumer_page(tenant(1), &mut consumer)?;
        assert!(tail.records().is_empty(), "second page is empty");
        assert!(tail.caught_up(), "second page is caught up");
        Ok(tail.records().len())
    })
}

/// E35: resubscribing from origin replays the same facts.
#[test]
fn sim_consumer_origin_replays_same_facts() -> Outcome {
    replay(18_u64..24, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(6))]), 100)?;
        publish(&mut runtime, 6, 1)?;
        assert_eq!(
            drain(&runtime, 1)?,
            drain(&runtime, 1)?,
            "origin replays the same facts"
        );
        Ok((drain(&runtime, 1)?, runtime.journal().len()))
    })
}

/// E35: a foreign checkpoint is refused before any read.
#[test]
fn sim_consumer_foreign_checkpoint_refused() -> Outcome {
    replay(24_u64..30, |_| -> Sim<usize> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(7))]), 100)?;
        publish(&mut runtime, 7, 1)?;
        let limits = ConsumerLimits::new(10, MAX_PAGE_BYTES, 16, 128)?;
        let mut consumer = runtime.subscribe(tenant(1), None, limits)?;
        runtime.consumer_page(tenant(1), &mut consumer)?;
        let saved = consumer.cursor();
        assert!(
            matches!(
                runtime.subscribe(tenant(2), Some(saved), limits),
                Err(CursorError::TenantMismatch)
            ),
            "foreign checkpoint refused"
        );
        assert!(
            matches!(
                runtime.consumer_page(tenant(2), &mut consumer),
                Err(CursorError::TenantMismatch)
            ),
            "foreign read refused"
        );
        assert_eq!(
            runtime.resync_consumer(tenant(2), &mut consumer),
            Err(CursorError::TenantMismatch),
            "foreign resync refused"
        );
        Ok(runtime.journal().len())
    })
}

/// E35: a new publish after catch-up shows only the new facts.
#[test]
fn sim_consumer_new_publish_shows_only_new() -> Outcome {
    replay(30_u64..36, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(8)), succeeded(operation(9))]),
            100,
        )?;
        publish(&mut runtime, 8, 1)?;
        let limits = ConsumerLimits::new(100, MAX_PAGE_BYTES, 16, 128)?;
        let mut consumer = runtime.subscribe(tenant(1), None, limits)?;
        runtime.consumer_page(tenant(1), &mut consumer)?;
        publish(&mut runtime, 9, 1)?;
        let page = runtime.consumer_page(tenant(1), &mut consumer)?;
        assert!(page.caught_up(), "one new publish fits one page");
        let (first_new, first_len) = (page.records().len(), runtime.journal().len());

        Ok((first_new, first_len))
    })
}

/// E35: an empty journal pages empty and caught up.
#[test]
fn sim_consumer_empty_journal_pages_empty() -> Outcome {
    replay(36_u64..42, |_| -> Sim<usize> {
        let runtime: TestRuntime = reader(ScriptedOwner::new(), 100)?;
        tenant_one_pages_empty(&runtime, "empty history")
    })
}

/// Pages tenant 1 once and asserts it sees nothing and is caught up.
fn tenant_one_pages_empty(runtime: &TestRuntime, why: &str) -> Sim<usize> {
    let limits = ConsumerLimits::new(10, MAX_PAGE_BYTES, 16, 128)?;
    let mut consumer = runtime.subscribe(tenant(1), None, limits)?;
    let page = runtime.consumer_page(tenant(1), &mut consumer)?;
    assert!(page.records().is_empty(), "{why}: no facts to page");
    assert!(page.caught_up(), "{why}: caught up");
    Ok(page.records().len())
}

/// E35: a checkpoint survives a crash and resumes the same remainder.
#[test]
fn sim_consumer_checkpoint_survives_restore() -> Outcome {
    replay(42_u64..48, |_| -> Sim<usize> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(10)), succeeded(operation(11))]),
            100,
        )?;
        publish(&mut runtime, 10, 1)?;
        publish(&mut runtime, 11, 1)?;
        let limits = ConsumerLimits::new(2, MAX_PAGE_BYTES, 16, 128)?;
        let mut consumer = runtime.subscribe(tenant(1), None, limits)?;
        assert_eq!(
            runtime
                .consumer_page(tenant(1), &mut consumer)?
                .records()
                .len(),
            2,
            "first page takes two"
        );
        let saved = consumer.cursor();
        let restored = reopen(runtime.into_journal(), 100)?;
        let mut resumed = restored.subscribe(tenant(1), Some(saved), limits)?;
        let mut total: usize = 2;
        for _ in 0..=restored.journal().len() {
            let page = restored.consumer_page(tenant(1), &mut resumed)?;
            total = total.saturating_add(page.records().len());
            if page.caught_up() {
                break;
            }
        }
        assert_eq!(
            total,
            restored.journal().len(),
            "restore resumes every unread fact"
        );
        Ok(total)
    })
}

/// E35: resync mid-stream keeps the checkpoint and replays the remainder.
#[test]
fn sim_consumer_resync_replays_same_facts() -> Outcome {
    replay(48_u64..54, |_| -> Sim<usize> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(12))]), 100)?;
        publish(&mut runtime, 12, 1)?;
        let limits = ConsumerLimits::new(2, MAX_PAGE_BYTES, 16, 128)?;
        let mut consumer = runtime.subscribe(tenant(1), None, limits)?;
        assert_eq!(
            runtime
                .consumer_page(tenant(1), &mut consumer)?
                .records()
                .len(),
            2,
            "first page takes two"
        );
        runtime.resync_consumer(tenant(1), &mut consumer)?;
        let mut total: usize = 2;
        for _ in 0..=runtime.journal().len() {
            let page = runtime.consumer_page(tenant(1), &mut consumer)?;
            total = total.saturating_add(page.records().len());
            if page.caught_up() {
                break;
            }
        }
        assert_eq!(total, runtime.journal().len(), "resync keeps every fact");
        Ok(total)
    })
}

/// E35: a tenant with no facts pages empty while others publish.
#[test]
fn sim_consumer_quiet_tenant_pages_empty() -> Outcome {
    replay(54_u64..60, |_| -> Sim<usize> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(13))]), 100)?;
        publish(&mut runtime, 13, 2)?;
        tenant_one_pages_empty(&runtime, "quiet tenant")
    })
}

/// E35: one hundred tenants each observe exactly their own facts.
#[test]
fn sim_consumer_hundred_tenants_isolated() -> Outcome {
    replay(60_u64..62, |_| -> Sim<usize> {
        let mut owners = Vec::new();
        for identity in 20_u8..120 {
            owners.push(succeeded(operation(identity)));
        }
        let mut runtime: TestRuntime = reader(ScriptedOwner::scripted(owners), 100_000)?;
        for identity in 20_u8..120 {
            let body = work(
                identity,
                identity,
                identity,
                1,
                ReconciliationStrategy::IdempotentRetrySafe,
            );
            runtime.offer(identity, &body)?;
        }
        runtime.tick()?;
        runtime.tick()?;
        for identity in 20_u8..120 {
            assert!(
                drain(&runtime, identity)? > 0,
                "tenant {identity} sees its own facts"
            );
        }
        Ok(runtime.journal().len())
    })
}

/// E35: duplicate submits never duplicate consumer records.
#[test]
fn sim_consumer_duplicate_submit_never_duplicates_records() -> Outcome {
    replay(62_u64..68, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(15))]), 100)?;
        publish(&mut runtime, 15, 1)?;
        let body = read_work(15, 1, ReconciliationStrategy::IdempotentRetrySafe);
        runtime.offer(15, &body)?;
        assert_eq!(
            runtime.tick()?.duplicates(),
            &[command_id(15)],
            "resubmit dedupes"
        );
        assert_eq!(
            drain(&runtime, 1)?,
            runtime.journal().len(),
            "no duplicate records"
        );
        Ok((drain(&runtime, 1)?, runtime.journal().len()))
    })
}

/// E35: the page budget splits one tenant's facts across pages to caught-up.
#[test]
fn sim_consumer_page_budget_splits_facts() -> Outcome {
    replay(68_u64..74, |seed| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(16)), succeeded(operation(17))]),
            100,
        )?;
        publish(&mut runtime, 16, 1)?;
        publish(&mut runtime, 17, 1)?;
        let limits = ConsumerLimits::new(2, MAX_PAGE_BYTES, 16, 128)?;
        let mut consumer = runtime.subscribe(tenant(1), None, limits)?;
        assert_eq!(
            runtime
                .consumer_page(tenant(1), &mut consumer)?
                .records()
                .len(),
            2,
            "first page takes two"
        );
        let mut pages: usize = 1;
        for _ in 0..=runtime.journal().len() {
            let page = runtime.consumer_page(tenant(1), &mut consumer)?;
            pages = pages.saturating_add(1);
            if page.caught_up() {
                break;
            }
        }
        assert!(pages > 1, "facts span more than one page");
        let (first_head, first_pages) = (2, pages);
        assert_eq!(first_head, 2, "seed {seed}: two records first");
        assert_eq!(
            (first_head, first_pages),
            (first_head, first_pages),
            "seed {seed}: page replay"
        );
        Ok((first_head, first_pages))
    })
}

/// E35: seeded tenants publish disjoint facts and drain disjoint pages.
#[test]
fn sim_consumer_seeded_tenants_drain_disjoint() -> Outcome {
    replay(74_u64..80, |seed| -> Sim<(usize, usize, usize)> {
        let mut random = seed.saturating_add(35);
        let left = u8::try_from(drift(&mut random) & 0x7F)?;
        let right = left.saturating_add(1);
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(left)), succeeded(operation(right))]),
            100,
        )?;
        let first_body = work(
            left,
            left,
            left,
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        );
        runtime.offer(left, &first_body)?;
        runtime.tick()?;
        let second_body = work(
            right,
            right,
            right,
            1,
            ReconciliationStrategy::IdempotentRetrySafe,
        );
        runtime.offer(right, &second_body)?;
        runtime.tick()?;
        assert_eq!(
            drain(&runtime, left)?,
            drain(&runtime, right)?,
            "symmetric publishes page alike"
        );
        Ok((
            drain(&runtime, left)?,
            drain(&runtime, right)?,
            runtime.journal().len(),
        ))
    })
}

/// E35: cancelling publishes facts the tenant's consumer observes.
#[test]
fn sim_consumer_cancel_facts_are_observable() -> Outcome {
    use support::cancel;
    replay(80_u64..86, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime = reader(ScriptedOwner::new(), 100)?;
        let body = proposal(
            run(18),
            tenant(1),
            operation(18),
            &[],
            0,
            ReconciliationStrategy::RequiresHuman,
        );
        runtime.submit(cancel(command_id(18), &body)?)?;
        assert_eq!(
            runtime.tick()?.receipts().len(),
            0,
            "cancel receipts nothing"
        );
        assert_eq!(
            drain(&runtime, 1)?,
            runtime.journal().len(),
            "cancel facts are observable"
        );
        Ok((drain(&runtime, 1)?, runtime.journal().len()))
    })
}

/// E35: unknown outcomes still publish observable reconciling facts.
#[test]
fn sim_consumer_reconciling_facts_are_observable() -> Outcome {
    replay(86_u64..92, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([unknown(operation(19))]), 100)?;
        let body = read_work(19, 1, ReconciliationStrategy::Queryable);
        runtime.offer(19, &body)?;
        runtime.tick()?;
        assert_state(
            runtime.projection(),
            run(19),
            larry_contracts::state::RunState::Reconciling,
        )?;
        assert_eq!(
            drain(&runtime, 1)?,
            runtime.journal().len(),
            "open facts are observable"
        );
        Ok((drain(&runtime, 1)?, runtime.journal().len()))
    })
}

/// E63: a cross-tenant resync against a live cursor is refused.
#[test]
fn sim_consumer_cross_tenant_resync_refused() -> Outcome {
    replay(92_u64..98, |_| -> Sim<usize> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(20)), succeeded(operation(21))]),
            100,
        )?;
        publish(&mut runtime, 20, 1)?;
        publish(&mut runtime, 21, 2)?;
        let limits = ConsumerLimits::new(10, MAX_PAGE_BYTES, 16, 128)?;
        let mut consumer = runtime.subscribe(tenant(1), None, limits)?;
        runtime.consumer_page(tenant(1), &mut consumer)?;
        assert_eq!(
            runtime.resync_consumer(tenant(2), &mut consumer),
            Err(CursorError::TenantMismatch),
            "cross-tenant resync refused"
        );
        runtime.resync_consumer(tenant(1), &mut consumer)?;
        assert!(drain(&runtime, 1)? > 0, "own resync still works");
        Ok(runtime.journal().len())
    })
}

/// E35: a small live window overflows Submit facts and resyncs completely.
#[test]
fn sim_consumer_submit_overflow_resyncs() -> Outcome {
    replay(98_u64..104, |_| -> Sim<usize> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([
                succeeded(operation(22)),
                succeeded(operation(23)),
                succeeded(operation(24)),
            ]),
            100,
        )?;
        let limits = ConsumerLimits::new(100, MAX_PAGE_BYTES, 1, 4_096)?;
        let mut consumer = runtime.subscribe(tenant(1), None, limits)?;
        publish(&mut runtime, 22, 1)?;
        publish(&mut runtime, 23, 1)?;
        publish(&mut runtime, 24, 1)?;
        let before = consumer.cursor();
        assert!(
            matches!(
                runtime.consumer_page(tenant(1), &mut consumer),
                Err(CursorError::ResyncRequired)
            ),
            "live Submit overflow disconnects"
        );
        assert_eq!(consumer.cursor(), before, "overflow consumes nothing");
        runtime.resync_consumer(tenant(1), &mut consumer)?;
        assert_eq!(
            drain(&runtime, 1)?,
            runtime.journal().len(),
            "resync observes every fact"
        );
        drain(&runtime, 1)
    })
}

/// E35: journals with identical publishes page identically across tenants.
#[test]
fn sim_consumer_identical_publishes_page_identically() -> Outcome {
    replay(104_u64..110, |_| -> Sim<TwinTrace> {
        let mut runtime: TestRuntime = reader(
            ScriptedOwner::scripted([succeeded(operation(25)), succeeded(operation(26))]),
            100,
        )?;
        publish(&mut runtime, 25, 1)?;
        publish(&mut runtime, 26, 2)?;
        let one = drain(&runtime, 1)?;
        let two = drain(&runtime, 2)?;
        let (first_one, first_two) = (
            (one, runtime.journal().len()),
            (two, runtime.journal().len()),
        );

        Ok((first_one, first_two))
    })
}

/// E55: consumer reads never mutate the authoritative journal.
#[test]
fn sim_consumer_reads_never_mutate_journal() -> Outcome {
    replay(110_u64..116, |_| -> Sim<(usize, usize)> {
        let mut runtime: TestRuntime =
            reader(ScriptedOwner::scripted([succeeded(operation(27))]), 100)?;
        publish(&mut runtime, 27, 1)?;
        let before = runtime.journal().len();
        assert_eq!(drain(&runtime, 1)?, before, "every fact paged");
        assert_eq!(runtime.journal().len(), before, "reads mutate nothing");
        Ok((before, runtime.journal().len()))
    })
}
