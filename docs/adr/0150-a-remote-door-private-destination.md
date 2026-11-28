---
type: ADR
title: "ADR 0150: a remote door refuses a stored route to a private destination"
description: "An MCP or ACP task on a stored route fails typed, before the key is read, when the route host is a private or reserved address literal, a numeric spelling of one, or a local name."
resource: /docs/adr/0150-a-remote-door-private-destination.md
tags: [larry, adr, model, route, ssrf]
generated: {by: claude-code, at: 2026-11-28}
verified: {by: claude-code, method: lgwks-box cargo nextest run, 2026-11-28}
status: accepted
stale_after: 2027-05-28
sources: [larry#627, larry#510, larry#588, logicalworks-crates#1170, INVARIANTS.md]
---

## Status

Accepted.

## Context

A route is tenant data. An MCP or ACP task made the host dial the host that the
route names. A route to `169.254.169.254` or to a local name sent the request and
the route key into the host's own network (CWE-918).

## Decision

`model::route_transport::is_private_destination` is the one address rule. It
refuses:

- an IPv4 literal that is loopback, private (RFC 1918), CGNAT (RFC 6598),
  link-local, multicast, documentation, benchmarking or reserved;
- an IPv6 literal outside global unicast, with a mapped or 6to4 form judged by
  the IPv4 address inside it;
- a numeric spelling of an address, such as `2852039166` or `0x7f000001`;
- a single-label name, and a name under `localhost`, `local`, `internal` or
  `home.arpa`.

`Settings::model_config` applies it when the door is remote and the route is
stored. It returns `RouteError::PrivateDestination` before the key is read. The
local door and the operator's `env` route are not judged.

## What this does not do

The rule reads the host as written. A public name that resolves to an inward
address passes, and so does DNS rebinding. That needs a check on the connected
address, and `lgwks_std::http` has no connect or resolver hook
(logicalworks-crates#1170). larry#510 stays open for it.

## Rejected

- Resolve the name before the send and judge the answer. The client resolves
  again, so a second answer passes. A check that can be passed reads as a
  defence and is not one.
- A direct `ureq` edge for its resolver trait. It is a new dependency and a
  second HTTP client beside the estate one.
- Judge the route at `larry route add`. The local door may use a private
  gateway, and a row stored before the rule would pass.

## Consequences

A remote task cannot use a gateway on a private network, whoever stored the
route. The operator reaches such a gateway through the local door or the `env`
route.

## Reverse when

logicalworks-crates#1170 gives a hook on the connected address, so the rule moves
there and covers names as well. Or the Director names a private gateway that a
remote door must reach.
