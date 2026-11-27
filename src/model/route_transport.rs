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
/// a local mock or sidecar and nowhere else.
pub(super) fn is_loopback(authority: &str) -> bool {
    let host = if let Some(rest) = authority.strip_prefix('[') {
        match rest.split_once(']') {
            Some((host, _)) => host,
            None => authority,
        }
    } else {
        match authority.rsplit_once(':') {
            Some((host, _)) => host,
            None => authority,
        }
    };
    if host.eq_ignore_ascii_case("localhost") || host == "::1" {
        return true;
    }
    let mut parts = host.split('.');
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
    fn sim_transport_authority_accepts_https_and_rejects_bad_url() {
        assert_eq!(
            authority_of("https://gateway.test/v1"),
            Some("gateway.test")
        );
        assert_eq!(authority_of("https://gateway.test?token=secret"), None);
    }

    #[test]
    fn sim_loopback_identifies_local_plaintext_only() {
        assert!(is_loopback("localhost:8080"));
        assert!(is_loopback("[::1]:8080"));
        assert!(is_loopback("127.0.0.1:8080"));
        assert!(!is_loopback("gateway.test:8080"));
    }
}
