//! The frame's paint (larry#239): every transcript row, the status row, the
//! bottom block and one painted frame from the state [`View`] reads. The
//! update (keys, pastes, sizes, ticks and the session's events in
//! [`super::app`]) and the render stay two jobs: this module never changes the
//! frame, it only reads it through shared refs.

use std::cell::Cell;
use std::collections::VecDeque;
use std::time::Duration;

use super::app::Phase;
use super::ask::{Reading, command_rows, lay, marked, offer};
use super::buffer::{Buffer, Frame};
use super::cells::Entry;
use super::composer::Composer;
use super::layout::split;
use super::pane::Pane;
use super::pending::{Pending, first_line};
use super::style::{Line, Span, Style, ellipsize, sanitize_line, text_width};
use super::worker::Decision;

/// The spinner's frames, one per 100 ms.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// What the paint reads: shared refs to the frame's state, built by
/// [`super::app::App::view`]. Every method below reads only these, so a paint
/// can never change what it shows.
pub(crate) struct View<'frame> {
    /// The transcript, oldest first.
    pub entries: &'frame VecDeque<Entry>,
    /// Where the machine is.
    pub phase: &'frame Phase,
    /// The text box.
    pub composer: &'frame Composer,
    /// The artifact pane.
    pub pane: &'frame Pane,
    /// Turns queued while a session runs.
    pub pending: &'frame Pending,
    /// Transcript rows scrolled up from the bottom.
    pub scroll: usize,
    /// Columns.
    pub width: usize,
    /// Rows.
    pub height: usize,
    /// When the running task started.
    pub started: Duration,
    /// The latest tick.
    pub now: Duration,
    /// A one-line hint over the footer until the next key.
    pub hint: &'frame Option<String>,
    /// Frames painted since the frame started (larry#325).
    pub frames: &'frame Cell<u64>,
    /// The frame that painted the question now asked, `None` until one carries it.
    pub painted: &'frame Cell<Option<u64>>,
    /// What of the command asked about has been on screen; the paint writes it.
    pub reading: &'frame Cell<Reading>,
}

/// `elapsed` the way Codex's status row shows it: `12s`, `1m 05s`, `1h 02m`.
pub(crate) fn compact(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    match (seconds.checked_div(60), seconds.checked_rem(60)) {
        (Some(0) | None, _) => format!("{seconds}s"),
        (Some(minutes), Some(rest)) if minutes < 60 => format!("{minutes}m {rest:02}s"),
        (Some(minutes), _) => match (minutes.checked_div(60), minutes.checked_rem(60)) {
            (Some(hours), Some(left)) => format!("{hours}h {left:02}m"),
            _ => format!("{minutes}m"),
        },
    }
}

/// Footer hints joined by ` · ` to fit `width`; a hint that does not fit is
/// dropped whole and `…` marks the cut (Codex `footer_hint.rs`).
pub(crate) fn hints(items: &[&str], width: usize) -> Line {
    let mut text = String::new();
    for (index, item) in items.iter().enumerate() {
        let sep = if index == 0 { "" } else { " · " };
        let more = index.saturating_add(1) < items.len();
        let next = text_width(&text)
            .saturating_add(text_width(sep))
            .saturating_add(text_width(item));
        if next.saturating_add(if more { 2 } else { 0 }) > width {
            text.push_str(if index == 0 { "…" } else { " …" });
            break;
        }
        text.push_str(sep);
        text.push_str(item);
    }
    vec![Span::new(text, Style::MUTED)]
}

impl View<'_> {
    /// Rows one Page Up or Page Down moves.
    pub(crate) fn page(&self) -> usize {
        self.height.checked_div(2).into_iter().sum::<usize>().max(1)
    }

    /// The chat column's inner width: one cell of margin each side.
    fn inner(&self) -> usize {
        split(self.width, self.height)
            .chat
            .width
            .saturating_sub(2)
            .max(1)
    }

    /// Every transcript row at the inner width, a blank row between entries.
    fn transcript_rows(&self) -> Vec<Line> {
        let width = self.inner();
        let mut rows = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if index > 0 {
                rows.push(Vec::new());
            }
            rows.extend(entry.lines(width));
        }
        rows
    }

    /// The most rows the transcript can scroll up.
    pub(crate) fn max_scroll(&self) -> usize {
        self.transcript_rows().len()
    }

    /// The status row while a session runs.
    fn status_row(&self) -> Option<Line> {
        let tick = self
            .now
            .as_millis()
            .checked_div(100)
            .and_then(|step| step.checked_rem(10))
            .and_then(|step| usize::try_from(step).ok());
        let spin = match tick.and_then(|step| SPINNER.get(step)) {
            Some(frame) => *frame,
            None => "•",
        };
        let elapsed = compact(self.now.saturating_sub(self.started));
        let running = format!(" ({elapsed} • esc to interrupt)");
        let (label, tail) = match self.phase {
            &Phase::Idle | &Phase::Asking(..) => return None,
            &Phase::Working(None) => (String::from("Working"), running),
            &Phase::Working(Some(ref command)) => {
                let first = first_line(command);
                (format!("Running {first}"), running)
            }
            &Phase::Stopping(true) => (
                String::from("Stopping"),
                String::from(" (the running command ends first)"),
            ),
            &Phase::Stopping(false) => (String::from("Stopping"), String::new()),
        };
        let room = self
            .inner()
            .saturating_sub(2)
            .saturating_sub(text_width(&tail));
        Some(vec![
            Span::new(format!("{spin} "), Style::ACCENT),
            Span::new(ellipsize(&sanitize_line(&label), room), Style::PLAIN.bold()),
            Span::new(tail, Style::MUTED),
        ])
    }

    /// The footer row: the one-line hint, else the keys that fit.
    fn foot(&self, keys: &[&str], width: usize) -> Line {
        match *self.hint {
            Some(ref hint) => vec![Span::new(ellipsize(hint, width), Style::WARN)],
            None if self.scroll > 0 => {
                let mut items = vec!["↓ pgdn newer"];
                items.extend_from_slice(keys);
                hints(&items, width)
            }
            None => hints(keys, width),
        }
    }

    /// The question in at most `height` rows; [`lay`] decides what a short screen keeps.
    fn question(
        &self,
        command: Option<&str>,
        choice: Decision,
        (width, height): (usize, usize),
    ) -> Vec<Line> {
        let (title, answers, keys) = offer(command.is_some());
        let body = match command {
            Some(command) => command_rows(command, width),
            None => Vec::new(),
        };
        let answers = answers
            .iter()
            .map(|&(decision, label)| {
                let (mark, style) = marked(decision == choice);
                Span::new(format!("{mark} {label}  "), style)
            })
            .collect();
        lay(
            self.reading,
            vec![Span::new(title, Style::WARN.bold())],
            body,
            answers,
            self.foot(keys, width),
            (width, height),
        )
    }

    /// The bottom block (status, popup or question, composer, footer) and the cursor's column
    /// and row in it.
    fn bottom(&self, width: usize, height: usize) -> (Vec<Line>, Option<(usize, usize)>) {
        // A question takes the block alone: its title and command outrank the status row and
        // the diff's one line (larry#325).
        if let &Phase::Asking(ref command, choice) = self.phase {
            return (
                self.question(command.as_deref(), choice, (width, height)),
                None,
            );
        }
        let mut rows: Vec<Line> = Vec::new();
        if let Some(status) = self.status_row() {
            rows.push(status);
        }
        // A screen too narrow for the pane keeps the change as one line (larry#244).
        if split(self.width, self.height).artifact.is_none()
            && let Some(summary) = self.pane.summary()
        {
            rows.push(vec![Span::new(ellipsize(&summary, width), Style::MUTED)]);
        }
        if let Some(popup) = self.composer.popup() {
            for (index, (name, about)) in popup.into_iter().enumerate() {
                let (mark, style) = marked(index == self.composer.selected_row());
                rows.push(vec![
                    Span::new(format!("{mark} {name}  "), style),
                    Span::new(about, Style::MUTED),
                ]);
            }
        }
        rows.push(vec![Span::new("─".repeat(width), Style::MUTED)]);
        rows.extend(self.pending.queue_lines(width));
        let (lines, (column, row)) = self.composer.render(width, "Ask Larry to do something");
        let cursor = Some((column, rows.len().saturating_add(row)));
        rows.extend(lines);
        let keys: &[&str] = if matches!(self.phase, &Phase::Idle) {
            &["enter send", "/ commands", "ctrl+j newline", "ctrl+c quit"]
        } else {
            &["enter queues", "esc interrupt", "pgup/pgdn scroll"]
        };
        rows.push(self.foot(keys, width));
        (rows, cursor)
    }

    /// The frame for the current state.
    pub(crate) fn draw(&self) -> Frame {
        if self.painted.get().is_none() && matches!(self.phase, Phase::Asking(..)) {
            self.painted.set(Some(self.frames.get()));
        }
        self.frames.set(self.frames.get().saturating_add(1));
        let mut buffer = Buffer::new(self.width, self.height);
        let regions = split(buffer.width(), buffer.height());
        let chat = regions.chat;
        let left = chat.x.saturating_add(1);
        let right = chat.right().saturating_sub(1).max(left.saturating_add(1));
        let width = right.saturating_sub(left);
        let (bottom, cursor) = self.bottom(width, chat.height);
        let bottom_rows = bottom.len().min(chat.height);
        let area = chat.height.saturating_sub(bottom_rows);
        let rows = self.transcript_rows();
        let lift = self.scroll.min(rows.len().saturating_sub(area));
        let end = rows.len().saturating_sub(lift);
        let start = end.saturating_sub(area);
        for (offset, line) in rows.get(start..end).into_iter().flatten().enumerate() {
            buffer.put_line(left, chat.y.saturating_add(offset), right, line);
        }
        let top = chat.y.saturating_add(area);
        let skip = bottom.len().saturating_sub(bottom_rows);
        for (offset, line) in bottom.iter().skip(skip).enumerate() {
            buffer.put_line(left, top.saturating_add(offset), right, line);
        }
        if let Some(pane) = regions.artifact {
            self.pane.paint(&mut buffer, pane);
        }
        let cursor = cursor.and_then(|(column, row)| {
            let y = top.saturating_add(row.checked_sub(skip)?);
            let x = left.saturating_add(column);
            (x < buffer.width() && y < buffer.height()).then_some((x, y))
        });
        Frame { buffer, cursor }
    }
}
