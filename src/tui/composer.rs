//! The text box at the bottom of the chat column (larry#239): readline keys,
//! history recall, bracketed paste and the slash popup, as in the Codex and
//! Claude Code composers. Its text is the task's instruction, so it holds at
//! most [`MAX_INSTRUCTION_BYTES`]; a paste past that is cut, and the person is
//! told.

use std::collections::VecDeque;

use super::style::{Line, Span, Style, char_width, sanitize};
use crate::executor::MAX_INSTRUCTION_BYTES;

/// Earlier submissions Up and Down recall.
pub(crate) const HISTORY: usize = 64;
/// Most rows the composer grows to; the cursor's row stays in view.
pub(crate) const MAX_ROWS: usize = 6;

/// One slash command: its name and what it does.
pub(crate) const COMMANDS: [(&str, &str); 6] = [
    ("/help", "keys and commands"),
    ("/status", "model, policy and folder"),
    ("/clear", "clear the chat"),
    ("/ps", "list background commands"),
    ("/stop", "end background commands"),
    ("/quit", "leave Larry"),
];

/// The composer's text, cursor, history and popup selection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Composer {
    /// The text.
    text: String,
    /// The cursor, a byte offset on a character boundary.
    cursor: usize,
    /// Earlier submissions, oldest first.
    history: VecDeque<String>,
    /// Which history entry Up last recalled.
    recall: Option<usize>,
    /// The popup's selected row.
    selected: usize,
}

impl Composer {
    /// The text.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Whether there is no text.
    pub(crate) fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The text changed: recall and the popup start over.
    fn edited(&mut self) {
        self.recall = None;
        self.selected = 0;
    }

    /// Inserts `text` at the cursor, sanitized, newlines kept; returns false
    /// when it was cut to keep the bound.
    pub(crate) fn insert(&mut self, text: &str) -> bool {
        let clean = sanitize(text, true);
        let room = MAX_INSTRUCTION_BYTES.saturating_sub(self.text.len());
        let mut end = clean.len().min(room);
        while !clean.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        let Some(kept) = clean.get(..end) else {
            return false;
        };
        self.text.insert_str(self.cursor, kept);
        self.cursor = self.cursor.saturating_add(kept.len());
        self.edited();
        end == clean.len()
    }

    /// The byte offset of the character before the cursor.
    fn before(&self) -> Option<usize> {
        self.text
            .get(..self.cursor)
            .and_then(|head| head.char_indices().next_back())
            .map(|(at, _)| at)
    }

    /// The byte offset just past the character after the cursor.
    fn after(&self) -> Option<usize> {
        let tail = self.text.get(self.cursor..)?;
        let first = tail.chars().next()?;
        Some(self.cursor.saturating_add(first.len_utf8()))
    }

    /// Deletes the character before the cursor.
    pub(crate) fn backspace(&mut self) {
        if let Some(at) = self.before() {
            self.text.replace_range(at..self.cursor, "");
            self.cursor = at;
            self.edited();
        }
    }

    /// Deletes the character after the cursor.
    pub(crate) fn delete(&mut self) {
        if let Some(end) = self.after() {
            self.text.replace_range(self.cursor..end, "");
            self.edited();
        }
    }

    /// Moves the cursor one character left.
    pub(crate) fn left(&mut self) {
        if let Some(at) = self.before() {
            self.cursor = at;
        }
    }

    /// Moves the cursor one character right.
    pub(crate) fn right(&mut self) {
        if let Some(end) = self.after() {
            self.cursor = end;
        }
    }

    /// The start of the cursor's line.
    fn line_start(&self) -> usize {
        match self
            .text
            .get(..self.cursor)
            .and_then(|head| head.rfind('\n'))
        {
            Some(at) => at.saturating_add(1),
            None => 0,
        }
    }

    /// The end of the cursor's line.
    fn line_end(&self) -> usize {
        match self
            .text
            .get(self.cursor..)
            .and_then(|tail| tail.find('\n'))
        {
            Some(at) => self.cursor.saturating_add(at),
            None => self.text.len(),
        }
    }

    /// Moves the cursor to the start of its line.
    pub(crate) fn home(&mut self) {
        self.cursor = self.line_start();
    }

    /// Moves the cursor to the end of its line.
    pub(crate) fn end(&mut self) {
        self.cursor = self.line_end();
    }

    /// Deletes from the start of the line to the cursor.
    pub(crate) fn kill_start(&mut self) {
        let start = self.line_start();
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.edited();
    }

    /// Deletes from the cursor to the end of the line.
    pub(crate) fn kill_end(&mut self) {
        let end = self.line_end();
        self.text.replace_range(self.cursor..end, "");
        self.edited();
    }

    /// Deletes the word before the cursor and the spaces after it.
    pub(crate) fn kill_word(&mut self) {
        let Some(head) = self.text.get(..self.cursor) else {
            return;
        };
        let trimmed = head.trim_end_matches(' ');
        let start = match trimmed.rfind([' ', '\n']) {
            Some(at) => at.saturating_add(1),
            None => 0,
        };
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.edited();
    }

    /// Clears the text.
    pub(crate) fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.edited();
    }

    /// Takes the text out for submission, keeping it in the history.
    pub(crate) fn take(&mut self) -> String {
        let text = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.edited();
        if !text.trim().is_empty() && self.history.back() != Some(&text) {
            if self.history.len() >= HISTORY {
                self.history.pop_front();
            }
            self.history.push_back(text.clone());
        }
        text
    }

    /// Up: the popup's previous row, else the previous history entry when the
    /// cursor is on the first line.
    pub(crate) fn up(&mut self) {
        if self.popup().is_some() {
            self.selected = self.selected.saturating_sub(1);
            return;
        }
        if self.line_start() != 0 {
            return;
        }
        let next = match self.recall {
            Some(at) => at.checked_sub(1),
            None => self.history.len().checked_sub(1),
        };
        if let Some(at) = next
            && let Some(entry) = self.history.get(at)
        {
            self.text.clone_from(entry);
            self.cursor = self.text.len();
            self.recall = Some(at);
        }
    }

    /// Down: the popup's next row, else the next history entry, else an empty box.
    pub(crate) fn down(&mut self) {
        if let Some(rows) = self.popup() {
            self.selected = self
                .selected
                .saturating_add(1)
                .min(rows.len().saturating_sub(1));
            return;
        }
        let Some(at) = self.recall else { return };
        let next = at.saturating_add(1);
        match self.history.get(next) {
            Some(entry) => {
                self.text.clone_from(entry);
                self.recall = Some(next);
            }
            None => {
                self.text.clear();
                self.recall = None;
            }
        }
        self.cursor = self.text.len();
    }

    /// The slash commands matching the text, while it is one word starting `/`.
    pub(crate) fn popup(&self) -> Option<Vec<(&'static str, &'static str)>> {
        let word = self.text.as_str();
        if !word.starts_with('/') || word.contains(char::is_whitespace) {
            return None;
        }
        let rows: Vec<_> = COMMANDS
            .iter()
            .copied()
            .filter(|&(name, _)| name.starts_with(word))
            .collect();
        (!rows.is_empty()).then_some(rows)
    }

    /// The popup's selected command.
    pub(crate) fn selected(&self) -> Option<&'static str> {
        self.popup()?.get(self.selected).map(|&(name, _)| name)
    }

    /// The popup row index that is selected.
    pub(crate) const fn selected_row(&self) -> usize {
        self.selected
    }

    /// Tab: completes the selected command.
    pub(crate) fn complete(&mut self) {
        if let Some(name) = self.selected() {
            self.text = name.to_owned();
            self.cursor = self.text.len();
        }
    }

    /// The composer's rows at `width` cells, `› ` before the first and two
    /// spaces before the rest, at most [`MAX_ROWS`] with the cursor's row in
    /// view, and the cursor's column and row among them.
    pub(crate) fn render(&self, width: usize, placeholder: &str) -> (Vec<Line>, (usize, usize)) {
        let room = width.saturating_sub(2).max(1);
        let mut rows: Vec<String> = vec![String::new()];
        let mut used = 0_usize;
        let mut cursor = (2_usize, 0_usize);
        for (at, character) in self.text.char_indices() {
            if at == self.cursor {
                cursor = (used.saturating_add(2), rows.len().saturating_sub(1));
            }
            let cells = char_width(character);
            if character == '\n' || used.saturating_add(cells) > room {
                rows.push(String::new());
                used = 0;
            }
            if character != '\n' {
                if let Some(row) = rows.last_mut() {
                    row.push(character);
                }
                used = used.saturating_add(cells);
            }
        }
        if self.cursor >= self.text.len() {
            if used >= room {
                rows.push(String::new());
                used = 0;
            }
            cursor = (used.saturating_add(2), rows.len().saturating_sub(1));
        }
        let skip = cursor.1.saturating_add(1).saturating_sub(MAX_ROWS);
        let lines = rows
            .iter()
            .enumerate()
            .skip(skip)
            .take(MAX_ROWS)
            .map(|(index, row)| {
                let lead = if index == 0 {
                    Span::new("› ", Style::ACCENT.bold())
                } else {
                    Span::new("  ", Style::PLAIN)
                };
                if self.text.is_empty() {
                    vec![lead, Span::new(placeholder, Style::MUTED)]
                } else {
                    vec![lead, Span::new(row.as_str(), Style::PLAIN)]
                }
            })
            .collect();
        (lines, (cursor.0, cursor.1.saturating_sub(skip)))
    }
}
