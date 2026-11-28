//! URL authority validation and the loopback plaintext exception.

use std::net::{Ipv4Addr, Ipv6Addr};

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
/// a local mock or sidecar and nowhere else: the name `localhost` (RFC 6761
/// section 6.3), an address in 127.0.0.0/8, or `[::1]` (RFC 4291), each with
/// nothing or a port after it. Everything else answers false and so needs HTTPS.
/// `std::net` reads the addresses: it refuses short, octal, hexadecimal and
/// zero-led quads (CVE-2021-29922) and zone ids.
pub(super) fn is_loopback(authority: &str) -> bool {
    match authority.strip_prefix('[') {
        // RFC 3986 section 3.2.2: brackets hold an IPv6 literal and only a port
        // may follow them, so `[::1].evil.test` names no loopback host.
        Some(rest) => match rest.split_once(']') {
            Some((literal, after)) => {
                let sound_tail = match after.strip_prefix(':') {
                    Some(port) => is_port(port),
                    None => after.is_empty(),
                };
                sound_tail && matches!(literal.parse::<Ipv6Addr>(), Ok(v6) if v6.is_loopback())
            }
            None => false,
        },
        // An unbracketed host holds no colon, so the first one starts the port.
        None => match authority.split_once(':') {
            Some((host, port)) => is_port(port) && is_loopback_host(host),
            None => is_loopback_host(authority),
        },
    }
}

/// Whether an unbracketed `host` is `localhost` or a dotted quad in 127.0.0.0/8.
fn is_loopback_host(host: &str) -> bool {
    // A trailing dot names the DNS root, so `localhost.` is `localhost`.
    let bare = match host.strip_suffix('.') {
        Some(stripped) => stripped,
        None => host,
    };
    bare.eq_ignore_ascii_case("localhost")
        || matches!(bare.parse::<Ipv4Addr>(), Ok(v4) if v4.is_loopback())
}

/// Whether `text` is a port number.
fn is_port(text: &str) -> bool {
    text.parse::<u16>().is_ok()
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
        for allowed in [
            "LOCALHOST:1",
            "localhost.",
            "127.0.0.1.",
            "127.255.255.254:65535",
            "[::1]",
            "[0:0:0:0:0:0:0:1]:8080",
        ] {
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
            "[::1].evil.test",
            "[::1]8080",
            "[::1]:",
            "[::1]:65536",
            "[localhost]",
            "[127.0.0.1]",
            "localhost:",
            "localhost:evil.test",
            "127.0.0.1:80.evil.test",
            "127.0.0.1:80:80",
            "127.000.000.001",
            "128.0.0.1",
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
