---
type: ADR
title: "ADR 0022: tenant bearer keys on the app edge's authenticator"
description: How larry serve admits a service tenant (a key shown once, its BLAKE3 digest in executor.db) and why approver sign-in is a separate PR.
resource: /docs/adr/0022-tenant-keys-on-the-app-edge.md
tags: [larry, adr, serve, auth, tenant-keys, lgwks-bot, track-2]
generated: {by: claude-code, at: 2026-11-09}
verified: {by: claude-code, method: full nextest run, live larry serve with issued keys}
status: accepted
stale_after: 2027-02-09
sources: [larry#89, larry#88, logicalworks-crates#880, RFC 6750, docs/adr/0021-serve-host-on-lgwks-bot-app.md]
---

# 0022: tenant bearer keys on the app edge's authenticator

Status: accepted, 2026-11-09 (larry#89); superseded in part by ADR 0040 (the bind, revoke and the key cap)

## Context

After larry#88, `larry serve` answered any request that named a valid tenant. larry#89 asks for a service tenant's
key that is shown once and stored only as a digest, with authentication kept apart from authorization
(`lgwks_bot::cap`, secure-authority). lgwks_bot 8.0.0 added `App::authenticate`, which judges every tenant request
before its verb, body or mailbox, and `App::route_as`, which hands a handler the `Principal` (crates#880).

## Decision

- `larry serve key <tenant>` draws 32 bytes from `lgwks_std::random` and prints `lk_` plus 64 hex characters,
  once. It stores the key's `lgwks_std::hash::blake3` digest in the state directory's owner-only `executor.db`
  (schema step 3, `tenant_keys`), under the store's existing open lock. Issuing again replaces the digest, so a
  key is rotated by issuing a new one.
- `larry serve` needs `LARRY_STATE_DIR` and reads every digest (at most 65,536) at start. Its authenticator takes
  the RFC 6750 bearer token and hashes it. It compares the result in constant time (`Digest::ct_eq`) with the
  digest stored for the tenant named in the path. A match is that tenant's `Principal`. Anything else gets the
  edge's `401` with `WWW-Authenticate: Bearer`, before a mailbox slot is spent.
- `GET /t/{tenant}/whoami` (`route_as`) returns the principal. `GET /healthz` stays the edge's open liveness probe.

Rejected:
- A second key file beside the store. It would be a second owner for state the ledger already locks and keeps
  owner-only.
- Storing keys encrypted. Larry never needs the key back, so a digest is enough and leaks nothing.
- Re-reading the store on every request. The authenticator runs on the driver and must do no I/O.

## Consequences

- A newly issued key takes effect at the next start; SIGTERM drains first, so a restart loses no request.
  Superseded for revoke by ADR 0040: a revoke takes effect at the next SIGHUP, and the store is re-read then.
- #89 stays open for approver sign-in, its human half. That will be Larry's first Loco use, behind a feature
  so the default cold build stays under 60 s (Loco measured 87.5 s in cargo-sweep); its own PR measures it.
- #90's runtime API routes are `route_as`, so every run is keyed by the authenticated tenant.
