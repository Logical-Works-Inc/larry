//! Markdown structure, frontmatter, link and heading checks, ported line by line from the Python
//! checker larry#53 replaced. Every rule here names the CPython behaviour it keeps.

use std::collections::{BTreeMap, BTreeSet};

use super::documents::{Documents, MAX_DOCUMENT_BYTES};
use super::error::GovernanceError;
use super::patterns::Patterns;
use super::pystr;

/// The frontmatter keys every governed document declares, verbatim from the Python `FIELDS`.
pub(crate) const FIELDS: [&str; 10] = [
    "type",
    "title",
    "description",
    "resource",
    "tags",
    "generated",
    "verified",
    "status",
    "stale_after",
    "sources",
];

/// The heading IDs of each document a fragment link has named so far, computed once per check.
pub(crate) type HeadingCache = BTreeMap<String, BTreeSet<String>>;

/// Frontmatter errors for one document, in Python's order.
fn frontmatter_errors(name: &str, text: &str, patterns: &Patterns) -> Vec<String> {
    let Some(group) = frontmatter_group(text) else {
        return vec![format!("{name}: missing frontmatter envelope")];
    };
    let mut errors = Vec::new();
    let mut keys: Vec<&str> = Vec::new();
    let mut last_key: Option<&str> = None;
    for line in pystr::splitlines(group) {
        if pystr::strip(line).is_empty() {
            continue;
        }
        if line.starts_with("  - ") && last_key == Some("sources") {
            continue;
        }
        let Some((key, value)) = frontmatter_field(line) else {
            errors.push(format!(
                "{name}: unsupported frontmatter line {}",
                patterns.classes.repr(line)
            ));
            continue;
        };
        keys.push(key);
        last_key = Some(key);
        if value.is_empty() && key != "sources" {
            errors.push(format!("{name}: empty frontmatter field {key}"));
        }
        for (opening, closing) in [("[", "]"), ("{", "}")] {
            if value.starts_with(opening) && !value.ends_with(closing) {
                errors.push(format!("{name}: unterminated frontmatter field {key}"));
            }
        }
    }
    let distinct: BTreeSet<&str> = keys.iter().copied().collect();
    if distinct.len() != keys.len() {
        errors.push(format!("{name}: duplicate frontmatter key"));
    }
    let missing: Vec<String> = FIELDS
        .iter()
        .filter(|field| !distinct.contains(*field))
        .map(|field| String::from(*field))
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect();
    if !missing.is_empty() {
        errors.push(format!(
            "{name}: missing frontmatter keys {}",
            patterns.classes.list(&missing)
        ));
    }
    errors
}

/// `\A---\n(.*?)\n---\n` under `re.S`: the text between the opening fence and the first closing one.
fn frontmatter_group(text: &str) -> Option<&str> {
    let body = text.strip_prefix("---\n")?;
    let end = body.find("\n---\n")?;
    body.get(..end)
}

/// `re.fullmatch(r"([a-z_]+):\s*(.*)", line)`: the key, and the value after its leading
/// whitespace. Trailing whitespace stays in the value, as it does in Python.
fn frontmatter_field(line: &str) -> Option<(&str, &str)> {
    let (key, rest) = line.split_once(':')?;
    if key.is_empty()
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        return None;
    }
    Some((key, rest.trim_start_matches(pystr::is_space)))
}

/// The visible lines, the fenced languages and the fence errors of one document.
pub(crate) struct Structure {
    /// Lines outside every fenced code block.
    visible: Vec<String>,
    /// The first token of each opening fence's info string, `""` when none.
    languages: Vec<String>,
    /// Fence errors, in the Python order.
    errors: Vec<String>,
}

impl Structure {
    /// The first token of each opening fence's info string, in order.
    pub(crate) fn languages(&self) -> &[String] {
        &self.languages
    }

    /// The visible lines joined by `\n`, the text links and headings are read from.
    #[cfg(test)]
    pub(crate) fn visible_text(&self) -> String {
        self.visible.join("\n")
    }
}

/// Port of `markdown_structure`: fences per CommonMark, with the repository's backtick-in-info
/// refusal.
pub(crate) fn markdown_structure(text: &str, patterns: &Patterns) -> Structure {
    let mut visible = Vec::new();
    let mut languages = Vec::new();
    let mut errors = Vec::new();
    let mut opened: Option<(char, usize)> = None;
    for line in pystr::splitlines(text) {
        let Some((marker, suffix)) = patterns.fence_parts(line) else {
            if opened.is_none() {
                visible.push(String::from(line));
            }
            continue;
        };
        let first = marker.chars().next();
        match opened {
            None => {
                if first == Some('`') && suffix.contains('`') {
                    errors.push(String::from("malformed backtick fence"));
                }
                opened = first.map(|character| (character, marker.len()));
                languages.push(pystr::split(suffix).into_iter().take(1).collect::<String>());
            }
            Some((open_char, open_len)) => {
                if first == Some(open_char)
                    && marker.len() >= open_len
                    && pystr::strip(suffix).is_empty()
                {
                    opened = None;
                }
            }
        }
    }
    if opened.is_some() {
        errors.push(String::from("unclosed code fence"));
    }
    Structure {
        visible,
        languages,
        errors,
    }
}

/// Every error for one document: frontmatter, byte size, fences, links and fragments. An error
/// CPython's `urlsplit` raises stops the whole check, as the `ValueError` did.
pub(crate) fn document_errors(
    name: &str,
    text: &str,
    documents: &Documents,
    patterns: &Patterns,
    headings: &mut HeadingCache,
) -> Result<Vec<String>, GovernanceError> {
    let mut errors = frontmatter_errors(name, text, patterns);
    if text.len() > MAX_DOCUMENT_BYTES {
        errors.push(format!("{name}: document exceeds byte limit"));
    }
    let structure = markdown_structure(text, patterns);
    errors.extend(
        structure
            .errors
            .iter()
            .map(|error| format!("{name}: {error}")),
    );
    let visible = structure.visible.join("\n");
    for target in patterns.link_targets(&visible) {
        if let Some(error) = link_error(name, target, documents, patterns, headings)? {
            errors.push(error);
        }
    }
    Ok(errors)
}

/// The error for one link destination, or none when it resolves.
fn link_error(
    name: &str,
    target: &str,
    documents: &Documents,
    patterns: &Patterns,
    headings: &mut HeadingCache,
) -> Result<Option<String>, GovernanceError> {
    let parts = urlsplit(target, patterns)?;
    if matches!(parts.scheme.as_str(), "https" | "http" | "mailto") {
        return Ok(None);
    }
    if !parts.scheme.is_empty() || !parts.netloc.is_empty() {
        return Ok(Some(format!("{name}: unsupported link scheme {target}")));
    }
    let decoded = percent_decode(&parts.path);
    let trimmed = decoded.trim_start_matches('/');
    let path = if trimmed.is_empty() { name } else { trimmed };
    let escaping = path.split('/').any(|part| part == "..");
    let Some(text) = documents.get(path).filter(|_| !escaping) else {
        return Ok(Some(format!(
            "{name}: unresolved or escaping local link {target}"
        )));
    };
    if parts.fragment.is_empty() {
        return Ok(None);
    }
    let fragment = percent_decode(&parts.fragment);
    let ids = headings
        .entry(String::from(path))
        .or_insert_with(|| heading_ids(text, patterns));
    if ids.contains(&fragment) {
        Ok(None)
    } else {
        Ok(Some(format!(
            "{name}: unresolved heading fragment {target}"
        )))
    }
}

/// The parts of a link destination, as CPython 3.12's `urlsplit` reports them.
struct Split {
    /// Lowercased scheme, `""` when none.
    scheme: String,
    /// Network location, `""` when none or empty.
    netloc: String,
    /// The path component.
    path: String,
    /// The fragment after the first `#`, `""` when none.
    fragment: String,
}

/// CPython 3.12 `urlsplit`, with its 2025 bracket fixes: strips the leading C0 controls and
/// spaces and every tab, CR and LF, reads a scheme, a netloc, then the path, query and fragment.
/// The NFKC netloc check is not ported: such a netloc is already an unsupported scheme here.
fn urlsplit(target: &str, patterns: &Patterns) -> Result<Split, GovernanceError> {
    let cleaned: String = target
        .trim_start_matches(|character: char| character <= ' ')
        .chars()
        .filter(|character| !matches!(*character, '\t' | '\r' | '\n'))
        .collect();
    let (scheme, rest) = split_scheme(&cleaned);
    let (netloc, after_netloc) = match rest.strip_prefix("//") {
        Some(authority) => {
            let end = match authority.find(['/', '?', '#']) {
                Some(position) => position,
                None => authority.len(),
            };
            authority.split_at(end)
        }
        None => ("", rest),
    };
    let open = netloc.contains('[');
    if open != netloc.contains(']') {
        return Err(GovernanceError::InvalidIpv6Url);
    }
    if open {
        check_bracketed_netloc(netloc, patterns)?;
    }
    let (before_fragment, fragment) = match after_netloc.split_once('#') {
        Some((head, tail)) => (head, tail),
        None => (after_netloc, ""),
    };
    let path = match before_fragment.split_once('?') {
        Some((head, _query)) => head,
        None => before_fragment,
    };
    Ok(Split {
        scheme,
        netloc: String::from(netloc),
        path: String::from(path),
        fragment: String::from(fragment),
    })
}

/// `_check_bracketed_netloc`: no text before the bracket, only a port after it, and a valid host.
fn check_bracketed_netloc(netloc: &str, patterns: &Patterns) -> Result<(), GovernanceError> {
    let host_and_port = match netloc.rsplit_once('@') {
        Some((_user, host)) => host,
        None => netloc,
    };
    let hostname = match host_and_port.split_once('[') {
        Some((before, bracketed)) => {
            if !before.is_empty() {
                return Err(GovernanceError::InvalidIpv6Url);
            }
            let (host, port) = match bracketed.split_once(']') {
                Some(parts) => parts,
                None => (bracketed, ""),
            };
            if !port.is_empty() && !port.starts_with(':') {
                return Err(GovernanceError::InvalidIpv6Url);
            }
            host
        }
        None => match host_and_port.split_once(']') {
            Some((host, _port)) => host,
            None => host_and_port,
        },
    };
    check_bracketed_host(hostname, patterns)
}

/// `_check_bracketed_host`: an IPvFuture literal, or an IPv6 address; IPv4 is refused.
fn check_bracketed_host(hostname: &str, patterns: &Patterns) -> Result<(), GovernanceError> {
    if let Some(future) = hostname.strip_prefix('v') {
        let valid = future.split_once('.').is_some_and(|(version, rest)| {
            !version.is_empty()
                && version.bytes().all(|byte| byte.is_ascii_hexdigit())
                && !rest.is_empty()
                && !rest.contains('\n')
        });
        return if valid {
            Ok(())
        } else {
            Err(GovernanceError::InvalidIpvFuture)
        };
    }
    if hostname.parse::<std::net::Ipv4Addr>().is_ok() {
        return Err(GovernanceError::BracketedIpv4);
    }
    let (address, scope) = match hostname.split_once('%') {
        Some((address, scope)) => (address, Some(scope)),
        None => (hostname, None),
    };
    let scope_valid = scope.is_none_or(|text| !text.is_empty() && !text.contains('%'));
    if scope_valid && address.parse::<std::net::Ipv6Addr>().is_ok() {
        return Ok(());
    }
    Err(GovernanceError::NotAnAddress {
        host: patterns.classes.repr(hostname),
    })
}

/// The scheme before the first `:`, when it is non-empty, starts with an ASCII letter and holds
/// only ASCII alphanumerics or `+-.`. Returns the lowercased scheme and the remainder.
fn split_scheme(text: &str) -> (String, &str) {
    let Some((head, tail)) = text.split_once(':') else {
        return (String::new(), text);
    };
    let starts_alpha = head
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic());
    let valid = starts_alpha
        && head
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "+-.".contains(character));
    if valid {
        (head.to_ascii_lowercase(), tail)
    } else {
        (String::new(), text)
    }
}

/// `urllib.parse.unquote`: `%XX` hex pairs become bytes; an invalid escape stays literal; the
/// result is decoded as UTF-8 with replacement, which equals CPython's per-ASCII-run decode
/// because a literal non-ASCII character never begins with a continuation byte.
pub(crate) fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        let escape = match byte {
            b'%' => index.checked_add(1).and_then(|start| {
                let end = start.checked_add(2)?;
                bytes.get(start..end).and_then(hex_pair)
            }),
            _ => None,
        };
        match escape {
            Some(decoded) => {
                out.push(decoded);
                index = index.saturating_add(3);
            }
            None => {
                out.push(byte);
                index = index.saturating_add(1);
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The byte of two ASCII hex digits, or `None`.
fn hex_pair(pair: &[u8]) -> Option<u8> {
    let high = char::from(*pair.first()?).to_digit(16)?;
    let low = char::from(*pair.get(1)?).to_digit(16)?;
    u8::try_from(high.checked_mul(16)?.checked_add(low)?).ok()
}

/// GitHub-style heading IDs over the visible lines of `text`, with `-N` for repeats.
pub(crate) fn heading_ids(text: &str, patterns: &Patterns) -> BTreeSet<String> {
    let structure = markdown_structure(text, patterns);
    let visible = structure.visible.join("\n");
    let mut result = BTreeSet::new();
    let mut occurrences: BTreeMap<String, usize> = BTreeMap::new();
    for title in patterns.heading_titles(&visible) {
        let slug = patterns.slug(title);
        let seen = occurrences.entry(slug.clone()).or_insert(0);
        let suffix = *seen;
        *seen = suffix.saturating_add(1);
        result.insert(if suffix == 0 {
            slug
        } else {
            format!("{slug}-{suffix}")
        });
    }
    result
}
