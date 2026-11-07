//! The governance regular expressions, compiled once per check, matched with CPython `re`
//! semantics. Each pattern ports a Python `re` call of the checker larry#53 replaced.
//!
//! Python's `\w` is `str.isalnum()` or `_`, its `\s` is `str.isspace()`; Rust's `\w` adds the
//! combining marks and connector punctuation and drops the `No` numerals, and its `\s` lacks
//! `\x1c`..`\x1f`. Every search therefore runs over a haystack in which each character the two
//! disagree on is replaced by a same-width character Rust classifies as Python does; every
//! returned slice is cut from the original text at the same offsets.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeMap;

use lgwks_std::pattern::{PatternError, Regex};

use super::error::GovernanceError;
use super::pystr::Classes;

/// One inventory of specification.md.
pub(crate) struct Inventory {
    /// The ID prefix: `R`, `D`, `E`, `C` or `A`.
    pub(crate) prefix: &'static str,
    /// The label the errors, the counts and log.md use.
    pub(crate) label: &'static str,
    /// The fewest IDs the inventory may hold.
    pub(crate) minimum: usize,
    /// The first number of the contiguous range.
    pub(crate) start: usize,
    /// The row pattern whose group 1 is the ID.
    row: &'static str,
}

/// The five inventories, in the Python order.
pub(crate) const INVENTORIES: [Inventory; 5] = [
    Inventory {
        prefix: "R",
        label: "requirements",
        minimum: 70,
        start: 1,
        row: r"(?m)^\| (R\d{2,}) \|",
    },
    Inventory {
        prefix: "D",
        label: "decisions",
        minimum: 35,
        start: 1,
        row: r"(?m)^### (D\d{2,}) [—-]",
    },
    Inventory {
        prefix: "E",
        label: "falsifiers",
        minimum: 54,
        start: 1,
        row: r"(?m)^\| (E\d{2,}) \|",
    },
    Inventory {
        prefix: "C",
        label: "execution_cells",
        minimum: 13,
        start: 0,
        row: r"(?m)^\| (C\d{2,}) \|",
    },
    Inventory {
        prefix: "A",
        label: "axes",
        minimum: 9,
        start: 1,
        row: r"(?m)^\| (A\d+) ",
    },
];

/// A Python word character Rust does not class as one, by UTF-8 width 2, 3 and 4.
const WORD_STAND_INS: [char; 3] = ['é', 'ア', '𐐀'];
/// A Python non-word character Rust classes as a word one, by UTF-8 width 2, 3 and 4.
const NON_WORD_STAND_INS: [char; 3] = ['¡', '※', '𝄞'];

/// Distinct non-ASCII characters whose stand-in one check remembers; past it each is reclassified.
const MAX_DECIDED: usize = 4096;

/// Compiled patterns shared by every governance check in one run; one check, one thread.
pub(crate) struct Patterns {
    /// The stand-in decided for each non-ASCII character seen so far, bounded by `MAX_DECIDED`.
    decided: RefCell<BTreeMap<char, Option<char>>>,
    /// The Unicode classes `repr` reads.
    pub(crate) classes: Classes,
    /// Python's `\w` for one character: `[\p{L}\p{N}_]`.
    python_word: Regex,
    /// Rust's `\w` for one character.
    rust_word: Regex,
    /// Fence line: `^ {0,3}(`{3,}|~{3,})(.*)$`, marker in group 1, suffix in group 2.
    fence: Regex,
    /// Link destination: `\]\(([^)]+)\)`, target in group 1.
    link: Regex,
    /// ATX heading: `(?m)^#{1,6}\s+(.+?)\s*#*\s*$`, title in group 1.
    heading: Regex,
    /// What a GitHub slug drops: `[^\w\- ]` with Python's `\w`.
    slug_drop: Regex,
    /// Each inventory's row pattern, in `INVENTORIES` order.
    pub(crate) rows: Vec<Regex>,
    /// Each inventory's reference pattern `\b{P}\d+\b`, in `INVENTORIES` order.
    pub(crate) references: Vec<Regex>,
    /// Each inventory's log line `(?m)^{label}: (\d+)$`, in `INVENTORIES` order.
    pub(crate) log_lines: Vec<Regex>,
    /// A requirement row: `(?m)^\| R\d{2,} \|.*$`.
    pub(crate) requirement_row: Regex,
    /// A decision reference: `\bD\d{2,}\b`.
    pub(crate) decision: Regex,
    /// A falsifier reference: `\bE\d{2,}\b`.
    pub(crate) falsifier: Regex,
    /// An execution cell row: `(?m)^\| C\d{2,} \|.*$`.
    pub(crate) cell_row: Regex,
    /// A cell's case list: `\bC\d{2,} (?:also )?owns (E\d{2,}(?:/E\d{2,})*)`, list in group 1.
    pub(crate) owns: Regex,
    /// The crash plan: `(?m)^planned_crash_cases: (.+)$`, cases in group 1.
    pub(crate) crash_line: Regex,
    /// A falsifier row: `(?m)^\| (E\d{2,}) \| (.+)$`, id in group 1, rest in group 2.
    pub(crate) falsifier_row: Regex,
    /// A fault action word, case-insensitive.
    pub(crate) fault: Regex,
    /// A proof obligation: `\bP[1-8]\b`.
    pub(crate) proof: Regex,
    /// A tracker cell row, `re.match`ed per line: `\|\s*(C\d{2})\s*\|`.
    pub(crate) tracker_cell: Regex,
    /// A tracker falsifier row, `re.match`ed per line: `\|\s*(E\d{2})\s*\|`.
    pub(crate) tracker_falsifier: Regex,
}

impl Patterns {
    /// Compiles every pattern, or reports the first one refused.
    pub(crate) fn new() -> Result<Self, GovernanceError> {
        let mut rows = Vec::with_capacity(INVENTORIES.len());
        let mut references = Vec::with_capacity(INVENTORIES.len());
        let mut log_lines = Vec::with_capacity(INVENTORIES.len());
        for inventory in &INVENTORIES {
            rows.push(compile(inventory.row)?);
            references.push(compile(&format!(r"\b{}\d+\b", inventory.prefix))?);
            log_lines.push(compile(&format!(r"(?m)^{}: (\d+)$", inventory.label))?);
        }
        Ok(Self {
            decided: RefCell::new(BTreeMap::new()),
            classes: Classes::new()?,
            python_word: compile(r"\A[\p{L}\p{N}_]\z")?,
            rust_word: compile(r"\A\w\z")?,
            fence: compile(r"\A {0,3}(`{3,}|~{3,})(.*)\z")?,
            link: compile(r"\]\(([^)]+)\)")?,
            heading: compile(r"(?m)^#{1,6}\s+(.+?)\s*#*\s*$")?,
            slug_drop: compile(r"[^\p{L}\p{N}_\- ]")?,
            rows,
            references,
            log_lines,
            requirement_row: compile(r"(?m)^\| R\d{2,} \|.*$")?,
            decision: compile(r"\bD\d{2,}\b")?,
            falsifier: compile(r"\bE\d{2,}\b")?,
            cell_row: compile(r"(?m)^\| C\d{2,} \|.*$")?,
            owns: compile(r"\bC\d{2,} (?:also )?owns (E\d{2,}(?:/E\d{2,})*)")?,
            crash_line: compile(r"(?m)^planned_crash_cases: (.+)$")?,
            falsifier_row: compile(r"(?m)^\| (E\d{2,}) \| (.+)$")?,
            fault: compile(
                r"(?i)\b(?:crash(?:es|ed|ing)?|kill(?:s|ed|ing)?|panic(?:s|ked|king)?)\b",
            )?,
            proof: compile(r"\bP[1-8]\b")?,
            tracker_cell: compile(r"\A\|\s*(C\d{2})\s*\|")?,
            tracker_falsifier: compile(r"\A\|\s*(E\d{2})\s*\|")?,
        })
    }

    /// `text` as Python's `re` classifies it, at the same byte offsets.
    fn haystack<'text>(&self, text: &'text str) -> Haystack<'text> {
        let plain = |byte: &u8| byte.is_ascii() && !(0x1c..=0x1f).contains(byte);
        if text.bytes().all(|byte| plain(&byte))
            || !text
                .chars()
                .any(|character| self.stand_in(character).is_some())
        {
            return Haystack {
                original: text,
                mapped: Cow::Borrowed(text),
            };
        }
        let mapped: String = text
            .chars()
            .map(|character| match self.stand_in(character) {
                Some(replacement) => replacement,
                None => character,
            })
            .collect();
        Haystack {
            original: text,
            mapped: Cow::Owned(mapped),
        }
    }

    /// The same-width character Rust classifies as Python classifies `character`, when the two
    /// engines disagree on it; `None` when they agree.
    fn stand_in(&self, character: char) -> Option<char> {
        if ('\u{1c}'..='\u{1f}').contains(&character) {
            return Some('\t');
        }
        if character.is_ascii() {
            return None;
        }
        if let Some(&known) = self.decided.borrow().get(&character) {
            return known;
        }
        let mut buffer = [0_u8; 4];
        let encoded = character.encode_utf8(&mut buffer);
        let python = self.python_word.is_match(encoded);
        let table = if python {
            WORD_STAND_INS
        } else {
            NON_WORD_STAND_INS
        };
        let decided = if python == self.rust_word.is_match(encoded) {
            None
        } else {
            character
                .len_utf8()
                .checked_sub(2)
                .and_then(|width| table.get(width).copied())
        };
        let mut cache = self.decided.borrow_mut();
        if cache.len() < MAX_DECIDED {
            cache.insert(character, decided);
        }
        decided
    }

    /// Python `re.findall` for one group: group `index` of every match, in order.
    pub(crate) fn groups<'text>(
        &self,
        regex: &Regex,
        text: &'text str,
        index: usize,
    ) -> Vec<&'text str> {
        let haystack = self.haystack(text);
        regex
            .find_all(&haystack.mapped)
            .filter_map(|found| {
                let captured = regex.captures(found.text)?;
                let piece = captured.get(index).copied().flatten()?;
                haystack.original_of(piece)
            })
            .collect()
    }

    /// Group `index` of the first match, as `re.search(...).group(index)`.
    pub(crate) fn group<'text>(
        &self,
        regex: &Regex,
        text: &'text str,
        index: usize,
    ) -> Option<&'text str> {
        let haystack = self.haystack(text);
        let captured = regex.captures(&haystack.mapped)?;
        let piece = captured.get(index).copied().flatten()?;
        haystack.original_of(piece)
    }

    /// Every whole match, as `re.findall` with no group.
    pub(crate) fn matches<'text>(&self, regex: &Regex, text: &'text str) -> Vec<&'text str> {
        let haystack = self.haystack(text);
        regex
            .find_all(&haystack.mapped)
            .filter_map(|found| haystack.original.get(found.start..found.end))
            .collect()
    }

    /// Whether the pattern matches anywhere, as `re.search(...) is not None`.
    pub(crate) fn is_match(&self, regex: &Regex, text: &str) -> bool {
        regex.is_match(&self.haystack(text).mapped)
    }

    /// The marker and suffix of a fence line, or `None` when the line is not a fence.
    pub(crate) fn fence_parts<'line>(&self, line: &'line str) -> Option<(&'line str, &'line str)> {
        let marker = self.group(&self.fence, line, 1)?;
        let suffix = self.group(&self.fence, line, 2)?;
        Some((marker, suffix))
    }

    /// Every link destination in `text`, in order.
    pub(crate) fn link_targets<'text>(&self, text: &'text str) -> Vec<&'text str> {
        self.groups(&self.link, text, 1)
    }

    /// Every ATX heading title in `text`, in order.
    pub(crate) fn heading_titles<'text>(&self, text: &'text str) -> Vec<&'text str> {
        self.groups(&self.heading, text, 1)
    }

    /// `re.sub(r"[^\w\- ]", "", title.lower()).replace(" ", "-")`.
    pub(crate) fn slug(&self, title: &str) -> String {
        self.slug_drop
            .replace_all(&title.to_lowercase(), "")
            .replace(' ', "-")
    }
}

/// A haystack and its Python-classified twin, the same length byte for byte.
struct Haystack<'text> {
    /// The text the caller passed.
    original: &'text str,
    /// The text the engine searches.
    mapped: Cow<'text, str>,
}

impl<'text> Haystack<'text> {
    /// The slice of the original text at the offsets `piece` occupies inside `mapped`.
    fn original_of(&self, piece: &str) -> Option<&'text str> {
        let start = piece
            .as_ptr()
            .addr()
            .checked_sub(self.mapped.as_ptr().addr())?;
        let end = start.checked_add(piece.len())?;
        self.original.get(start..end)
    }
}

/// Compiles one pattern, or names it in the refusal.
pub(crate) fn compile(pattern: &str) -> Result<Regex, GovernanceError> {
    Regex::new(pattern).map_err(|cause: PatternError| GovernanceError::Pattern {
        pattern: String::from(pattern),
        cause,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_boundaries_follow_python_around_marks_and_numerals()
    -> Result<(), Box<dyn std::error::Error>> {
        let patterns = Patterns::new()?;
        let falsifier = &patterns.falsifier;
        assert_eq!(
            patterns.matches(falsifier, "E12\u{301} E13"),
            ["E12", "E13"],
            "a combining mark is not \\w in Python"
        );
        assert!(
            patterns.matches(falsifier, "E12\u{b2}").is_empty(),
            "a superscript two is \\w in Python"
        );
        assert_eq!(
            patterns.matches(falsifier, "‿E14"),
            ["E14"],
            "connector punctuation is not \\w in Python"
        );
        let titles = patterns.heading_titles("#\u{1f}Title\u{301}\u{1f}#");
        assert_eq!(
            titles,
            ["Title\u{301}"],
            "\\x1f is \\s in Python and the original text is returned"
        );
        Ok(())
    }

    #[test]
    fn slugs_match_github_and_python() -> Result<(), Box<dyn std::error::Error>> {
        let patterns = Patterns::new()?;
        let cases: [(&str, &str); 4] = [
            ("Self-maintenance", "self-maintenance"),
            ("Cell 11 — execution", "cell-11--execution"),
            ("`larry` (ADR 0010)!", "larry-adr-0010"),
            ("Café x\u{b2} y\u{301}", "café-x\u{b2}-y"),
        ];
        for (title, expected) in cases {
            assert_eq!(patterns.slug(title), expected, "slug of {title:?}");
        }
        Ok(())
    }
}
