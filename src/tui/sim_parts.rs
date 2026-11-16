//! Seeded simulations of the frame's parts against reference models (larry#239, ADR 0045):
//! the composer's edits, the footer hints, the elapsed format, the welcome box, a command's
//! output tail and the grid's bounds. Each oracle is a model written here from the rule, never
//! the code's own output. Sources: GNU Readline's emacs bindings (C-a, C-e, C-u, C-k, C-w),
//! Codex `footer_hint.rs` and `status_indicator_widget.rs`, ECMA-48 for the grid.

use std::time::Duration;

use super::app::{compact, hints};
use super::buffer::Buffer;
use super::cells::{Entry, Header, OUTPUT_TAIL, split_observation};
use super::composer::Composer;
use super::style::{Style, text_width};

/// Seeds each simulation runs.
const SEEDS: u64 = 256;

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
            let text = pieces[draw.pick(pieces.len())];
            edit(&mut composer, op, text);
            model.apply(op, text);
            let expected: String = model.chars.iter().collect();
            assert_eq!(composer.text(), expected, "seed {seed} step {step} op {op}");
            assert_eq!(composer.is_empty(), expected.is_empty());
        }
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
                Ok(String::from(hostile[draw.pick(hostile.len())]))
            } else {
                Err(String::from(hostile[draw.pick(hostile.len())]))
            },
            directory: String::from(hostile[draw.pick(hostile.len())]),
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
        let entry = Entry::Ran {
            command: String::from("make"),
            status,
            output: parsed,
            passed,
        };
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
