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

/// Whether `authority` names a destination a remote door never sends to (larry#627): an
/// address that is not public, a numeric spelling of an address, or a name only this host
/// or its own network answers. A public name that resolves inward is not caught here; the
/// connected peer check is larry#510.
pub(super) fn is_private_destination(authority: &str) -> bool {
    match authority.strip_prefix('[') {
        Some(rest) => !rest.split_once(']').is_some_and(
            |(literal, _)| matches!(literal.parse::<Ipv6Addr>(), Ok(v6) if is_public_v6(v6)),
        ),
        None => !is_public_host(host_of(authority)),
    }
}

/// Whether an unbracketed `host` is a public address or a public DNS name.
fn is_public_host(host: &str) -> bool {
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    if let Ok(v4) = name.parse::<Ipv4Addr>() {
        return is_public_v4(v4);
    }
    // One label is answered by the host's own search list, never by public DNS.
    name.rsplit_once('.').is_some_and(|(_, last)| {
        !ends_in_a_number(last)
            && !matches!(last, "localhost" | "local" | "internal")
            && !name.ends_with(".home.arpa")
    })
}

/// Whether a last label makes `inet_aton` read the whole host as an address, as the
/// WHATWG URL rule "ends in a number" reads it: `127.1`, `0177.0.0.1`, `0x7f.0.0.0x1`.
fn ends_in_a_number(label: &str) -> bool {
    match label.strip_prefix("0x") {
        Some(hex) => hex.bytes().all(|byte| byte.is_ascii_hexdigit()),
        None => label.bytes().all(|byte| byte.is_ascii_digit()),
    }
}

/// Whether `address` is outside every special-purpose IPv4 block (RFC 6890).
fn is_public_v4(address: Ipv4Addr) -> bool {
    let [first, second, third, _] = address.octets();
    let reserved = first == 0
        || first >= 240
        || (first == 100 && (64..128).contains(&second))
        || (first == 192 && second == 0 && third == 0)
        || (first == 198 && (18..20).contains(&second));
    !(reserved
        || address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_multicast()
        || address.is_documentation())
}

/// Whether `address` is global unicast (2000::/3, RFC 4291). A mapped or 6to4 address
/// answers for the IPv4 address it carries; loopback, unique local, link local, site
/// local, multicast and the unspecified address are all outside the block.
fn is_public_v6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_v4(mapped);
    }
    let [first, second, third, ..] = address.segments();
    if first == 0x2002 {
        let [high, upper] = second.to_be_bytes();
        let [lower, low] = third.to_be_bytes();
        return is_public_v4(Ipv4Addr::new(high, upper, lower, low));
    }
    (first & 0xe000) == 0x2000 && !(first == 0x2001 && second == 0x0db8)
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
    use super::{
        PROXY_VARIABLES, authority_of, is_loopback, is_private_destination, is_proxy_variable,
        plaintext_proxy,
    };

    /// Every block RFC 6890 and RFC 4291 set aside at both of its edges, each spelling a
    /// resolver reads as an address, and the names only this network answers.
    const PRIVATE: &[&str] = &[
        "0.0.0.0",
        "0.255.255.255",
        "10.0.0.0",
        "10.255.255.255",
        "100.64.0.0",
        "100.127.255.255",
        "127.0.0.1",
        "127.255.255.255",
        "169.254.0.0",
        "169.254.169.254",
        "172.16.0.0",
        "172.31.255.255",
        "192.0.0.1",
        "192.0.2.1",
        "192.168.0.1",
        "198.18.0.0",
        "198.19.255.255",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "240.0.0.1",
        "255.255.255.255",
        "127.0.0.1.",
        "127.1",
        "2130706433",
        "0x7f000001",
        "0177.0.0.1",
        "010.0.0.1",
        "1.2.3.0x4",
        "8.8.8.8.8",
        "localhost",
        "LOCALHOST.",
        "gateway.localhost",
        "printer.local",
        "vault.internal",
        "Metadata.Google.INTERNAL",
        "nas.home.arpa",
        "metadata",
        "",
        "[::]",
        "[::1]",
        "[::ffff:127.0.0.1]",
        "[::ffff:169.254.169.254]",
        "[fc00::1]",
        "[fdff::1]",
        "[fe80::1]",
        "[fec0::1]",
        "[ff02::1]",
        "[64:ff9b::a00:1]",
        "[2001:db8::1]",
        "[2002:a00:1::1]",
        "[2002:7f00:1::]",
        "[fe80::1%eth0]",
        "[not-v6]",
        "[::1",
    ];

    /// The addresses one step outside each block, and ordinary public names.
    const PUBLIC: &[&str] = &[
        "1.0.0.0",
        "8.8.8.8",
        "9.255.255.255",
        "11.0.0.0",
        "100.63.255.255",
        "100.128.0.0",
        "126.255.255.255",
        "128.0.0.0",
        "169.253.255.255",
        "169.255.0.0",
        "172.15.255.255",
        "172.32.0.0",
        "192.0.1.1",
        "192.167.255.255",
        "192.169.0.0",
        "198.17.255.255",
        "198.20.0.0",
        "223.255.255.255",
        "gateway.test",
        "api.example.com.",
        "internal.example.com",
        "local.example.com",
        "localhost.example.com",
        "9router.example.com",
        "xn--fsq.example",
        "[2000::1]",
        "[2606:4700::1111]",
        "[3fff:ffff::1]",
        "[2001:db9::1]",
        "[2002:808:808::1]",
        "[::ffff:8.8.8.8]",
    ];

    /// Every private spelling is refused and every public one passes, with and without a
    /// port.
    #[test]
    fn sim_every_private_destination_is_refused_and_public_hosts_pass() {
        for &host in PRIVATE {
            for authority in [String::from(host), format!("{host}:443")] {
                assert!(
                    is_private_destination(&authority),
                    "{authority:?} is private"
                );
            }
        }
        for &host in PUBLIC {
            for authority in [String::from(host), format!("{host}:8443")] {
                assert!(
                    !is_private_destination(&authority),
                    "{authority:?} is public"
                );
            }
        }
    }

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

    /// The authority of `host` on `port`, as a route URL spells it.
    fn at(host: &str, port: &str) -> String {
        format!("{host}:{port}")
    }

    #[test]
    fn loopback_identifies_local_plaintext_only() {
        for host in ["localhost", "[::1]", "127.0.0.1"] {
            assert!(is_loopback(&at(host, "8080")), "{host} is loopback");
        }
        assert!(!is_loopback(&at("gateway.test", "8080")));
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
            at("LOCALHOST", "1"),
            String::from("localhost."),
            String::from("127.0.0.1."),
            at("127.255.255.254", "65535"),
            String::from("[::1]"),
            at("[0:0:0:0:0:0:0:1]", "8080"),
        ] {
            assert!(is_loopback(&allowed), "{allowed} is loopback");
        }
        // A port past 65,535 and a second port are no port at all.
        for hostile in [at("[::1]", "65536"), at("127.0.0.1", "80:80")] {
            assert!(!is_loopback(&hostile), "{hostile:?} is not loopback");
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
            "[localhost]",
            "[127.0.0.1]",
            "localhost:",
            "localhost:evil.test",
            "127.0.0.1:80.evil.test",
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
