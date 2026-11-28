//! Seeded simulations of the frame's parts against reference models (larry#239, ADR 0045):
//! the composer's edits, the footer hints, the elapsed format, the welcome box, a command's
//! output tail and the grid's bounds. Each oracle is a model written here from the rule, never
//! the code's own output. Sources: GNU Readline's emacs bindings (C-a, C-e, C-u, C-k, C-w),
//! Codex `footer_hint.rs` and `status_indicator_widget.rs`, ECMA-48 for the grid.

use std::time::Duration;

use super::app::{App, Effect, Input, Key, MAX_ENTRIES};
use super::buffer::Buffer;
use super::cells::{Entry, Header, OUTPUT_TAIL, split_observation};
use super::composer::Composer;
use super::paint::{compact, hints};
use super::style::{Style, text_width};
use super::worker::{Decision, Event};

/// Seeds each simulation runs.
const SEEDS: u64 = 256;

/// How many keys [`key_at`] draws from.
pub(super) const KEY_COUNT: usize = 14;

/// The key the seeded input draws at `index`; the last key answers every index past the table.
pub(super) const fn key_at(index: usize) -> Key {
    match index {
        0 => Key::Char('y'),
        1 => Key::Char('n'),
        2 => Key::Char('a'),
        3 => Key::Char('/'),
        4 => Key::Enter,
        5 => Key::NewLine,
        6 => Key::Backspace,
        7 => Key::Left,
        8 => Key::Up,
        9 => Key::PageUp,
        10 => Key::PageDown,
        11 => Key::Tab,
        12 => Key::Esc,
        _ => Key::KillWord,
    }
}

/// A splitmix64 stream: the one source of every choice in a run.
struct Draw(u64);

impl Draw {
    /// The next value.
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ (mixed >> 31)
    }

    /// An index in `0..len`, or 0 for an empty range.
    fn pick(&mut self, len: usize) -> usize {
        let bound = u64::try_from(len).into_iter().sum();
        let drawn: u64 = self.next().checked_rem(bound).into_iter().sum();
        usize::try_from(drawn).into_iter().sum()
    }

    /// The item of `items` this draw picks, or the empty text for an empty list.
    fn word<'items>(&mut self, items: &[&'items str]) -> &'items str {
        match items.get(self.pick(items.len())) {
            Some(&item) => item,
            None => "",
        }
    }
}

/// The readline rules on a plain character list: the reference the composer must match.
#[derive(Default)]
struct Model {
    /// The text, one char per entry.
    chars: Vec<char>,
    /// The cursor, a char index.
    cursor: usize,
}

impl Model {
    /// The start of the cursor's line.
    fn line_start(&self) -> usize {
        let head: &[char] = match self.chars.get(..self.cursor) {
            Some(head) => head,
            None => &[],
        };
        match head.iter().rposition(|character| *character == '\n') {
            Some(at) => at.saturating_add(1),
            None => 0,
        }
    }

    /// The end of the cursor's line.
    fn line_end(&self) -> usize {
        let tail: &[char] = match self.chars.get(self.cursor..) {
            Some(tail) => tail,
            None => &[],
        };
        match tail.iter().position(|character| *character == '\n') {
            Some(at) => self.cursor.saturating_add(at),
            None => self.chars.len(),
        }
    }

    /// Removes `start..end` and puts the cursor at `start`.
    fn cut(&mut self, start: usize, end: usize) {
        if start <= end && end <= self.chars.len() {
            self.chars.drain(start..end);
            self.cursor = start;
        }
    }

    /// Applies edit `op`, inserting `text` when it inserts.
    fn apply(&mut self, op: usize, text: &str) {
        match op {
            0 => {
                for character in text.chars() {
                    self.chars.insert(self.cursor, character);
                    self.cursor = self.cursor.saturating_add(1);
                }
            }
            1 if self.cursor > 0 => self.cut(self.cursor.saturating_sub(1), self.cursor),
            2 if self.cursor < self.chars.len() => {
                let cursor = self.cursor;
                self.cut(cursor, cursor.saturating_add(1));
            }
            3 => self.cursor = self.cursor.saturating_sub(1),
            4 => self.cursor = self.cursor.saturating_add(1).min(self.chars.len()),
            5 => self.cursor = self.line_start(),
            6 => self.cursor = self.line_end(),
            7 => self.cut(self.line_start(), self.cursor),
            8 => {
                let end = self.line_end();
                let cursor = self.cursor;
                self.cut(cursor, end);
                self.cursor = cursor;
            }
            9 => {
                let mut start = self.cursor;
                while start > 0 && self.chars.get(start.saturating_sub(1)) == Some(&' ') {
                    start = start.saturating_sub(1);
                }
                while start > 0
                    && !matches!(self.chars.get(start.saturating_sub(1)), Some(&(' ' | '\n')))
                {
                    start = start.saturating_sub(1);
                }
                self.cut(start, self.cursor);
            }
            10 => *self = Self::default(),
            _ => {}
        }
    }
}

/// Applies edit `op` to the composer.
fn edit(composer: &mut Composer, op: usize, text: &str) {
    match op {
        0 => {
            composer.insert(text);
        }
        1 => composer.backspace(),
        2 => composer.delete(),
        3 => composer.left(),
        4 => composer.right(),
        5 => composer.home(),
        6 => composer.end(),
        7 => composer.kill_start(),
        8 => composer.kill_end(),
        9 => composer.kill_word(),
        10 => composer.clear(),
        _ => {}
    }
}

/// The monkey on the keyboard: seeded readline edits over spaces, newlines and multi-byte
/// text. The composer's text equals the reference model's after every edit.
#[test]
fn sim_composer_edits_match_the_readline_model() {
    let pieces = ["a", "bc", " ", "  ", "\n", "é", "表示", "word ", "x\ny"];
    for seed in 0..SEEDS {
        let mut draw = Draw(seed);
        let mut composer = Composer::default();
        let mut model = Model::default();
        for step in 0..64 {
            let op = draw.pick(11);
            let text = draw.word(&pieces);
            edit(&mut composer, op, text);
            model.apply(op, text);
            let expected: String = model.chars.iter().collect();
            assert_eq!(composer.text(), expected, "seed {seed} step {step} op {op}");
            assert_eq!(composer.is_empty(), expected.is_empty());
        }
    }
}

/// The line endings a paste carries for one line break: xterm's bracketed paste (mode 2004)
/// sends CR, a Windows clipboard CR LF, a Unix one LF.
const BREAKS: [&str; 3] = ["\r", "\r\n", "\n"];

/// The pasted lines: what is pasted, and what the person must get (CWE-150: no control byte).
const PASTED: [(&str, &str); 7] = [
    ("alpha", "alpha"),
    ("é表", "é表"),
    ("a\x1bb", "ab"),
    ("c\x07\x00d", "cd"),
    ("e\u{9b}\x7ff", "ef"),
    ("g\th", "g    h"),
    ("i\x0bj\x0ck", "ijk"),
];

/// The paste from another terminal: for every seed each line break, alone or in a run, is one
/// newline in the composer and in the task Enter sends, and no other control byte survives
/// (larry#605). A CR then an LF is the one CR LF break. Catches the lines of a pasted task
/// joined, doubled, or sent to the model other than as the person saw them.
#[test]
fn sim_paste_keeps_every_line_break_as_one_newline() {
    for ending in BREAKS {
        let mut composer = Composer::default();
        composer.insert(&format!("a{ending}b"));
        assert_eq!(composer.text(), "a\nb", "{ending:?}");
        let mut app = App::new(header(), Vec::new(), 100, 40);
        app.handle(Input::Paste(format!("a{ending}b")));
        assert_eq!(
            app.handle(Input::Key(Key::Enter)),
            vec![Effect::Start(String::from("a\nb"))],
            "{ending:?}"
        );
    }
    for seed in 0..SEEDS {
        let mut draw = Draw(seed);
        let mut pasted = String::from("head");
        let mut kept = String::from("head");
        for _line in 0..=draw.pick(12) {
            let mut after_cr = false;
            for _break in 0..=draw.pick(2) {
                let ending = draw.word(&BREAKS);
                pasted.push_str(ending);
                if !(after_cr && ending == "\n") {
                    kept.push('\n');
                }
                after_cr = ending == "\r";
            }
            let (sent, seen) = PASTED.get(draw.pick(PASTED.len())).copied().unzip();
            pasted.extend(sent);
            kept.extend(seen);
        }
        let mut composer = Composer::default();
        assert!(composer.insert(&pasted), "seed {seed}: nothing is cut");
        assert_eq!(composer.text(), kept, "seed {seed}: {pasted:?}");
        let mut app = App::new(header(), Vec::new(), 100, 40);
        let submitted = app.handle(Input::Paste(pasted.clone()));
        assert!(submitted.is_empty(), "seed {seed}: a paste submits nothing");
        assert_eq!(
            app.handle(Input::Key(Key::Enter)),
            vec![Effect::Start(kept)],
            "seed {seed}: {pasted:?}"
        );
    }
}

/// The footer never runs past its width, drops a hint whole, marks the cut, keeps the
/// rest in order, and never starts with a separator.
#[test]
fn sim_footer_hints_fit_their_width_and_drop_whole_hints() {
    let pool = [
        "enter send",
        "/ commands",
        "ctrl+j newline",
        "esc stop",
        "pgup scroll",
    ];
    for seed in 0..SEEDS {
        let mut draw = Draw(seed);
        let count = 1 + draw.pick(pool.len());
        let items: Vec<&str> = pool.iter().copied().take(count).collect();
        let full = items.join(" · ");
        let width = draw.pick(text_width(&full).saturating_add(4));
        let line = hints(&items, width);
        let text: String = line.iter().map(|span| span.text.as_str()).collect();
        assert!(!text.starts_with(" ·"), "seed {seed}: {text:?}");
        if text_width(&full) <= width {
            assert_eq!(text, full, "seed {seed}: everything fits at {width}");
            continue;
        }
        assert!(
            text.ends_with('…'),
            "seed {seed}: the cut is marked: {text:?}"
        );
        let kept = text.trim_end_matches('…').trim_end();
        assert!(
            full.starts_with(kept),
            "seed {seed}: whole hints in order: {text:?}"
        );
        assert!(
            kept.is_empty() || items.iter().any(|item| kept.ends_with(item)),
            "seed {seed}: no hint is cut in half: {text:?}"
        );
        assert!(
            text_width(&text) <= width.max(1),
            "seed {seed}: {text:?} at {width}"
        );
    }
}

/// The elapsed format of the status row: seconds, then minutes and padded seconds, then hours
/// and padded minutes, at every boundary.
#[test]
fn sim_elapsed_format_matches_its_boundaries() {
    let cases = [
        (0, "0s"),
        (59, "59s"),
        (60, "1m 00s"),
        (61, "1m 01s"),
        (3599, "59m 59s"),
        (3600, "1h 00m"),
        (3661, "1h 01m"),
        (86_399, "23h 59m"),
    ];
    for (seconds, shown) in cases {
        assert_eq!(compact(Duration::from_secs(seconds)), shown, "{seconds}s");
    }
}

/// The weird-bytes fool in the welcome box: a directory or model name with newlines, tabs,
/// escapes and 300 characters still paints a box whose rows are one line each and all as wide.
#[test]
fn sim_welcome_box_rows_stay_one_line_and_one_width() {
    let hostile = [
        "~/a\nb\tc",
        "\x1b[31m~/red",
        &"x".repeat(300),
        "表示/目录",
        "",
    ];
    for seed in 0..SEEDS {
        let mut draw = Draw(seed);
        let header = Header {
            version: String::from("0.1.0"),
            model: if draw.pick(2) == 0 {
                Ok(String::from(draw.word(&hostile)))
            } else {
                Err(String::from(draw.word(&hostile)))
            },
            directory: String::from(draw.word(&hostile)),
            blocked: None,
        };
        let width = 12 + draw.pick(80);
        let lines = Entry::Header(header).lines(width);
        let widths: Vec<usize> = lines
            .iter()
            .map(|line| line.iter().map(|span| text_width(&span.text)).sum())
            .collect();
        assert!(
            widths.windows(2).all(|pair| pair.first() == pair.get(1)),
            "seed {seed}: {widths:?}"
        );
        for span in lines.iter().flatten() {
            assert!(
                !span.text.chars().any(char::is_control),
                "seed {seed}: {:?}",
                span.text
            );
        }
    }
}

/// A command's output tail: a failed run shows its status, at most [`OUTPUT_TAIL`] output
/// lines show and the rest are counted, and a run with nothing shows `(no output)`.
#[test]
fn sim_ran_tail_counts_hidden_lines_and_marks_failures() {
    for seed in 0..SEEDS {
        let mut draw = Draw(seed);
        let lines = draw.pick(12);
        let passed = draw.pick(2) == 0;
        let output: Vec<String> = (0..lines).map(|line| format!("line {line}")).collect();
        let observation = format!("exit status 3\nstdout:\n{}\nstderr:\n", output.join("\n"));
        let (status, parsed) = split_observation(&observation);
        assert_eq!(status, "exit status 3");
        assert_eq!(parsed, output, "seed {seed}: the parser keeps every line");
        let entry = Entry::ran(String::from("make"), &status, parsed, passed);
        let text: Vec<String> = entry
            .lines(60)
            .iter()
            .map(|line| line.iter().map(|span| span.text.as_str()).collect())
            .collect();
        let hidden = lines.saturating_sub(OUTPUT_TAIL);
        assert_eq!(
            text.iter()
                .any(|row| row.contains(&format!("+{hidden} lines"))),
            hidden > 0,
            "seed {seed}: {text:?}"
        );
        assert_eq!(
            text.iter().any(|row| row.contains("exit status 3")),
            !passed
        );
        assert_eq!(
            text.iter().any(|row| row.contains("(no output)")),
            lines == 0 && passed,
            "seed {seed}: {text:?}"
        );
        let shown = text.iter().filter(|row| row.contains("line ")).count();
        assert_eq!(shown, lines.min(OUTPUT_TAIL), "seed {seed}: {text:?}");
    }
}

/// A header for the simulations.
pub(super) fn header() -> Header {
    Header {
        version: String::from("0.0.0"),
        model: Ok(String::from("sim-model")),
        directory: String::from("~/sim"),
        blocked: None,
    }
}

/// Bytes the transcript keeps in its entries' text.
fn transcript_bytes(entries: &std::collections::VecDeque<Entry>) -> usize {
    entries
        .iter()
        .map(|entry| match *entry {
            Entry::Ran {
                ref command,
                ref status,
                ref output,
                ..
            } => command
                .len()
                .saturating_add(status.len())
                .saturating_add(output.iter().map(String::len).sum::<usize>()),
            Entry::User(ref text)
            | Entry::Declined(ref text)
            | Entry::Said(ref text)
            | Entry::Notice(_, ref text) => text.len(),
            Entry::Info(ref rows) => rows.iter().map(String::len).sum(),
            Entry::Header(_) => 0,
        })
        .fold(0, usize::saturating_add)
}

/// The byte bound a full transcript holds however large its command outputs were (larry#337):
/// 512 entries of at most six rows of 512 characters, four bytes a character, plus a command.
const TRANSCRIPT_BOUND: usize = 8 * 1024 * 1024;

/// A transcript of 512 large outputs stays under [`TRANSCRIPT_BOUND`] (larry#337). Each event
/// prints 32 lines of 1,024 characters: 16 MiB before the fix, because a `Ran` entry kept every
/// output line and every frame re-wrapped them all.
#[test]
fn sim_tui_a_full_transcript_of_large_outputs_stays_bounded() {
    let line = "x".repeat(1024);
    let output: String = (0..32)
        .map(|_| line.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let observation = format!("exit 0\nstdout:\n{output}\nstderr:\n");
    let mut app = App::new(header(), Vec::new(), 120, 34);
    for _ in 0..MAX_ENTRIES {
        app.handle(Input::Worker(Event::Finished {
            observation: observation.clone(),
            passed: true,
        }));
    }
    let stored = transcript_bytes(app.entries());
    assert!(
        stored < TRANSCRIPT_BOUND,
        "the transcript holds {stored} bytes, past the {TRANSCRIPT_BOUND} byte bound"
    );
}

/// A command the person approves shows every character the sanitizer drops as a
/// `⟨U+XXXX⟩` marker, so the command on screen is the one that runs (larry#337).
#[test]
fn sim_tui_an_approved_command_shows_each_dropped_character() {
    use super::style::sanitize_shown;
    for (shown, want) in [
        ("rm\u{200b} -rf /", "rm⟨U+200B⟩ -rf /"),
        ("a\u{202e}b", "a⟨U+202E⟩b"),
        ("line\u{2028}break", "line⟨U+2028⟩break"),
        ("a\u{9b}b", "a⟨U+009B⟩b"),
    ] {
        assert_eq!(sanitize_shown(shown), want, "{shown:?}");
    }
    let mut app = App::new(header(), Vec::new(), 100, 30);
    app.handle(Input::Worker(Event::Ask(String::from("echo a\u{200b}b"))));
    let frame = app.draw();
    let rows: String = (0..frame.buffer.height())
        .map(|row| frame.buffer.row_text(row))
        .collect();
    assert!(rows.contains("⟨U+200B⟩"), "{rows:?}");
    assert!(!rows.contains('\u{200b}'), "{rows:?}");
}

/// Names in a review and the hand-off line are the folder's, not Larry's: whatever they hold,
/// no painted row carries an escape, a bidi control or a format character.
#[test]
fn sim_tui_apply_review_lines_are_sanitized() {
    use super::sim_tui::{HOSTILE, Seeded};
    for seed in 0..SEEDS {
        let mut rng = Seeded::new(seed);
        let mut app = App::new(header(), Vec::new(), 60 + rng.pick(160), 20 + rng.pick(40));
        let lines: Vec<String> = (0..1 + rng.pick(4))
            .map(|_round| rng.word(&HOSTILE).to_owned())
            .collect();
        app.handle(Input::Worker(Event::Review(lines)));
        app.handle(Input::Worker(Event::Handed {
            line: rng.word(&HOSTILE).to_owned(),
            applied: rng.pick(2) == 1,
        }));
        let frame = app.draw();
        for row in 0..frame.buffer.height() {
            let text = frame.buffer.row_text(row);
            assert!(
                !text
                    .chars()
                    .any(|ch| ch == '\u{1b}' || ('\u{202A}'..='\u{202E}').contains(&ch)),
                "seed {seed} row {row}: {text:?}"
            );
        }
    }
}

/// The saboteur's coordinates: a write on a row past the grid changes nothing and reports no
/// cell written; one past the right edge is cut there.
#[test]
fn sim_grid_writes_outside_the_grid_change_nothing() {
    for seed in 0..SEEDS {
        let mut draw = Draw(seed);
        let (width, height) = (1 + draw.pick(40), 1 + draw.pick(20));
        let mut grid = Buffer::new(width, height);
        let before = grid.clone();
        let x = draw.pick(width.saturating_mul(2));
        let below = height.saturating_add(draw.pick(4));
        assert_eq!(
            grid.put(x, below, width, "abc表", Style::PLAIN),
            x,
            "seed {seed}"
        );
        assert_eq!(grid, before, "seed {seed}: a row past the grid wrote");
        let row = draw.pick(height);
        let end = grid.put(
            0,
            row,
            usize::MAX,
            &"z".repeat(width.saturating_add(5)),
            Style::PLAIN,
        );
        assert_eq!(end, width, "seed {seed}: cut at the right edge");
        assert_eq!(grid.row_text(row), "z".repeat(width));
    }
}

/// The text an app's frame paints, row by row.
pub(super) fn shown(app: &App) -> String {
    let frame = app.draw();
    (0..frame.buffer.height())
        .map(|row| frame.buffer.row_text(row))
        .collect::<Vec<String>>()
        .join("\n")
}

/// The question's own bounds (larry#325): on every screen a person can answer on, a command of
/// every height still paints the title and its first row, and the middle a crop took is one
/// marker row that counts it exactly, so nobody reads a command from its tail.
#[test]
fn sim_tui_asking_never_crops_the_question_or_the_first_command_row() {
    for width in [120, 80] {
        for height in 2..=64 {
            for rows in 1_usize..=64 {
                let command = (1..=rows)
                    .map(|row| format!("cmd {row}"))
                    .collect::<Vec<String>>()
                    .join("\n");
                let mut app = App::new(header(), Vec::new(), width, height);
                app.handle(Input::Key(Key::Char('x')));
                app.handle(Input::Key(Key::Enter));
                app.handle(Input::Worker(Event::Ask(command)));
                let screen = shown(&app);
                assert!(
                    screen.contains("Run this command?"),
                    "{width}x{height}, {rows} rows: {screen:?}"
                );
                assert!(
                    screen.contains("$ cmd 1"),
                    "{width}x{height}, {rows} rows: {screen:?}"
                );
                let painted = screen.lines().filter(|row| row.contains("cmd ")).count();
                let cut = screen
                    .lines()
                    .find(|row| row.contains(" cut …"))
                    .and_then(|row| row.split_whitespace().nth(1))
                    .and_then(|count| count.parse::<usize>().ok());
                let fits = rows.saturating_add(3) <= height;
                assert_eq!(
                    cut.is_some(),
                    !fits && height >= 3,
                    "{width}x{height}, {rows} rows: the cut is named once: {screen:?}"
                );
                if height >= 5 {
                    let shown = match cut {
                        Some(count) => painted.saturating_add(count),
                        None => painted,
                    };
                    assert_eq!(shown, rows, "{width}x{height}, {rows} rows: {screen:?}");
                }
                if height >= 5 {
                    assert!(
                        screen.contains("› No"),
                        "{width}x{height}, {rows} rows: the answer starts at No: {screen:?}"
                    );
                }
            }
        }
    }
}

/// A key read before the frame that paints the question answers nothing and is counted, so a
/// key typed while the model worked can never consent to a command nobody read; once a frame
/// has painted the question, Enter alone confirms the highlighted answer, which is No
/// (larry#325).
#[test]
fn sim_tui_a_key_before_the_questions_frame_is_discarded() {
    let mut app = App::new(header(), Vec::new(), 100, 30);
    app.handle(Input::Key(Key::Char('x')));
    app.handle(Input::Key(Key::Enter));
    app.handle(Input::Worker(Event::Ask(String::from("rm -rf /"))));
    let early = app.handle(Input::Key(Key::Char('y')));
    assert!(
        early.is_empty(),
        "a y read before the question decided {early:?}"
    );
    let painted = shown(&app);
    assert!(
        painted.contains("ignored (1 so far)"),
        "the drop is counted: {painted:?}"
    );
    let answer = app.handle(Input::Key(Key::Enter));
    assert_eq!(
        answer,
        vec![super::app::Effect::Decide(Decision::Decline)],
        "Enter alone runs nothing"
    );
}
