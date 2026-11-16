//! Seeded simulations of the artifact pane (larry#244, ADR 0047). One seed drives every
//! artifact, size and key; no clock, no terminal. Oracles are written here from the rule
//! (a bound, a count, a reference window), never from the pane's own output. Sources:
//! ECMA-48 for the escape check, CVE-2021-42574 (Trojan Source) and CVE-2003-0063 (xterm
//! escape injection) for the hostile text, `git diff` unified format for the hunk rules.

use std::collections::BTreeSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};

use super::{DIFF_ROWS, DiffView, FILES_LISTED, OUTPUT_ROWS, Pane, ROW_CHARS, Tab};
use crate::tui::app::{App, Input, Key};
use crate::tui::buffer::{Buffer, Frame, Rect, render};
use crate::tui::cells::{Entry, Header};
use crate::tui::sim_tui::{HOSTILE, Seeded};
use crate::tui::style::Style;
use crate::tui::worker::{Ended, Event, Receipt};

/// Seeds each simulation runs.
const SEEDS: u64 = 256;
/// A hostile line of ten mebibytes.
const TEN_MIB: usize = 10_485_760;

/// One of `items`.
fn choose<'a>(rng: &mut Seeded, items: &'a [&'a str]) -> &'a str {
    match items.get(rng.pick(items.len())) {
        Some(item) => item,
        None => "",
    }
}

/// The most characters a stored row holds: [`ROW_CHARS`] read, a tab made four.
fn row_bound() -> usize {
    ROW_CHARS.saturating_mul(4)
}

/// A receipt whose every text is hostile.
fn receipt(rng: &mut Seeded) -> Receipt {
    Receipt {
        digest: choose(rng, &HOSTILE).to_owned(),
        accepted: rng.pick(2) == 0,
        commands: rng.pick(300),
        declined: rng.pick(300),
        interrupted: rng.pick(3),
        handoff: choose(rng, &HOSTILE).to_owned(),
    }
}

/// One line of a hunk, hostile or plain: any first byte a diff line may start with.
fn hunk_line(rng: &mut Seeded) -> String {
    let heads = [
        "+",
        "-",
        " ",
        "\\ No newline at end of file",
        "@@ -9 +9 @@ ",
        "--- x",
        "+++ y",
        "diff --git a/z b/z",
        "",
    ];
    format!("{}{}", choose(rng, &heads), choose(rng, &HOSTILE))
}

/// A diff of up to four files of `lines` hunk lines each, names and lines hostile.
fn random_diff(rng: &mut Seeded, lines: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    for file in 0..rng.pick(5) {
        let name = format!("f{file}{}", choose(rng, &HOSTILE));
        parts.push(format!(
            "diff --git a/{name} b/{name}\nindex 1..2 100644\n--- a/{name}\n+++ b/{name}\n@@ -1 +1 @@\n"
        ));
        for _ in 0..lines {
            parts.push(hunk_line(rng));
            parts.push(String::from(if rng.pick(5) == 0 { "\r\n" } else { "\n" }));
        }
    }
    parts.concat()
}

/// `files` files of `lines` added lines each, the first with a ten mebibyte line as well
/// when `giant`. Each file holds four header and hunk-mark rows besides its added lines.
fn big_diff(files: usize, lines: usize, giant: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    for file in 0..files {
        parts.push(format!(
            "diff --git a/g{file} b/g{file}\n--- a/g{file}\n+++ b/g{file}\n@@ -0,0 +1 @@\n"
        ));
        if giant && file == 0 {
            parts.push(format!("+{}\n", "z".repeat(TEN_MIB)));
        }
        for line in 0..lines {
            parts.push(format!("+line {line}\n"));
        }
    }
    parts.concat()
}

/// A pane after a seeded run of events; `giant` adds a 100,000-line output, a ten mebibyte
/// line and a 100,000-line diff.
fn populate(rng: &mut Seeded, giant: bool) -> Pane {
    let mut pane = Pane::new();
    for _ in 0..rng.pick(24) {
        match rng.pick(9) {
            0 => pane.started(choose(rng, &HOSTILE)),
            1 | 2 => {
                let count = if giant { 100_000 } else { rng.pick(40) };
                let mut lines: Vec<String> = Vec::new();
                if giant {
                    lines.push("z".repeat(TEN_MIB));
                }
                for line in 0..count {
                    lines.push(format!("line {line}{}", choose(rng, &HOSTILE)));
                }
                let passed = rng.pick(2) == 0;
                pane.finished(choose(rng, &HOSTILE), choose(rng, &HOSTILE), &lines, passed);
            }
            3 => {
                let lines = rng.pick(30);
                let text = if giant {
                    big_diff(1, 100_000, true)
                } else {
                    random_diff(rng, lines)
                };
                pane.set_diff(&text);
            }
            4 => pane.set_receipt(receipt(rng)),
            5 => pane.next_tab(),
            6 | 7 => pane.page(rng.pick(2) == 0, rng.pick(60)),
            _ if rng.pick(10) == 0 => pane.begin(),
            _ => {}
        }
    }
    pane
}

/// The diff `pane` holds, or an empty one that fails every count asserted on it.
fn diff_of(pane: &Pane) -> DiffView {
    match pane.diff {
        Some(ref diff) => diff.clone(),
        None => DiffView::parse(""),
    }
}

/// Fills every cell of `buffer` with `Q`, so a write outside an area shows.
fn sentinel(buffer: &mut Buffer) {
    let line = "Q".repeat(buffer.width());
    for row in 0..buffer.height() {
        buffer.put(0, row, buffer.width(), &line, Style::PLAIN);
    }
}

/// The bytes a full repaint of `buffer` writes.
fn wire(buffer: Buffer) -> Vec<u8> {
    let mut out = Vec::new();
    render(
        None,
        &Frame {
            buffer,
            cursor: None,
        },
        &mut out,
    );
    out
}

/// Whether `bytes` hold only the frame's own sequences and no control or bidi character.
fn wire_is_clean(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let own = text.split('\x1b').skip(1).all(|piece| {
        piece.starts_with('[')
            && piece
                .chars()
                .skip(1)
                .find(|ch| !(ch.is_ascii_digit() || matches!(ch, ';' | '?')))
                .is_some_and(|last| matches!(last, 'H' | 'J' | 'm' | 'h' | 'l'))
    });
    own && !text.contains("\x1b]")
        && !text.contains([
            '\u{7}', '\u{8}', '\0', '\r', '\u{202e}', '\u{2066}', '\u{2069}',
        ])
}

/// A hostile diff or output, at any size, paints only inside its area, leaves no escape or
/// bidi byte on the wire and never grows past its bounds. The saboteur's cases: ESC and OSC
/// in names and lines, bidi in the receipt, a ten mebibyte line, 100,000 lines, an area of
/// zero rows or columns, an area cut by the screen's edge. Catches a paint that forgets
/// `sanitize` or `limit`.
#[test]
fn sim_pane_hostile_artifacts_paint_inside_bounds() {
    for seed in 0..SEEDS {
        let mut rng = Seeded::new(seed);
        let pane = populate(&mut rng, seed.checked_rem(32) == Some(0));
        let x = rng.pick(40).saturating_add(1);
        let y = rng.pick(4);
        let (width, height) = (rng.pick(200), rng.pick(90));
        let cols = x.saturating_add(width).saturating_add(rng.pick(3));
        let rows = y.saturating_add(height).saturating_add(rng.pick(3));
        let mut buffer = Buffer::new(cols, rows);
        sentinel(&mut buffer);
        let area = Rect {
            x,
            y,
            width,
            height,
        };
        pane.paint(&mut buffer, area);
        let outside = "Q".repeat(buffer.width());
        let before = "Q".repeat(x.saturating_sub(1));
        let after = "Q".repeat(buffer.width().saturating_sub(area.right()));
        for row in 0..buffer.height() {
            let text = buffer.row_text(row);
            if row < area.y || row >= area.bottom() || width == 0 {
                assert_eq!(
                    text, outside,
                    "seed {seed} row {row}: wrote outside the rows"
                );
                continue;
            }
            assert!(
                text.starts_with(&before),
                "seed {seed} row {row}: left of the rule"
            );
            assert!(
                text.ends_with(&after),
                "seed {seed} row {row}: right of the area"
            );
        }
        assert!(
            wire_is_clean(&wire(buffer)),
            "seed {seed}: a foreign byte on the wire"
        );
        assert!(pane.output.len() <= OUTPUT_ROWS, "seed {seed}");
        let diff_rows = diff_of(&pane).rows;
        assert!(
            diff_rows.len() <= DIFF_ROWS.saturating_add(FILES_LISTED).saturating_add(4),
            "seed {seed}"
        );
        for row in pane.output.iter().chain(diff_rows.iter()) {
            let text = &row.1;
            assert!(
                text.chars().count() <= row_bound(),
                "seed {seed}: a row past its bound"
            );
            assert!(!text.chars().any(char::is_control), "seed {seed}: {text:?}");
        }
    }
}

/// The reference window: where a pane of `len` rows scrolled `scroll` shows in `body` rows,
/// from the top (`from_top`) or from the newest row.
fn reference(len: usize, body: usize, scroll: usize, from_top: bool) -> (usize, usize) {
    let room = len.saturating_sub(body);
    if from_top {
        let start = scroll.min(room);
        (start, start.saturating_add(body).min(len))
    } else {
        let end = len.saturating_sub(scroll.min(room));
        (end.saturating_sub(body), end)
    }
}

/// Whatever the pages, the window stays inside the rows, is a full body when the content is
/// longer than it, equals the reference window, and reaches both ends of every tab. The
/// fat-finger's key repeat: a page per row either way on every tab.
#[test]
fn sim_pane_scroll_never_leaves_the_content() {
    for seed in 0..SEEDS {
        let mut rng = Seeded::new(seed);
        let mut pane = populate(&mut rng, false);
        let lines = rng.pick(40);
        pane.set_diff(&random_diff(&mut rng, lines));
        pane.set_receipt(receipt(&mut rng));
        let body = rng.pick(30).saturating_add(4);
        for _ in 0..rng.pick(60) {
            pane.page(rng.pick(2) == 0, body);
            let tab = pane.tab;
            let len = pane.len(tab);
            let (start, end) = pane.window(tab, body);
            assert!(
                start <= end && end <= len,
                "seed {seed}: {start}..{end} of {len}"
            );
            assert_eq!(
                end.saturating_sub(start),
                len.min(body),
                "seed {seed}: a short window"
            );
            let (scroll, from_top) = match tab {
                Tab::Output => (pane.back, false),
                Tab::Diff => (pane.diff_top, true),
                Tab::Receipt => (pane.receipt_top, true),
            };
            assert_eq!(
                (start, end),
                reference(len, body, scroll, from_top),
                "seed {seed}"
            );
            if rng.pick(4) == 0 {
                pane.next_tab();
            }
        }
        for tab in Tab::ALL {
            while pane.tab != tab {
                pane.next_tab();
            }
            let len = pane.len(tab);
            for _ in 0..=len {
                pane.page(true, body);
            }
            assert_eq!(
                pane.window(tab, body).0,
                0,
                "seed {seed}: {tab:?} at its first row"
            );
            for _ in 0..=len {
                pane.page(false, body);
            }
            assert_eq!(
                pane.window(tab, body).1,
                len,
                "seed {seed}: {tab:?} at its last row"
            );
        }
    }
}

/// The hash of the wire a pane paints on a `width` by `height` screen.
fn frame_hash(pane: &Pane, width: usize, height: usize) -> u64 {
    let mut buffer = Buffer::new(width, height);
    let area = Rect {
        x: 45,
        y: 0,
        width: width.saturating_sub(45),
        height,
    };
    pane.paint(&mut buffer, area);
    let mut hasher = DefaultHasher::new();
    wire(buffer).hash(&mut hasher);
    hasher.finish()
}

/// One seed gives one frame: each seed run twice from scratch hashes the same, and the seeds
/// do not collapse to a few frames. The trace is the wire of the whole paint.
#[test]
fn sim_pane_same_seed_same_frame_hash() {
    let mut distinct = BTreeSet::new();
    for seed in 0..64 {
        let hash = |seed: u64| {
            let mut rng = Seeded::new(seed);
            let pane = populate(&mut rng, false);
            frame_hash(&pane, 120, 34)
        };
        let first = hash(seed);
        assert_eq!(first, hash(seed), "seed {seed} replayed differently");
        distinct.insert(first);
    }
    assert!(
        distinct.len() > 16,
        "seeds collapse to {} frames",
        distinct.len()
    );
}

/// The text the Diff and Receipt tabs of `pane` paint on a 100 by 20 screen.
fn both_tabs(pane: &Pane) -> String {
    let mut text = String::new();
    for tab in [Tab::Diff, Tab::Receipt] {
        let mut view = pane.clone();
        while view.tab != tab {
            view.next_tab();
        }
        let mut buffer = Buffer::new(100, 20);
        let area = Rect {
            x: 1,
            y: 0,
            width: 99,
            height: 20,
        };
        view.paint(&mut buffer, area);
        for row in 0..20 {
            text.push_str(&buffer.row_text(row));
        }
    }
    text
}

/// Two panes fed at once with the same file name and different bodies and receipts, in a
/// seeded interleaving: each paints only its own rows. Catches a shared static or cache.
#[test]
fn sim_pane_two_panes_fed_at_once_never_mix() {
    for seed in 0..SEEDS {
        let mut rng = Seeded::new(seed);
        let (mut first, mut second) = (Pane::new(), Pane::new());
        let mark = |tag: &str| format!("diff --git a/same.txt b/same.txt\n@@ -1 +1 @@\n+{tag}\n");
        let (mut a_receipt, mut b_receipt) = (receipt(&mut rng), receipt(&mut rng));
        a_receipt.digest = String::from("aaaaaaaaaaaa");
        b_receipt.digest = String::from("bbbbbbbbbbbb");
        let mut steps: Vec<usize> = (0..4).collect();
        for at in (1..steps.len()).rev() {
            steps.swap(at, rng.pick(at.saturating_add(1)));
        }
        for step in steps {
            match step {
                0 => first.set_diff(&mark("only-a")),
                1 => second.set_diff(&mark("only-b")),
                2 => first.set_receipt(a_receipt.clone()),
                _ => second.set_receipt(b_receipt.clone()),
            }
        }
        for (pane, own, other) in [(&first, "a", "b"), (&second, "b", "a")] {
            let text = both_tabs(pane);
            assert!(text.contains(&format!("only-{own}")), "seed {seed}");
            assert!(text.contains(&own.repeat(12)), "seed {seed}");
            assert!(
                !text.contains(&format!("only-{other}")),
                "seed {seed}: crossed"
            );
            assert!(!text.contains(&other.repeat(12)), "seed {seed}: crossed");
        }
    }
}

/// A header for a simulated app.
fn header() -> Header {
    Header {
        version: String::from("0.0.0"),
        model: Ok(String::from("sim-model")),
        directory: String::from("~/sim"),
        blocked: None,
    }
}

/// The wire of an app's full repaint.
fn app_wire(app: &App) -> Vec<u8> {
    let mut out = Vec::new();
    render(None, &app.draw(), &mut out);
    out
}

/// The text of the first `rows` rows of `app`'s frame.
fn screen(app: &App, rows: usize) -> String {
    let frame = app.draw();
    (0..rows).map(|row| frame.buffer.row_text(row)).collect()
}

/// The kill -9 and restart answer: the pane is a fold over the event log, so a fresh app fed
/// the same log paints the same frame, and a new task never paints the last task's diff. The
/// durable owner of the diff and the receipt is the sealed report in the ledger.
#[test]
fn sim_pane_replay_of_the_event_log_rebuilds_the_same_frame() {
    for seed in 0..SEEDS {
        let mut rng = Seeded::new(seed);
        let observation = format!(
            "exit status 1\nstdout:\n{}\nstderr:\n",
            choose(&mut rng, &HOSTILE)
        );
        let mut log: Vec<Input> = vec![
            Input::Key(Key::Char('x')),
            Input::Key(Key::Enter),
            Input::Worker(Event::Started(String::from("make"))),
            Input::Worker(Event::Finished {
                observation,
                passed: rng.pick(2) == 0,
            }),
            Input::Worker(Event::Diff(random_diff(&mut rng, 20))),
            Input::Worker(Event::Receipt(receipt(&mut rng))),
        ];
        for _ in 0..rng.pick(8) {
            let key = if rng.pick(2) == 0 {
                Key::Tab
            } else {
                Key::PageUp
            };
            log.push(Input::Key(key));
        }
        let mut live = App::new(header(), Vec::new(), 120, 34);
        let mut replay = App::new(header(), Vec::new(), 120, 34);
        for input in &log {
            live.handle(input.clone());
        }
        for input in log {
            replay.handle(input);
        }
        assert_eq!(app_wire(&live), app_wire(&replay), "seed {seed}");
        replay.handle(Input::Worker(Event::Ended(Ended::Done)));
        replay.handle(Input::Key(Key::Char('y')));
        replay.handle(Input::Key(Key::Enter));
        assert!(
            !screen(&replay, 34).contains("files, +"),
            "seed {seed}: a stale diff survived a new task"
        );
    }
}

/// The saboteur drives each bound past its limit: 100,000 output lines, a diff of 100,000
/// lines over 100 files, a ten mebibyte line. Stored rows stop at the bound, every dropped
/// row is counted exactly, and the next task starts from nothing.
#[test]
fn sim_pane_saturates_each_bound_counts_the_drop_and_recovers() {
    let mut pane = Pane::new();
    let mut lines = vec!["z".repeat(TEN_MIB)];
    lines.extend((0..100_000).map(|n| format!("line {n}")));
    let offered = lines.len().saturating_add(1);
    pane.finished("make", "exit 0", &lines, true);
    assert_eq!(pane.output.len(), OUTPUT_ROWS);
    assert_eq!(
        pane.output.len().saturating_add(pane.dropped),
        offered,
        "kept or counted"
    );
    assert!(
        pane.output
            .iter()
            .all(|row| row.1.chars().count() <= row_bound())
    );
    pane.set_diff(&big_diff(100, 1000, true));
    let diff = diff_of(&pane);
    // 100 files of 1,004 rows each (four marks and 1,000 added lines), and the ten mebibyte line.
    assert_eq!((diff.files, diff.added, diff.removed), (100, 100_001, 0));
    assert_eq!(diff.hidden_files, 100 - FILES_LISTED);
    assert_eq!(diff.hidden_rows, 100_401 - DIFF_ROWS);
    // Total, listed files, the hidden-files row, a blank, the kept hunks, the hidden-rows row.
    assert_eq!(diff.rows.len(), 1 + FILES_LISTED + 1 + 1 + DIFF_ROWS + 1);
    pane.begin();
    assert_eq!(
        (pane.output.len(), pane.dropped, pane.len(Tab::Diff)),
        (0, 0, 0)
    );
    pane.finished("ls", "exit 0", &[String::from("a")], true);
    assert_eq!(
        pane.output.len(),
        2,
        "a fresh task is not held back by the last one"
    );
}

/// Each hidden row is counted exactly: random sizes against the reference counts, and the
/// output marker is what the pane paints at the oldest end of its tab.
#[test]
fn sim_pane_markers_count_every_dropped_row() {
    for seed in 0..64 {
        let mut rng = Seeded::new(seed);
        let (files, lines) = (rng.pick(130), rng.pick(120));
        let mut pane = Pane::new();
        pane.set_diff(&big_diff(files, lines, false));
        let diff = diff_of(&pane);
        let hunk_rows = files.saturating_mul(lines.saturating_add(4));
        assert_eq!(
            diff.hidden_rows,
            hunk_rows.saturating_sub(DIFF_ROWS),
            "seed {seed}"
        );
        assert_eq!(
            diff.hidden_files,
            files.saturating_sub(FILES_LISTED),
            "seed {seed}"
        );
        assert_eq!(
            (diff.added, diff.removed),
            (files.saturating_mul(lines), 0),
            "seed {seed}"
        );
        let has = |want: String| diff.rows.iter().any(|row| row.1 == want);
        assert_eq!(
            has(format!("… {} more lines", diff.hidden_rows)),
            diff.hidden_rows > 0
        );
        assert_eq!(
            has(format!("  … {} more files", diff.hidden_files)),
            diff.hidden_files > 0
        );
        let mut out = Pane::new();
        let ran = rng.pick(1500);
        let output: Vec<String> = (0..ran).map(|n| format!("o{n}")).collect();
        out.finished("c", "s", &output, true);
        let offered = ran.saturating_add(1).saturating_add(usize::from(ran == 0));
        assert_eq!(
            out.output.len().saturating_add(out.dropped),
            offered,
            "seed {seed}"
        );
        if out.dropped > 0 {
            for _ in 0..out.len(Tab::Output) {
                out.page(true, 27);
            }
            let mut buffer = Buffer::new(100, 30);
            let area = Rect {
                x: 1,
                y: 0,
                width: 99,
                height: 30,
            };
            out.paint(&mut buffer, area);
            let shown: String = (0..30).map(|row| buffer.row_text(row)).collect();
            assert!(
                shown.contains(&format!("… {} earlier rows", out.dropped)),
                "seed {seed}"
            );
        }
    }
}

/// The hunk state machine: a removed line whose text starts with two dashes is removed and
/// paints BAD, an added line starting with two pluses paints GOOD, the real headers and the
/// `@@` rows are muted, context is plain, and the next file's header is a header again.
/// A `starts_with("---")` shortcut fails this.
#[test]
fn sim_pane_diff_state_machine_colours_a_removed_dashes_line() {
    let text = "diff --git a/x b/x\nindex 1..2 100644\n--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n context\n--- gone\n+++ new\n-plain\n+plain\n\\ No newline at end of file\ndiff --git a/y b/y\n--- a/y\n+++ b/y\n@@ -1 +1 @@\n-y\n";
    let view = DiffView::parse(text);
    let style_of = |wanted: &str| {
        view.rows
            .iter()
            .find(|row| row.1 == wanted)
            .map(|row| row.0)
    };
    assert_eq!(style_of("--- gone"), Some(Style::BAD));
    assert_eq!(style_of("+++ new"), Some(Style::GOOD));
    assert_eq!(style_of("-plain"), Some(Style::BAD));
    assert_eq!(style_of("+plain"), Some(Style::GOOD));
    assert_eq!(style_of("--- a/x"), Some(Style::MUTED));
    assert_eq!(style_of("+++ b/y"), Some(Style::MUTED));
    assert_eq!(style_of("--- a/y"), Some(Style::MUTED));
    assert_eq!(style_of("@@ -1,3 +1,3 @@"), Some(Style::MUTED));
    assert_eq!(style_of(" context"), Some(Style::PLAIN));
    assert_eq!(style_of("\\ No newline at end of file"), Some(Style::MUTED));
    // Added: `+++ new`, `+plain`. Removed: `--- gone`, `-plain`, `-y`. Neither header counts.
    assert_eq!((view.files, view.added, view.removed), (2, 2, 3));
    let listed: Vec<&str> = view.rows.iter().map(|row| row.1.as_str()).collect();
    assert!(
        listed.contains(&"  x  +2 -2") && listed.contains(&"  y  +0 -1"),
        "{listed:?}"
    );
}

/// Tab visits each tab once and returns; an arriving artifact selects its own tab.
#[test]
fn sim_pane_tab_cycle_visits_each_tab_once_and_returns() {
    let mut pane = Pane::new();
    let mut seen = vec![pane.tab];
    for _ in 0..3 {
        pane.next_tab();
        seen.push(pane.tab);
    }
    assert_eq!(seen, [Tab::Output, Tab::Diff, Tab::Receipt, Tab::Output]);
    pane.set_diff("");
    assert_eq!(pane.tab, Tab::Diff);
    pane.set_receipt(Receipt {
        digest: String::from("0123456789ab"),
        accepted: true,
        commands: 1,
        declined: 0,
        interrupted: 0,
        handoff: String::from("applied"),
    });
    assert_eq!(pane.tab, Tab::Receipt);
    pane.started("ls");
    assert_eq!(pane.tab, Tab::Output);
}

/// A frame costs the body, not the diff: for diffs of 10 to 100,000 lines the pane
/// materialises at most its body's rows, while the rival, the transcript path that wraps
/// every row of an `Entry::Info`, materialises every line.
#[test]
fn sim_pane_paints_body_rows_not_diff_rows() {
    for lines in [10_usize, 1000, 4096, 100_000] {
        let mut pane = Pane::new();
        pane.set_diff(&big_diff(1, lines, false));
        let body = Pane::body(34);
        let (start, end) = pane.window(Tab::Diff, body);
        let shown = end.saturating_sub(start);
        assert!(shown <= body, "{lines}: {shown} rows shown");
        let rival = Entry::Info((0..lines).map(|n| format!("+line {n}")).collect());
        assert_eq!(
            rival.lines(60).len(),
            lines,
            "the rival materialises every row"
        );
        assert_eq!(
            shown,
            pane.len(Tab::Diff).min(body),
            "{lines}: the window is the body"
        );
        assert!(
            lines <= body || shown < lines,
            "{lines}: the pane shows fewer rows"
        );
    }
}

/// The wiring through the app: on a wide screen PgDn and Tab move the pane; on a narrow
/// screen there is no pane and the chat keeps one `diff:` line.
#[test]
fn sim_pane_keys_move_the_pane_wide_and_the_chat_narrow() {
    let diff = big_diff(1, 200, false);
    let mut wide = App::new(header(), Vec::new(), 120, 34);
    wide.handle(Input::Worker(Event::Diff(diff.clone())));
    let before = screen(&wide, 34);
    assert!(
        before.contains("1 files, +200 -0"),
        "the diff is in the pane"
    );
    wide.handle(Input::Key(Key::PageDown));
    assert_ne!(before, screen(&wide, 34), "PgDn moved the pane");
    wide.handle(Input::Key(Key::Tab));
    assert!(
        screen(&wide, 34).contains("The sealed receipt shows here"),
        "Tab moved to Receipt"
    );
    let mut narrow = App::new(header(), Vec::new(), 80, 34);
    narrow.handle(Input::Worker(Event::Diff(diff)));
    let text = screen(&narrow, 34);
    assert!(
        text.contains("diff: 1 files, +200 -0"),
        "the one-line summary"
    );
    assert!(!text.contains("Receipt"), "no pane on a narrow screen");
}
