---
type: ADR
title: "ADR 0143: a proxy variable never carries a plaintext model call"
description: "An http model route is refused while a proxy variable is set, unless NO_PROXY holds its host."
resource: /docs/adr/0143-no-proxy-for-plaintext.md
tags: [larry, adr, model, transport, security]
generated: {by: claude-code, at: 2026-11-28}
verified: {by: claude-code, method: lgwks-box cargo nextest run, 2026-11-28}
status: accepted
stale_after: 2027-05-28
sources: [larry#602, larry#615, larry#203, logicalworks-crates#1169, CWE-319, INVARIANTS.md]
---

## Status

Accepted.

## Context

Plaintext is allowed to a loopback host only, for a local mock or sidecar.
`lgwks_std::http` 5.0.0 builds its ureq 3.5.1 agent without a proxy setting, so
ureq takes `ALL_PROXY`, `HTTPS_PROXY` or `HTTP_PROXY` from the environment for
every address, loopback included. With one set, `larry model` sent the loopback
request and its bearer key to the proxy in the clear (CWE-319). The gateway
received nothing.

## Decision

`ModelConfig::for_route`, the one place a model config is built, refuses an
`http://` route as `ModelError::PlaintextProxy` while a proxy variable is set,
before the key is read. The route runs when the list ureq reads holds its host
or `*`: `NO_PROXY` when set, even empty, else `no_proxy`, split at commas and
not trimmed. The environment snapshot holds the eight names. An https call is
unaffected: the proxy sees only the tunnel.

## Consequences

An operator with a proxy variable and a local sidecar has to name the sidecar in
`NO_PROXY`. The refusal says so. Suffix and prefix patterns that ureq honours are
not honoured here, so Larry refuses more than ureq would proxy and never less. A
non-UTF-8 proxy variable refuses the door, as every other setting does.

Name an IPv6 host with its brackets. ureq 3.5.1 matches the list against the
host as the URL writes it, so `::1` does not name `[::1]`. Measured through the
binary (larry#615): an https route to `[::1]` with `NO_PROXY=::1` goes to the
proxy, and with `NO_PROXY=[::1]` it does not. Larry refuses the plaintext call
on the first list and the refusal names `[::1]`.

Rejected: a hand written HTTP client for loopback calls. It is a second client
beside `lgwks_std`. When logicalworks-crates#1169 lands a proxy setting, the
client turns the proxy off for loopback and this refusal goes. That is also the
measurement that reverses this decision: the journey reaching the gateway with
`HTTP_PROXY` set and no `NO_PROXY`.
