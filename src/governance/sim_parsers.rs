//! Seeded simulations of the CPython semantics the checker reproduces, each against an
//! independent reference written from the specification rather than from the port. Sources:
//! the CPython 3.12 `str` documentation (`splitlines`, `split`, `strip`, `isspace`), RFC 8259
//! and `_json.c` `ensure_ascii`, Python `repr(float)` (shortest round-trip), the Unicode 15
//! UTF-8 well-formed table (Table 3-7) with CPython's maximal-subpart error positions, and
//! CommonMark 0.31.2 fenced code blocks (4.5). Requirement: the port equals the reference on
//! every generated input, including where Rust's own behaviour differs from Python's.

use std::fmt::Write as _;

use lgwks_std::json::{self, Value};
use lgwks_std::seeded::Seeded;

use super::patterns::Patterns;
use super::pyjson;
use super::pystr;
use super::sim_support::{SimError, SimResult, pick};

/// Seeds each simulation runs.
const SEEDS: u64 = 64;
/// Generated inputs per seed.
const CASES: usize = 16;

/// Every character Python's `str.isspace` accepts, from the CPython 3.12 Unicode table.
const PYTHON_SPACES: [char; 29] = [
    '\t', '\n', '\u{0b}', '\u{0c}', '\r', '\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}', ' ', '\u{85}',
    '\u{a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}',
    '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}', '\u{2029}', '\u{202f}',
    '\u{205f}', '\u{3000}',
];
/// The characters Python's `str.splitlines` breaks on, `\r\n` aside.
const PYTHON_BREAKS: [char; 10] = [
    '\n', '\r', '\u{0b}', '\u{0c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}', '\u{2029}',
];
/// Characters that are neither space nor break, ASCII and not, including the zero-width space
/// Python does not count as space.
const PLAIN: [char; 8] = ['a', 'Z', '0', '_', 'é', '\u{200b}', '—', '𝄞'];

/// A seeded text of up to `max` characters drawn from `alphabet`, with `\r\n` pairs mixed in.
fn text_from(rng: &mut Seeded, alphabet: &[char], max: usize) -> Result<String, SimError> {
    let mut text = String::new();
    for _ in 0..rng.index(max.saturating_add(1))? {
        if rng.below(8)? == 0 {
            text.push_str("\r\n");
        } else {
            text.push(*pick(rng, alphabet)?);
        }
    }
    Ok(text)
}

/// The reference `splitlines`: one walk, a break ends a line, `\r\n` is one break, and the text
/// after the last break is a line only when it is not empty.
fn reference_splitlines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if PYTHON_BREAKS.contains(&character) {
            if character == '\r' && characters.peek() == Some(&'\n') {
                characters.next();
            }
            lines.push(std::mem::take(&mut line));
        } else {
            line.push(character);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The reference `split()`: runs of Python whitespace separate words; no empty word.
fn reference_split(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    for character in text.chars() {
        if PYTHON_SPACES.contains(&character) {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(character);
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// The reference `strip()`: the characters from the first to the last that is not a space.
fn reference_strip(text: &str) -> String {
    let characters: Vec<char> = text.chars().collect();
    let kept = |character: &char| !PYTHON_SPACES.contains(character);
    match (
        characters.iter().position(kept),
        characters.iter().rposition(kept),
    ) {
        (Some(first), Some(last)) => characters.get(first..=last).into_iter().flatten().collect(),
        _ => String::new(),
    }
}

/// `str.splitlines`, `str.split()` and `str.strip()` equal their references on texts mixing
/// every Python space and break with plain characters.
#[test]
fn sim_python_line_and_word_splitting_match_their_references() -> SimResult {
    let alphabet: Vec<char> = PYTHON_SPACES
        .iter()
        .chain(PYTHON_BREAKS.iter())
        .chain(PLAIN.iter())
        .copied()
        .collect();
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        for _ in 0..CASES {
            let text = text_from(&mut rng, &alphabet, 40)?;
            assert_eq!(
                pystr::splitlines(&text),
                reference_splitlines(&text),
                "seed {seed}: splitlines of {text:?}"
            );
            assert_eq!(
                pystr::split(&text),
                reference_split(&text),
                "seed {seed}: split of {text:?}"
            );
            assert_eq!(
                pystr::strip(&text),
                reference_strip(&text),
                "seed {seed}: strip of {text:?}"
            );
        }
    }
    Ok(())
}

/// The reference `ensure_ascii` string literal, escaping by code point: astral characters as a
/// computed surrogate pair, never through `encode_utf16`.
fn reference_json_string(text: &str) -> Result<String, SimError> {
    let mut out = String::from("\"");
    for character in text.chars() {
        let code = u32::from(character);
        let written = match character {
            '"' => out.write_str("\\\""),
            '\\' => out.write_str("\\\\"),
            '\n' => out.write_str("\\n"),
            '\r' => out.write_str("\\r"),
            '\t' => out.write_str("\\t"),
            '\u{08}' => out.write_str("\\b"),
            '\u{0c}' => out.write_str("\\f"),
            ' '..='~' => out.write_char(character),
            _ if code > 0xffff => {
                let offset = code.saturating_sub(0x1_0000);
                let high = 0xd800_u32.saturating_add(offset.div_euclid(1024));
                let low = 0xdc00_u32.saturating_add(offset.rem_euclid(1024));
                write!(out, "\\u{high:04x}\\u{low:04x}")
            }
            _ => write!(out, "\\u{code:04x}"),
        };
        if written.is_err() {
            return Err(SimError::Absent(String::from("room to escape")));
        }
    }
    out.push('"');
    Ok(out)
}

/// Every generated string dumps as the reference escapes it, and parses back to itself.
#[test]
fn sim_json_strings_escape_like_python_and_round_trip() -> SimResult {
    let alphabet = [
        'a', '~', ' ', '"', '\\', '\n', '\r', '\t', '\u{08}', '\u{0c}', '\u{0}', '\u{1f}',
        '\u{7f}', '<', 'é', '\u{2028}', '\u{fffd}', '😀', '𝄞',
    ];
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        for _ in 0..CASES {
            let text = text_from(&mut rng, &alphabet, 24)?;
            let dumped = pyjson::dumps(&Value::from(text.as_str()), None);
            assert_eq!(
                dumped,
                reference_json_string(&text)?,
                "seed {seed}: dump of {text:?}"
            );
            assert!(
                dumped.is_ascii(),
                "seed {seed}: ensure_ascii output is ASCII"
            );
            let back: Value = json::from_str(&dumped).map_err(SimError::Json)?;
            assert_eq!(
                back.as_str(),
                Some(text.as_str()),
                "seed {seed}: {dumped} parses back"
            );
        }
    }
    Ok(())
}

/// The significant digits of a float's text: every digit before any exponent, without the
/// leading and trailing zeros that only place the point.
fn significant(text: &str) -> String {
    let mantissa = match text.split_once(['e', 'E']) {
        Some((before, _)) => before,
        None => text,
    };
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    String::from(digits.trim_start_matches('0').trim_end_matches('0'))
}

/// Python's `repr(float)` is the shortest text that round-trips, fixed for decimal exponents in
/// `-4..16` and `d.ddde±NN` outside it: checked on finite floats drawn from raw bits.
#[test]
fn sim_float_repr_round_trips_in_the_python_layout() -> SimResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        for _ in 0..CASES {
            let float = f64::from_bits(rng.next_u64());
            if !float.is_finite() {
                continue;
            }
            let shown = pyjson::float_repr(float);
            let parsed: f64 = shown
                .parse()
                .map_err(|_| SimError::Absent(format!("a float in {shown}")))?;
            assert_eq!(
                parsed.to_bits(),
                float.to_bits(),
                "seed {seed}: {shown} round-trips"
            );
            let rust = format!("{:e}", float.abs());
            assert_eq!(
                significant(&shown),
                significant(&rust),
                "seed {seed}: {shown} keeps the shortest digits of {rust}"
            );
            let exponent: i32 = match rust.split_once('e') {
                Some((_, exponent)) => exponent
                    .parse()
                    .map_err(|_| SimError::Absent(String::from("an exponent")))?,
                None => return Err(Box::new(SimError::Absent(String::from("an exponent")))),
            };
            let fixed = (-4..16).contains(&exponent) || parsed.to_bits().trailing_zeros() >= 63;
            assert_eq!(
                !shown.contains('e'),
                fixed,
                "seed {seed}: {shown} layout for exponent {exponent}"
            );
            if fixed {
                assert!(
                    shown.contains('.'),
                    "seed {seed}: fixed {shown} always has a point"
                );
            } else {
                let tail = shown
                    .rsplit(['+', '-'])
                    .next()
                    .ok_or_else(|| SimError::Absent(String::from("a tail")))?;
                assert!(
                    tail.len() >= 2,
                    "seed {seed}: {shown} writes at least two exponent digits"
                );
            }
        }
    }
    Ok(())
}

/// The reference UTF-8 decoder of Unicode Table 3-7: the error at the first ill-formed
/// sequence, as CPython reports it (start, length of the maximal subpart, reason); `None` when
/// the bytes are well formed.
fn reference_decode_error(bytes: &[u8]) -> Option<String> {
    let mut start = 0;
    while let Some(&lead) = bytes.get(start) {
        let (need, first): (usize, std::ops::RangeInclusive<u8>) = match lead {
            0x00..=0x7f => (0, 0x80..=0xbf),
            0xc2..=0xdf => (1, 0x80..=0xbf),
            0xe0 => (2, 0xa0..=0xbf),
            0xed => (2, 0x80..=0x9f),
            0xe1..=0xef => (2, 0x80..=0xbf),
            0xf0 => (3, 0x90..=0xbf),
            0xf4 => (3, 0x80..=0x8f),
            0xf1..=0xf3 => (3, 0x80..=0xbf),
            _ => return Some(subpart_error(lead, start, 1, "invalid start byte")),
        };
        let mut taken = 1_usize;
        while taken <= need {
            let range = if taken == 1 {
                first.clone()
            } else {
                0x80..=0xbf
            };
            match bytes.get(start.saturating_add(taken)) {
                Some(byte) if range.contains(byte) => taken = taken.saturating_add(1),
                Some(_) => {
                    return Some(subpart_error(
                        lead,
                        start,
                        taken,
                        "invalid continuation byte",
                    ));
                }
                None => {
                    return Some(subpart_error(
                        lead,
                        start,
                        bytes.len().saturating_sub(start),
                        "unexpected end of data",
                    ));
                }
            }
        }
        start = start.saturating_add(taken);
    }
    None
}

/// CPython's wording for the `length` bytes at `start`, whose first byte is `lead`.
fn subpart_error(lead: u8, start: usize, length: usize, reason: &str) -> String {
    if length <= 1 {
        format!("'utf-8' codec can't decode byte 0x{lead:02x} in position {start}: {reason}")
    } else {
        let last = start.saturating_add(length).saturating_sub(1);
        format!("'utf-8' codec can't decode bytes in position {start}-{last}: {reason}")
    }
}

/// Every byte string built from well-formed, truncated and ill-formed fragments gets the
/// decode error the reference decoder gives, and well-formed ones decode.
#[test]
fn sim_utf8_errors_match_a_table_3_7_decoder() -> SimResult {
    let fragments: [&[u8]; 16] = [
        b"a",
        b"\xc3\xa9",
        b"\xe2\x80\x94",
        b"\xf0\x9d\x84\x9e",
        b"\xff",
        b"\x80",
        b"\xc0\xaf",
        b"\xe0\x80\x80",
        b"\xed\xa0\x80",
        b"\xf4\x90\x80\x80",
        b"\xe2\x82",
        b"\xf0\x9d\x84",
        b"\xc3",
        b"\xf5",
        b"\xe2\x28",
        b"\xf0\x28\x8c\x28",
    ];
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        for _ in 0..CASES {
            let mut bytes = Vec::new();
            for _ in 0..rng.index(6)?.saturating_add(1) {
                bytes.extend_from_slice(pick(&mut rng, &fragments)?);
            }
            let port = match std::str::from_utf8(&bytes) {
                Ok(_) => None,
                Err(error) => Some(pystr::utf8_error(&bytes, &error)),
            };
            assert_eq!(
                port,
                reference_decode_error(&bytes),
                "seed {seed}: decode of {bytes:x?}"
            );
        }
    }
    Ok(())
}

/// Characters a heading title is built from, each with the slug text Python keeps for it: a
/// `\w` character lowercased, `-` kept, space as `-`, anything else dropped.
const SLUG_ALPHABET: [(char, &str); 14] = [
    ('A', "a"),
    ('z', "z"),
    ('7', "7"),
    ('_', "_"),
    ('-', "-"),
    (' ', "-"),
    ('É', "é"),
    ('ア', "ア"),
    ('\u{b2}', "\u{b2}"),
    ('\u{301}', ""),
    ('—', ""),
    ('!', ""),
    ('`', ""),
    ('‿', ""),
];

/// Heading slugs equal the tagged-alphabet reference: Python's `\w`, not Rust's, decides what stays.
#[test]
fn sim_slugs_follow_the_python_word_class() -> SimResult {
    let patterns = Patterns::new()?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        for _ in 0..CASES {
            let (mut title, mut expected) = (String::new(), String::new());
            for _ in 0..rng.index(20)? {
                let &(character, kept) = pick(&mut rng, &SLUG_ALPHABET)?;
                title.push(character);
                expected.push_str(kept);
            }
            assert_eq!(
                patterns.slug(&title),
                expected,
                "seed {seed}: slug of {title:?}"
            );
        }
    }
    Ok(())
}

/// Fence lines: up to three spaces of indent and a run of three or more of one marker make a
/// fence, whose marker is the whole run and whose suffix is the rest; anything else is not.
#[test]
fn sim_fence_lines_match_the_commonmark_opener_rule() -> SimResult {
    let patterns = Patterns::new()?;
    let suffixes = ["", "mermaid", " text", "~x", "`y", "\u{1f}é"];
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        for _ in 0..CASES {
            let indent = rng.index(6)?;
            let marker = *pick(&mut rng, &['`', '~'])?;
            let run = rng.index(7)?;
            let suffix = *pick(&mut rng, &suffixes)?;
            let line = format!(
                "{}{}{suffix}",
                " ".repeat(indent),
                String::from(marker).repeat(run)
            );
            let after_indent = line
                .get(indent..)
                .ok_or_else(|| SimError::Absent(String::from("the indent")))?;
            let whole_run = after_indent
                .chars()
                .take_while(|character| *character == marker)
                .count();
            let expected =
                (indent <= 3 && whole_run >= 3).then(|| after_indent.split_at(whole_run));
            assert_eq!(
                patterns.fence_parts(&line),
                expected,
                "seed {seed}: fence parts of {line:?}"
            );
        }
    }
    Ok(())
}
