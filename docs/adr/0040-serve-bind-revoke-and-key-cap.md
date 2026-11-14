---
type: ADR
title: "ADR 0040: serve refuses plaintext binds, revokes keys on SIGHUP and caps the key store"
description: Why larry serve refuses a non-loopback bind without an opt-in, revokes a tenant key on the next SIGHUP, refuses startup past the key cap, and compares an unknown tenant against a fixed dummy digest.
resource: /docs/adr/0040-serve-bind-revoke-and-key-cap.md
tags: [larry, adr, serve, auth, tenant-keys, revoke, plaintext, lgwks-bot]
generated: {by: claude-code, at: 2026-11-14}
verified: {by: claude-code, method: tests/serve_journey.rs, tests/sim_serve.rs, tests/sim_serve_hostile.rs and src/serve.rs run on Oracle, 2026-11-14}
status: accepted
stale_after: 2027-02-14
sources: [larry#210, larry#200, logicalworks-crates#1042, docs/adr/0021-serve-host-on-lgwks-bot-app.md, docs/adr/0022-tenant-keys-on-the-app-edge.md]
---

# 0040: serve refuses plaintext binds, revokes keys on SIGHUP and caps the key store

Status: accepted, 2026-11-14 (larry#210)

## Context

After ADR 0022 `larry serve` accepted any bind address, so `LARRY_SERVE_ADDR=0.0.0.0:8787` sent bearer keys
over plaintext HTTP with no warning. It had no revoke: a digest was read once at start, so a removed key
kept working until a restart. It loaded at most 65,536 rows and dropped the rest silently. It returned `401`
for an unknown tenant before the constant-time compare, so the timing showed which names held keys.

`lgwks_bot::app` 8.4.0 has no TLS (`App::serve` runs plain axum on the listener) and no per-tenant rate,
concurrency or body knob (`App::command_limits` covers commands only). Its authenticator runs on the driver,
before the body, and must do no I/O.

## Decision

- Bind: a loopback address binds as before. Any other address is refused with `ServeError::Plaintext`,
  naming the address, exit 1, unless `LARRY_SERVE_PLAINTEXT` is exactly `1`. Any other set value is refused
  with `ServeError::PlaintextFlag`, naming the value. The edge has no TLS; gap logicalworks-crates#1042
  tracks TLS and per-tenant limits. No limiter is built beside the edge.
- Revoke: `larry serve revoke <tenant>` deletes the tenant's digest from `executor.db` and prints one JSON
  line. An unkeyed tenant is refused with `ServeError::NoKey`, exit 1.
- Live reload: the authenticator reads a snapshot (`Arc<RwLock<Keys>>`). The signal watcher, which is
  joined before the serve returns, re-reads the store on SIGHUP and swaps the snapshot. Each reload prints
  one stdout line: `{"reloaded": true, "keyed_tenants": n}`, or `reloaded: false` with the cause. A store
  that cannot be read, or is past the cap, swaps in no keys (fail closed). A poisoned lock admits no one.
- Cap: a store with more than 65,536 rows refuses startup with `ServeError::KeyCap`, naming its count and
  the cap. Rows are never truncated. One revoke brings the store back to the cap and the host starts.
- Unknown tenant: `Keys::stored` returns the tenant's digest or a fixed dummy digest. `Keys::admit` runs
  one constant-time compare on either path and then requires that the tenant holds a key, so an unknown
  tenant and a known tenant with a wrong key get the same `401` by the same path.

Rejected:
- Reading executor.db on every request. The authenticator must do no I/O, as ADR 0022 already recorded.
- Restart-only revoke. It leaves a revoked key live, which is the defect.
- TLS in Larry. The estate owns serving; a second TLS stack beside the edge is a second owner.
- A Larry rate limiter. Per-tenant limits belong to the edge; they are filed as a gap.

## Consequences

- A revoke is live at the next SIGHUP and durable at the next start. A `kill -9` between the revoke and
  the signal leaves the old key live until the signal or restart; the revoke test proves the durable case.
- Off unix there is no SIGHUP, so a revoke takes effect at the next start. This ADR states it; the
  portable claim covers the unix targets CI runs.
- Non-loopback service needs the opt-in until the edge has TLS (gap logicalworks-crates#1042).
- Measured on Oracle, 2026-11-14, debug build, 1,000 keyed probes per column over fresh connections
  (`a_revoked_key_gets_401_on_its_next_request_with_no_restart`): before the revoke `acme` p50 343.406 us,
  p95 403.847 us, p99 442.288 us; after the reload `beta` p50 338.406 us, p95 397.247 us, p99 458.889 us.
  The per-request cost of the snapshot read lock is not isolated; the two columns are the whole-path
  measurement.
- ADR 0022's "takes effect at the next start" is superseded for live revoke by this ADR; its other rules stand.
