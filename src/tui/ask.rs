//! The question a session asks and the person answers (larry#325, larry#521): its title, the
//! command's rows, the answers in the order ←/→ walks them, and what of the command the person
//! has had on screen.
//!
//! No command is approved unread. Every character of it reaches a row (a dropped one as its
//! marker, spaces kept), a command taller than the screen is a window the arrows, `PgUp`,
//! `PgDn`, `Home` and `End` move behind one counted `… N rows cut …` row, and a yes is refused
//! until every row has been painted. A no is always taken.

use std::cell::Cell;
use std::ops::Range;

use super::app::{App, Effect, Key, Phase};
use super::cells::Entry;
use super::style::{Line, Span, Style, char_width, sanitize_shown};
use super::worker::Decision;

/// Cells the prompt (`$ `) or its indent takes before a command row.
const LEAD: usize = 2;

/// The marker and style of a row of choices, highlighted or not.
pub(crate) fn marked(on: bool) -> (&'static str, Style) {
    if on {
        ("›", Style::ACCENT.bold())
    } else {
        (" ", Style::PLAIN)
    }
}

/// What is asked: the title, the answers in the order ←/→ walks them, and the footer keys.
pub(crate) type Offer = (
    &'static str,
    &'static [(Decision, &'static str)],
    &'static [&'static str],
);

/// The question about a command that `running`, or the review of the session's change.
/// A review has no "always": the diff of the person's own folder is asked about every time.
pub(crate) fn offer(running: bool) -> Offer {
    if running {
        (
            "△ Run this command?",
            &[
                (Decision::Run, "Yes"),
                (Decision::Always, "Always"),
                (Decision::Decline, "No"),
            ],
            &["y yes", "a always", "n no", "←/→ select", "enter confirm"],
        )
    } else {
        (
            "△ Apply this change to your folder?",
            &[(Decision::Run, "Apply"), (Decision::Decline, "Discard")],
            &["y apply", "n discard", "←/→ select", "enter confirm"],
        )
    }
}

/// `command` as rows of at most `width` cells, the prompt on each of its lines. The cut is by
/// character, so no space is dropped and what is on screen is the command that runs. A
/// character no cell can hold where it falls (a combining mark with nothing before it, a wide
/// character on a one-cell row) is named like every other dropped one.
pub(crate) fn command_rows(command: &str, width: usize) -> Vec<Line> {
    let room = width.saturating_sub(LEAD).max(1);
    let mut rows = Vec::new();
    for line in sanitize_shown(command).split_terminator('\n') {
        let mut lead = Span::new("$ ", Style::MUTED);
        let mut text = String::new();
        let mut used = 0_usize;
        for character in line.chars() {
            let cells = char_width(character);
            let named = (cells == 0 && used == 0) || cells > room;
            let shown = if named {
                format!("⟨U+{:04X}⟩", u32::from(character))
            } else {
                character.to_string()
            };
            for part in shown.chars() {
                let size = char_width(part);
                if used.saturating_add(size) > room {
                    let full = std::mem::take(&mut text);
                    let prompt = std::mem::replace(&mut lead, Span::new("  ", Style::PLAIN));
                    rows.push(vec![prompt, Span::new(full, Style::PLAIN)]);
                    used = 0;
                }
                text.push(part);
                used = used.saturating_add(size);
            }
        }
        rows.push(vec![lead, Span::new(text, Style::PLAIN)]);
    }
    rows
}

/// The window on the command asked about and what of it has been on screen. The paint writes
/// it, the keys move it, and the next question starts a new one.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Reading {
    /// Columns the rows were cut to; another width numbers the rows anew.
    width: usize,
    /// Rows the command takes.
    rows: usize,
    /// Rows of it the screen holds.
    window: usize,
    /// The first row of the window.
    top: usize,
    /// Rows from the start that have been painted.
    head: usize,
    /// The first row from which every later row has been painted.
    tail: usize,
    /// Whether the screen is too short for the whole question.
    cut: bool,
    /// Whether every row has been painted; once read, a resize does not unread it.
    done: bool,
}

impl Reading {
    /// The rows no frame has shown yet, `None` once every row has been.
    pub(crate) fn unread(&self) -> Option<usize> {
        (!self.done).then(|| self.tail.saturating_sub(self.head))
    }

    /// Counts the rows `shown` as painted. Every key moves the window by at most its own
    /// height or to an end, so what was painted is a start run and an end run.
    fn mark(&mut self, width: usize, rows: usize, shown: Range<usize>) {
        if width != self.width || rows != self.rows {
            self.head = 0;
            self.tail = rows;
        }
        self.width = width;
        self.rows = rows;
        if shown.start <= self.head {
            self.head = self.head.max(shown.end);
        }
        if shown.end >= self.tail {
            self.tail = self.tail.min(shown.start);
        }
        self.done |= self.head >= self.tail;
    }

    /// Moves the window of a command taller than the screen; `false` when `key` moves nothing
    /// here, so it belongs to whoever else takes it.
    pub(crate) fn scroll(&mut self, key: Key) -> bool {
        if !self.cut || self.rows == 0 {
            return false;
        }
        let last = self.rows.saturating_sub(self.window);
        let top = match key {
            Key::Up => self.top.saturating_sub(1),
            Key::Down => self.top.saturating_add(1),
            Key::PageUp => self.top.saturating_sub(self.window),
            Key::PageDown => self.top.saturating_add(self.window),
            Key::Home => 0,
            Key::End => last,
            _ => return false,
        };
        self.top = top.min(last);
        true
    }
}

/// The one row that stands for what the screen cut: how many rows, and for a command how many
/// of them are still unread and which keys reach them.
fn cut_marker(gone: usize, reading: &Reading) -> Line {
    let noun = if gone == 1 { "row" } else { "rows" };
    let rest = match reading.unread() {
        _ if reading.rows == 0 => String::new(),
        Some(unread) => format!(" {unread} unread · pgdn ↓ end"),
        None => String::from(" all read · pgup ↑ home"),
    };
    vec![Span::new(
        format!("… {gone} {noun} cut …{rest}"),
        Style::MUTED,
    )]
}

/// The question in `budget` rows of `width` cells: all of it when it fits. Otherwise the title,
/// a window of the command, one counted row for what was cut, and the answers and keys that
/// still fit, in that order of keeping. `reading` learns what this frame shows of the command.
pub(crate) fn lay(
    reading: &Cell<Reading>,
    title: Line,
    body: Vec<Line>,
    answers: Line,
    foot: Line,
    (width, budget): (usize, usize),
) -> Vec<Line> {
    let mut state = reading.get();
    let count = body.len();
    let total = count.saturating_add(3);
    state.cut = total > budget;
    state.window = if state.cut {
        budget.saturating_sub(4).max(1).min(count)
    } else {
        count
    };
    state.top = state.top.min(count.saturating_sub(state.window));
    let mut rows = vec![title];
    rows.extend(body.into_iter().skip(state.top).take(state.window));
    // A row counts as read only when this frame has the columns and the line to paint it.
    if width > LEAD && (budget >= 2 || count == 0) {
        let end = state.top.saturating_add(state.window);
        state.mark(width, count, state.top..end);
    } else {
        state.rows = count;
    }
    if !state.cut {
        rows.extend([answers, foot]);
    } else if rows.len() < budget {
        let kept = budget.saturating_sub(rows.len()).saturating_sub(1).min(2);
        let gone = total.saturating_sub(rows.len()).saturating_sub(kept);
        rows.push(cut_marker(gone, &state));
        rows.extend(
            [answers, foot]
                .into_iter()
                .skip(2_usize.saturating_sub(kept)),
        );
    }
    rows.truncate(budget);
    reading.set(state);
    rows
}

/// What a yes to a command not yet read gets instead.
fn refusal(unread: usize) -> String {
    match unread {
        0 => String::from("The screen is too small to show it. n declines."),
        1 => String::from("1 row unread. PgDn first; n declines."),
        more => format!("{more} rows unread. PgDn first; n declines."),
    }
}

/// A key while the session asks about `command`, `choice` highlighted.
/// The same keys answer the review of a session's change (no `command`), which has no
/// "always": its `a` falls to the ignored keys.
pub(super) fn answer(
    app: &mut App,
    key: Key,
    command: Option<String>,
    choice: Decision,
) -> Vec<Effect> {
    let decision = match key {
        Key::Char('y' | 'Y') => Decision::Run,
        Key::Char('a' | 'A') if command.is_some() => Decision::Always,
        Key::Char('n' | 'N') | Key::Esc | Key::Interrupt => Decision::Decline,
        Key::Enter => choice,
        Key::Left | Key::Right => {
            let order = offer(command.is_some()).1;
            let at = order.iter().position(|&(item, _)| item == choice);
            let next = match (key, at) {
                (Key::Left, Some(index)) => index.checked_sub(1),
                (_, Some(index)) => index.checked_add(1),
                (_, None) => None,
            };
            if let Some(&(picked, _)) = next.and_then(|index| order.get(index)) {
                app.phase = Phase::Asking(command, picked);
            }
            return Vec::new();
        }
        Key::Char(_)
        | Key::NewLine
        | Key::Backspace
        | Key::Delete
        | Key::Up
        | Key::Down
        | Key::Home
        | Key::End
        | Key::PageUp
        | Key::PageDown
        | Key::Tab
        | Key::EndOfInput
        | Key::KillStart
        | Key::KillEnd
        | Key::KillWord => return Vec::new(),
    };
    if decision != Decision::Decline
        && command.is_some()
        && let Some(unread) = app.reading.get().unread()
    {
        app.hint = Some(refusal(unread));
        return Vec::new();
    }
    match decision {
        Decision::Decline => command
            .into_iter()
            .for_each(|command| app.push(Entry::Declined(command))),
        Decision::Always => app.always = true,
        Decision::Run => {}
    }
    app.phase = Phase::Working(None);
    let mut effects = vec![Effect::Decide(decision)];
    if key == Key::Interrupt {
        app.phase = Phase::Stopping(false);
        effects.push(Effect::Interrupt);
    }
    effects
}
