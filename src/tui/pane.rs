//! The artifact pane (larry#244, ADR 0047): the right side of a wide screen shows the task's
//! command output, its diff and its sealed receipt, one tab each.
//!
//! The pane is a fold over the session's events and a pure paint over a rectangle. Every
//! artifact is cut and sanitized once, when it arrives, and stored as rows with a bound:
//! [`OUTPUT_ROWS`] output rows (the oldest drops and is counted), [`DIFF_ROWS`] diff rows
//! after the file list (the rest is counted), [`FILES_LISTED`] files, [`ROW_CHARS`]
//! characters a row. A frame then paints only the rows its body shows, so its cost follows
//! the screen, never the size of the diff or the output. The pane holds no clock, no I/O and
//! no state outside itself: two panes never meet.
//!
//! Tabs cycle Output, Diff, Receipt. A command that starts selects Output, a diff that
//! arrives selects Diff (it arrives while the person is asked), a receipt selects Receipt.

use std::collections::VecDeque;

use super::buffer::{Buffer, Rect};
use super::style::{Line, Span, Style, ellipsize, sanitize};
use super::worker::Receipt;

/// Most output rows kept; the oldest goes first and is counted.
pub(crate) const OUTPUT_ROWS: usize = 512;
/// Most hunk rows kept under the file list; the rest is counted.
pub(crate) const DIFF_ROWS: usize = 4096;
/// Most files the file list names; the rest is counted.
pub(crate) const FILES_LISTED: usize = 64;
/// Most characters one row keeps.
pub(crate) const ROW_CHARS: usize = 512;
/// Rows the pane keeps for its tab line, a gap and its footer.
const CHROME: usize = 3;

/// One tab of the pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    /// The commands' output.
    Output,
    /// The change the session made.
    Diff,
    /// The sealed report's receipt.
    Receipt,
}

impl Tab {
    /// The tabs in the order Tab walks them.
    const ALL: [Self; 3] = [Self::Output, Self::Diff, Self::Receipt];

    /// The tab after this one; the last wraps to the first.
    pub(crate) const fn next(self) -> Self {
        match self {
            Self::Output => Self::Diff,
            Self::Diff => Self::Receipt,
            Self::Receipt => Self::Output,
        }
    }

    /// The tab's label.
    const fn label(self) -> &'static str {
        match self {
            Self::Output => "Output",
            Self::Diff => "Diff",
            Self::Receipt => "Receipt",
        }
    }

    /// What the tab says while it holds nothing.
    const fn empty(self) -> &'static str {
        match self {
            Self::Output => "Command output shows here.",
            Self::Diff => "The change shows here when a task ends with one.",
            Self::Receipt => "The sealed receipt shows here when a task ends.",
        }
    }
}

/// One painted row: its style and its already sanitized text.
type Row = (Style, String);

/// `text` cut to [`ROW_CHARS`] characters and made safe: no control or bidi character.
fn cut(text: &str) -> String {
    sanitize(&text.chars().take(ROW_CHARS).collect::<String>(), false)
}

/// The first line of `text`, or nothing.
fn first_line(text: &str) -> String {
    text.lines().take(1).collect()
}

/// The new name in the rest of a `diff --git a/old b/new` line.
fn name_of(rest: &str) -> &str {
    match rest.rsplit_once(" b/") {
        Some((_, name)) => name,
        None => rest,
    }
}

/// The change as rows: a total, the file list, a blank, then the hunks.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DiffView {
    /// Every row, in paint order.
    rows: Vec<Row>,
    /// Files the diff names.
    files: usize,
    /// Added lines in the whole diff.
    added: usize,
    /// Removed lines in the whole diff.
    removed: usize,
    /// Hunk rows past [`DIFF_ROWS`], counted not kept.
    hidden_rows: usize,
    /// Files past [`FILES_LISTED`], counted not listed.
    hidden_files: usize,
}

impl DiffView {
    /// Reads the unified diff `text`. A hunk state machine decides each line: before a file's
    /// first `@@` every line is header, inside a hunk the first byte says added (`+`), removed
    /// (`-`), a note (`\`) or context, so a removed line that reads `--- x` is removed, and the
    /// real `--- a/x` header is not.
    fn parse(text: &str) -> Self {
        let mut listed: Vec<(String, usize, usize)> = Vec::new();
        let (mut files, mut added, mut removed, mut hidden_rows) =
            (0_usize, 0_usize, 0_usize, 0_usize);
        let mut hunks: Vec<Row> = Vec::new();
        let mut in_hunk = false;
        let mut current: Option<usize> = None;
        for line in text.lines() {
            let style = if let Some(rest) = line.strip_prefix("diff --git ") {
                in_hunk = false;
                files = files.saturating_add(1);
                current = if listed.len() < FILES_LISTED {
                    listed.push((cut(name_of(rest)), 0, 0));
                    listed.len().checked_sub(1)
                } else {
                    None
                };
                Style::PLAIN.bold()
            } else if line.starts_with("@@") {
                in_hunk = true;
                Style::MUTED
            } else if !in_hunk {
                Style::MUTED
            } else {
                match line.as_bytes().first() {
                    Some(&b'+') => {
                        added = added.saturating_add(1);
                        if let Some(entry) = current.and_then(|at| listed.get_mut(at)) {
                            entry.1 = entry.1.saturating_add(1);
                        }
                        Style::GOOD
                    }
                    Some(&b'-') => {
                        removed = removed.saturating_add(1);
                        if let Some(entry) = current.and_then(|at| listed.get_mut(at)) {
                            entry.2 = entry.2.saturating_add(1);
                        }
                        Style::BAD
                    }
                    Some(&b'\\') => Style::MUTED,
                    Some(_) | None => Style::PLAIN,
                }
            };
            if hunks.len() < DIFF_ROWS {
                hunks.push((style, cut(line)));
            } else {
                hidden_rows = hidden_rows.saturating_add(1);
            }
        }
        let hidden_files = files.saturating_sub(listed.len());
        let mut rows: Vec<Row> = vec![(
            Style::PLAIN.bold(),
            format!("{files} files, +{added} -{removed}"),
        )];
        for (name, plus, minus) in listed {
            rows.push((Style::PLAIN, format!("  {name}  +{plus} -{minus}")));
        }
        if hidden_files > 0 {
            rows.push((Style::MUTED, format!("  … {hidden_files} more files")));
        }
        rows.push((Style::PLAIN, String::new()));
        rows.extend(hunks);
        if hidden_rows > 0 {
            rows.push((Style::MUTED, format!("… {hidden_rows} more lines")));
        }
        Self {
            rows,
            files,
            added,
            removed,
            hidden_rows,
            hidden_files,
        }
    }
}

/// The artifact pane's whole state.
#[derive(Debug, Clone)]
pub(crate) struct Pane {
    /// The tab shown.
    tab: Tab,
    /// The output rows, oldest first, at most [`OUTPUT_ROWS`].
    output: VecDeque<Row>,
    /// Output rows pushed out of the ring.
    dropped: usize,
    /// The command running now, as one row of text.
    running: Option<String>,
    /// The current live-output line, bounded before it is painted.
    stream_line: String,
    /// Whether this command produced a live stream event.
    stream_seen: bool,
    /// Live-output chunks dropped at the worker event bound.
    stream_dropped_chunks: usize,
    /// Whether the current command header is already in the output ring.
    header_pushed: bool,
    /// The change, once the session offers it.
    diff: Option<DiffView>,
    /// The sealed report's receipt, once the session ends.
    receipt: Option<Receipt>,
    /// Output rows scrolled back from the newest.
    back: usize,
    /// Diff rows scrolled down from the top.
    diff_top: usize,
    /// Receipt rows scrolled down from the top.
    receipt_top: usize,
}

impl Pane {
    /// An empty pane on its Output tab.
    pub(crate) const fn new() -> Self {
        Self {
            tab: Tab::Output,
            output: VecDeque::new(),
            dropped: 0,
            running: None,
            stream_line: String::new(),
            stream_seen: false,
            stream_dropped_chunks: 0,
            header_pushed: false,
            diff: None,
            receipt: None,
            back: 0,
            diff_top: 0,
            receipt_top: 0,
        }
    }

    /// A new task starts: every artifact of the last one is released.
    pub(crate) fn begin(&mut self) {
        *self = Self::new();
    }

    /// The rows the pane's body shows on a screen `height` rows tall.
    pub(crate) const fn body(height: usize) -> usize {
        height.saturating_sub(CHROME)
    }

    /// `command` started: its first line shows as running, on the Output tab.
    pub(crate) fn started(&mut self, command: &str) {
        self.running = Some(cut(&first_line(command)));
        self.stream_line.clear();
        self.stream_seen = false;
        self.stream_dropped_chunks = 0;
        self.header_pushed = true;
        self.push((
            Style::ACCENT.bold(),
            format!("$ {}", cut(&first_line(command))),
        ));
        self.tab = Tab::Output;
        self.back = 0;
    }

    /// Adds a bounded live-output chunk. A line may span callbacks, but its retained prefix is
    /// capped at [`ROW_CHARS`], and every completed row enters the existing output ring.
    pub(crate) fn stream(&mut self, chunk: &str) {
        self.stream_seen = true;
        self.tab = Tab::Output;
        self.back = 0;
        for part in chunk.split_inclusive('\n') {
            let complete = part.ends_with('\n');
            let body = match part.strip_suffix('\n') {
                Some(body) => body,
                None => part,
            };
            let safe = sanitize(body, false);
            let room = ROW_CHARS.saturating_sub(self.stream_line.chars().count());
            self.stream_line.extend(safe.chars().take(room));
            if complete {
                let line = std::mem::take(&mut self.stream_line);
                self.push((Style::PLAIN, line));
            }
        }
    }

    /// Records live-output chunks dropped by the worker event bound.
    pub(crate) fn stream_dropped(&mut self, chunks: usize) {
        self.stream_dropped_chunks = self.stream_dropped_chunks.saturating_add(chunks);
        self.tab = Tab::Output;
        self.back = 0;
    }

    /// The live-output drop count, for the seeded pane contract.
    #[cfg(test)]
    pub(crate) const fn stream_dropped_chunks(&self) -> usize {
        self.stream_dropped_chunks
    }

    /// Adds `row`, dropping the oldest past [`OUTPUT_ROWS`].
    fn push(&mut self, row: Row) {
        if self.output.len() >= OUTPUT_ROWS {
            self.output.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.output.push_back(row);
    }

    /// The running command ended: its header, its status when it failed and its output lines.
    pub(crate) fn finished(
        &mut self,
        command: &str,
        status: &str,
        output: &[String],
        passed: bool,
    ) {
        if !self.stream_line.is_empty() {
            let line = std::mem::take(&mut self.stream_line);
            self.push((Style::PLAIN, line));
        }
        let streamed = self.stream_seen || self.stream_dropped_chunks > 0;
        self.running = None;
        self.tab = Tab::Output;
        self.back = 0;
        if !self.header_pushed {
            self.push((
                Style::ACCENT.bold(),
                format!("$ {}", cut(&first_line(command))),
            ));
        }
        if !passed {
            self.push((Style::BAD, cut(status)));
        }
        if streamed {
            if self.stream_dropped_chunks > 0 {
                self.push((
                    Style::MUTED,
                    format!(
                        "… {} live output chunks dropped",
                        self.stream_dropped_chunks
                    ),
                ));
            }
        } else {
            // Only the newest rows can stay, so the older ones are counted, never copied.
            let skip = output.len().saturating_sub(OUTPUT_ROWS);
            self.dropped = self.dropped.saturating_add(skip);
            for line in output.iter().skip(skip) {
                self.push((Style::PLAIN, cut(line)));
            }
            if output.is_empty() && passed {
                self.push((Style::MUTED, String::from("(no output)")));
            }
        }
        self.header_pushed = false;
        self.stream_seen = false;
    }

    /// The change arrived as a unified diff: parsed once, shown from its top.
    pub(crate) fn set_diff(&mut self, text: &str) {
        self.diff = Some(DiffView::parse(text));
        self.tab = Tab::Diff;
        self.diff_top = 0;
    }

    /// The sealed receipt arrived.
    pub(crate) fn set_receipt(&mut self, receipt: Receipt) {
        self.receipt = Some(receipt);
        self.tab = Tab::Receipt;
        self.receipt_top = 0;
    }

    /// Tab: the next tab.
    pub(crate) fn next_tab(&mut self) {
        self.tab = self.tab.next();
    }

    /// The one line a narrow screen keeps in the chat, when the task made a change.
    pub(crate) fn summary(&self) -> Option<String> {
        self.diff.as_ref().map(|diff| {
            format!(
                "diff: {} files, +{} -{}",
                diff.files, diff.added, diff.removed
            )
        })
    }

    /// The receipt rows.
    fn receipt_rows(receipt: &Receipt) -> Vec<Row> {
        let (verdict, style) = if receipt.accepted {
            ("accepted", Style::GOOD)
        } else {
            ("not accepted", Style::WARN)
        };
        vec![
            (Style::PLAIN, format!("digest    {}", cut(&receipt.digest))),
            (style, format!("verdict   {verdict}")),
            (
                Style::PLAIN,
                format!(
                    "commands  {} run, {} declined, {} interrupted",
                    receipt.commands, receipt.declined, receipt.interrupted
                ),
            ),
            (Style::PLAIN, format!("change    {}", cut(&receipt.handoff))),
        ]
    }

    /// Rows `tab` holds.
    fn len(&self, tab: Tab) -> usize {
        match tab {
            Tab::Output => usize::from(self.dropped > 0)
                .saturating_add(self.output.len())
                .saturating_add(usize::from(self.running.is_some())),
            Tab::Diff => match self.diff {
                Some(ref diff) => diff.rows.len(),
                None => 0,
            },
            Tab::Receipt => usize::from(self.receipt.is_some()).saturating_mul(4),
        }
    }

    /// The `body` rows of `tab` that show, as `start..end` into its rows.
    fn window(&self, tab: Tab, body: usize) -> (usize, usize) {
        let len = self.len(tab);
        let room = len.saturating_sub(body);
        match tab {
            Tab::Output => {
                let end = len.saturating_sub(self.back.min(room));
                (end.saturating_sub(body), end)
            }
            Tab::Diff | Tab::Receipt => {
                let top = if tab == Tab::Diff {
                    self.diff_top
                } else {
                    self.receipt_top
                };
                let start = top.min(room);
                (start, start.saturating_add(body).min(len))
            }
        }
    }

    /// Page Up (`up`) or Page Down in the shown tab, on a body `body` rows tall: Page Up
    /// shows earlier rows, and no page leaves the content.
    pub(crate) fn page(&mut self, up: bool, body: usize) {
        let step = body.checked_div(2).into_iter().sum::<usize>().max(1);
        let room = self.len(self.tab).saturating_sub(body);
        match (self.tab, up) {
            (Tab::Output, true) => self.back = self.back.saturating_add(step).min(room),
            (Tab::Output, false) => self.back = self.back.saturating_sub(step),
            (Tab::Diff, true) => self.diff_top = self.diff_top.saturating_sub(step),
            (Tab::Diff, false) => self.diff_top = self.diff_top.saturating_add(step).min(room),
            (Tab::Receipt, true) => self.receipt_top = self.receipt_top.saturating_sub(step),
            (Tab::Receipt, false) => {
                self.receipt_top = self.receipt_top.saturating_add(step).min(room);
            }
        }
    }

    /// Row `index` of the shown tab.
    fn row(&self, index: usize) -> Option<Row> {
        match self.tab {
            Tab::Output => {
                let lead = usize::from(self.dropped > 0);
                if index < lead {
                    return Some((Style::MUTED, format!("… {} earlier rows", self.dropped)));
                }
                let at = index.saturating_sub(lead);
                match self.output.get(at) {
                    Some(row) => Some(row.clone()),
                    None if at == self.output.len() => self
                        .running
                        .as_ref()
                        .map(|command| (Style::MUTED, format!("… running {command}"))),
                    None => None,
                }
            }
            Tab::Diff => self.diff.as_ref()?.rows.get(index).cloned(),
            Tab::Receipt => Self::receipt_rows(self.receipt.as_ref()?)
                .get(index)
                .cloned(),
        }
    }

    /// Paints the pane into `area`: a rule on its left edge, the tab line, the shown tab's
    /// rows and a footer. Nothing is written outside `area` but the rule's own column.
    pub(crate) fn paint(&self, buffer: &mut Buffer, area: Rect) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let rule = area.x.saturating_sub(1);
        for row in area.y..area.bottom() {
            buffer.put(rule, row, area.x, "│", Style::MUTED);
        }
        let left = area.x.saturating_add(2);
        let inner = area.width.saturating_sub(4).max(1);
        let limit = area.right().min(left.saturating_add(inner));
        let mut at = left;
        for tab in Tab::ALL {
            let style = if tab == self.tab {
                Style::ACCENT.bold()
            } else {
                Style::MUTED
            };
            at = buffer.put(at, area.y, limit, tab.label(), style);
            at = buffer.put(at, area.y, limit, "  ", Style::PLAIN);
        }
        let body = Self::body(area.height);
        if body == 0 {
            return;
        }
        let first = area.y.saturating_add(2);
        if self.len(self.tab) == 0 {
            let note = vec![Span::new(ellipsize(self.tab.empty(), inner), Style::MUTED)];
            buffer.put_line(left, first, limit, &note);
        }
        let (start, end) = self.window(self.tab, body);
        for (offset, index) in (start..end).enumerate() {
            if let Some((style, text)) = self.row(index) {
                let line: Line = vec![Span::new(ellipsize(&text, inner), style)];
                buffer.put_line(left, first.saturating_add(offset), limit, &line);
            }
        }
        let foot = area.bottom().saturating_sub(1);
        let hint = vec![Span::new(
            ellipsize("tab switch · pgup/pgdn scroll", inner),
            Style::MUTED,
        )];
        buffer.put_line(left, foot, limit, &hint);
    }
}

/// Seeded simulations of the pane (larry#244).
#[cfg(test)]
#[path = "sim_pane.rs"]
mod sim;
