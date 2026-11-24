//! The cell grid one frame paints, and the bytes that move the terminal from the
//! last frame to the next (larry#239).
//!
//! Source: ECMA-48 CUP (`CSI row;col H`), ED (`CSI 2 J`) and SGR; DEC private
//! mode 25 (cursor) and 2026 (synchronized output, the terminal-wg
//! specification Contour, kitty, WezTerm, iTerm2 and foot implement, and
//! xterm.js ignores). Only cells that changed are written, so a frame that
//! moves one spinner glyph costs a few bytes; the whole update sits inside one
//! synchronized-output pair, so a terminal that honours it never shows half a
//! frame, and one that does not still sees only the changed cells.

use super::style::{Line, Style, char_width};

/// Widest screen the grid holds, in cells; a wider terminal is painted to here.
pub(crate) const MAX_COLUMNS: usize = 1000;
/// Tallest screen the grid holds, in rows.
pub(crate) const MAX_ROWS: usize = 500;

/// A rectangle of cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Rect {
    /// Left column.
    pub(crate) x: usize,
    /// Top row.
    pub(crate) y: usize,
    /// Columns.
    pub(crate) width: usize,
    /// Rows.
    pub(crate) height: usize,
}

impl Rect {
    /// The column just past the right edge.
    pub(crate) const fn right(&self) -> usize {
        self.x.saturating_add(self.width)
    }

    /// The row just past the bottom edge.
    pub(crate) const fn bottom(&self) -> usize {
        self.y.saturating_add(self.height)
    }
}

/// One cell. A grid is built and dropped for every frame, so a cell owns no heap unless a
/// combining mark joined it (larry#415).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cell {
    /// The character the cell shows; `None` for the right half of a wide character.
    base: Option<char>,
    /// The combining marks after `base`; `None` for every other cell.
    marks: Option<Box<str>>,
    /// How it is painted.
    style: Style,
    /// Whether the symbol takes two cells.
    wide: bool,
}

impl Cell {
    /// A blank cell.
    const fn blank() -> Self {
        Self {
            base: Some(' '),
            marks: None,
            style: Style::PLAIN,
            wide: false,
        }
    }

    /// Whether this is the right half of a wide character.
    const fn is_tail(&self) -> bool {
        self.base.is_none()
    }

    /// Joins the combining mark `mark` to the character shown.
    fn join(&mut self, mark: char) {
        let mut text = match self.marks.take() {
            Some(marks) => String::from(marks),
            None => String::new(),
        };
        text.push(mark);
        self.marks = Some(text.into_boxed_str());
    }

    /// Appends what the cell shows to `out`: the character, then its marks.
    fn write_to(&self, out: &mut Vec<u8>) {
        if let Some(base) = self.base {
            let mut utf8 = [0_u8; 4];
            out.extend_from_slice(base.encode_utf8(&mut utf8).as_bytes());
        }
        if let Some(ref marks) = self.marks {
            out.extend_from_slice(marks.as_bytes());
        }
    }
}

/// The grid of one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Buffer {
    /// Columns.
    width: usize,
    /// Rows.
    height: usize,
    /// Row-major cells.
    cells: Vec<Cell>,
}

impl Buffer {
    /// A blank grid of `width` by `height`, each held to its bound.
    pub(crate) fn new(width: usize, height: usize) -> Self {
        let width = width.min(MAX_COLUMNS);
        let height = height.min(MAX_ROWS);
        Self {
            width,
            height,
            cells: vec![Cell::blank(); width.saturating_mul(height)],
        }
    }

    /// Columns.
    pub(crate) const fn width(&self) -> usize {
        self.width
    }

    /// Rows.
    pub(crate) const fn height(&self) -> usize {
        self.height
    }

    /// The index of the cell at `x`, `y`, when it is on the grid.
    fn index(&self, x: usize, y: usize) -> Option<usize> {
        (x < self.width && y < self.height).then(|| y.saturating_mul(self.width).saturating_add(x))
    }

    /// Blanks the half of a wide character that `x`, `y` does not cover, so
    /// no half character is left behind when a cell is overwritten.
    fn split_wide(&mut self, x: usize, y: usize) {
        let Some(at) = self.index(x, y) else { return };
        let (tail, wide) = match self.cells.get(at) {
            Some(cell) => (cell.is_tail(), cell.wide),
            None => return,
        };
        let partner = if tail {
            x.checked_sub(1)
        } else if wide {
            x.checked_add(1)
        } else {
            None
        };
        if let Some(other) = partner.and_then(|column| self.index(column, y))
            && let Some(cell) = self.cells.get_mut(other)
        {
            *cell = Cell::blank();
        }
    }

    /// Writes `text` on row `y` from column `x`, never at or past column
    /// `limit`, and returns the column after the last cell written. Control
    /// characters are skipped; a combining mark joins the cell before it; a
    /// wide character that does not fit is not written.
    pub(crate) fn put(
        &mut self,
        x: usize,
        y: usize,
        limit: usize,
        text: &str,
        style: Style,
    ) -> usize {
        let limit = limit.min(self.width);
        let mut column = x;
        let mut last: Option<usize> = None;
        for character in text.chars() {
            if character.is_control() {
                continue;
            }
            let cells = char_width(character);
            if cells == 0 {
                if let Some(cell) = last.and_then(|at| self.cells.get_mut(at)) {
                    cell.join(character);
                }
                continue;
            }
            if column.saturating_add(cells) > limit {
                break;
            }
            self.split_wide(column, y);
            let Some(at) = self.index(column, y) else {
                break;
            };
            if let Some(cell) = self.cells.get_mut(at) {
                *cell = Cell {
                    base: Some(character),
                    marks: None,
                    style,
                    wide: cells > 1,
                };
            }
            if cells > 1 {
                let tail = column.saturating_add(1);
                self.split_wide(tail, y);
                if let Some(cell) = self
                    .index(tail, y)
                    .and_then(|next| self.cells.get_mut(next))
                {
                    *cell = Cell {
                        base: None,
                        marks: None,
                        style,
                        wide: false,
                    };
                }
            }
            last = Some(at);
            column = column.saturating_add(cells);
        }
        column
    }

    /// Writes every span of `line` on row `y` from column `x`, never at or past
    /// `limit`; returns the column after the last cell written.
    pub(crate) fn put_line(&mut self, x: usize, y: usize, limit: usize, line: &Line) -> usize {
        line.iter().fold(x, |column, span| {
            self.put(column, y, limit, &span.text, span.style)
        })
    }

    /// How many cells own heap text: the ones a combining mark joined.
    #[cfg(test)]
    pub(crate) fn heap_cells(&self) -> usize {
        self.cells
            .iter()
            .filter(|cell| cell.marks.is_some())
            .count()
    }

    /// The text of row `y`, wide characters once; a test reads the frame with it.
    #[cfg(test)]
    pub(crate) fn row_text(&self, y: usize) -> String {
        let start = y.saturating_mul(self.width);
        let end = start.saturating_add(self.width);
        let mut text = Vec::new();
        for cell in self.cells.get(start..end).into_iter().flatten() {
            cell.write_to(&mut text);
        }
        String::from_utf8_lossy(&text).into_owned()
    }
}

/// One frame: the grid and where the text cursor sits, if it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frame {
    /// The cells.
    pub(crate) buffer: Buffer,
    /// The text cursor's column and row; `None` hides it.
    pub(crate) cursor: Option<(usize, usize)>,
}

/// Appends a CUP to `x`, `y` (0-based) to `out`.
fn cup(out: &mut Vec<u8>, x: usize, y: usize) {
    out.extend_from_slice(b"\x1b[");
    out.extend_from_slice(y.saturating_add(1).to_string().as_bytes());
    out.push(b';');
    out.extend_from_slice(x.saturating_add(1).to_string().as_bytes());
    out.push(b'H');
}

/// Appends to `out` the bytes that turn the screen showing `previous` into
/// `next`: nothing at all when no cell and not the cursor changed; every cell
/// after a clear when there is no previous frame or its size differs.
pub(crate) fn render(previous: Option<&Frame>, next: &Frame, out: &mut Vec<u8>) {
    let grid = &next.buffer;
    let base = previous
        .filter(|frame| frame.buffer.width == grid.width && frame.buffer.height == grid.height);
    let mut body: Vec<u8> = Vec::new();
    if base.is_none() {
        body.extend_from_slice(b"\x1b[0m\x1b[2J");
    }
    let mut style: Option<Style> = None;
    let mut at: Option<(usize, usize)> = None;
    for (index, cell) in grid.cells.iter().enumerate() {
        if cell.is_tail() {
            continue;
        }
        let unchanged = base.is_some_and(|frame| {
            let same = frame.buffer.cells.get(index) == Some(cell);
            let tail = index.saturating_add(1);
            same && (!cell.wide || frame.buffer.cells.get(tail) == grid.cells.get(tail))
        });
        if unchanged {
            continue;
        }
        let (x, y) = match (index.checked_rem(grid.width), index.checked_div(grid.width)) {
            (Some(x), Some(y)) => (x, y),
            _ => continue,
        };
        if at != Some((x, y)) {
            cup(&mut body, x, y);
        }
        if style != Some(cell.style) {
            cell.style.sgr(&mut body);
            style = Some(cell.style);
        }
        cell.write_to(&mut body);
        // After a wide character the next write moves explicitly: terminals
        // disagree on some widths, and a wrong guess would shift the row.
        at = (!cell.wide).then(|| (x.saturating_add(1), y));
    }
    let cursor_moved = base.is_none_or(|frame| frame.cursor != next.cursor);
    if body.is_empty() && !cursor_moved {
        return;
    }
    out.extend_from_slice(b"\x1b[?2026h\x1b[?25l");
    out.extend_from_slice(&body);
    out.extend_from_slice(b"\x1b[0m");
    if let Some((x, y)) = next.cursor {
        cup(out, x, y);
        out.extend_from_slice(b"\x1b[?25h");
    }
    out.extend_from_slice(b"\x1b[?2026l");
}
