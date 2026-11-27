//! URL authority validation and the loopback plaintext exception.

use super::route::MAX_URL_BYTES;

/// The host and port of a bare http or https base URL, `None` when `url` is not one.
pub(super) fn authority_of(url: &str) -> Option<&str> {
    if url.len() > MAX_URL_BYTES
        || url.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || matches!(character, '?' | '#' | '\\' | '@')
        })
    {
        return None;
    }
    let (scheme, rest) = url.split_once("://")?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split('/').next()?;
    let well_formed = !authority.is_empty()
        && authority.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':' | b'[' | b']')
        });
    well_formed.then_some(authority)
}

/// Whether `authority` names a host on this machine, where plaintext is safe for
/// a local mock or sidecar and nowhere else. Loopback is the name `localhost`
/// (RFC 6761 section 6.3; DNS names compare case-insensitively), the
/// 127.0.0.0/8 block in dotted-quad text and `::1` (RFC 4291): a suffix of a
/// special name, a short or out-of-range quad, an IPv4-mapped address, the
/// unspecified address and a malformed literal all answer false, so they fail
/// closed to HTTPS.
pub(super) fn is_loopback(authority: &str) -> bool {
    let host = if let Some(rest) = authority.strip_prefix('[') {
        match rest.split_once(']') {
            Some((host, _)) => host,
            None => authority,
        }
    } else {
        match authority.rsplit_once(':') {
            Some((host, _)) if !host.contains(':') => host,
            // More than one colon outside brackets names no host:port pair:
            // RFC 3986 section 3.2.2 brackets every IP literal, so an
            // unbracketed multi-colon string can only be a malformed
            // authority. Answering loopback would let plaintext at it.
            Some(_) => return false,
            None => authority,
        }
    };
    // The same special name in fully-qualified form (RFC 6761 section 6.3):
    // a trailing dot names the root, not a subdomain, so `localhost.` is
    // `localhost`. Anything still dotted afterwards is a public suffix.
    // Only the bare `::1` is loopback: a zone id names an interface.
    let bare = match host.strip_suffix('.') {
        Some(stripped) => stripped,
        None => host,
    };
    if bare.eq_ignore_ascii_case("localhost") || host == "::1" {
        return true;
    }
    let mut parts = bare.split('.');
    if parts.next() != Some("127") {
        return false;
    }
    let tail = [parts.next(), parts.next(), parts.next()];
    parts.next().is_none()
        && tail
            .iter()
            .all(|part| part.is_some_and(|value| !value.is_empty() && value.parse::<u8>().is_ok()))
}

#[cfg(test)]
mod tests {
    use super::{authority_of, is_loopback};

    #[test]
    fn transport_authority_accepts_https_and_rejects_bad_url() {
        assert_eq!(
            authority_of("https://gateway.test/v1"),
            Some("gateway.test")
        );
        assert_eq!(authority_of("https://gateway.test?token=secret"), None);
    }

    #[test]
    fn loopback_identifies_local_plaintext_only() {
        assert!(is_loopback("localhost:8080"));
        assert!(is_loopback("[::1]:8080"));
        assert!(is_loopback("127.0.0.1:8080"));
        assert!(!is_loopback("gateway.test:8080"));
    }

    /// Every hostile host answers exactly, from RFC 6761 section 6.3 (only
    /// the name `localhost` is special) and RFC 4291 (loopback is
    /// 127.0.0.0/8 and `::1`, and nothing else).
    #[test]
    fn loopback_refuses_every_non_loopback_host() {
        // A special name with a public suffix is an ordinary public name, not
        // the special name itself. DNS names compare case-insensitively, so
        // uppercase still names the special `localhost` itself, and a trailing
        // dot is the same name in fully-qualified form.
        for allowed in ["LOCALHOST:1", "localhost.", "127.0.0.1."] {
            assert!(is_loopback(allowed), "{allowed} is loopback");
        }
        // Anything else fails closed to HTTPS: a suffixed special name, a
        // non-quad form, a malformed literal, a routed IPv4-mapped address,
        // the unspecified address, and colons outside brackets (RFC 3986
        // 3.2.2 brackets every IP literal, so a multi-colon string is never
        // a host:port pair).
        for hostile in [
            "127.0.0.1.evil.test",
            "localhost.evil.test",
            "localhost..",
            "[::1%lo0]",
            "127.0.0.256",
            "127.1",
            "0177.0.0.1",
            "2130706433",
            "0x7f.0.0.1",
            "[::1",
            "[::ffff:127.0.0.1]",
            "[::ffff:127.0.0.1]:8080",
            "0.0.0.0",
            "::1:8080",
            "",
        ] {
            assert!(!is_loopback(hostile), "{hostile:?} is not loopback");
        }
        // Nothing to parse: a URL with an empty authority, and a URL past the
        // 2048-byte bound, are both refused.
        assert_eq!(authority_of(""), None);
        assert_eq!(authority_of("https:///path"), None);
        let capped = format!("https://gateway.test/{}", "a".repeat(2048 - 21));
        assert_eq!(capped.len(), 2048);
        assert!(authority_of(&capped).is_some());
        assert_eq!(authority_of(&format!("{capped}a")), None);
    }
}
