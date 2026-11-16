//! The transcript's entries and the lines each paints (larry#239), in the Codex
//! TUI's vocabulary: `› ` for what the person typed, `• Ran` with an `└` output
//! tail for a command, `• ` for the model's summary, `■ ` for a notice. Every
//! text here came from a model, a command or a paste, and is sanitized before
//! it becomes a span.

use super::style::{Line, Span, Style, ellipsize, sanitize, text_width, wrap};

/// Output lines a finished command shows under it; the rest is counted.
pub(crate) const OUTPUT_TAIL: usize = 5;

/// One transcript entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Entry {
    /// The welcome box: version, model and directory.
    Header(Header),
    /// What the person asked for.
    User(String),
    /// A command that ran: its text, its status line and the output it printed.
    Ran {
        /// The command.
        command: String,
        /// `exit status 0`, `timed out…`, as the model was shown it.
        status: String,
        /// Its output lines, stdout then stderr.
        output: Vec<String>,
        /// Whether it exited 0.
        passed: bool,
    },
    /// A command the person declined; it never ran.
    Declined(String),
    /// The model's summary at the end of a task.
    Said(String),
    /// A notice: a refusal, a stop, a hint.
    Notice(Style, String),
    /// Lines a slash command printed.
    Info(Vec<String>),
}

/// What the welcome box shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Header {
    /// Larry's version.
    pub(crate) version: String,
    /// The model the next task goes to, or why there is none.
    pub(crate) model: Result<String, String>,
    /// The working directory, `~` for home.
    pub(crate) directory: String,
    /// Why tasks cannot run commands here, when they cannot.
    pub(crate) blocked: Option<String>,
}

/// The status line and the output lines of one observation: `status`, then
/// `stdout:` and its text, then `stderr:` and its text (`executor::session`).
pub(crate) fn split_observation(observation: &str) -> (String, Vec<String>) {
    let clean = sanitize(observation, true);
    let (status, rest) = match clean.split_once('\n') {
        Some((status, rest)) => (status.to_owned(), rest),
        None => (clean.clone(), ""),
    };
    let body = match rest.strip_prefix("stdout:\n") {
        Some(body) => body,
        None => rest,
    };
    let (stdout, stderr) = match (
        body.rsplit_once("\nstderr:\n"),
        body.strip_suffix("\nstderr:"),
    ) {
        (Some((out, err)), _) => (out, err),
        (None, Some(out)) => (out, ""),
        (None, None) => (body, ""),
    };
    let lines = stdout
        .lines()
        .chain(stderr.lines())
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect();
    (status, lines)
}

/// `text` split into its lines, each wrapped to `width` after `first` (row
/// one) and `rest` (every later row).
fn paragraph(text: &str, style: Style, width: usize, first: &Line, rest: &Line) -> Vec<Line> {
    let clean = sanitize(text, true);
    let mut out = Vec::new();
    for (index, line) in clean.split('\n').enumerate() {
        let lead = if index == 0 { first } else { rest };
        out.extend(wrap(&[Span::new(line, style)], width, lead, rest));
    }
    out
}

/// The welcome box, as wide as `width` allows up to 56 cells.
fn header_lines(header: &Header, width: usize) -> Vec<Line> {
    let inner = width.min(56).saturating_sub(4);
    let rule: String = "─".repeat(inner.saturating_add(2));
    // Each row holds exactly `inner` cells: a label wider than a narrow box is cut too.
    let row = |spans: Line| -> Line {
        let mut room = inner;
        let mut line = vec![Span::new("│ ", Style::MUTED)];
        for span in spans {
            let cells = text_width(&span.text);
            if cells <= room {
                room = room.saturating_sub(cells);
                line.push(span);
            } else {
                line.push(Span::new(ellipsize(&span.text, room), span.style));
                room = 0;
                break;
            }
        }
        line.push(Span::new(" ".repeat(room), Style::PLAIN));
        line.push(Span::new(" │", Style::MUTED));
        line
    };
    let fit =
        |text: &str, label: usize| ellipsize(&sanitize(text, false), inner.saturating_sub(label));
    let model = match header.model {
        Ok(ref model) => Span::new(fit(model, 11), Style::PLAIN),
        Err(ref why) => Span::new(fit(why, 11), Style::WARN),
    };
    let mut out = vec![
        vec![Span::new(format!("╭{rule}╮"), Style::MUTED)],
        row(vec![
            Span::new(">_ ", Style::ACCENT.bold()),
            Span::new("Larry", Style::PLAIN.bold()),
            Span::new(fit(&format!(" (v{})", header.version), 8), Style::MUTED),
        ]),
        row(Vec::new()),
        row(vec![Span::new("model:     ", Style::MUTED), model]),
        row(vec![
            Span::new("directory: ", Style::MUTED),
            Span::new(fit(&header.directory, 11), Style::PLAIN),
        ]),
        vec![Span::new(format!("╰{rule}╯"), Style::MUTED)],
    ];
    if let Some(ref blocked) = header.blocked {
        let mark = vec![Span::new("■ ", Style::WARN)];
        let indent = vec![Span::new("  ", Style::PLAIN)];
        out.extend(paragraph(blocked, Style::WARN, width, &mark, &indent));
    }
    out
}

impl Entry {
    /// The lines this entry paints in a column `width` cells wide.
    pub(crate) fn lines(&self, width: usize) -> Vec<Line> {
        let indent = vec![Span::new("  ", Style::PLAIN)];
        match *self {
            Self::Header(ref header) => header_lines(header, width),
            Self::User(ref text) => {
                let mark = vec![Span::new("› ", Style::ACCENT.bold())];
                paragraph(text, Style::PLAIN, width, &mark, &indent)
            }
            Self::Ran {
                ref command,
                ref status,
                ref output,
                passed,
            } => ran_lines(command, status, output, passed, width),
            Self::Declined(ref command) => {
                let mark = vec![
                    Span::new("• ", Style::WARN),
                    Span::new("Declined ", Style::WARN.bold()),
                ];
                paragraph(command, Style::MUTED, width, &mark, &indent)
            }
            Self::Said(ref text) => {
                let mark = vec![Span::new("• ", Style::PLAIN)];
                paragraph(text, Style::PLAIN, width, &mark, &indent)
            }
            Self::Notice(style, ref text) => {
                let mark = vec![Span::new("■ ", style)];
                paragraph(text, style, width, &mark, &indent)
            }
            Self::Info(ref rows) => rows
                .iter()
                .flat_map(|row| paragraph(row, Style::MUTED, width, &indent, &indent))
                .collect(),
        }
    }
}

/// The lines of a finished command: `• Ran <command>`, then its status when it
/// failed, then the last [`OUTPUT_TAIL`] output lines, each cut to one row.
fn ran_lines(
    command: &str,
    status: &str,
    output: &[String],
    passed: bool,
    width: usize,
) -> Vec<Line> {
    let dot = if passed { Style::GOOD } else { Style::BAD };
    let mark = vec![Span::new("• ", dot), Span::new("Ran ", Style::PLAIN.bold())];
    let indent = vec![Span::new("  ", Style::PLAIN)];
    let mut out = paragraph(command, Style::PLAIN, width, &mark, &indent);
    let room = width.saturating_sub(4);
    let mut tail: Vec<Line> = Vec::new();
    if !passed {
        tail.push(vec![Span::new(ellipsize(status, room), Style::BAD)]);
    }
    let hidden = output.len().saturating_sub(OUTPUT_TAIL);
    if hidden > 0 {
        tail.push(vec![Span::new(format!("… +{hidden} lines"), Style::MUTED)]);
    }
    for line in output.iter().skip(hidden) {
        tail.push(vec![Span::new(ellipsize(line, room), Style::MUTED)]);
    }
    if tail.is_empty() {
        tail.push(vec![Span::new("(no output)", Style::MUTED)]);
    }
    for (index, mut line) in tail.into_iter().enumerate() {
        let lead = if index == 0 { "  └ " } else { "    " };
        line.insert(0, Span::new(lead, Style::MUTED));
        out.push(line);
    }
    out
}
