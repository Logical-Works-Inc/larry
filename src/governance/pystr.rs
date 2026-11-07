//! CPython 3.12 string semantics the governance messages and rules depend on: `str.splitlines`,
//! `str.isspace`, `str.strip`, `str.split`, `repr(str)`, `repr(list)`, ASCII-target `casefold`,
//! and the text of a `UnicodeDecodeError` from the UTF-8 codec.

use std::str::Utf8Error;

use lgwks_std::pattern::Regex;

use super::error::GovernanceError;
use super::patterns::compile;

/// Unicode classes Python's `str` methods read and the Rust standard library does not expose.
pub(crate) struct Classes {
    /// Characters `str.isprintable` refuses: `Cc`, `Cf`, `Co`, `Cn`, `Zl`, `Zp`, `Zs` but space.
    nonprintable: Regex,
}

impl Classes {
    /// Compiles the classes, or reports the pattern refused.
    pub(crate) fn new() -> Result<Self, GovernanceError> {
        Ok(Self {
            nonprintable: compile(r"\A[\p{Cc}\p{Cf}\p{Co}\p{Cn}\p{Zl}\p{Zp}\p{Zs}]\z")?,
        })
    }

    /// Python `str.isprintable` for one non-ASCII character.
    fn printable(&self, character: char) -> bool {
        let mut buffer = [0_u8; 4];
        !self
            .nonprintable
            .is_match(character.encode_utf8(&mut buffer))
    }

    /// Python `repr(str)`: quotes chosen as CPython chooses them, and every character CPython
    /// escapes written as `\\`, `\t`, `\n`, `\r`, `\xNN`, `\uNNNN` or `\UNNNNNNNN`.
    pub(crate) fn repr(&self, text: &str) -> String {
        let quote = if text.contains('\'') && !text.contains('"') {
            '"'
        } else {
            '\''
        };
        let mut out = String::with_capacity(text.len().saturating_add(2));
        out.push(quote);
        for character in text.chars() {
            match character {
                '\\' => out.push_str("\\\\"),
                '\t' => out.push_str("\\t"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                found if found == quote => {
                    out.push('\\');
                    out.push(found);
                }
                found if found.is_ascii() && !found.is_ascii_control() => out.push(found),
                found if !found.is_ascii() && self.printable(found) => out.push(found),
                found => out.push_str(&escape(found)),
            }
        }
        out.push(quote);
        out
    }

    /// Python `repr(list[str])`: `['a', 'b']`, `[]` when empty.
    pub(crate) fn list(&self, items: &[String]) -> String {
        let parts: Vec<String> = items.iter().map(|item| self.repr(item)).collect();
        format!("[{}]", parts.join(", "))
    }
}

/// The escape CPython's `repr` writes for a character it does not print.
fn escape(character: char) -> String {
    let code = u32::from(character);
    if code <= 0xff {
        format!("\\x{code:02x}")
    } else if code <= 0xffff {
        format!("\\u{code:04x}")
    } else {
        format!("\\U{code:08x}")
    }
}

/// Python `str.isspace`: Rust's `White_Space` plus the four information separators.
pub(crate) fn is_space(character: char) -> bool {
    character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
}

/// Python `str.strip()`.
pub(crate) fn strip(text: &str) -> &str {
    text.trim_matches(is_space)
}

/// Python `str.split()` with no separator: runs of whitespace split, no empty pieces.
pub(crate) fn split(text: &str) -> Vec<&str> {
    text.split(is_space)
        .filter(|piece| !piece.is_empty())
        .collect()
}

/// Python `str.splitlines()`: breaks on `\n`, `\r`, `\r\n`, `\x0b`, `\x0c`, `\x1c`, `\x1d`,
/// `\x1e`, `\x85`, U+2028 and U+2029, with no trailing empty line and none for `""`.
pub(crate) fn splitlines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let Some((index, found)) = rest
            .char_indices()
            .find(|&(_, character)| is_line_break(character))
        else {
            lines.push(rest);
            break;
        };
        let (line, tail) = rest.split_at(index);
        lines.push(line);
        let width = if tail.starts_with("\r\n") {
            2
        } else {
            found.len_utf8()
        };
        let Some(after) = tail.get(width..) else {
            break;
        };
        rest = after;
    }
    lines
}

/// The characters `str.splitlines` breaks on.
const fn is_line_break(character: char) -> bool {
    matches!(
        character,
        '\n' | '\r'
            | '\u{0b}'
            | '\u{0c}'
            | '\u{1c}'
            | '\u{1d}'
            | '\u{1e}'
            | '\u{85}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

/// Whether `needle.casefold()` occurs in `haystack.casefold()`, for an ASCII `needle`: lowercase
/// plus every full case folding whose result is ASCII (`ß`, `ẞ`, `ſ` and the Latin ligatures).
pub(crate) fn casefold_contains(haystack: &str, needle: &str) -> bool {
    let mut folded = String::with_capacity(haystack.len());
    for character in haystack.chars() {
        match character {
            'ß' | 'ẞ' => folded.push_str("ss"),
            'ſ' => folded.push('s'),
            'ﬀ' => folded.push_str("ff"),
            'ﬁ' => folded.push_str("fi"),
            'ﬂ' => folded.push_str("fl"),
            'ﬃ' => folded.push_str("ffi"),
            'ﬄ' => folded.push_str("ffl"),
            'ﬅ' | 'ﬆ' => folded.push_str("st"),
            other => folded.extend(other.to_lowercase()),
        }
    }
    folded.contains(&needle.to_ascii_lowercase())
}

/// The text of the `UnicodeDecodeError` CPython's UTF-8 codec raises for `bytes`.
pub(crate) fn utf8_error(bytes: &[u8], error: &Utf8Error) -> String {
    let start = error.valid_up_to();
    let lead = match bytes.get(start) {
        Some(byte) => *byte,
        None => 0,
    };
    let (end, reason) = match error.error_len() {
        Some(length) => {
            let reason = if (0xc2..=0xf4).contains(&lead) {
                "invalid continuation byte"
            } else {
                "invalid start byte"
            };
            (start.saturating_add(length), reason)
        }
        None => (bytes.len(), "unexpected end of data"),
    };
    let last = end.saturating_sub(1);
    if last <= start {
        format!("'utf-8' codec can't decode byte 0x{lead:02x} in position {start}: {reason}")
    } else {
        format!("'utf-8' codec can't decode bytes in position {start}-{last}: {reason}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitlines_matches_python_on_every_break() -> Result<(), Box<dyn std::error::Error>> {
        let cases: [(&str, &[&str]); 6] = [
            ("", &[]),
            ("a", &["a"]),
            ("a\n", &["a"]),
            ("a\r\nb\rc\n\nd", &["a", "b", "c", "", "d"]),
            (
                "a\u{1c}b\u{85}c\u{2028}d\u{0b}e",
                &["a", "b", "c", "d", "e"],
            ),
            ("\n\n", &["", ""]),
        ];
        for (text, expected) in cases {
            assert_eq!(
                splitlines(text),
                expected,
                "splitlines of {text:?} must match Python"
            );
        }
        Ok(())
    }

    #[test]
    fn repr_matches_python_quotes_and_escapes() -> Result<(), Box<dyn std::error::Error>> {
        let classes = Classes::new()?;
        let cases: [(&str, &str); 7] = [
            ("plain", "'plain'"),
            ("it's", "\"it's\""),
            ("both ' and \"", "'both \\' and \"'"),
            ("tab\there\\", "'tab\\there\\\\'"),
            ("\u{0}\u{7f}\u{a0}", "'\\x00\\x7f\\xa0'"),
            ("é—\u{200b}", "'é—\\u200b'"),
            ("\u{e0001}", "'\\U000e0001'"),
        ];
        for (text, expected) in cases {
            assert_eq!(
                classes.repr(text),
                expected,
                "repr of {text:?} must match Python"
            );
        }
        Ok(())
    }

    #[test]
    fn utf8_errors_match_the_python_codec_text() -> Result<(), Box<dyn std::error::Error>> {
        let cases: [(&[u8], &str); 5] = [
            (
                b"\xff",
                "'utf-8' codec can't decode byte 0xff in position 0: invalid start byte",
            ),
            (
                b"ab\xe2\x28",
                "'utf-8' codec can't decode byte 0xe2 in position 2: invalid continuation byte",
            ),
            (
                b"\xe2\x82\x28",
                "'utf-8' codec can't decode bytes in position 0-1: invalid continuation byte",
            ),
            (
                b"a\xf0\x90\x80",
                "'utf-8' codec can't decode bytes in position 1-3: unexpected end of data",
            ),
            (
                b"\xe2",
                "'utf-8' codec can't decode byte 0xe2 in position 0: unexpected end of data",
            ),
        ];
        for (bytes, expected) in cases {
            let Err(error) = std::str::from_utf8(bytes) else {
                return Err("the vector must be invalid UTF-8".into());
            };
            assert_eq!(
                utf8_error(bytes, &error),
                expected,
                "decode error of {bytes:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn casefold_finds_ascii_anchors_behind_full_foldings() -> Result<(), Box<dyn std::error::Error>>
    {
        assert!(
            casefold_contains("SELF-MAINTENANCE", "Self-maintenance"),
            "upper case folds"
        );
        assert!(casefold_contains("Mooſ", "MOOS"), "long s folds to s");
        assert!(
            casefold_contains("Stra\u{df}e", "strasse"),
            "sharp s folds to ss"
        );
        assert!(
            !casefold_contains("Moo", "Nova"),
            "an absent anchor stays absent"
        );
        Ok(())
    }
}
