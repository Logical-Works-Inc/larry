//! Seeded simulations of `/export` and `/recap` (larry#609, ADR 0133): the paths refused, the
//! file's bound, hostile text inside its fences, a write that fails and the recap's lines. The
//! file is read back from the disk. Source: `CommonMark` 0.31 section 4.5 (fenced code),
//! CWE-22 (path traversal), CWE-59 (link following), POSIX `ENOSPC`.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::symlink;
use std::path::Path;

use lgwks_std::seeded::Seeded;

use super::{
    ExportError, NOTE_BYTES, PUBLISH_TRIES, block, create, file_name, markdown, recap, store,
};
use crate::executor::{ExecError, sim_tree};
use crate::sim_error::SimError;
use crate::tui::app::{App, Effect, Input, Key};
use crate::tui::cells::{Entry, Header};
use crate::tui::session_store::SessionStore;
use crate::tui::sessions::{MAX_STORED_BYTES, Snapshot};
use crate::tui::style::Style;
use crate::tui::worker::Receipt;

/// What a test returns.
type Outcome = Result<(), SimError>;

/// A welcome box.
fn header() -> Header {
    Header {
        version: String::from("0"),
        model: Ok(String::from("mock-model")),
        directory: String::from("~/work"),
        blocked: None,
    }
}

/// A receipt with this digest and this change.
fn receipt(digest: &str, accepted: bool, handoff: &str) -> Receipt {
    Receipt {
        digest: digest.to_owned(),
        accepted,
        commands: 2,
        declined: 1,
        interrupted: 0,
        handoff: handoff.to_owned(),
    }
}

/// A snapshot holding `entries`.
fn holding(entries: Vec<Entry>) -> Snapshot {
    let mut kept = Snapshot::empty();
    kept.entries = entries;
    kept
}

/// A finished command.
fn ran(passed: bool) -> Entry {
    Entry::ran(String::from("true"), "exit status 0", Vec::new(), passed)
}

/// Which refusal `outcome` is.
fn kind<T>(outcome: &Result<T, ExportError>) -> &'static str {
    match *outcome {
        Ok(_) => "written",
        Err(ExportError::Outside(_)) => "outside",
        Err(ExportError::Git(_)) => "git",
        Err(ExportError::Exists(_)) => "exists",
        Err(ExportError::Part(_)) => "part",
        Err(ExportError::Folder { .. }) => "folder",
        Err(ExportError::Write { .. }) => "write",
    }
}

/// Every path that is not a new file inside the folder is its own typed refusal, and nothing
/// outside the folder, behind a link, inside `.git` or already there changes.
#[test]
fn sim_export_is_refused_outside_the_folder() -> Outcome {
    let root = sim_tree::scratch("tui-export-paths")?;
    let folder = root.join("work");
    let elsewhere = root.join("elsewhere");
    for made in [&folder.join("sub"), &folder.join(".git"), &elsewhere] {
        fs::create_dir_all(made)?;
    }
    fs::write(folder.join("kept.md"), "mine")?;
    symlink(&elsewhere, folder.join("out"))?;
    symlink(elsewhere.join("aimed.md"), folder.join("link.md"))?;
    let absolute = elsewhere.join("absolute.md").display().to_string();
    let long = "n".repeat(300);
    let cases = [
        ("..", "outside"),
        (".", "outside"),
        ("../above.md", "outside"),
        ("sub/../../above.md", "outside"),
        (absolute.as_str(), "outside"),
        (".git/hook", "git"),
        (".GIT/hook", "git"),
        ("sub/.Git/hook", "git"),
        ("kept.md", "exists"),
        ("sub", "exists"),
        ("link.md", "exists"),
        ("out/through.md", "folder"),
        ("none/missing.md", "folder"),
        ("kept.md/under.md", "folder"),
        (long.as_str(), "write"),
        (".larry-export-mine.part", "part"),
        ("sub/.larry-export-mine.part", "part"),
        (".larry-export-notes.md", "written"),
        ("notes.part", "written"),
        ("./top.md", "written"),
        ("sub/deep.md", "written"),
    ];
    for (target, expected) in cases {
        let outcome = create(&folder, target, b"x", store);
        assert_eq!(kind(&outcome), expected, "{target}: {outcome:?}");
    }
    assert_eq!(
        fs::read_dir(&elsewhere)?.count(),
        0,
        "nothing left the folder"
    );
    assert_eq!(fs::read_dir(folder.join(".git"))?.count(), 0);
    assert!(!root.join("above.md").exists());
    assert_eq!(fs::read_to_string(folder.join("kept.md"))?, "mine");
    assert_eq!(fs::read_to_string(folder.join("top.md"))?, "x");
    assert_eq!(fs::read_to_string(folder.join("sub/deep.md"))?, "x");
    Ok(())
}

/// A write the device refuses is tried once and leaves no part file, so the same name works once
/// there is room.
#[test]
fn sim_a_failed_write_leaves_no_file() -> Outcome {
    let folder = sim_tree::scratch("tui-export-full")?;
    let mut tried = 0_u8;
    let full = create(
        &folder,
        "notes.md",
        b"half",
        |file: &mut File, bytes: &[u8]| {
            tried = tried.saturating_add(1);
            file.write_all(bytes)?;
            Err(io::Error::from(io::ErrorKind::StorageFull))
        },
    );
    assert_eq!(kind(&full), "write", "{full:?}");
    assert_eq!(tried, 1, "a full disk is not another terminal's sweep");
    assert_eq!(disk(&folder)?, [], "a part file stayed");
    let again = create(&folder, "notes.md", b"whole", store);
    assert_eq!(kind(&again), "written", "{again:?}");
    assert_eq!(fs::read_to_string(folder.join("notes.md"))?, "whole");
    Ok(())
}

/// Every file of `folder` and its bytes: the disk a kill -9 at this instant leaves behind.
fn disk(folder: &Path) -> io::Result<Vec<(OsString, Vec<u8>)>> {
    let mut held = Vec::new();
    for entry in fs::read_dir(folder)? {
        let name = entry?.file_name();
        let bytes = fs::read(folder.join(&name))?;
        held.push((name, bytes));
    }
    held.sort();
    Ok(held)
}

/// A writer killed after any number of bytes, none to all of them, leaves no file under the
/// target's name, and the next `/export` of that name succeeds and sweeps what was left.
/// Source: POSIX `kill(2)`, `link(2)`.
#[test]
fn sim_export_killed_at_any_byte_never_shows_a_part_file() -> Outcome {
    let folder = sim_tree::scratch("tui-export-killed")?;
    let whole = b"# Larry session\n\n## larry\n\n```text\ndone\n```\n\n";
    let target = folder.join("notes.md");
    for cut in 0..=whole.len() {
        let mut left = Vec::new();
        let stopped = create(
            &folder,
            "notes.md",
            whole,
            |file: &mut File, bytes: &[u8]| {
                file.write_all(bytes.get(..cut).ok_or(io::ErrorKind::UnexpectedEof)?)?;
                left = disk(&folder)?;
                Err(io::Error::from(io::ErrorKind::Interrupted))
            },
        );
        assert_eq!(kind(&stopped), "write", "{cut}: {stopped:?}");
        // The process died at `left`: no clean-up ran, so the disk goes back to that instant.
        for (name, bytes) in left {
            fs::write(folder.join(name), bytes)?;
        }
        assert!(!target.exists(), "{cut}: a part file stands under the name");
        let again = create(&folder, "notes.md", whole, store);
        assert_eq!(kind(&again), "written", "{cut}: {again:?}");
        assert_eq!(
            disk(&folder)?,
            [(OsString::from("notes.md"), whole.to_vec())],
            "{cut}: the part file is swept"
        );
        fs::remove_file(&target)?;
    }
    Ok(())
}

/// A part file a dead export left goes with the next export of its folder. A locked one, which
/// a running export holds, a link, a folder, another folder's and a person's file of nearly the
/// name stay, and a file published just before its writer died keeps its bytes.
#[test]
fn sim_export_sweeps_only_dead_part_files() -> Outcome {
    let root = sim_tree::scratch("tui-export-sweep")?;
    let folder = root.join("work");
    let sub = folder.join("sub");
    fs::create_dir_all(&sub)?;
    let outside = root.join("outside");
    fs::write(&outside, "mine")?;
    for made in [
        ".larry-export-dead.part",
        ".larry-export-live.part",
        ".larry-export-notes.md",
        "notes.part",
        "sub/.larry-export-deeper.part",
        "before.md",
    ] {
        fs::write(folder.join(made), "whole")?;
    }
    symlink(&outside, folder.join(".larry-export-link.part"))?;
    fs::create_dir(folder.join(".larry-export-folder.part"))?;
    // Killed between the publication and the removal of the part file.
    fs::hard_link(
        folder.join("before.md"),
        folder.join(".larry-export-linked.part"),
    )?;
    let running = File::open(folder.join(".larry-export-live.part"))?;
    running.try_lock().map_err(io::Error::from)?;

    let next = create(&folder, "next.md", b"x", store);
    assert!(matches!(next, Ok(None)), "{next:?}");
    let mut names: Vec<OsString> = fs::read_dir(&folder)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<io::Result<_>>()?;
    names.sort();
    assert_eq!(
        names,
        [
            ".larry-export-folder.part",
            ".larry-export-link.part",
            ".larry-export-live.part",
            ".larry-export-notes.md",
            "before.md",
            "next.md",
            "notes.part",
            "sub",
        ]
    );
    assert_eq!(fs::read_to_string(folder.join("before.md"))?, "whole");
    assert_eq!(fs::read_to_string(&outside)?, "mine");
    assert!(sub.join(".larry-export-deeper.part").exists());
    let deeper = create(&folder, "sub/next.md", b"x", store);
    assert!(matches!(deeper, Ok(None)), "{deeper:?}");
    assert!(!sub.join(".larry-export-deeper.part").exists());
    Ok(())
}

/// Another terminal's sweep that takes a part file before its writer locked it costs one more
/// try; one that takes every try ends typed and leaves nothing.
#[test]
fn sim_export_tries_again_when_another_sweep_takes_the_part_file() -> Outcome {
    let folder = sim_tree::scratch("tui-export-taken")?;
    for takes in 0..=PUBLISH_TRIES {
        let mut tried = 0_u8;
        let outcome = create(
            &folder,
            "notes.md",
            b"whole",
            |file: &mut File, bytes: &[u8]| {
                if tried < takes {
                    for (name, _) in disk(&folder)? {
                        fs::remove_file(folder.join(name))?;
                    }
                }
                tried = tried.saturating_add(1);
                store(file, bytes)
            },
        );
        if takes < PUBLISH_TRIES {
            assert_eq!(kind(&outcome), "written", "{takes}: {outcome:?}");
            assert_eq!(tried, takes.saturating_add(1), "{takes}");
            assert_eq!(
                disk(&folder)?,
                [(OsString::from("notes.md"), b"whole".to_vec())]
            );
            fs::remove_file(folder.join("notes.md"))?;
        } else {
            assert_eq!(kind(&outcome), "write", "{takes}: {outcome:?}");
            assert_eq!(tried, PUBLISH_TRIES, "the tries are bounded");
            assert_eq!(disk(&folder)?, []);
        }
    }
    Ok(())
}

/// The file holds at most [`MAX_STORED_BYTES`]: the newest entries that fit, in order, the older
/// ones counted, and every receipt digest. The bound admits its size and refuses one past it.
#[test]
fn sim_export_holds_its_bound_and_counts_what_it_left_out() -> Outcome {
    let mut kept = Snapshot::empty();
    kept.receipts = vec![receipt("abc123", true, "applied")];
    let (empty, ..) = markdown("name: x", &kept);
    let room = MAX_STORED_BYTES
        .saturating_sub(empty.len())
        .saturating_sub(NOTE_BYTES)
        .saturating_sub(block("larry", "").len());
    kept.entries = vec![Entry::Said("a".repeat(room))];
    let (text, held, left_out) = markdown("name: x", &kept);
    assert_eq!((held, left_out), (1, 0));
    assert_eq!(text.len(), MAX_STORED_BYTES.saturating_sub(NOTE_BYTES));
    assert!(!text.contains("left out"));
    kept.entries = vec![Entry::Said("a".repeat(room.saturating_add(1)))];
    let (text, held, left_out) = markdown("name: x", &kept);
    assert_eq!((held, left_out), (0, 1));
    assert!(text.contains("1 older entries are left out") && !text.contains("aaaa"));

    kept.entries = vec![Entry::Header(header())];
    for at in 0..100_u32 {
        kept.entries
            .push(Entry::Said(format!("entry {at:03} {}", "x".repeat(65_536))));
    }
    let folder = sim_tree::scratch("tui-export-bound")?;
    let said = super::export(&folder, "", "session", "mock-model", &kept)?;
    let text = fs::read_to_string(folder.join("session.md"))?;
    let (_, held, left_out) = markdown("name: x", &kept);
    assert_eq!(held.saturating_add(left_out), 100);
    assert!(left_out > 0 && text.len() <= MAX_STORED_BYTES);
    assert_eq!(
        said,
        [
            format!("exported to session.md ({held} entries, 1 receipts)."),
            format!("{left_out} older entries did not fit in 4 MiB and are left out."),
        ]
    );
    assert!(!text.contains("entry 000") && text.contains("abc123 accepted"));
    let before = text
        .find("entry 098")
        .ok_or(SimError::Missing("entry 98"))?;
    let after = text
        .find("entry 099")
        .ok_or(SimError::Missing("entry 99"))?;
    assert!(before < after, "the file keeps the order");
    Ok(())
}

/// Every entry kind and every receipt, written as the file the person reads.
#[test]
fn sim_export_writes_every_entry_kind_and_receipt() {
    let kept = {
        let mut kept = holding(vec![
            Entry::Header(header()),
            Entry::User(String::from("write it")),
            Entry::Ran {
                command: String::from("printf a"),
                status: String::from("exit status 0"),
                output: vec![String::from("a"), String::from("b")],
                hidden: 2,
                passed: true,
            },
            ran(false),
            Entry::Declined(String::from("rm x")),
            Entry::Said(String::from("done")),
            Entry::Notice(Style::WARN, String::from("careful")),
            Entry::Info(vec![String::from("one"), String::from("two")]),
        ]);
        kept.receipts = vec![
            receipt("abc123", true, "applied"),
            receipt("def456", false, "discarded"),
        ];
        kept
    };
    let (text, held, left_out) = markdown("name: x", &kept);
    assert_eq!((held, left_out), (7, 0));
    let expected = [
        "# Larry session\n\n",
        "## session\n\n```text\nname: x\n```\n\n",
        "## task\n\n```text\nwrite it\n```\n\n",
        "## ran, passed\n\n```text\n$ printf a\nexit status 0\n2 earlier lines not kept\na\nb\n```\n\n",
        "## ran, failed\n\n```text\n$ true\nexit status 0\n\n```\n\n",
        "## declined, never ran\n\n```text\nrm x\n```\n\n",
        "## larry\n\n```text\ndone\n```\n\n",
        "## notice\n\n```text\ncareful\n```\n\n",
        "## info\n\n```text\none\ntwo\n```\n\n",
        "## receipts\n\n```text\n",
        "abc123 accepted, 2 run, 1 declined, 0 interrupted, change applied\n",
        "def456 not accepted, 2 run, 1 declined, 0 interrupted, change discarded\n```\n\n",
    ]
    .concat();
    assert_eq!(text, expected);
    let (none, ..) = markdown("name: x", &Snapshot::empty());
    assert!(
        none.ends_with("## receipts\n\n```text\nnone\n```\n\n"),
        "{none}"
    );
}

/// How a file's text broke its own shape.
#[derive(Debug, PartialEq, Eq)]
enum Broken {
    /// A line outside every fence that is none of the file's own.
    Loose(String),
    /// A fence nothing closed.
    Open,
}

/// The headings of `text` as `CommonMark` reads its fences.
fn headings(text: &str) -> Result<Vec<&str>, Broken> {
    let mut open = None;
    let mut found = Vec::new();
    for line in text.lines() {
        if let Some(length) = open {
            let bare = line.trim_start_matches(' ');
            let indent = line.len().saturating_sub(bare.len());
            let bare = bare.trim_end();
            if indent <= 3 && bare.len() >= length && bare.bytes().all(|byte| byte == b'`') {
                open = None;
            }
        } else if let Some(heading) = line.strip_prefix("## ") {
            found.push(heading);
        } else if line.starts_with("```") && line.ends_with("text") {
            open = Some(line.len().saturating_sub(4));
        } else if !line.is_empty() && line != "# Larry session" {
            return Err(Broken::Loose(line.to_owned()));
        }
    }
    match open {
        Some(_) => Err(Broken::Open),
        None => Ok(found),
    }
}

/// Seeded text built to close a fence, forge a heading or drive a terminal.
fn hostile(rng: &mut Seeded) -> Result<String, SimError> {
    const PIECES: [&str; 10] = [
        "`",
        "```",
        "\n",
        "# forged",
        "## receipts",
        "\x1b[2J",
        "\u{202e}",
        "text",
        " ",
        "\n   ````````",
    ];
    let mut out = String::new();
    for _ in 0..rng.below(24)? {
        let piece = PIECES.get(rng.index(PIECES.len())?);
        out.push_str(piece.ok_or(SimError::Missing("a piece"))?);
    }
    Ok(out)
}

/// No model, command or folder text closes its fence, forges a heading or puts a control or
/// bidirectional character in the file.
#[test]
fn sim_export_never_lets_text_break_out_of_the_file() -> Outcome {
    for seed in 0..256_u64 {
        let mut rng = Seeded::from_seed(seed);
        let mut kept = holding(vec![
            Entry::User(hostile(&mut rng)?),
            Entry::Ran {
                command: hostile(&mut rng)?,
                status: hostile(&mut rng)?,
                output: vec![hostile(&mut rng)?, hostile(&mut rng)?],
                hidden: rng.index(3)?,
                passed: true,
            },
            Entry::Declined(hostile(&mut rng)?),
            Entry::Said(hostile(&mut rng)?),
            Entry::Notice(Style::BAD, hostile(&mut rng)?),
            Entry::Info(vec![hostile(&mut rng)?]),
        ]);
        kept.receipts = vec![receipt("abc123", true, &hostile(&mut rng)?)];
        let (text, ..) = markdown(&hostile(&mut rng)?, &kept);
        assert_eq!(
            headings(&text),
            Ok(vec![
                "session",
                "task",
                "ran, passed",
                "declined, never ran",
                "larry",
                "notice",
                "info",
                "receipts",
            ]),
            "seed {seed}: {text:?}"
        );
        assert!(
            text.chars()
                .all(|found| found == '\n' || (!found.is_control() && found != '\u{202e}')),
            "seed {seed}: {text:?}"
        );
    }
    Ok(())
}

/// The default file is the session's name made safe for a file system.
#[test]
fn sim_export_names_the_file_after_the_session() {
    for (name, file) in [
        ("Session 1", "session-1.md"),
        ("../Fix: the retry!", "fix-the-retry.md"),
        ("\u{65e5}\u{672c}", "session.md"),
    ] {
        assert_eq!(file_name(name), file);
    }
}

/// `/recap` counts the session, names its newest tasks, the answer to the newest one and the
/// receipts, and never reads an older answer as the newest task's.
#[test]
fn sim_recap_sums_the_session() {
    assert_eq!(
        recap(&Snapshot::empty()),
        ["nothing to recap: no task in this session yet."]
    );
    let mut entries = vec![
        Entry::User(String::from("task 1")),
        ran(true),
        Entry::Said(String::from("an old answer")),
    ];
    entries.extend((2..=7).map(|task| Entry::User(format!("task {task}"))));
    entries.extend([
        ran(false),
        ran(true),
        ran(true),
        Entry::Declined(String::from("rm x")),
        Entry::Notice(Style::BAD, String::from("refused")),
    ]);
    let mut kept = holding(entries);
    kept.receipts = vec![
        receipt("aaa", true, "applied"),
        receipt("bbb", false, "discarded"),
    ];
    assert_eq!(
        recap(&kept),
        [
            "recap: 7 tasks, 4 commands (3 passed, 1 failed), 1 declined.",
            "2 tasks before these.",
            "asked: task 3",
            "asked: task 4",
            "asked: task 5",
            "asked: task 6",
            "asked: task 7",
            "the newest task has no answer.",
            "2 receipts, 1 accepted; newest bbb, change discarded.",
        ]
    );
    let one = holding(vec![
        Entry::User("y".repeat(200)),
        ran(true),
        Entry::Said(String::from("done\nand \x1b[2Jclean")),
    ]);
    let lines = recap(&one);
    assert_eq!(
        lines.first().map(String::as_str),
        Some("recap: 1 task, 1 command (1 passed, 0 failed), 0 declined.")
    );
    assert_eq!(
        lines.get(1).map(|line| line.chars().count()),
        Some("asked: ".len().saturating_add(80)),
        "{lines:?}"
    );
    assert_eq!(
        lines.get(2).map(String::as_str),
        Some("last answer: done and [2Jclean")
    );
    assert_eq!(lines.len(), 3, "no receipt, no receipt line: {lines:?}");
}

/// Types `text` and Enter, and returns what the driver must do.
fn enter(app: &mut App, text: &str) -> Vec<Effect> {
    app.handle(Input::Paste(text.to_owned()));
    app.handle(Input::Key(Key::Enter))
}

/// `/export` reaches the driver only when idle; `/recap` answers in the frame, a running task or
/// not, and changes nothing it summed.
#[test]
fn sim_export_waits_for_the_task_and_recap_changes_nothing() {
    let mut app = App::new(header(), Vec::new(), 80, 24);
    assert_eq!(
        enter(&mut app, "/export a.md"),
        [Effect::Export(String::from("a.md"))]
    );
    assert_eq!(enter(&mut app, "/export"), [Effect::Export(String::new())]);
    assert_eq!(enter(&mut app, "/ex"), [Effect::Export(String::new())]);
    assert_eq!(
        enter(&mut app, "write it"),
        [Effect::Start(String::from("write it"))]
    );
    let before: Vec<Entry> = app.entries().iter().cloned().collect();
    assert_eq!(enter(&mut app, "/export b.md"), []);
    assert_eq!(app.entries().len(), before.len(), "nothing ran or queued");
    app.handle(Input::Key(Key::KillStart));
    assert_eq!(enter(&mut app, "/recap"), []);
    let after: Vec<Entry> = app.entries().iter().cloned().collect();
    assert_eq!(after.get(..before.len()), Some(before.as_slice()));
    assert_eq!(after.last(), Some(&Entry::Info(recap(&holding(before)))));
}

/// A terminal whose settings are refused holds no row and still exports, once.
#[test]
fn sim_export_through_the_driver_needs_no_row() -> Outcome {
    let folder = sim_tree::scratch("tui-export-driver")?;
    let mut app = App::new(header(), Vec::new(), 80, 24);
    let store = SessionStore::open(&Err(ExecError::PathUnset), &folder, "mock-model");
    store.export(&mut app, "");
    assert_eq!(
        app.entries().back(),
        Some(&Entry::Info(vec![String::from(
            "exported to session.md (0 entries, 0 receipts)."
        )]))
    );
    let text = fs::read_to_string(folder.join("session.md"))?;
    let about = format!(
        "name: session\nfolder: {}\nmodel: mock-model\n",
        folder.display()
    );
    assert!(text.contains(&about), "{text}");
    store.export(&mut app, "");
    assert_eq!(
        app.entries().back(),
        Some(&Entry::Notice(
            Style::BAD,
            String::from(
                "session.md already exists; /export never overwrites. Name another: /export <file>."
            )
        ))
    );
    assert_eq!(fs::read_to_string(folder.join("session.md"))?, text);
    Ok(())
}
