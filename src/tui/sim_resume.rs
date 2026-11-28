//! Seeded simulations of the session commands (larry#256, larry#603, ADR 0133):
//! `/resume`, `/new`, `/fork` and `/rename` over the driver and the state
//! database, no terminal. Rows are read back through `SQLite` itself. Source:
//! `SQLite` atomic commit; the lost update and its version check (RFC 9110
//! section 13.1.1).

use std::path::Path;

use lgwks_deps::storefront::rusqlite::{Connection, params};

use super::{Outcome, driven, driver, header, receipt, settings_for, snapshot};
use crate::executor::{ExecError, SessionStoreError, sim_tree};
use crate::sim_error::SimError;
use crate::tui::app::{App, Effect, Input, Key, SessionCommand};
use crate::tui::cells::Entry;
use crate::tui::session_store::{LISTED, SessionStore};
use crate::tui::sessions::{MAX_SESSIONS, Snapshot, find};
use crate::tui::worker::{Ended, Event};

/// What one terminal is: its frame and its store.
type Terminal = (App, SessionStore);

/// A terminal on `state` that has adopted its own row.
fn terminal(state: &Path) -> Result<Terminal, SimError> {
    let (mut app, mut store) = driver(state)?;
    store.adopt(&mut app);
    Ok((app, store))
}

/// Adds `text` to the transcript and holds the frame in its row.
fn say(held: &mut Terminal, text: &str) {
    held.0.transcript().push(Entry::Said(text.to_owned()));
    held.1.save_app(&mut held.0);
}

/// Runs `command` and returns the lines the frame answered with.
fn run(held: &mut Terminal, command: SessionCommand) -> Vec<String> {
    held.1.command(&mut held.0, command);
    match held.0.snapshot().entries.pop() {
        Some(Entry::Info(lines)) => lines,
        Some(Entry::Notice(_, text)) => vec![text],
        other => vec![format!("{other:?}")],
    }
}

/// `/resume <target>`.
fn resume(held: &mut Terminal, target: &str) -> Vec<String> {
    run(held, SessionCommand::Resume(target.to_owned()))
}

/// Every row under `state` as (id, name, transcript), oldest first.
fn rows(state: &Path) -> Result<Vec<(String, String, String)>, SimError> {
    let store = Connection::open(state.join("executor.db"))?;
    let mut statement =
        store.prepare("SELECT id, name, transcript FROM tui_sessions ORDER BY rowid")?;
    let mapped = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    Ok(mapped.collect::<Result<_, _>>()?)
}

/// The row at `at`.
fn row(state: &Path, at: usize) -> Result<(String, String, String), SimError> {
    rows(state)?
        .get(at)
        .cloned()
        .ok_or(SimError::Missing("a row at that place"))
}

/// A seeded session left by a killed terminal comes back whole: transcript, pane
/// and every receipt digest, and no command left running in the pane. The next
/// save writes the resumed row, not the resuming terminal's own.
#[test]
fn sim_resume_brings_back_the_transcript_and_pane() -> Outcome {
    for seed in 0..16_u64 {
        let state = sim_tree::scratch("tui-resume-whole")?;
        let mut first = terminal(&state)?;
        for entry in snapshot(seed)?.entries {
            first.0.transcript().push(entry);
        }
        for event in [
            Event::Diff(String::from("diff --git a/x b/x\n@@ -0,0 +1 @@\n+x\n")),
            Event::Receipt(receipt("abc123def456")),
            Event::Started(String::from("sleep 9")),
        ] {
            first.0.handle(Input::Worker(event));
        }
        first.1.save_app(&mut first.0);
        let left = first.0.snapshot();
        let (left_id, ..) = row(&state, 0)?;
        drop(first);

        let mut fresh = terminal(&state)?;
        assert_eq!(
            resume(&mut fresh, ""),
            [format!("1. Session 1 ({left_id})")],
            "seed {seed}: the listing leaves out this terminal's own row"
        );
        assert_eq!(resume(&mut fresh, "1"), ["resumed Session 1."]);
        let back = fresh.0.snapshot();
        let mut entries = left.entries.clone();
        entries.push(Entry::Info(vec![String::from("resumed Session 1.")]));
        assert_eq!(back.entries, entries, "seed {seed}: the transcript is back");
        assert_eq!(back.receipts, left.receipts, "seed {seed}");
        assert_eq!(back.pane, left.pane.clone().resumed(), "seed {seed}");
        assert_ne!(back.pane, left.pane, "no command runs in a resumed pane");

        let own = row(&state, 1)?;
        say(&mut fresh, "after the resume");
        assert!(row(&state, 0)?.2.contains("after the resume"));
        assert_eq!(row(&state, 1)?, own, "the other row is left as it was");
    }
    Ok(())
}

/// `/fork` copies this frame into a new row that takes the later turns while the
/// original is unchanged; `/fork <name>` copies that session's stored frame, never
/// the one on screen.
#[test]
fn sim_fork_diverges_and_the_original_is_unchanged() -> Outcome {
    let state = sim_tree::scratch("tui-resume-fork")?;
    let mut held = terminal(&state)?;
    say(&mut held, "shared turn");
    let original = row(&state, 0)?;
    assert_eq!(
        run(&mut held, SessionCommand::Fork(String::new())),
        ["forked Session 1 as Session 1 fork."]
    );
    say(&mut held, "only in the fork");
    assert_eq!(row(&state, 0)?, original, "the original row is unchanged");
    let fork = row(&state, 1)?;
    assert_eq!(fork.1, "Session 1 fork");
    assert!(fork.2.contains("shared turn") && fork.2.contains("only in the fork"));

    assert_eq!(
        run(&mut held, SessionCommand::Fork(String::from("Session 1"))),
        ["forked Session 1 as Session 1 fork."]
    );
    let copy = row(&state, 2)?;
    assert!(copy.2.contains("shared turn"));
    assert!(
        !copy.2.contains("only in the fork"),
        "a named fork copies the named session: {}",
        copy.2
    );
    let shown = held.0.snapshot().entries;
    assert!(
        !shown.contains(&Entry::Said(String::from("only in the fork"))),
        "the frame shows the copy: {shown:?}"
    );
    Ok(())
}

/// `/new` holds an empty frame in a new row, numbered on, and leaves the row
/// behind as it was.
#[test]
fn sim_new_starts_an_empty_session() -> Outcome {
    let state = sim_tree::scratch("tui-resume-new")?;
    let mut held = terminal(&state)?;
    say(&mut held, "the first session");
    let first = row(&state, 0)?;
    assert_eq!(run(&mut held, SessionCommand::New), ["started Session 2."]);
    let started = Entry::Info(vec![String::from("started Session 2.")]);
    assert_eq!(
        held.0.snapshot().entries,
        [Entry::Header(header()), started]
    );
    assert_eq!(row(&state, 0)?, first);
    assert_eq!(row(&state, 1)?.1, "Session 2");
    assert_eq!(run(&mut held, SessionCommand::New), ["started Session 3."]);
    assert_eq!(rows(&state)?.len(), 3);
    Ok(())
}

/// Types `text` into the box and presses Enter.
fn typed(app: &mut App, text: &str) -> Vec<Effect> {
    for character in text.chars() {
        app.handle(Input::Key(Key::Char(character)));
    }
    app.handle(Input::Key(Key::Enter))
}

/// "Stop asking" and the `/ps` list belong to the session left behind: neither
/// carries into the one shown next.
#[test]
fn sim_a_switch_carries_no_consent_or_jobs() -> Outcome {
    let state = sim_tree::scratch("tui-resume-consent")?;
    let mut held = terminal(&state)?;
    held.0
        .handle(Input::Worker(Event::Ask(String::from("printf hi"))));
    held.0.draw();
    held.0.handle(Input::Key(Key::Char('a')));
    held.0
        .handle(Input::Worker(Event::Started(String::from("printf hi"))));
    held.0.handle(Input::Worker(Event::Ended(Ended::Done)));
    assert!(held.0.always(), "the person said always");
    run(&mut held, SessionCommand::New);
    assert!(!held.0.always(), "consent does not carry over");
    typed(&mut held.0, "/ps");
    let shown = format!("{:?}", held.0.snapshot().entries);
    assert!(!shown.contains("printf hi"), "no job carries over: {shown}");
    Ok(())
}

/// `/rename` names this row, which goes on being saved; a refused name changes
/// nothing.
#[test]
fn sim_rename_names_this_row_and_refuses_a_bad_name() -> Outcome {
    let state = sim_tree::scratch("tui-resume-rename")?;
    let mut held = terminal(&state)?;
    assert_eq!(
        run(&mut held, SessionCommand::Rename(String::from("  demo "))),
        ["renamed to demo."]
    );
    for bad in ["", "two\nlines"] {
        let said = run(&mut held, SessionCommand::Rename(bad.to_owned()));
        assert!(
            said.iter()
                .all(|line| line.starts_with("tui session: bad tui session name")),
            "{said:?}"
        );
    }
    say(&mut held, "still saved");
    let kept = row(&state, 0)?;
    assert_eq!(kept.1, "demo");
    assert!(kept.2.contains("still saved"));
    assert_eq!(
        run(&mut held, SessionCommand::Fork(String::new())),
        ["forked demo as demo fork."]
    );
    Ok(())
}

/// A row a second terminal resumed has one writer: the first is told once, writes
/// and renames nothing, and `/fork` keeps what it shows.
#[test]
fn sim_two_terminals_on_one_row_keep_one_writer() -> Outcome {
    let state = sim_tree::scratch("tui-resume-writers")?;
    let mut first = terminal(&state)?;
    say(&mut first, "written by the first");
    let (id, ..) = row(&state, 0)?;
    let mut second = terminal(&state)?;
    assert_eq!(resume(&mut second, &id), ["resumed Session 1."]);
    say(&mut second, "written by the second");
    let taken = row(&state, 0)?;

    let moved = format!(
        "tui session: tui session {id} was written by another terminal; \
         /fork keeps what is shown here"
    );
    assert_eq!(
        run(&mut first, SessionCommand::Rename(String::from("mine"))),
        vec![moved.clone()]
    );
    say(&mut first, "a late write");
    say(&mut first, "another");
    assert_eq!(row(&state, 0)?, taken, "the second writer's row stands");
    let told = super::notices(&mut first.0)
        .into_iter()
        .filter(|text| text.starts_with("sessions are not saved:"))
        .collect::<Vec<_>>();
    assert_eq!(told, [format!("sessions are not saved: {moved}")]);

    assert_eq!(
        run(&mut first, SessionCommand::Fork(String::new())),
        ["forked Session 1 as Session 1 fork."]
    );
    say(&mut first, "saved again");
    let kept = row(&state, 2)?;
    assert!(kept.2.contains("a late write") && kept.2.contains("saved again"));
    Ok(())
}

/// Another tenant's row and another folder's row are neither listed nor reached,
/// by number, name or full id, and no row is added.
#[test]
fn sim_another_tenant_or_folder_is_never_reached() -> Outcome {
    let state = sim_tree::scratch("tui-resume-scope")?;
    let elsewhere = state.join("elsewhere");
    std::fs::create_dir(&elsewhere)?;
    let mut theirs = driven(&state, settings_for(&state, "globex")?);
    theirs.1.adopt(&mut theirs.0);
    let acme = Ok(settings_for(&state, "acme")?);
    let mut other = (
        App::new(header(), Vec::new(), 120, 34),
        SessionStore::open(&acme, &elsewhere, "mock-model"),
    );
    other.1.adopt(&mut other.0);
    let mut mine = driven(&state, settings_for(&state, "acme")?);
    mine.1.adopt(&mut mine.0);

    assert_eq!(
        resume(&mut mine, ""),
        ["no other saved session for this folder."]
    );
    let ids = rows(&state)?;
    let unknown = |named: &str| format!("tui session: unknown tui session {named}");
    for target in ids.iter().take(2) {
        assert_eq!(resume(&mut mine, &target.0), [unknown(&target.0)]);
        assert_eq!(
            run(&mut mine, SessionCommand::Fork(target.0.clone())),
            [unknown(&target.0)]
        );
    }
    for named in ["Session 1", "1"] {
        assert_eq!(resume(&mut mine, named), [unknown(named)]);
    }
    assert_eq!(rows(&state)?.len(), 3, "no row was added");
    Ok(())
}

/// A target names one row by its name or the start of its id; a name outranks an
/// id, and two matches are refused, never guessed.
#[test]
fn sim_a_target_names_exactly_one_row() {
    let pair = |id: &str, name: &str| (id.to_owned(), name.to_owned());
    let held = [
        pair("aa1", "demo"),
        pair("aa2", "twin"),
        pair("bb3", "twin"),
    ];
    for (target, id) in [("demo", "aa1"), ("aa2", "aa2"), ("bb", "bb3")] {
        assert_eq!(find(&held, target).ok().as_deref(), Some(id), "{target}");
    }
    for target in ["twin", "aa"] {
        let refused = find(&held, target);
        assert!(
            matches!(refused, Err(SessionStoreError::Ambiguous { .. })),
            "{target}: {refused:?}"
        );
    }
    let missing = find(&held, "none");
    assert!(
        matches!(missing, Err(SessionStoreError::NoSuchSession { .. })),
        "{missing:?}"
    );
    let shadow = [pair("aa1", "bb"), pair("bb3", "other")];
    assert_eq!(find(&shadow, "bb").ok().as_deref(), Some("aa1"));
}

/// A number means the row the person read it beside, even after another terminal's
/// save reorders the folder.
#[test]
fn sim_a_listed_number_stays_with_its_row() -> Outcome {
    let state = sim_tree::scratch("tui-resume-numbers")?;
    let mut older = terminal(&state)?;
    run(&mut older, SessionCommand::Rename(String::from("older")));
    let mut newer = terminal(&state)?;
    run(&mut newer, SessionCommand::Rename(String::from("newer")));
    say(&mut newer, "newest");
    let mut reader = terminal(&state)?;
    let listed = resume(&mut reader, "");
    let starts: Vec<&str> = listed
        .iter()
        .filter_map(|line| line.split(" (").next())
        .collect();
    assert_eq!(starts, ["1. newer", "2. older"], "newest first");
    say(&mut older, "now the older row is the newest");
    assert_eq!(resume(&mut reader, "2"), ["resumed older."]);
    Ok(())
}

/// Adds `held` rows of this sim's tenant and folder, each saying `bytes` bytes,
/// stamped in insertion order.
fn fill(state: &Path, held: u32, bytes: usize) -> Result<(), SimError> {
    let store = Connection::open(state.join("executor.db"))?;
    let (tenant, folder): (String, String) = store.query_row(
        "SELECT tenant, folder FROM tui_sessions LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let mut kept = Snapshot::empty();
    kept.entries = vec![Entry::Said("x".repeat(bytes))];
    let transcript = kept.to_json().map_err(ExecError::TuiSession)?;
    store.execute(
        "WITH RECURSIVE held(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM held WHERE n < ?1) \
         INSERT INTO tui_sessions (tenant, id, name, folder, model, updated, transcript) \
         SELECT ?2, printf('row-%05d', n), printf('held %05d', n), ?3, 'mock-model', n, ?4 \
         FROM held",
        params![held, tenant, folder, transcript],
    )?;
    Ok(())
}

/// A listing shows the newest [`LISTED`] rows and counts the rest, which a name
/// still reaches.
#[test]
fn sim_a_long_listing_is_cut_and_counted() -> Outcome {
    for (extra, counted) in [(0_usize, None), (3, Some("3 older"))] {
        let state = sim_tree::scratch("tui-resume-long")?;
        let mut reader = terminal(&state)?;
        fill(&state, u32::try_from(LISTED.saturating_add(extra))?, 1)?;
        let listed = resume(&mut reader, "");
        assert_eq!(
            listed.len(),
            LISTED.saturating_add(usize::from(counted.is_some())),
            "{listed:?}"
        );
        assert_eq!(
            listed.get(LISTED).cloned(),
            counted.map(|more| format!("{more}; /resume <name or id> reaches them.")),
        );
        assert_eq!(resume(&mut reader, "held 00001"), ["resumed held 00001."]);
    }
    Ok(())
}

/// A full tenant, a damaged row and a store that is not there are each named, and
/// the frame and its row stay as they were.
#[test]
fn sim_a_refused_command_leaves_the_frame_and_its_row() -> Outcome {
    let state = sim_tree::scratch("tui-resume-refused")?;
    let mut held = terminal(&state)?;
    say(&mut held, "kept");
    fill(&state, MAX_SESSIONS.saturating_sub(1), 1)?;
    let full = format!("tui session: the tui session store is full at {MAX_SESSIONS} sessions");
    for command in [SessionCommand::New, SessionCommand::Fork(String::new())] {
        assert_eq!(run(&mut held, command), vec![full.clone()]);
    }
    Connection::open(state.join("executor.db"))?.execute(
        "UPDATE tui_sessions SET transcript = 'not json' WHERE id = 'row-00001'",
        [],
    )?;
    assert_eq!(
        resume(&mut held, "row-00001"),
        ["tui session: tui session row-00001 is damaged"]
    );
    say(&mut held, "still mine");
    let kept = row(&state, 0)?;
    assert!(kept.2.contains("kept") && kept.2.contains("still mine"));

    // The tenant is full, so this terminal adopts no row of its own.
    let mut late = terminal(&state)?;
    assert_eq!(
        run(&mut late, SessionCommand::Rename(String::from("x"))),
        ["this session is not saved; /new starts a saved one."]
    );
    let unset = Err(ExecError::StateDirUnset);
    let mut off = (
        App::new(header(), Vec::new(), 120, 34),
        SessionStore::open(&unset, &state, "mock-model"),
    );
    assert_eq!(
        resume(&mut off, ""),
        ["sessions are off until the settings are fixed; see /status."]
    );
    let read = super::settings(&state)?;
    std::fs::remove_file(state.join("executor.db"))?;
    std::fs::create_dir_all(state.join("executor.db"))?;
    let mut shut = driven(&state, read);
    let said = resume(&mut shut, "");
    assert!(
        said.iter()
            .all(|line| line.starts_with("sessions are not saved: ")),
        "{said:?}"
    );
    Ok(())
}

/// Each session command reaches the driver typed whole or picked from the popup;
/// while a task runs it waits in the box and is never queued to the model.
#[test]
fn sim_session_commands_reach_the_driver_only_when_idle() {
    let frame = || App::new(header(), Vec::new(), 120, 34);
    let name = String::from;
    for (text, command) in [
        ("/new", SessionCommand::New),
        ("/resume", SessionCommand::Resume(name(""))),
        ("/resume  a b ", SessionCommand::Resume(name("a b"))),
        ("/fork 2", SessionCommand::Fork(name("2"))),
        ("/rename demo", SessionCommand::Rename(name("demo"))),
        ("/ren", SessionCommand::Rename(name(""))),
        ("/resu", SessionCommand::Resume(name(""))),
    ] {
        assert_eq!(
            typed(&mut frame(), text),
            [Effect::Session(command)],
            "{text}"
        );
    }
    assert_eq!(typed(&mut frame(), "/resumed"), []);

    let mut busy = frame();
    assert_eq!(typed(&mut busy, "a task"), [Effect::Start(name("a task"))]);
    assert_eq!(typed(&mut busy, "/resume 1"), []);
    busy.handle(Input::Worker(Event::Ended(Ended::Done)));
    assert!(
        !busy
            .snapshot()
            .entries
            .contains(&Entry::User(name("/resume 1"))),
        "nothing was queued to the model"
    );
    assert_eq!(
        busy.handle(Input::Key(Key::Enter)),
        [Effect::Session(SessionCommand::Resume(name("1")))],
        "the command waited in the box"
    );
}
