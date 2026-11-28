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

/// The variables ureq 3.5.1 takes a proxy from, in the order it tries them. The
/// client under `lgwks_std::http` reads them for every scheme and address, and has
/// no setting to stop it (gap logicalworks-crates#1169).
const PROXY_VARIABLES: [&str; 6] = [
    "ALL_PROXY",
    "all_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
];

/// The lists ureq skips the proxy by: the first of these that is set, even empty.
const BYPASS_VARIABLES: [&str; 2] = ["NO_PROXY", "no_proxy"];

/// Whether `name` is a variable that decides where a model call is sent.
pub(crate) fn is_proxy_variable(name: &str) -> bool {
    PROXY_VARIABLES.contains(&name) || BYPASS_VARIABLES.contains(&name)
}

/// The host in `authority` as ureq matches it against a bypass list: no port,
/// brackets kept.
fn host_of(authority: &str) -> &str {
    let host = if authority.starts_with('[') {
        authority.split_inclusive(']').next()
    } else {
        authority.split(':').next()
    };
    match host {
        Some(host) => host,
        None => authority,
    }
}

/// The proxy variable that would carry a plaintext call to `base_url`, key and
/// all, with the host to name in `NO_PROXY` (larry#602). `None` for https, with no
/// proxy variable set, or when the list ureq reads holds that host or `*`; its
/// suffix and prefix patterns are not honoured, so Larry never allows a call ureq
/// would proxy.
pub(super) fn plaintext_proxy<'url>(
    base_url: &'url str,
    read: &impl Fn(&str) -> Option<String>,
) -> Option<(&'static str, &'url str)> {
    let authority = authority_of(base_url).filter(|_| base_url.starts_with("http://"))?;
    let host = host_of(authority);
    let variable = PROXY_VARIABLES
        .into_iter()
        .find(|name| read(name).is_some_and(|value| !value.is_empty()))?;
    let direct = BYPASS_VARIABLES
        .into_iter()
        .find_map(read)
        .is_some_and(|list| {
            list.split(',')
                .any(|entry| entry == "*" || entry.eq_ignore_ascii_case(host))
        });
    (!direct).then_some((variable, host))
}

#[cfg(test)]
mod tests {
    use super::{PROXY_VARIABLES, authority_of, is_loopback, is_proxy_variable, plaintext_proxy};

    /// Every proxy variable, bypass spelling, list shape and loopback host: a
    /// plaintext call is allowed only past a list that holds its host or `*`.
    #[test]
    fn sim_a_proxy_refuses_only_an_unbypassed_plaintext_call() {
        let lists = [
            ("", false),
            ("*", true),
            ("{host}", true),
            ("{HOST}", true),
            ("other.test,{host}", true),
            ("other.test, {host}", false),
            ("{host}:9", false),
            ("other.test", false),
        ];
        for host in ["127.0.0.1", "localhost", "[::1]"] {
            for url in [format!("http://{host}:9/v1"), format!("http://{host}/v1")] {
                for name in PROXY_VARIABLES {
                    let refused = Some((name, host));
                    for bypass in ["NO_PROXY", "no_proxy"] {
                        for (list, direct) in lists {
                            let list = list
                                .replace("{host}", host)
                                .replace("{HOST}", &host.to_ascii_uppercase());
                            let read = |asked: &str| match asked {
                                _ if asked == name => Some(String::from("http://proxy.test:3128")),
                                _ if asked == bypass => Some(list.clone()),
                                _ => None,
                            };
                            assert_eq!(
                                plaintext_proxy(&url, &read),
                                refused.filter(|_| !direct),
                                "{url} {name} {bypass}={list}"
                            );
                        }
                    }
                    let shadowed = |asked: &str| match asked {
                        "NO_PROXY" => Some(String::new()),
                        "no_proxy" => Some(String::from("*")),
                        _ => (asked == name).then(|| String::from("http://proxy.test:3128")),
                    };
                    assert_eq!(plaintext_proxy(&url, &shadowed), refused, "{url} {name}");
                    let empty = |asked: &str| (asked == name).then(String::new);
                    assert_eq!(plaintext_proxy(&url, &empty), None, "{url} {name} empty");
                }
                let unset = |_: &str| None;
                assert_eq!(plaintext_proxy(&url, &unset), None, "{url} unset");
            }
        }
        let all = |_: &str| Some(String::from("proxy.test"));
        assert_eq!(plaintext_proxy("https://gateway.test/v1", &all), None);
    }

    #[test]
    fn sim_only_the_eight_proxy_names_are_read() {
        for name in PROXY_VARIABLES.into_iter().chain(["NO_PROXY", "no_proxy"]) {
            assert!(is_proxy_variable(name), "{name}");
        }
        for name in ["Http_Proxy", "FTP_PROXY", "PROXY", "NO_PROXY_", ""] {
            assert!(!is_proxy_variable(name), "{name:?}");
        }
    }

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
