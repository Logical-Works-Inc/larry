//! E35 public-runtime journeys. The consumer is real; the unused effect owner
//! is explicitly a fixture. One seed controls publication, interruption,
//! replay and fault positions. No wall-clock or external owner claim is made.

#[path = "sim/e35.rs"]
mod sim_e35;

use std::collections::BTreeMap;
use std::time::SystemTime;

use larry_contracts::authority::BudgetUnit;
use larry_contracts::command::{Command, CommandContent, CommandKind};
use larry_contracts::effect::ReconciliationStrategy;
use larry_contracts::ids::{CommandId, ContentDigest, OperationId, RunId, TenantId};
use larry_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};
use larry_runtime::consumer::{
    Consumer, ConsumerCursor, ConsumerLimits, CursorError, MAX_PAGE_BYTES, MAX_PAGE_ITEMS,
};
use larry_runtime::journal::{Journal, JournalEntry};
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::payload::WorkProposal;
use larry_runtime::projection::FoldProjection;
use larry_runtime::runtime::{Runtime, RuntimeSettings, restore};

/// Concrete runtime, driven exclusively through public entrypoints.
type Subject = Runtime<FoldProjection, ScriptedOwner>;
/// Errors propagate through tests instead of panicking on setup.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Reproducible, portable tenant identity.
fn tenant(value: u128) -> TenantId {
    TenantId::from_bytes(value.to_be_bytes())
}

/// Pinned settings; steering needs no effect authority or owner interaction.
fn settings() -> RuntimeSettings {
    RuntimeSettings::with_defaults(
        PolicySnapshot::new(1, Default::default()),
        CapabilitySnapshot::new(Default::default()),
        ValiditySnapshot::new(SystemTime::UNIX_EPOCH),
        BudgetUnit::Requests,
        0,
    )
}

/// Fresh runtime or restoration of an authoritative journal in hand.
fn subject(journal: Option<Journal>) -> TestResult<Subject> {
    let owner = ScriptedOwner::new();
    let clock = lgwks_bot::clock::Clock::virtual_at(std::time::Duration::ZERO);
    Ok(match journal {
        Some(journal) => restore(settings(), owner, clock, journal)?,
        None => Runtime::new(settings(), owner, clock)?,
    })
}

/// Independent tenant run identities in a shared journal.
/// Steering records facts without authorizing an effect or reserving a run.
fn command(identity: u128, scope: TenantId, padding: usize) -> TestResult<Command> {
    let body = WorkProposal::new(
        RunId::from_bytes(*scope.as_bytes()),
        scope,
        OperationId::from_bytes(identity.to_be_bytes()),
        Default::default(),
        0,
        BudgetUnit::Requests,
        ReconciliationStrategy::Queryable,
    );
    let mut payload = lgwks_std::json::to_value(&body)?;
    if padding != 0 {
        payload
            .as_object_mut()
            .ok_or("proposal must encode an object")?
            .insert("padding".to_owned(), "x".repeat(padding).into());
    }
    Ok(Command::new(
        CommandId::from_bytes(identity.to_be_bytes()),
        scope,
        Vec::new(),
        CommandContent::new(
            CommandKind::Steer,
            lgwks_std::json::to_vec(&payload)?.into_boxed_slice(),
        ),
    )?)
}

/// Publishes exactly one command and its acceptance fact at the real boundary.
fn publish(runtime: &mut Subject, identity: u128, scope: TenantId) -> TestResult {
    runtime.submit(command(identity, scope, 0)?)?;
    let tick = runtime.tick()?;
    assert_eq!(tick.admitted().len(), 1, "one actual admission");
    assert!(
        runtime.owner().dispatched().is_empty(),
        "reading/steering never dispatches"
    );
    Ok(())
}

/// Independent oracle: enumerate and filter authoritative entries directly.
fn expected(runtime: &Subject, scope: TenantId) -> BTreeMap<usize, JournalEntry> {
    runtime
        .journal()
        .entries()
        .iter()
        .enumerate()
        .filter_map(|(offset, entry)| {
            let actual = match *entry {
                JournalEntry::Command(ref command) => command.tenant(),
                JournalEntry::Event(ref event) => event.tenant(),
                _ => return None,
            };
            (actual == scope).then(|| (offset.saturating_add(1), entry.clone()))
        })
        .collect()
}

/// Applies one public page with idempotent sequence deduplication.
fn read(
    runtime: &Subject,
    scope: TenantId,
    consumer: &mut Consumer,
    seen: &mut BTreeMap<usize, JournalEntry>,
) -> TestResult<bool> {
    let page = runtime.consumer_page(scope, consumer)?;
    assert!(page.records().len() <= MAX_PAGE_ITEMS, "count is bounded");
    assert!(page.decoded_bytes() <= MAX_PAGE_BYTES, "bytes are bounded");
    for record in page.records() {
        if let Some(prior) = seen.insert(record.sequence(), record.entry().clone()) {
            assert_eq!(
                &prior,
                record.entry(),
                "duplicate sequence never changes content"
            );
        }
    }
    Ok(page.caught_up())
}

/// Finite drain proves progress instead of letting a broken cursor spin forever.
fn drain(
    runtime: &Subject,
    scope: TenantId,
    consumer: &mut Consumer,
    seen: &mut BTreeMap<usize, JournalEntry>,
) -> TestResult {
    for _ in 0..=runtime.journal().len() {
        if read(runtime, scope, consumer, seen)? {
            return Ok(());
        }
    }
    Err("consumer failed to catch up within one read per journal entry".into())
}

/// Invalid bounds, exact count/byte edges and noisy neighbors are observable.
#[test]
fn e35_bounds_and_tenant_window() -> TestResult {
    for bounds in [
        (0, 1, 1, 8),
        (101, 1, 1, 8),
        (1, 0, 1, 8),
        (1, MAX_PAGE_BYTES.saturating_add(1), 1, 8),
        (1, 1, 0, 8),
        (1, 1, 257, 4096),
        (1, 1, 1, 0),
        (1, 1, 1, 8_388_609),
    ] {
        assert_eq!(
            ConsumerLimits::new(bounds.0, bounds.1, bounds.2, bounds.3),
            Err(CursorError::InvalidBounds),
            "invalid ceiling {bounds:?}"
        );
    }
    let mut runtime = subject(None)?;
    publish(&mut runtime, 1, tenant(1))?;
    let mut probe = runtime.subscribe(
        tenant(1),
        None,
        ConsumerLimits::new(1, MAX_PAGE_BYTES, 256, 4096)?,
    )?;
    let bytes = runtime
        .consumer_page(tenant(1), &mut probe)?
        .decoded_bytes();
    let mut exact = runtime.subscribe(
        tenant(1),
        None,
        ConsumerLimits::new(
            100,
            bytes,
            256,
            std::mem::size_of::<usize>().saturating_mul(2),
        )?,
    )?;
    let page = runtime.consumer_page(tenant(1), &mut exact)?;
    assert_eq!(
        page.records().len(),
        1,
        "decoded byte limit splits the page"
    );
    assert_eq!(page.decoded_bytes(), bytes, "exact byte limit is admitted");
    for identity in 2..=140 {
        publish(&mut runtime, identity, tenant(2))?;
    }
    assert!(
        runtime.consumer_page(tenant(1), &mut exact)?.caught_up(),
        "foreign traffic cannot overflow this window"
    );
    let mut foreign = runtime.subscribe(tenant(2), None, ConsumerLimits::default())?;
    assert_eq!(
        runtime
            .consumer_page(tenant(2), &mut foreign)?
            .records()
            .len(),
        MAX_PAGE_ITEMS,
        "default pages stop at exactly 100 entries"
    );
    publish(&mut runtime, 141, tenant(1))?;
    assert!(
        runtime.consumer_page(tenant(1), &mut exact).is_ok(),
        "two descriptors exactly fit the portable byte budget"
    );
    publish(&mut runtime, 142, tenant(1))?;
    publish(&mut runtime, 143, tenant(1))?;
    assert!(
        matches!(
            runtime.consumer_page(tenant(1), &mut exact),
            Err(CursorError::ResyncRequired)
        ),
        "byte-intersected window refuses excess"
    );
    Ok(())
}

/// Forged and stale checkpoints fail before reads or recovery change state.
#[test]
fn e35_stale_cross_tenant_and_corrupt_cursors() -> TestResult {
    let mut runtime = subject(None)?;
    publish(&mut runtime, 1, tenant(1))?;
    let mut consumer = runtime.subscribe(tenant(1), None, ConsumerLimits::default())?;
    runtime.consumer_page(tenant(1), &mut consumer)?;
    let saved = consumer.cursor();
    assert!(
        matches!(
            runtime.subscribe(tenant(2), Some(saved), ConsumerLimits::default()),
            Err(CursorError::TenantMismatch)
        ),
        "foreign cursor refused"
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
    assert_eq!(consumer.cursor(), saved, "scope refusals are atomic");
    let mut other = subject(None)?;
    publish(&mut other, 2, tenant(1))?;
    assert!(
        matches!(
            other.subscribe(tenant(1), Some(saved), ConsumerLimits::default()),
            Err(CursorError::StaleCursor)
        ),
        "same-length changed history refused"
    );
    let mut unread = runtime.subscribe(tenant(1), None, ConsumerLimits::default())?;
    assert!(
        matches!(
            other.consumer_page(tenant(1), &mut unread),
            Err(CursorError::StaleCursor)
        ),
        "even an unread subscription binds its captured live history"
    );
    other.resync_consumer(tenant(1), &mut unread)?;
    assert_eq!(
        other.consumer_page(tenant(1), &mut unread)?.records().len(),
        2,
        "explicit resync can recover an unchanged empty prefix"
    );
    let empty = subject(None)?;
    assert_eq!(
        empty.resync_consumer(tenant(1), &mut consumer),
        Err(CursorError::StaleCursor),
        "truncated history refused"
    );
    assert!(
        matches!(
            empty.consumer_page(tenant(1), &mut consumer),
            Err(CursorError::StaleCursor)
        ),
        "stale active consumer refused"
    );
    for field in ["consumed", "sequence", "prefix"] {
        let mut wire = lgwks_std::json::to_value(&saved)?;
        let replacement = if field == "prefix" {
            lgwks_std::json::to_value(&ContentDigest::over(&[b"corruption"]))?
        } else {
            lgwks_std::json::to_value(&usize::MAX)?
        };
        *wire.get_mut(field).ok_or("checkpoint field absent")? = replacement;
        let corrupt: ConsumerCursor = lgwks_std::json::from_value(wire)?;
        assert!(
            matches!(
                runtime.subscribe(tenant(1), Some(corrupt), ConsumerLimits::default()),
                Err(CursorError::StaleCursor)
            ),
            "corrupt {field} refused"
        );
    }
    // Corrupt actual recorded bytes but retain every envelope digest. This
    // is an observation-integrity test, not an admission-validation change.
    let original = command(91, tenant(1), 0)?;
    let mut journal = Journal::new(2)?;
    journal.accept_command(original.clone())?;
    let recorded = subject(Some(journal))?;
    let mut original_consumer = recorded.subscribe(tenant(1), None, ConsumerLimits::default())?;
    recorded.consumer_page(tenant(1), &mut original_consumer)?;
    let mut wire = lgwks_std::json::to_value(&original)?;
    let payload = wire
        .get_mut("content")
        .and_then(|value| value.get_mut("payload"))
        .and_then(lgwks_std::json::Value::as_array_mut)
        .ok_or("encoded command payload absent")?;
    *payload.first_mut().ok_or("payload empty")? = 33_u8.into();
    let corrupted: Command = lgwks_std::json::from_value(wire)?;
    let mut journal = Journal::new(2)?;
    journal.accept_command(corrupted)?;
    let changed = subject(Some(journal))?;
    assert!(
        matches!(
            changed.subscribe(
                tenant(1),
                Some(original_consumer.cursor()),
                ConsumerLimits::default()
            ),
            Err(CursorError::StaleCursor)
        ),
        "changed bytes cannot hide behind an unchanged claimed digest"
    );
    let malformed = lgwks_std::json::from_str::<ConsumerCursor>("{\"sequence\":-1}");
    assert!(malformed.is_err(), "malformed token cannot become a cursor");
    let mut restarted = other.subscribe(tenant(1), None, ConsumerLimits::default())?;
    assert_eq!(
        other
            .consumer_page(tenant(1), &mut restarted)?
            .records()
            .len(),
        2,
        "explicit origin resync replays all history"
    );
    Ok(())
}

/// An indivisible oversized fact never turns into an empty successful page.
#[test]
fn e35_oversized_entry_never_skipped() -> TestResult {
    let mut journal = Journal::new(2)?;
    journal.accept_command(command(1, tenant(1), MAX_PAGE_BYTES)?)?;
    let runtime = subject(Some(journal))?;
    let mut consumer = runtime.subscribe(tenant(1), None, ConsumerLimits::default())?;
    let before = consumer.cursor();
    let result = runtime.consumer_page(tenant(1), &mut consumer);
    assert!(
        matches!(result, Err(CursorError::EntryTooLarge { required, limit }) if required > limit && limit == MAX_PAGE_BYTES),
        "oversize explicitly refused"
    );
    assert_eq!(
        consumer.cursor(),
        before,
        "oversize cannot consume the fact"
    );
    runtime.resync_consumer(tenant(1), &mut consumer)?;
    assert!(
        matches!(
            runtime.consumer_page(tenant(1), &mut consumer),
            Err(CursorError::EntryTooLarge { .. })
        ),
        "resync cannot hide oversize"
    );
    assert_eq!(runtime.journal().len(), 1, "history remains intact");
    Ok(())
}

/// Bounded caller-owned subscriptions and 5,000 distinct tenant histories.
#[test]
fn e35_many_consumers_and_tenants() -> TestResult {
    let mut journal = Journal::new(10_000)?;
    for identity in 1..=5_000 {
        journal.accept_command(command(identity, tenant(identity), 0)?)?;
    }
    let runtime = subject(Some(journal))?;
    for identity in 1..=5_000 {
        let mut consumer = runtime.subscribe(tenant(identity), None, ConsumerLimits::default())?;
        let page = runtime.consumer_page(tenant(identity), &mut consumer)?;
        assert_eq!(
            page.records().len(),
            1,
            "exactly this tenant's accepted command"
        );
        assert!(page.caught_up(), "no foreign backlog");
    }
    for count in [100, 1_000, 10_000, 100_000] {
        let mut consumers: Vec<Consumer> = (0..count)
            .map(|_| runtime.subscribe(tenant(1), None, ConsumerLimits::default()))
            .collect::<Result<_, _>>()?;
        for consumer in &mut consumers {
            assert_eq!(
                runtime.consumer_page(tenant(1), consumer)?.records().len(),
                1,
                "simultaneously live consumers have independent checkpoints"
            );
        }
        assert_eq!(
            runtime.journal().len(),
            5_000,
            "consumer count never mutates authoritative history"
        );
    }
    Ok(())
}

/// One owned producer thread races two consumers on the real shared runtime;
/// a bounded test notification channel owns coordination, never journal facts.
#[test]
fn e35_concurrent_publication_and_recovery() -> TestResult {
    let runtime = std::sync::Mutex::new(subject(None)?);
    let limits = ConsumerLimits::new(3, MAX_PAGE_BYTES, 4, 4096)?;
    let mut consumers = {
        let guard = runtime.lock().map_err(|_| "runtime lock poisoned")?;
        [
            guard.subscribe(tenant(1), None, limits)?,
            guard.subscribe(tenant(2), None, limits)?,
        ]
    };
    let mut histories = [BTreeMap::new(), BTreeMap::new()];
    let (notify, arrivals) = std::sync::mpsc::sync_channel(1);
    std::thread::scope(|threads| -> TestResult {
        let publisher = threads.spawn(|| -> Result<(), String> {
            let run = || -> TestResult {
                for batch in 0_u128..64 {
                    {
                        let mut guard = runtime.lock().map_err(|_| "runtime lock poisoned")?;
                        for item in 1_u128..=8 {
                            let identity = batch.saturating_mul(8).saturating_add(item);
                            publish(&mut guard, identity, tenant((item & 1).saturating_add(1)))?;
                        }
                    }
                    notify.send(())?;
                }
                Ok(())
            };
            let result = run().map_err(|error| error.to_string());
            drop(notify);
            result
        });
        for () in arrivals {
            let guard = runtime.lock().map_err(|_| "runtime lock poisoned")?;
            for (index, consumer) in consumers.iter_mut().enumerate() {
                let scope = tenant(u128::try_from(index)?.saturating_add(1));
                let saved = consumer.cursor();
                if let Err(error) = guard.consumer_page(scope, consumer) {
                    assert_eq!(
                        error,
                        CursorError::ResyncRequired,
                        "slow threaded consumer explicitly disconnects"
                    );
                    assert_eq!(consumer.cursor(), saved, "no silent skip at overflow");
                    guard.resync_consumer(scope, consumer)?;
                } else {
                    // Deliberately lose this page reply, then retry the saved checkpoint.
                    *consumer = guard.subscribe(scope, Some(saved), limits)?;
                }
                let seen = histories.get_mut(index).ok_or("tenant history missing")?;
                drain(&guard, scope, consumer, seen)?;
            }
        }
        publisher.join().map_err(|_| "publisher panicked")??;
        Ok(())
    })?;
    let guard = runtime.lock().map_err(|_| "runtime lock poisoned")?;
    for (index, seen) in histories.iter().enumerate() {
        assert_eq!(
            *seen,
            expected(&guard, tenant(u128::try_from(index)?.saturating_add(1))),
            "threaded reader sees every scoped fact"
        );
    }
    Ok(())
}
