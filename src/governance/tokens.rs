//! The token reader of the temp-path rule (larry#293): Rust source reduced to names, path
//! separators, punctuation, string literals and other literals, with comments, lifetimes and char
//! literals skipped, so a rule can follow the names a program binds instead of matching text.
//!
//! Every scan is bounded by the source's own length, so a fault in the reader ends the walk
//! instead of running forever. Source: The Rust Reference on tokens and literals (string, raw
//! string, char and byte forms) and macros-by-example (macro arguments are syntax tokens).

use std::ops::Range;

/// One lexical token of Rust source, without its position; a name or a string borrows the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Token<'source> {
    /// A name: identifier, keyword or raw identifier with `r#` removed.
    Name(&'source str),
    /// The `::` path separator.
    PathSep,
    /// One punctuation character.
    Punct(char),
    /// A string literal's content, between its quotes.
    Text(&'source str),
    /// A number, a char or another literal that never holds a name the rule reads.
    Literal,
}

/// A token and the 1-based line it starts on.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Lexeme<'source> {
    /// What the token is.
    pub(crate) token: Token<'source>,
    /// The line the token starts on.
    pub(crate) line: usize,
}

/// The tokens of `text`, with comments, lifetimes and char literals skipped and every literal
/// kept as one token, so a string's content is data.
pub(crate) fn lex(text: &str) -> Vec<Lexeme<'_>> {
    let bytes = text.as_bytes();
    let mut lexemes = Vec::new();
    let mut line = 1_usize;
    let mut at = 0_usize;
    for _ in 0..bytes.len().saturating_add(1) {
        let Some(byte) = bytes.get(at).copied() else {
            break;
        };
        if byte == b'\n' {
            line = line.saturating_add(1);
            at = at.saturating_add(1);
            continue;
        }
        if byte.is_ascii_whitespace() {
            at = at.saturating_add(1);
            continue;
        }
        if let Some((content, hashes)) = raw_start(bytes, at) {
            let started = line;
            let (token, next, added) = raw_string(text, content, hashes);
            lexemes.push(Lexeme {
                token: Token::Text(token),
                line: started,
            });
            line = line.saturating_add(added);
            at = next;
            continue;
        }
        if byte == b'"' {
            let started = line;
            let (token, next, added) = plain_string(text, at.saturating_add(1));
            lexemes.push(Lexeme {
                token: Token::Text(token),
                line: started,
            });
            line = line.saturating_add(added);
            at = next;
            continue;
        }
        if (byte == b'b' || byte == b'c') && bytes.get(at.saturating_add(1)).copied() == Some(b'"')
        {
            let started = line;
            let (token, next, added) = plain_string(text, at.saturating_add(2));
            lexemes.push(Lexeme {
                token: Token::Text(token),
                line: started,
            });
            line = line.saturating_add(added);
            at = next;
            continue;
        }
        if byte == b'\'' {
            at = char_or_lifetime(bytes, at);
            continue;
        }
        if is_name_start(byte) {
            let (name, next) = name_at(text, bytes, at);
            lexemes.push(Lexeme {
                token: Token::Name(name),
                line,
            });
            at = next;
            continue;
        }
        if byte.is_ascii_digit() {
            at = number_at(bytes, at);
            lexemes.push(Lexeme {
                token: Token::Literal,
                line,
            });
            continue;
        }
        match byte {
            b'/' if bytes.get(at.saturating_add(1)).copied() == Some(b'/') => {
                at = line_comment(bytes, at);
            }
            b'/' if bytes.get(at.saturating_add(1)).copied() == Some(b'*') => {
                let (next, added) = block_comment(bytes, at);
                line = line.saturating_add(added);
                at = next;
            }
            _ => {
                let (token, next) = punctuation(byte, bytes, at);
                lexemes.push(Lexeme { token, line });
                at = next;
            }
        }
    }
    lexemes
}

/// Whether each token sits inside a macro invocation's arguments, whose token trees are code.
pub(crate) fn macro_mask(lexemes: &[Lexeme<'_>]) -> Vec<bool> {
    let mut mask = vec![false; lexemes.len()];
    let mut groups: Vec<bool> = Vec::new();
    for (index, lexeme) in lexemes.iter().enumerate() {
        if matches!(lexeme.token, Token::Punct(')' | ']' | '}')) && !groups.is_empty() {
            groups.truncate(groups.len().saturating_sub(1));
        }
        mask[index] = groups.iter().any(|macro_group| *macro_group);
        if matches!(lexeme.token, Token::Punct('(' | '[' | '{')) {
            let invocation = index > 0
                && matches!(
                    lexemes
                        .get(index.saturating_sub(1))
                        .map(|before| before.token),
                    Some(Token::Punct('!'))
                );
            groups.push(invocation);
        }
    }
    mask
}

/// The text of `range`, empty when the range leaves the buffer or splits a character: a reader
/// that walked off the end has no bytes there, and the caller's own bounds already refused the
/// case.
fn slice_of(text: &str, range: Range<usize>) -> &str {
    let inside = range.start <= range.end
        && range.end <= text.len()
        && text.is_char_boundary(range.start)
        && text.is_char_boundary(range.end);
    if inside { &text[range] } else { "" }
}

/// Whether `byte` starts a name.
fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || !byte.is_ascii()
}

/// Whether `byte` continues a name.
fn is_name_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || !byte.is_ascii()
}

/// The start of a raw string literal at `at`: the content's offset and the closing hash count.
fn raw_start(bytes: &[u8], at: usize) -> Option<(usize, usize)> {
    let mut cursor = match bytes.get(at).copied() {
        Some(b'r') => at,
        Some(b'b' | b'c') if bytes.get(at.saturating_add(1)).copied() == Some(b'r') => {
            at.saturating_add(1)
        }
        _ => return None,
    };
    cursor = cursor.saturating_add(1);
    let mut hashes = 0_usize;
    for _ in 0..bytes.len().saturating_add(1) {
        if bytes.get(cursor).copied() != Some(b'#') {
            break;
        }
        hashes = hashes.saturating_add(1);
        cursor = cursor.saturating_add(1);
    }
    if bytes.get(cursor).copied() == Some(b'"') {
        Some((cursor.saturating_add(1), hashes))
    } else {
        None
    }
}

/// The content of the raw string that opens at `content`, its end offset and the lines it spans.
fn raw_string(text: &str, content: usize, hashes: usize) -> (&str, usize, usize) {
    let bytes = text.as_bytes();
    let mut at = content;
    let mut line = 0_usize;
    let mut end = text.len();
    for _ in 0..bytes.len().saturating_add(1) {
        let Some(byte) = bytes.get(at).copied() else {
            break;
        };
        if byte == b'\n' {
            line = line.saturating_add(1);
        }
        if byte == b'"' && closes(bytes, at.saturating_add(1), hashes) {
            end = at;
            break;
        }
        at = at.saturating_add(1);
    }
    let (slice, next) = if end < text.len() {
        (
            slice_of(text, content..end),
            end.saturating_add(1).saturating_add(hashes),
        )
    } else {
        (slice_of(text, content..text.len()), text.len())
    };
    (slice, next, line)
}

/// Whether `hashes` `#` characters follow `from`.
fn closes(bytes: &[u8], from: usize, hashes: usize) -> bool {
    (0..hashes).all(|offset| bytes.get(from.saturating_add(offset)).copied() == Some(b'#'))
}

/// The content of the plain string whose body starts at `from`, its end offset and its newline
/// count.
fn plain_string(text: &str, from: usize) -> (&str, usize, usize) {
    let bytes = text.as_bytes();
    let mut at = from;
    let mut escaped = false;
    let mut line = 0_usize;
    for _ in 0..bytes.len().saturating_add(1) {
        let Some(byte) = bytes.get(at).copied() else {
            break;
        };
        if escaped {
            escaped = false;
            at = at.saturating_add(1);
            continue;
        }
        match byte {
            b'\\' => escaped = true,
            b'"' => return (slice_of(text, from..at), at.saturating_add(1), line),
            b'\n' => line = line.saturating_add(1),
            _ => {}
        }
        at = at.saturating_add(1);
    }
    (slice_of(text, from..text.len()), text.len(), line)
}

/// The offset after the char literal or lifetime at `at`; neither holds a name the rule reads.
fn char_or_lifetime(bytes: &[u8], at: usize) -> usize {
    let after = at.saturating_add(1);
    match bytes.get(after).copied() {
        Some(b'\\') => {
            let mut cursor = after.saturating_add(1);
            for _ in 0..bytes.len().saturating_add(1) {
                match bytes.get(cursor).copied() {
                    Some(b'\'') => return cursor.saturating_add(1),
                    Some(b'\n') | None => return cursor,
                    Some(_) => cursor = cursor.saturating_add(1),
                }
            }
            cursor
        }
        Some(byte) if is_name_start(byte) => {
            let mut cursor = after;
            for _ in 0..bytes.len().saturating_add(1) {
                if bytes.get(cursor).copied().is_some_and(is_name_continue) {
                    cursor = cursor.saturating_add(1);
                } else {
                    break;
                }
            }
            if bytes.get(cursor).copied() == Some(b'\'') {
                cursor.saturating_add(1)
            } else {
                cursor
            }
        }
        Some(_) => {
            // The char literal's own text decides how many bytes one character takes; a byte
            // sequence the file does not spell as UTF-8 is consumed by the length the decoder
            // reports, so the literal ends where the source places its closing quote.
            let width = match bytes.get(after..) {
                Some(rest) => match std::str::from_utf8(rest) {
                    Ok(text) => match text.chars().next() {
                        Some(found) => found.len_utf8(),
                        None => 1,
                    },
                    Err(cause) => match cause.error_len() {
                        Some(length) => length,
                        None => rest.len(),
                    },
                },
                None => 1,
            };
            let cursor = after.saturating_add(width);
            if bytes.get(cursor).copied() == Some(b'\'') {
                cursor.saturating_add(1)
            } else {
                cursor
            }
        }
        None => bytes.len(),
    }
}

/// The name at `at`, its `r#` prefix removed, and the offset after it.
fn name_at<'source>(text: &'source str, bytes: &'source [u8], at: usize) -> (&'source str, usize) {
    let raw = bytes.get(at).copied() == Some(b'r')
        && bytes.get(at.saturating_add(1)).copied() == Some(b'#')
        && bytes
            .get(at.saturating_add(2))
            .copied()
            .is_some_and(is_name_start);
    let start = if raw { at.saturating_add(2) } else { at };
    let mut cursor = start;
    for _ in 0..bytes.len().saturating_add(1) {
        if bytes.get(cursor).copied().is_some_and(is_name_continue) {
            cursor = cursor.saturating_add(1);
        } else {
            break;
        }
    }
    (slice_of(text, start..cursor), cursor)
}

/// The offset after the number at `at`; a decimal point counts only between digits, so `..` stays
/// two dots.
fn number_at(bytes: &[u8], at: usize) -> usize {
    let mut cursor = at;
    for _ in 0..bytes.len().saturating_add(1) {
        let Some(byte) = bytes.get(cursor).copied() else {
            break;
        };
        let decimal = byte == b'.'
            && bytes
                .get(cursor.saturating_add(1))
                .copied()
                .is_some_and(|next| next.is_ascii_digit());
        if byte.is_ascii_alphanumeric() || byte == b'_' || decimal {
            cursor = cursor.saturating_add(1);
        } else {
            break;
        }
    }
    cursor
}

/// The token `byte` starts and the offset after it; a doubled colon is one [`Token::PathSep`].
fn punctuation(byte: u8, bytes: &[u8], at: usize) -> (Token<'static>, usize) {
    if byte == b':' && bytes.get(at.saturating_add(1)).copied() == Some(b':') {
        return (Token::PathSep, at.saturating_add(2));
    }
    (Token::Punct(char::from(byte)), at.saturating_add(1))
}

/// The offset past the line comment that starts at `at`.
fn line_comment(bytes: &[u8], at: usize) -> usize {
    let mut cursor = at;
    for _ in 0..bytes.len().saturating_add(1) {
        match bytes.get(cursor).copied() {
            Some(b'\n') | None => return cursor,
            Some(_) => cursor = cursor.saturating_add(1),
        }
    }
    cursor
}

/// The offset past the block comment that starts at `at`, and the newlines it holds; blocks nest,
/// as Rust's do.
fn block_comment(bytes: &[u8], at: usize) -> (usize, usize) {
    let mut cursor = at.saturating_add(2);
    let mut depth = 1_usize;
    let mut line = 0_usize;
    for _ in 0..bytes.len().saturating_add(1) {
        let Some(byte) = bytes.get(cursor).copied() else {
            break;
        };
        if byte == b'\n' {
            line = line.saturating_add(1);
        }
        if byte == b'/' && bytes.get(cursor.saturating_add(1)).copied() == Some(b'*') {
            depth = depth.saturating_add(1);
            cursor = cursor.saturating_add(2);
            continue;
        }
        if byte == b'*' && bytes.get(cursor.saturating_add(1)).copied() == Some(b'/') {
            depth = depth.saturating_sub(1);
            cursor = cursor.saturating_add(2);
            if depth == 0 {
                break;
            }
            continue;
        }
        cursor = cursor.saturating_add(1);
    }
    (cursor, line)
}

#[cfg(test)]
mod tests {
    use super::{Token, lex, macro_mask};

    /// The Rust Reference on string literals: a raw string ends at the first quote its own hash count
    /// closes, so the quotes inside it are content and the name after it is code (larry#293).
    #[test]
    fn a_raw_string_ends_at_its_own_hash_count() {
        let tokens: Vec<Token<'_>> = lex(r##"r#"a "name""# after"##)
            .iter()
            .map(|lexeme| lexeme.token)
            .collect();
        assert_eq!(
            tokens,
            vec![Token::Text("a \"name\""), Token::Name("after")],
            "{tokens:?}"
        );
    }

    /// The Rust Reference on tokens: a lifetime and a char literal are not names, so a name that
    /// looks like `id` inside one is data and only the name after it is code (larry#293).
    #[test]
    fn lifetimes_and_char_literals_are_not_names() {
        let tokens: Vec<Token<'_>> = lex("'a 'x' '\\'' id")
            .iter()
            .map(|lexeme| lexeme.token)
            .collect();
        assert_eq!(tokens, vec![Token::Name("id")], "{tokens:?}");
    }

    /// The Rust Reference on comments: a block comment nests, holds no names, and its newlines still
    /// count, so the name after it keeps the line the file places it on (larry#293).
    #[test]
    fn a_nested_block_comment_holds_no_names_and_keeps_the_line() {
        let lexemes = lex("/* id /* pid */ temp_dir */\nlet");
        let tokens: Vec<Token<'_>> = lexemes.iter().map(|lexeme| lexeme.token).collect();
        assert_eq!(tokens, vec![Token::Name("let")], "{tokens:?}");
        assert_eq!(lexemes.first().map(|lexeme| lexeme.line), Some(2));
    }

    /// The Rust Reference on macros by example: an invocation's arguments are syntax tokens, so the
    /// string inside one is code, and the name outside it is not (larry#293).
    #[test]
    fn a_macro_argument_is_code_and_the_name_after_it_is_not() {
        let lexemes = lex("format!(\"x-{pid}\") temp_dir");
        let mask = macro_mask(&lexemes);
        let inside: Vec<Token<'_>> = lexemes
            .iter()
            .zip(mask.iter().copied())
            .filter(|entry| entry.1)
            .map(|(lexeme, _in_macro)| lexeme.token)
            .collect();
        assert_eq!(inside, vec![Token::Text("x-{pid}")], "{inside:?}");
    }

    /// The Rust Reference on tokens: `::` is one path separator, and a range's `..` stays two dots,
    /// so neither is read as a number (larry#293).
    #[test]
    fn a_path_separator_is_one_token_and_a_range_is_not_a_number() {
        let tokens: Vec<Token<'_>> = lex("std::env 0..n")
            .iter()
            .map(|lexeme| lexeme.token)
            .collect();
        assert_eq!(
            tokens,
            vec![
                Token::Name("std"),
                Token::PathSep,
                Token::Name("env"),
                Token::Literal,
                Token::Punct('.'),
                Token::Punct('.'),
                Token::Name("n"),
            ],
            "{tokens:?}"
        );
    }

    /// The Rust Reference on literals: input that ends inside one still ends the reader, so a torn
    /// file is a token, never a walk past the buffer (larry#293).
    #[test]
    fn an_unterminated_string_ends_the_reader() {
        let tokens: Vec<Token<'_>> = lex("\"abc").iter().map(|lexeme| lexeme.token).collect();
        assert_eq!(tokens, vec![Token::Text("abc")], "{tokens:?}");
    }

    /// The Rust Reference on string literals: a byte string and a C string hold text, so the name
    /// after each of them is code and the reader does not stop at either (larry#293).
    #[test]
    fn a_byte_string_and_a_c_string_hold_text() {
        let tokens: Vec<Token<'_>> = lex("b\"id\" c\"pid\" after")
            .iter()
            .map(|lexeme| lexeme.token)
            .collect();
        assert_eq!(
            tokens,
            vec![Token::Text("id"), Token::Text("pid"), Token::Name("after"),],
            "{tokens:?}"
        );
    }

    /// The Rust Reference on string literals: a backslash escapes the quote after it, so the string
    /// ends at the next unescaped quote and the name after it is code (larry#293).
    #[test]
    fn an_escaped_quote_stays_inside_its_string() {
        let tokens: Vec<Token<'_>> = lex("\"a\\\"b\" after")
            .iter()
            .map(|lexeme| lexeme.token)
            .collect();
        assert_eq!(
            tokens,
            vec![Token::Text("a\\\"b"), Token::Name("after")],
            "{tokens:?}"
        );
    }

    /// The Rust Reference on identifiers: `r#type` is the name without its `r#` prefix, and the
    /// name after it is separate (larry#293).
    #[test]
    fn a_raw_identifier_is_the_name_without_its_prefix() {
        let tokens: Vec<Token<'_>> = lex("r#type pid")
            .iter()
            .map(|lexeme| lexeme.token)
            .collect();
        assert_eq!(
            tokens,
            vec![Token::Name("type"), Token::Name("pid")],
            "{tokens:?}"
        );
    }
}
