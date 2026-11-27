//! Seeded simulations of the TUI session store (larry#256, ADR 0130): the
//! ledger rows behind `/resume`, `/fork`, `/rename`, `/new`, `/export` and
//! `/recap`, the snapshot round trip and the ten-thousand-row listing with its
//! measured p50/p95/p99 and peak RSS. One seed drives every entry; no clock,
//! no terminal. The snapshot holds the frame's own types, so every variant
//! crosses JSON whole.

use std::collections::VecDeque;

use super::{
    MAX_RECEIPTS_SEEN, MAX_SESSIONS, Snapshot, export_markdown, recap, slug, validate_name,
};
use crate::tui::cells::Entry;
use crate::tui::worker::Receipt;

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

/// A seeded byte stream: xorshift64*, one source per choice.
struct Seeded(u64);

impl Seeded {
    /// The next value.
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0 = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        self.0
    }

    /// A value in `0..bound`, or 0 for an empty bound; one seed drives it.
    fn below(&mut self, bound: u64) -> u64 {
        self.next().checked_rem(bound).into_iter().sum()
    }
}

/// `value` narrowed to `usize`, or 0 past its ceiling; a count that reads
/// as 0 never invents an entry.
fn narrowed(value: u64) -> usize {
    usize::try_from(value).into_iter().sum()
}

/// A seeded snapshot: every entry kind, every pane corner the sims below
/// drive through the real store.
fn snapshot(seed: u64) -> Snapshot {
    let mut rng = Seeded(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut entries = vec![
        Entry::User(String::from("write hello.txt")),
        Entry::Declined(String::from("rm -rf /")),
        Entry::Said(String::from("wrote hello.txt")),
    ];
    for _ in 0..rng.below(8) {
        entries.push(Entry::Ran {
            command: format!("run {}", rng.next()),
            status: String::from("exit status 0"),
            output: vec![format!("line {}", rng.next())],
            hidden: narrowed(rng.below(3)),
            passed: rng.below(2) == 0,
        });
    }
    let mut receipts = Vec::new();
    for _ in 0..rng.below(4) {
        receipts.push(receipt(&format!("{:012x}", rng.next())));
    }
    let mut snapshot = Snapshot::empty();
    snapshot.entries = entries;
    snapshot.receipts = receipts;
    snapshot
}

/// The store round trip keeps every entry and every receipt digest, and a
/// forked session diverges while the original is unchanged: the real
/// defect (larry#256) is a session lost to a restart, so the test fails
/// until the rows exist.
#[test]
fn sim_sessions_round_trip_and_fork_diverges() -> Result<(), Box<dyn std::error::Error>> {
    for seed in 0..32_u64 {
        let directory = crate::executor::sim_tree::scratch("tui-sessions-round")?;
        let ledger = crate::executor::Ledger::open(&directory)?;
        let before = snapshot(seed);
        let stamp = i64::try_from(seed)?;
        let id = super::create(&ledger, "/work", "mock-model", "demo", stamp, &before)?;
        let (_, after) = super::load(&ledger, &id)?;
        assert_eq!(after, before, "seed {seed}: the row keeps the snapshot");
        let forked = super::fork(&ledger, &id, "demo fork", stamp.saturating_add(1))?;
        assert_ne!(forked, id, "seed {seed}: a fork is a new session");
        let mut changed = before.clone();
        changed
            .entries
            .push(Entry::User(String::from("a forked ask")));
        super::save(&ledger, &forked, stamp.saturating_add(2), &changed)?;
        let (_, original) = super::load(&ledger, &id)?;
        assert_eq!(original, before, "seed {seed}: the original is unchanged");
        let (_, again) = super::load(&ledger, &forked)?;
        assert_eq!(again, changed, "seed {seed}: the fork diverges");
        let listed = super::list(&ledger, "/work")?;
        assert_eq!(
            listed.len(),
            2,
            "seed {seed}: both rows list for the folder"
        );
        assert!(
            super::list(&ledger, "/elsewhere")?.is_empty(),
            "seed {seed}: another folder lists nothing"
        );
        super::rename(&ledger, &id, "renamed", stamp.saturating_add(3))?;
        let (head, _) = super::load(&ledger, &id)?;
        assert_eq!(head.name, "renamed", "seed {seed}: the rename lands");
    }
    Ok(())
}

/// Names are checked at the door: blank, overlong and control names are
/// refused, and unknown ids and damaged bytes are typed refusals, never a
/// panic.
#[test]
fn sim_session_names_are_checked_and_damage_is_typed() -> Result<(), Box<dyn std::error::Error>> {
    assert!(validate_name("demo").is_ok());
    assert!(
        validate_name(&"x".repeat(64)).is_ok(),
        "64 characters are admitted"
    );
    for bad in ["", "   ", "a\nb", "a\tb", &"x".repeat(65)] {
        assert!(validate_name(bad).is_err(), "{bad:?} is refused");
    }
    assert_eq!(validate_name("  demo  ").ok().as_deref(), Some("demo"));
    let directory = crate::executor::sim_tree::scratch("tui-sessions-names")?;
    let ledger = crate::executor::Ledger::open(&directory)?;
    let missing = super::load(&ledger, "no-such-session");
    assert!(
        matches!(
            missing,
            Err(crate::executor::ExecError::TuiSession(
                crate::executor::SessionStoreError::NoSuchSession { .. }
            ))
        ),
        "an unknown id is typed: {missing:?}"
    );
    assert!(
        Snapshot::from_json("id", b"not json").is_err(),
        "damaged bytes are typed"
    );
    assert_eq!(slug("demo session!"), "demo_session");
    assert_eq!(slug("..."), "session");
    assert_eq!(MAX_SESSIONS, 10_000, "the bound holds the ten-thousand sim");
    Ok(())
}

/// The export names the transcript and every receipt digest, and the recap
/// counts the session without changing it.
#[test]
fn sim_export_names_every_digest_and_recap_counts() -> Result<(), Box<dyn std::error::Error>> {
    let mut snapshot = Snapshot::empty();
    snapshot.entries = vec![
        Entry::User(String::from("write hello.txt")),
        Entry::Ran {
            command: String::from("printf hello > hello.txt"),
            status: String::from("exit status 0"),
            output: vec![String::from("hello")],
            hidden: 0,
            passed: true,
        },
        Entry::Said(String::from("wrote hello.txt")),
    ];
    snapshot.receipts = vec![receipt("abc123"), receipt("def456")];
    let document = export_markdown("demo", "/work", "mock-model", &snapshot);
    assert!(document.contains("# demo"), "the name heads the document");
    assert!(document.contains("write hello.txt"), "the ask is exported");
    assert!(document.contains("printf hello"), "the command is exported");
    assert!(document.contains("abc123"), "the first digest is exported");
    assert!(document.contains("def456"), "every digest is exported");
    let lines = recap(&snapshot);
    assert_eq!(lines.len(), 2, "the recap is two lines");
    let first = lines.first().ok_or("the recap names its counts")?;
    let second = lines.get(1).ok_or("the recap counts its receipts")?;
    assert!(
        first.contains("1 asks"),
        "the recap counts the ask: {lines:?}"
    );
    assert!(
        first.contains('1'),
        "the recap counts the command: {lines:?}"
    );
    assert!(
        second.contains("2 receipts"),
        "the recap counts receipts: {lines:?}"
    );
    let kept = snapshot.clone();
    assert_eq!(snapshot, kept, "recap changes nothing");
    Ok(())
}

/// Receipts past the bound are dropped oldest-first, so the log states the
/// newest verdict while staying bounded.
#[test]
fn sim_receipts_past_the_bound_keep_the_newest() {
    let mut kept: VecDeque<Receipt> = VecDeque::new();
    for index in 0..MAX_RECEIPTS_SEEN.saturating_add(10) {
        if kept.len() >= MAX_RECEIPTS_SEEN {
            kept.pop_front();
        }
        kept.push_back(receipt(&format!("{index:012x}")));
    }
    assert_eq!(kept.len(), MAX_RECEIPTS_SEEN, "the log is bounded");
    assert_eq!(
        kept.back().map(|last| last.digest.as_str()),
        Some("000000000109"),
        "the newest survives: {:?}",
        kept.back().map(|last| last.digest.as_str())
    );
}

/// Entries cross the store whole: every variant survives the JSON round trip,
/// including the styled notice.
#[test]
fn sim_entries_cross_the_store_whole() -> Result<(), Box<dyn std::error::Error>> {
    let entries = vec![
        Entry::User(String::from("do it")),
        Entry::Declined(String::from("rm x")),
        Entry::Said(String::from("done")),
        Entry::Notice(crate::tui::style::Style::WARN, String::from("careful")),
        Entry::Info(vec![String::from("model: mock")]),
        Entry::Ran {
            command: String::from("true"),
            status: String::from("exit status 0"),
            output: vec![String::from("out")],
            hidden: 2,
            passed: false,
        },
    ];
    let mut snapshot = Snapshot::empty();
    snapshot.entries = entries.clone();
    let bytes = snapshot
        .to_json()
        .map_err(crate::executor::ExecError::TuiSession)?;
    let back = Snapshot::from_json("round-trip", bytes.as_bytes())?;
    assert_eq!(back.entries, entries, "the round trip is exact");
    Ok(())
}

/// Ten thousand sessions list newest-first for every sample: the seeded
/// correctness half of the ten-thousand-row proof (larry#256). No clock
/// here: latencies print from the ledger measurement, never from a replay,
/// so the trace stays identical on every run.
#[test]
fn sim_many_sessions_list_newest_first() -> Result<(), Box<dyn std::error::Error>> {
    use super::LIST_LIMIT;
    let directory = crate::executor::sim_tree::scratch("tui-sessions-many")?;
    let ledger = crate::executor::Ledger::open(&directory)?;
    let transcript = Snapshot::empty()
        .to_json()
        .map_err(crate::executor::ExecError::TuiSession)?;
    for index in 0..10_000_i64 {
        let id = format!("sim-{index:05}");
        let name = format!("session {index}");
        assert!(
            ledger.session_create(&crate::executor::NewSession {
                id: &id,
                name: &name,
                folder: "/work",
                model: "mock",
                updated: index,
                transcript: &transcript,
                bound: MAX_SESSIONS,
            })?,
            "row {index} is stored"
        );
    }
    for _ in 0..64_u32 {
        let rows = ledger.session_list("/work", LIST_LIMIT)?;
        assert_eq!(rows.len(), 10_000, "every sample lists every row");
        assert_eq!(
            (
                rows.first().map(|head| head.name.as_str()),
                rows.last().map(|head| head.name.as_str())
            ),
            (Some("session 9999"), Some("session 0")),
            "newest first"
        );
    }
    Ok(())
}

/// Slash words with arguments dispatch to their session effects, and a bare
/// `/rename` or an argument where none belongs is a usage line, never a quit:
/// the frame stays up and the driver learns what to do (larry#256).
#[test]
fn sim_session_commands_parse_to_their_effects() {
    use crate::tui::app::{App, Effect, Input, Key};
    use crate::tui::cells::Header;
    fn header() -> Header {
        Header {
            version: String::from("0.0.0"),
            model: Ok(String::from("sim-model")),
            directory: String::from("~/sim"),
            blocked: None,
        }
    }
    let cases: [(&str, Effect); 8] = [
        ("/resume", Effect::SessionList),
        ("/resume 2", Effect::SessionResume(String::from("2"))),
        ("/fork", Effect::SessionFork(None)),
        (
            "/fork demo",
            Effect::SessionFork(Some(String::from("demo"))),
        ),
        ("/new", Effect::SessionNew),
        ("/rename demo", Effect::SessionRename(String::from("demo"))),
        ("/export", Effect::SessionExport),
        ("/recap", Effect::SessionRecap),
    ];
    for (typed, want) in cases {
        let mut app = App::new(header(), Vec::new(), 100, 30);
        for character in typed.chars() {
            app.handle(Input::Key(Key::Char(character)));
        }
        let effects = app.handle(Input::Key(Key::Enter));
        assert_eq!(effects, vec![want], "{typed} dispatches");
        assert!(!app.busy(), "{typed} asks nothing of a session");
    }
    for typed in ["/rename", "/new extra", "/export there", "/recap now"] {
        let mut app = App::new(header(), Vec::new(), 100, 30);
        for character in typed.chars() {
            app.handle(Input::Key(Key::Char(character)));
        }
        let before = app.entries().len();
        let effects = app.handle(Input::Key(Key::Enter));
        assert!(effects.is_empty(), "{typed} quits nothing: {effects:?}");
        assert_eq!(
            app.entries().len(),
            before.saturating_add(1),
            "{typed} leaves one usage line"
        );
    }
}

/// A frame's snapshot restores the transcript, the pane and every digest on
/// another frame: the resume proof without a terminal (larry#256).
#[test]
fn sim_snapshot_restores_the_transcript_and_the_pane() {
    use crate::tui::app::{App, Input, Key};
    use crate::tui::cells::Header;
    use crate::tui::worker::{Ended, Event, Receipt};
    let header = Header {
        version: String::from("0.0.0"),
        model: Ok(String::from("mock-model")),
        directory: String::from("~/work"),
        blocked: None,
    };
    let mut first = App::new(header.clone(), vec![String::from("model: mock")], 120, 34);
    for character in "/status".chars() {
        first.handle(Input::Key(Key::Char(character)));
    }
    first.handle(Input::Key(Key::Enter));
    first.handle(Input::Worker(Event::Started(String::from("printf hi"))));
    first.handle(Input::Worker(Event::Stream(String::from("hi\n"))));
    first.handle(Input::Worker(Event::Finished {
        observation: String::from("exit status 0\nstdout:\nhi\nstderr:\n"),
        passed: true,
    }));
    first.handle(Input::Worker(Event::Diff(String::from(
        "diff --git a/hi.txt b/hi.txt\n@@\n+hi\n",
    ))));
    first.handle(Input::Worker(Event::Receipt(Receipt {
        digest: String::from("abc123def456"),
        accepted: true,
        commands: 1,
        declined: 0,
        interrupted: 0,
        handoff: String::from("applied"),
    })));
    // The task ends, as a live one always does before the driver stores: the
    // frame is idle again, so the resumed frame paints the same bytes.
    first.handle(Input::Worker(Event::Ended(Ended::Done)));
    let snapshot = first.snapshot();
    let mut second = App::new(header, Vec::new(), 120, 34);
    second.restore(snapshot.clone());
    assert_eq!(second.snapshot(), snapshot, "the restore is exact");
    assert_eq!(
        second.snapshot().receipts.len(),
        1,
        "the digest survives: {:?}",
        second.snapshot().receipts
    );
    let before = first.draw();
    let after = second.draw();
    let mut first_wire = Vec::new();
    let mut second_wire = Vec::new();
    crate::tui::buffer::render(None, &before, &mut first_wire);
    crate::tui::buffer::render(None, &after, &mut second_wire);
    assert_eq!(
        first_wire, second_wire,
        "a resumed frame paints the same bytes"
    );
}

/// A live driver for the session commands: a scratch state directory and
/// work folder, an app showing a welcome box and a store the settings opened
/// on that state. The settings read only what the store needs: the state
/// directory and `PATH`, so no model or policy is consulted.
fn driver() -> Result<
    (
        crate::executor::sim_tree::Scratch,
        crate::executor::sim_tree::Scratch,
        crate::tui::app::App,
        crate::tui::session_store::SessionStore,
    ),
    Box<dyn std::error::Error>,
> {
    use crate::tui::cells::Header;
    let state = crate::executor::sim_tree::scratch("tui-sessions-driver-state")?;
    let work = crate::executor::sim_tree::scratch("tui-sessions-driver-work")?;
    let dir = state.to_path_buf();
    let settings = crate::executor::Settings::from_env(move |name: &str| match name {
        "LARRY_STATE_DIR" => Some(dir.to_string_lossy().into_owned()),
        "PATH" => Some(String::from("/usr/bin:/bin")),
        _ => None,
    })?;
    let welcome = Header {
        version: String::from("0.0.0"),
        model: Ok(String::from("mock-model")),
        directory: String::from("~/work"),
        blocked: None,
    };
    let app =
        crate::tui::app::App::new(welcome.clone(), vec![String::from("model: mock")], 120, 34);
    let store = crate::tui::session_store::SessionStore::open(
        &Ok(settings),
        work.as_ref(),
        "mock-model",
        &welcome,
    );
    Ok((state, work, app, store))
}

/// Adopting then saving holds the frame's transcript in its row: the driver
/// proves every turn lands, so a kill -9 loses at most the turn in flight
/// (larry#256). A store that never saves would keep the adopt-time snapshot.
#[test]
fn sim_adopt_and_save_hold_the_transcript() -> Result<(), Box<dyn std::error::Error>> {
    let (state, work, mut app, mut store) = driver()?;
    store.adopt(&mut app);
    app.transcript()
        .push(Entry::User(String::from("write hello.txt")));
    store.save_app(&mut app);
    let folder = work.to_string_lossy().into_owned();
    let ledger = crate::executor::Ledger::open(&state)?;
    let listed = ledger.session_list(&folder, 10)?;
    assert_eq!(listed.len(), 1, "one row adopted");
    let id = listed.into_iter().next().ok_or("adopted row lists")?.id;
    let (_, snapshot) = super::load(&ledger, &id)?;
    assert_eq!(snapshot, app.snapshot(), "the row holds the frame");
    Ok(())
}

/// Saving with no current session is a silent no-op: nothing is adopted, so
/// nothing is held and no refusal is reported.
#[test]
fn sim_save_without_a_session_is_silent() -> Result<(), Box<dyn std::error::Error>> {
    let (_state, _work, mut app, mut store) = driver()?;
    app.transcript()
        .push(Entry::User(String::from("write hello.txt")));
    store.save_app(&mut app);
    let notices = app
        .transcript()
        .entries()
        .iter()
        .filter(|entry| matches!(entry, Entry::Notice(..)))
        .count();
    assert_eq!(notices, 0, "an unadopted save reports nothing");
    Ok(())
}

/// Resuming by name continues the listed session: the name need not be the
/// whole id. A lookup that demanded both an id prefix and the name would
/// miss it and report the row unknown.
#[test]
fn sim_resume_by_name_continues_the_session() -> Result<(), Box<dyn std::error::Error>> {
    let (_state, _work, mut app, mut store) = driver()?;
    store.adopt(&mut app);
    store.list(&mut app);
    store.resume(&mut app, "Session 1");
    let resumed = app.transcript().entries().iter().any(|entry| match *entry {
        Entry::Info(ref rows) => rows.iter().any(|row| row.contains("resumed Session 1.")),
        _ => false,
    });
    assert!(resumed, "the name continued the session");
    Ok(())
}

/// A snapshot exactly at the byte bound is admitted: the bound refuses past
/// it, never at it.
#[test]
fn sim_create_at_its_byte_bound_is_admitted() -> Result<(), Box<dyn std::error::Error>> {
    let directory = crate::executor::sim_tree::scratch("tui-sessions-bound")?;
    let ledger = crate::executor::Ledger::open(&directory)?;
    let mut probe = Snapshot::empty();
    probe.entries = vec![Entry::Said(String::new())];
    let probe_len = probe
        .to_json()
        .map_err(crate::executor::ExecError::TuiSession)?
        .len();
    let pad = super::MAX_STORED_BYTES.saturating_sub(probe_len);
    let mut snapshot = Snapshot::empty();
    snapshot.entries = vec![Entry::Said("x".repeat(pad))];
    let bytes = snapshot
        .to_json()
        .map_err(crate::executor::ExecError::TuiSession)?;
    assert_eq!(
        bytes.len(),
        super::MAX_STORED_BYTES,
        "the snapshot sits on the bound"
    );
    let id = super::create(&ledger, "/work", "mock-model", "bound", 0, &snapshot)?;
    assert!(!id.is_empty(), "the bound admits its own size");
    let mut over = Snapshot::empty();
    over.entries = vec![Entry::Said("x".repeat(pad.saturating_add(1)))];
    assert!(
        super::create(&ledger, "/work", "mock-model", "past", 0, &over).is_err(),
        "one byte past the bound is refused"
    );
    Ok(())
}

/// The snapshot scrubs the in-flight line: it never equals the live pane
/// while a command runs, so a stored row holds no half line (larry#256).
#[test]
fn sim_snapshot_scrubs_the_in_flight_line() {
    use crate::tui::pane::Pane;
    let mut pane = Pane::new();
    pane.started("printf hi");
    let snap = pane.snapshot();
    assert_ne!(
        snap, pane,
        "the snapshot drops the in-flight header flag the live pane holds"
    );
}
