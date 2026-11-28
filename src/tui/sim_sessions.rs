//! Seeded simulations of the TUI session row (larry#256, larry#578, ADR 0133):
//! the snapshot the state database holds, its bounds, and the driver that
//! adopts and saves it. One seed drives every entry; no terminal. Stored bytes
//! are read back through `SQLite` itself, never through the code that wrote
//! them. Source: `SQLite` atomic commit; the bounds in ADR 0133.

use std::path::Path;

use lgwks_deps::storefront::rusqlite::Connection;
use lgwks_std::seeded::Seeded;

use super::{MAX_RECEIPTS_SEEN, MAX_SESSIONS, MAX_STORED_BYTES, Owner, Snapshot, validate_name};
use crate::executor::{ExecError, Ledger, SessionStoreError, Settings, sim_tree};
use crate::sim_error::SimError;
use crate::tui::app::{App, Input, Key};
use crate::tui::cells::{Entry, Header};
use crate::tui::session_store::{SessionStore, Transcript};
use crate::tui::worker::{Event, Receipt};

/// What a test returns.
type Outcome = Result<(), SimError>;

/// How many bytes `kept` encodes to.
fn bytes(kept: &Snapshot) -> Result<usize, SimError> {
    let encoded = kept.to_json().map_err(ExecError::TuiSession)?;
    Ok(encoded.len())
}

/// One stored receipt with this digest.
fn receipt(digest: &str) -> Receipt {
    Receipt {
        digest: digest.to_owned(),
        accepted: true,
        commands: 1,
        declined: 0,
        interrupted: 0,
        handoff: String::from("applied"),
    }
}

/// A seeded snapshot: every entry kind, the styled notice included.
fn snapshot(seed: u64) -> Result<Snapshot, SimError> {
    let mut rng = Seeded::from_seed(seed);
    let mut kept = Snapshot::empty();
    kept.entries = vec![
        Entry::User(String::from("write hello.txt")),
        Entry::Declined(String::from("rm -rf /")),
        Entry::Said(String::from("wrote hello.txt")),
        Entry::Notice(crate::tui::style::Style::WARN, String::from("careful")),
        Entry::Info(vec![String::from("model: mock")]),
    ];
    for _ in 0..rng.below(8)? {
        kept.entries.push(Entry::Ran {
            command: format!("run {}", rng.next_u64()),
            status: String::from("exit status 0"),
            output: vec![format!("line {}", rng.next_u64())],
            hidden: rng.index(3)?,
            passed: rng.below(2)? == 0,
        });
    }
    for _ in 0..rng.below(4)? {
        kept.receipts
            .push(receipt(&format!("{:012x}", rng.next_u64())));
    }
    Ok(kept)
}

/// The update stamp and the snapshot of every row under `state`, oldest first.
fn stored(state: &Path) -> Result<Vec<(i64, Snapshot)>, SimError> {
    let store = Connection::open(state.join("executor.db"))?;
    let mut statement =
        store.prepare("SELECT updated, transcript FROM tui_sessions ORDER BY rowid")?;
    let mapped = statement.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut rows = Vec::new();
    for row in mapped {
        let (updated, transcript) = row?;
        rows.push((updated, lgwks_std::json::from_slice(transcript.as_bytes())?));
    }
    Ok(rows)
}

/// The owner of the rows the store sims write.
fn owner() -> Owner {
    Owner {
        tenant: String::from("tenant-a"),
        folder: String::from("/work"),
        model: String::from("mock-model"),
    }
}

/// A tenant holds [`MAX_SESSIONS`] rows and no more: the next create is a typed
/// refusal naming the bound and writes nothing.
#[test]
fn sim_a_full_tenant_is_refused_typed() -> Outcome {
    let directory = sim_tree::scratch("tui-sessions-full")?;
    let ledger = Ledger::open(&directory)?;
    let connection = Connection::open(directory.join("executor.db"))?;
    connection.execute(
        "WITH RECURSIVE held(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM held WHERE n < ?1) \
         INSERT INTO tui_sessions (tenant, id, name, folder, model, updated, transcript) \
         SELECT 'tenant-a', 'row-' || n, 'held', '/work', 'mock-model', n, '{}' FROM held",
        [MAX_SESSIONS.saturating_sub(1)],
    )?;
    super::create(&ledger, &owner(), "last", 0, &Snapshot::empty())?;
    let refused = super::create(&ledger, &owner(), "past", 0, &Snapshot::empty());
    assert!(
        matches!(
            refused,
            Err(ExecError::TuiSession(SessionStoreError::Full {
                bound: MAX_SESSIONS
            }))
        ),
        "the bound is typed: {refused:?}"
    );
    let held: u32 =
        connection.query_row("SELECT COUNT(*) FROM tui_sessions", [], |row| row.get(0))?;
    assert_eq!(held, MAX_SESSIONS, "the refusal wrote nothing");
    Ok(())
}

/// The row keeps the snapshot whole: every entry kind and every receipt digest
/// reads back equal, and a save replaces it. The defect (larry#256) is a session
/// lost to a restart.
#[test]
fn sim_sessions_hold_the_snapshot_whole() -> Outcome {
    for seed in 0..32_u64 {
        let directory = sim_tree::scratch("tui-sessions-round")?;
        let ledger = Ledger::open(&directory)?;
        let before = snapshot(seed)?;
        let stamp = i64::try_from(seed)?;
        let id = super::create(&ledger, &owner(), "demo", stamp, &before)?;
        assert_eq!(
            stored(&directory)?,
            [(stamp, before.clone())],
            "seed {seed}"
        );
        let mut changed = before;
        changed
            .entries
            .push(Entry::User(String::from("a later ask")));
        super::save(&ledger, &owner(), &id, stamp.saturating_add(1), &changed)?;
        assert_eq!(
            stored(&directory)?,
            [(stamp.saturating_add(1), changed)],
            "seed {seed}: the save lands"
        );
    }
    Ok(())
}

/// Names are checked at the door, and a save of no row is a typed refusal.
#[test]
fn sim_session_names_and_missing_rows_are_typed() -> Outcome {
    assert!(
        validate_name(&"x".repeat(64)).is_ok(),
        "64 characters are admitted"
    );
    for bad in ["", "   ", "a\nb", "a\tb", &"x".repeat(65)] {
        assert!(validate_name(bad).is_err(), "{bad:?} is refused");
    }
    assert_eq!(validate_name("  demo  ").ok().as_deref(), Some("demo"));
    let directory = sim_tree::scratch("tui-sessions-names")?;
    let ledger = Ledger::open(&directory)?;
    let missing = super::save(&ledger, &owner(), "no-such-session", 0, &Snapshot::empty());
    assert!(
        matches!(
            missing,
            Err(ExecError::TuiSession(
                SessionStoreError::NoSuchSession { .. }
            ))
        ),
        "an unknown id is typed: {missing:?}"
    );
    Ok(())
}

/// A snapshot exactly at the byte bound is admitted by a create and a save; one
/// byte past it is refused by both.
#[test]
fn sim_the_byte_bound_admits_its_size_and_refuses_one_past() -> Outcome {
    let directory = sim_tree::scratch("tui-sessions-bound")?;
    let ledger = Ledger::open(&directory)?;
    let sized = |pad: usize| {
        let mut kept = Snapshot::empty();
        kept.entries = vec![Entry::Said("x".repeat(pad))];
        kept
    };
    let pad = MAX_STORED_BYTES.saturating_sub(bytes(&sized(0))?);
    let (full, over) = (sized(pad), sized(pad.saturating_add(1)));
    assert_eq!(bytes(&full)?, MAX_STORED_BYTES);
    let id = super::create(&ledger, &owner(), "bound", 0, &full)?;
    super::save(&ledger, &owner(), &id, 1, &full)?;
    for refused in [
        super::create(&ledger, &owner(), "past", 2, &over).map(drop),
        super::save(&ledger, &owner(), &id, 3, &over),
    ] {
        assert!(
            matches!(
                refused,
                Err(ExecError::TuiSession(SessionStoreError::TooLarge { .. }))
            ),
            "one byte past the bound is refused: {refused:?}"
        );
    }
    assert_eq!(
        stored(&directory)?,
        [(1, full)],
        "the refusals wrote nothing"
    );
    Ok(())
}

/// The welcome box a sim frame shows.
fn header() -> Header {
    Header {
        version: String::from("0.0.0"),
        model: Ok(String::from("mock-model")),
        directory: String::from("~/work"),
        blocked: None,
    }
}

/// Receipts past the bound are dropped oldest-first.
#[test]
fn sim_receipts_past_the_bound_keep_the_newest() {
    let mut transcript = Transcript::new(header());
    for index in 0..MAX_RECEIPTS_SEEN.saturating_add(10) {
        transcript.note_receipt(&receipt(&format!("{index:012x}")));
    }
    let kept = transcript.snapshot(crate::tui::pane::Pane::new()).receipts;
    assert_eq!(kept.len(), MAX_RECEIPTS_SEEN, "the log is bounded");
    assert_eq!(
        (
            kept.first().map(|first| first.digest.as_str()),
            kept.last().map(|last| last.digest.as_str())
        ),
        (Some("00000000000a"), Some("000000000109")),
        "the oldest go first"
    );
}

/// The settings that name `state` as the state directory.
fn settings(state: &Path) -> Result<Settings, SimError> {
    settings_for(state, "")
}

/// The settings of `tenant` over `state`; an empty name is the store's default.
fn settings_for(state: &Path, tenant: &'static str) -> Result<Settings, SimError> {
    let dir = state.to_path_buf();
    Ok(Settings::from_env(move |name: &str| match name {
        "LARRY_STATE_DIR" => Some(dir.to_string_lossy().into_owned()),
        "LARRY_TENANT" => Some(String::from(tenant)),
        "PATH" => Some(String::from("/usr/bin:/bin")),
        _ => None,
    })?)
}

/// Two tenants on one state directory hold their own rows: each row names its
/// tenant and holds only that tenant's transcript, so a later resume can never
/// list or write another tenant's session (larry#211, larry#578).
#[test]
fn sim_two_tenants_hold_their_own_session_rows() -> Outcome {
    let state = sim_tree::scratch("tui-sessions-tenants")?;
    for tenant in ["acme", "globex"] {
        let (mut app, mut store) = driven(&state, settings_for(&state, tenant)?);
        store.adopt(&mut app);
        app.transcript()
            .push(Entry::Said(format!("only for {tenant}")));
        store.save_app(&mut app);
    }
    let connection = Connection::open(state.join("executor.db"))?;
    let mut statement =
        connection.prepare("SELECT tenant, transcript FROM tui_sessions ORDER BY rowid")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let &[(ref first, ref first_kept), (ref second, ref second_kept)] = rows.as_slice() else {
        return Err(SimError::Missing("one row for each tenant"));
    };
    assert_ne!(first, second, "each row names its own tenant");
    assert!(first_kept.contains("only for acme") && !first_kept.contains("only for globex"));
    assert!(second_kept.contains("only for globex") && !second_kept.contains("only for acme"));
    Ok(())
}

/// A frame beside a store `settings` opens for `state`.
fn driven(state: &Path, settings: Settings) -> (App, SessionStore) {
    let app = App::new(header(), vec![String::from("model: mock")], 120, 34);
    let store = SessionStore::open(&Ok(settings), state, "mock-model");
    (app, store)
}

/// A frame beside a store the settings opened on `state`.
fn driver(state: &Path) -> Result<(App, SessionStore), SimError> {
    Ok(driven(state, settings(state)?))
}

/// The notices the frame shows.
fn notices(app: &mut App) -> Vec<String> {
    app.transcript()
        .entries()
        .iter()
        .filter_map(|entry| match *entry {
            Entry::Notice(_, ref text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Adopting then saving holds the frame in its row, receipts and pane included,
/// stamped with the wall clock's milliseconds.
#[test]
fn sim_adopt_and_save_hold_the_frame() -> Outcome {
    let state = sim_tree::scratch("tui-sessions-driver")?;
    let (mut app, mut store) = driver(&state)?;
    store.adopt(&mut app);
    for character in "/status".chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    app.handle(Input::Key(Key::Enter));
    app.handle(Input::Worker(Event::Started(String::from("printf hi"))));
    app.handle(Input::Worker(Event::Receipt(receipt("abc123def456"))));
    store.save_app(&mut app);
    let rows = stored(&state)?;
    let (updated, kept) = rows
        .first()
        .cloned()
        .ok_or(SimError::Missing("one adopted row"))?;
    assert_eq!(rows.len(), 1, "one row adopted");
    assert_eq!(kept, app.snapshot(), "the row holds the frame");
    assert_eq!(kept.receipts, [receipt("abc123def456")]);
    assert!(
        updated > 1_600_000_000_000,
        "the stamp is wall clock milliseconds: {updated}"
    );
    Ok(())
}

/// A transcript past the byte bound is still saved: the row leaves out the
/// oldest entries, keeps the newest, and says nothing. A frame holds up to
/// 8 MiB and a row 4 MiB, so a long session reaches this.
#[test]
fn sim_a_long_session_keeps_being_saved() -> Outcome {
    let state = sim_tree::scratch("tui-sessions-long")?;
    let (mut app, mut store) = driver(&state)?;
    store.adopt(&mut app);
    for index in 0..8_u8 {
        let said = format!("{index}{}", "x".repeat(700_000));
        app.transcript().push(Entry::Said(said));
    }
    store.save_app(&mut app);
    let seen = notices(&mut app);
    assert!(seen.is_empty(), "no refusal to report: {seen:?}");
    let frame = app.snapshot();
    let rows = stored(&state)?;
    let (_, kept) = rows
        .first()
        .cloned()
        .ok_or(SimError::Missing("one adopted row"))?;
    assert!(bytes(&kept)? <= MAX_STORED_BYTES, "within the bound");
    assert_eq!(kept.entries.len(), 5, "as many of the newest as fit");
    assert!(
        frame.entries.ends_with(&kept.entries),
        "the oldest are the ones left out"
    );
    Ok(())
}

/// With no row adopted a save does nothing and reports nothing.
#[test]
fn sim_save_without_a_session_is_silent() -> Outcome {
    let state = sim_tree::scratch("tui-sessions-unadopted")?;
    let (mut app, mut store) = driver(&state)?;
    store.save_app(&mut app);
    assert!(notices(&mut app).is_empty(), "nothing to report");
    assert!(stored(&state)?.is_empty(), "nothing written");
    Ok(())
}

/// A save the database refuses leaves one notice, however many turns follow.
#[test]
fn sim_a_refused_save_leaves_one_notice() -> Outcome {
    let state = sim_tree::scratch("tui-sessions-gone")?;
    let (mut app, mut store) = driver(&state)?;
    store.adopt(&mut app);
    Connection::open(state.join("executor.db"))?.execute("DELETE FROM tui_sessions", [])?;
    for _ in 0..3 {
        store.save_app(&mut app);
    }
    let seen = notices(&mut app);
    assert_eq!(seen.len(), 1, "one notice, not one a turn: {seen:?}");
    assert!(
        seen.iter()
            .all(|text| text.starts_with("sessions are not saved: tui session: unknown")),
        "{seen:?}"
    );
    Ok(())
}

/// A database that will not open is named once; refused settings add nothing,
/// because the welcome box already says why.
#[test]
fn sim_a_database_that_will_not_open_is_named() -> Outcome {
    let state = sim_tree::scratch("tui-sessions-damaged")?;
    // The settings read the database too, so it is damaged after they have.
    let read = settings(&state)?;
    std::fs::remove_file(state.join("executor.db"))?;
    std::fs::create_dir_all(state.join("executor.db"))?;
    let (mut app, mut store) = driven(&state, read);
    store.adopt(&mut app);
    store.save_app(&mut app);
    let seen = notices(&mut app);
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(
        seen.iter()
            .all(|text| text.starts_with("sessions are not saved: ")),
        "{seen:?}"
    );
    let refused = Err(ExecError::StateDirUnset);
    let mut silent = SessionStore::open(&refused, &state, "mock-model");
    let mut app = App::new(header(), Vec::new(), 120, 34);
    silent.adopt(&mut app);
    assert!(notices(&mut app).is_empty(), "the welcome box names it");
    Ok(())
}

/// The snapshot scrubs the in-flight line, so a stored row holds no half line.
#[test]
fn sim_snapshot_scrubs_the_in_flight_line() {
    let mut pane = crate::tui::pane::Pane::new();
    pane.started("printf hi");
    assert_ne!(
        pane.snapshot(),
        pane,
        "the snapshot drops the in-flight header flag the live pane holds"
    );
}
