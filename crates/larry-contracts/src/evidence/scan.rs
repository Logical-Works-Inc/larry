//! The content scan: defence in depth behind the type boundary.
//!
//! Plain, deterministic text matching over the item lowercased (no regex). It looks for structured
//! markers a grader, rubric or scorer leaves behind, and it fails closed: a hit refuses the whole
//! context. It can over-refuse (a log that prints `"passed": 3` is refused) and it cannot prove a
//! text is clean, which is why the provenance type, not this scan, is the boundary.
//!
//! - rubric: a line starting `rubric` (after list or quote marks), or a JSON key `"rubric"`;
//! - grader verdict: a JSON key `"verdict"`, `"passed"`, `"reward"`, `"graded"`, `"grader"` or
//!   `"resolved"`, or a line starting with one of those words then `:` or `=`;
//! - score: a JSON key `"score"`, or `score` followed by `:`, `=` or spaces and then a digit
//!   (`score: 7/10`, `Score = 3`).

use super::LeakKind;

/// Words that, as a JSON key or a line start, mark a grader's verdict.
const VERDICT_WORDS: [&str; 6] = [
    "verdict", "passed", "reward", "graded", "grader", "resolved",
];

/// The first leak `bytes` shows, in the order rubric, verdict, score.
pub(super) fn scan(bytes: &[u8]) -> Option<LeakKind> {
    let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    if json_key(&text, "rubric") || text.lines().any(rubric_line) {
        return Some(LeakKind::RubricLine);
    }
    if VERDICT_WORDS.iter().any(|word| json_key(&text, word)) || text.lines().any(verdict_line) {
        return Some(LeakKind::GraderVerdict);
    }
    if json_key(&text, "score") || text.lines().any(score_line) {
        return Some(LeakKind::Score);
    }
    None
}

/// Whether `"key"` appears followed (after whitespace) by a colon.
fn json_key(text: &str, key: &str) -> bool {
    let needle = format!("\"{key}\"");
    text.match_indices(&needle).any(|(start, found)| {
        text.get(start.saturating_add(found.len())..)
            .is_some_and(|rest| rest.trim_start().starts_with(':'))
    })
}

/// A line with its leading whitespace and list or quote marks removed.
fn body(line: &str) -> &str {
    line.trim_start_matches(|ch: char| ch.is_whitespace() || "-*>#".contains(ch))
}

/// Whether the line starts with the word `rubric`.
fn rubric_line(line: &str) -> bool {
    body(line)
        .strip_prefix("rubric")
        .is_some_and(|rest| !rest.starts_with(|ch: char| ch.is_ascii_alphanumeric()))
}

/// Whether the line starts with a verdict word and then `:` or `=`.
fn verdict_line(line: &str) -> bool {
    let line = body(line);
    VERDICT_WORDS.iter().any(|word| {
        line.strip_prefix(word).is_some_and(|rest| {
            let rest = rest.trim_start();
            rest.starts_with(':') || rest.starts_with('=')
        })
    })
}

/// Whether the line has `score` followed by a separator and a digit.
fn score_line(line: &str) -> bool {
    line.match_indices("score").any(|(start, found)| {
        line.get(start.saturating_add(found.len())..)
            .is_some_and(|rest| {
                rest.trim_start_matches([' ', ':', '='])
                    .starts_with(|ch: char| ch.is_ascii_digit())
                    && rest.starts_with([' ', ':', '='])
            })
    })
}
