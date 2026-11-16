//! Text, tones and wrapping for the terminal frame (larry#239).
//!
//! A tone names a meaning, never a colour: the frame paints it with the
//! terminal's own 16-colour palette (ECMA-48 SGR 30-37, 39), so a light or a
//! dark terminal theme keeps its own look, as Goose's ANSI default and Codex do.
//! Every run of cells is written with a full SGR that starts from reset (SGR 0),
//! so a bold cell never loses its colour the way AppCUI's crossterm backend
//! lost it (SGR 0 sent after the colour). Widths are Unicode display widths
//! (UAX #11), so CJK and emoji take two cells and combining marks none.

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

/// Whether `character` reorders text around it (Unicode bidi controls): a
/// command shown with one could read differently from the command that runs.
const fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

/// `text` with every control and bidi character removed and each tab made
/// four spaces; newlines stay when `keep_newlines`. Model replies, command
/// output and pasted text cross this before they reach a cell, so none of them
/// can move the cursor, change the title or recolour the frame.
pub(crate) fn sanitize(text: &str, keep_newlines: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\n' if keep_newlines => out.push('\n'),
            '\t' => out.push_str("    "),
            _ if character.is_control() || is_bidi_control(character) => {}
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
