//! Text, tones and wrapping for the terminal frame (larry#239).
//!
//! A tone names a meaning, never a colour: the frame paints it with the
//! terminal's own 16-colour palette (ECMA-48 SGR 30-37, 39), so a light or a
//! dark terminal theme keeps its own look, as Goose's ANSI default and Codex do.
//! Every run of cells is written with a full SGR that starts from reset (SGR 0),
//! so a bold cell never loses its colour the way AppCUI's crossterm backend
//! lost it (SGR 0 sent after the colour). Widths are Unicode display widths
//! (UAX #11), so CJK and emoji take two cells and combining marks none.

use std::time::Duration;

use unicode_width::UnicodeWidthChar as _;

/// What a run of text means to the person reading it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Tone {
    /// Body text in the terminal's default colour.
    #[default]
    Plain,
    /// Secondary text: hints, output, labels.
    Muted,
    /// Larry's accent: the prompt mark, the selection.
    Accent,
    /// A command that passed.
    Good,
    /// A question, a declined command, a policy notice.
    Warn,
    /// A failure or a refusal.
    Bad,
}

/// How one cell is painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Style {
    /// The cell's meaning.
    tone: Tone,
    /// Whether the cell is bold.
    bold: bool,
}

impl Style {
    /// Body text.
    pub(crate) const PLAIN: Self = Self::of(Tone::Plain);
    /// Secondary text.
    pub(crate) const MUTED: Self = Self::of(Tone::Muted);
    /// The accent.
    pub(crate) const ACCENT: Self = Self::of(Tone::Accent);
    /// A pass.
    pub(crate) const GOOD: Self = Self::of(Tone::Good);
    /// A warning or a question.
    pub(crate) const WARN: Self = Self::of(Tone::Warn);
    /// A failure.
    pub(crate) const BAD: Self = Self::of(Tone::Bad);

    /// The style of `tone`, not bold.
    pub(crate) const fn of(tone: Tone) -> Self {
        Self { tone, bold: false }
    }

    /// The same style, bold.
    pub(crate) const fn bold(self) -> Self {
        Self {
            tone: self.tone,
            bold: true,
        }
    }

    /// Appends the full SGR sequence of this style to `out`: reset first, then
    /// every attribute, so no earlier run's attribute survives into this one.
    pub(crate) fn sgr(self, out: &mut Vec<u8>) {
        out.extend_from_slice(b"\x1b[0");
        if self.bold {
            out.extend_from_slice(b";1");
        }
        let colour: &[u8] = match self.tone {
            Tone::Plain => b"",
            Tone::Muted => b";2",
            Tone::Accent => b";36",
            Tone::Good => b";32",
            Tone::Warn => b";33",
            Tone::Bad => b";31",
        };
        out.extend_from_slice(colour);
        out.push(b'm');
    }
}

/// One run of text in one style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Span {
    /// The text; sanitized before it is shown.
    pub(crate) text: String,
    /// How it is painted.
    pub(crate) style: Style,
}

impl Span {
    /// A run of `text` in `style`.
    pub(crate) fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

/// One painted row of spans.
pub(crate) type Line = Vec<Span>;

/// The cells `character` takes: none for a combining mark or a control (UAX #11
/// gives them no width), two for a wide character.
pub(crate) fn char_width(character: char) -> usize {
    character.width().into_iter().sum()
}

/// The cells `text` takes.
pub(crate) fn text_width(text: &str) -> usize {
    text.chars().map(char_width).fold(0, usize::saturating_add)
}

/// The cells `line` takes.
pub(crate) fn line_width(line: &[Span]) -> usize {
    line.iter()
        .map(|span| text_width(&span.text))
        .fold(0, usize::saturating_add)
}

/// Every character that draws nothing, draws as blank or breaks a line: the union of Unicode
/// 16.0.0 `Default_Ignorable_Code_Point` (which already holds every relevant `Cf`, plus marks and
/// fillers such as U+034F, U+115F..U+1160, U+3164, U+FFA0 and the variation selectors), general
/// category `Cf`, and the `Zl`/`Zp` separators, as one sorted table of inclusive, disjoint ranges.
/// Sources: `Default_Ignorable_Code_Point` in `DerivedCoreProperties.txt` and
/// `extracted/DerivedGeneralCategory.txt` (https://www.unicode.org/Public/16.0.0/ucd/).
/// Such a character draws nothing or as blank, so a command shown with one reads differently from
/// the command that runs (CVE-2021-42574, "invisible variable" homoglyph attacks).
const INVISIBLE: [(char, char); 25] = [
    ('\u{00ad}', '\u{00ad}'),
    ('\u{034f}', '\u{034f}'),
    ('\u{0600}', '\u{0605}'),
    ('\u{061c}', '\u{061c}'),
    ('\u{06dd}', '\u{06dd}'),
    ('\u{070f}', '\u{070f}'),
    ('\u{0890}', '\u{0891}'),
    ('\u{08e2}', '\u{08e2}'),
    ('\u{115f}', '\u{1160}'),
    ('\u{17b4}', '\u{17b5}'),
    ('\u{180b}', '\u{180f}'),
    ('\u{200b}', '\u{200f}'),
    ('\u{2028}', '\u{202e}'),
    ('\u{2060}', '\u{206f}'),
    ('\u{3164}', '\u{3164}'),
    ('\u{fe00}', '\u{fe0f}'),
    ('\u{feff}', '\u{feff}'),
    ('\u{ffa0}', '\u{ffa0}'),
    ('\u{fff0}', '\u{fffb}'),
    ('\u{110bd}', '\u{110bd}'),
    ('\u{110cd}', '\u{110cd}'),
    ('\u{13430}', '\u{1343f}'),
    ('\u{1bca0}', '\u{1bca3}'),
    ('\u{1d173}', '\u{1d17a}'),
    ('\u{e0000}', '\u{e0fff}'),
];

/// Whether `character` draws nothing, draws as blank or breaks a line: default-ignorable, `Cf`,
/// `Zl` or `Zp`. The table is sorted and disjoint, so this is a binary search.
fn is_invisible(character: char) -> bool {
    let code = u32::from(character);
    INVISIBLE
        .binary_search_by(|&(start, end)| {
            let (start, end) = (u32::from(start), u32::from(end));
            if code < start {
                std::cmp::Ordering::Greater
            } else if code > end {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Pushes the visible stand-in for one dropped character: `⟨U+200B⟩`.
fn push_marker(out: &mut String, character: char) {
    out.push('⟨');
    out.push_str("U+");
    let code = u32::from(character);
    let digits: u32 = if code > 0xfffff {
        6
    } else if code > 0xffff {
        5
    } else {
        4
    };
    for index in 0..digits {
        let shift = digits
            .saturating_sub(1)
            .saturating_sub(index)
            .saturating_mul(4);
        let nibble = match code.checked_shr(shift) {
            Some(value) => value & 0xf,
            None => 0,
        };
        let digit = match char::from_digit(nibble, 16) {
            Some(digit) => digit.to_ascii_uppercase(),
            None => '0',
        };
        out.push(digit);
    }
    out.push('⟩');
}

/// `text` with every control, default-ignorable, format and separator character removed and each
/// tab made four spaces; newlines stay when `keep_newlines`. Model replies, command output and
/// pasted text cross this before they reach a cell, so none of them can move the cursor,
/// change the title, recolour the frame or hide a character from the person.
pub(crate) fn sanitize(text: &str, keep_newlines: bool) -> String {
    filtered(text, keep_newlines, false)
}

/// [`sanitize`] for one line that can never hold `\n`: the command's first
/// line, a queued turn's first line, an output line already split. It takes no
/// flag, so no dead branch is left for a mutant to flip (larry#262).
pub(crate) fn sanitize_line(text: &str) -> String {
    filtered(text, false, false)
}

/// [`sanitize`] for a command the person approves: each dropped character is shown as
/// `⟨U+XXXX⟩`, so the command on screen is the command that runs (larry#337).
pub(crate) fn sanitize_shown(text: &str) -> String {
    filtered(text, true, true)
}

/// `text` filtered for a cell: tabs become spaces, and every control or invisible character
/// (default-ignorable, `Cf`, `Zl`, `Zp`) is dropped, or shown as a marker when `mark`.
fn filtered(text: &str, keep_newlines: bool, mark: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\n' if keep_newlines => out.push('\n'),
            '\t' => out.push_str("    "),
            _ if character.is_control() || is_invisible(character) => {
                if mark {
                    push_marker(&mut out, character);
                }
            }
            _ => out.push(character),
        }
    }
    out
}

/// One word or one run of spaces, with its style.
struct Piece<'a> {
    /// The text.
    text: &'a str,
    /// Its style.
    style: Style,
    /// Whether it is spaces.
    blank: bool,
}

/// `spans` cut into words and runs of spaces.
fn pieces(spans: &[Span]) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    for span in spans {
        let mut start = 0_usize;
        let mut blank = None;
        for (at, character) in span.text.char_indices() {
            let space = character == ' ';
            if blank.is_some_and(|was| was != space) {
                if let Some(text) = span.text.get(start..at) {
                    out.push(Piece {
                        text,
                        style: span.style,
                        blank: !space,
                    });
                }
                start = at;
            }
            blank = Some(space);
        }
        if let (Some(text), Some(space)) = (span.text.get(start..), blank) {
            out.push(Piece {
                text,
                style: span.style,
                blank: space,
            });
        }
    }
    out
}

/// Pushes `text` onto `line`, merging it into the last span when the style matches.
fn push(line: &mut Line, text: &str, style: Style) {
    match line.last_mut() {
        Some(last) if last.style == style => last.text.push_str(text),
        Some(_) | None => line.push(Span::new(text, style)),
    }
}

/// `spans` word-wrapped to `width` cells. The first row starts with `first`,
/// every later row with `rest`; a word wider than a row is cut by character,
/// and spaces at a break are dropped. Newlines in a span are not breaks here:
/// callers split lines first.
pub(crate) fn wrap(spans: &[Span], width: usize, first: &[Span], rest: &[Span]) -> Vec<Line> {
    let mut rows: Vec<Line> = Vec::new();
    let mut row: Line = first.to_vec();
    let mut used = line_width(first);
    let indent = line_width(rest);
    let fresh = |rows: &mut Vec<Line>, row: &mut Line, used: &mut usize| {
        rows.push(std::mem::replace(row, rest.to_vec()));
        *used = indent;
    };
    for piece in pieces(spans) {
        let size = text_width(piece.text);
        if piece.blank {
            if used.saturating_add(size) <= width && used > indent {
                push(&mut row, piece.text, piece.style);
                used = used.saturating_add(size);
            }
            continue;
        }
        if used.saturating_add(size) > width
            && used > indent
            && size <= width.saturating_sub(indent)
        {
            fresh(&mut rows, &mut row, &mut used);
        }
        for character in piece.text.chars() {
            let cells = char_width(character);
            if used.saturating_add(cells) > width && used > indent {
                fresh(&mut rows, &mut row, &mut used);
            }
            let mut buffer = [0_u8; 4];
            push(&mut row, character.encode_utf8(&mut buffer), piece.style);
            used = used.saturating_add(cells);
        }
    }
    rows.push(row);
    for line in &mut rows {
        trim_end(line);
    }
    rows
}

/// Drops trailing spaces from `line`.
fn trim_end(line: &mut Line) {
    while let Some(last) = line.last_mut() {
        let kept = last.text.trim_end_matches(' ').len();
        last.text.truncate(kept);
        if last.text.is_empty() {
            line.pop();
        } else {
            break;
        }
    }
}

/// `text` cut to at most `width` cells, ending in `…` when it was cut.
pub(crate) fn ellipsize(text: &str, width: usize) -> String {
    if text_width(text) <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut used = 0_usize;
    let room = width.saturating_sub(1);
    for character in text.chars() {
        let cells = char_width(character);
        if used.saturating_add(cells) > room {
            break;
        }
        out.push(character);
        used = used.saturating_add(cells);
    }
    if width > 0 {
        out.push('…');
    }
    out
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

/// `said`, or `fallback` when the session said nothing.
pub(crate) fn or_said(said: Option<String>, fallback: &str) -> String {
    match said {
        Some(text) => text,
        None => fallback.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{is_invisible, push_marker, sanitize, sanitize_shown};

    /// A marker names four hex digits in the Basic Multilingual Plane and six above it, each
    /// sanitizer keeps the flags its caller asks for, and every dropped format character is
    /// named (larry#337). Sources: Unicode 16.0.0 `DerivedGeneralCategory.txt`; CVE-2021-42574.
    #[test]
    fn sim_markers_name_every_dropped_character_and_the_sanitizers_keep_their_flags() {
        let mut at_the_top = String::new();
        push_marker(&mut at_the_top, '\u{ffff}');
        assert_eq!(at_the_top, "⟨U+FFFF⟩");
        let mut above = String::new();
        push_marker(&mut above, '\u{e0041}');
        assert_eq!(above, "⟨U+E0041⟩");
        assert_eq!(sanitize("a\u{200b}b", false), "ab");
        assert_eq!(sanitize("a\nb", true), "a\nb");
        assert_eq!(sanitize_shown("a\nb"), "a\nb");
        assert_eq!(sanitize_shown("a\u{200b}b"), "a⟨U+200B⟩b");
    }

    /// The one invisible table is sorted, disjoint and complete: every code point of every range
    /// is dropped, every character this issue names is inside one, and a printable neighbour is
    /// not. Sources: Unicode 16.0.0 `Default_Ignorable_Code_Point` in `DerivedCoreProperties.txt`
    /// and `Cf`, `Zl`, `Zp` in `extracted/DerivedGeneralCategory.txt` (larry#369).
    #[test]
    fn sim_the_invisible_table_is_sorted_disjoint_and_complete() {
        for (previous, next) in super::INVISIBLE.iter().zip(super::INVISIBLE.iter().skip(1)) {
            assert!(
                previous.1 < next.0,
                "ranges overlap or are unordered at {:?}",
                next.0
            );
        }
        for &(start, end) in &super::INVISIBLE {
            assert!(start <= end, "backwards range {start:?}..{end:?}");
            for code in u32::from(start)..=u32::from(end) {
                if let Some(character) = char::from_u32(code) {
                    assert!(is_invisible(character), "{character:?} must be dropped");
                }
            }
        }
        for character in [
            '\u{034f}',
            '\u{115f}',
            '\u{1160}',
            '\u{3164}',
            '\u{ffa0}',
            '\u{fe00}',
            '\u{fe0f}',
            '\u{e0100}',
            '\u{e01ef}',
            '\u{180b}',
            '\u{180c}',
            '\u{180d}',
            '\u{180f}',
        ] {
            assert!(is_invisible(character), "{character:?} must be dropped");
        }
        for character in ['\u{0041}', '\u{ac00}'] {
            assert!(!is_invisible(character), "{character:?} must be shown");
        }
    }
}
