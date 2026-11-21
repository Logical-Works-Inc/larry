//! The question a session asks and the person answers (larry#325): its title, the answers in
//! the order ←/→ walks them, the highlighted marker, and the crop that keeps the title and the
//! first command row on screen while one counted marker row names the rows a long command lost.

use super::style::{Line, Span, Style};
use super::worker::Decision;

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

/// The row that names how many rows a crop took out of the question.
fn cut_marker(rows: usize) -> Line {
    let noun = if rows == 1 { "row" } else { "rows" };
    vec![Span::new(format!("… {rows} {noun} cut …"), Style::MUTED)]
}

/// `rows` cut to `budget` rows: the first `keep` rows and the last rows stay, and one marker
/// row between them names every row the cut took. A screen too short for the head keeps the
/// head's own first rows, so a question that lost everything else still starts at its title.
/// A plan with no `keep` is no question: its rows are left as they are.
pub(crate) fn fit(mut rows: Vec<Line>, keep: usize, budget: usize) -> Vec<Line> {
    if keep == 0 || rows.len() <= budget {
        return rows;
    }
    let keep = keep.min(rows.len()).min(budget);
    let rest = rows.split_off(keep);
    let room = budget.saturating_sub(keep);
    if room == 0 {
        return rows;
    }
    let tail = room.saturating_sub(1).min(rest.len());
    let cut = rest.len().saturating_sub(tail);
    rows.push(cut_marker(cut));
    rows.extend(rest.iter().skip(rest.len().saturating_sub(tail)).cloned());
    rows
}
