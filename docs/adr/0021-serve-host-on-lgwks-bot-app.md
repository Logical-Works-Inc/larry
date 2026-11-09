---
type: ADR
title: "ADR 0021: larry serve runs on lgwks_bot's app edge, not Loco"
description: Why the HTTP host is lgwks_bot::app::App::serve in the default binary, and where Loco still fills a gap.
resource: /docs/adr/0021-serve-host-on-lgwks-bot-app.md
tags: [larry, adr, serve, http, lgwks-bot, loco, track-2]
generated: {by: claude-code, at: 2026-11-08}
verified: {by: claude-code, method: cold builds of main and the branch, full nextest run, live larry serve}
status: accepted
stale_after: 2027-02-08
sources: [larry#88, larry#70, logicalworks-crates#791, logicalworks-crates#879, logicalworks-crates#880, docs/dependencies/cargo-sweep.md]
---

# 0021: larry serve runs on lgwks_bot's app edge, not Loco

Status: accepted, 2026-11-08 (larry#88)

## Context

larry#88 was written on 2026-11-03, when the estate had TCP only: Loco behind a `serve` feature, 544 crates
and 87.5 s cold. lgwks_bot 8.0.1 ships HTTP serving: `lgwks_bot::app` (crates#791) serves routes as
per-tenant cell transitions over axum through `lgwks_deps`, with bounded mailboxes, a 1 MiB body ceiling,
`503` with `Retry-After`, granted effects and parked approvals. Estate first, and code once: Loco for routes
would be a second HTTP owner beside it.

## Decision

`larry serve` builds one `App` and calls `App::serve`. It binds `LARRY_SERVE_ADDR` (default
`127.0.0.1:8787`), registers SIGTERM and SIGINT before it prints its one serving line, and drains and exits 0
on either. The edge itself answers `GET` and `HEAD /healthz` with no tenant (liveness, crates#879); the
readiness probe is `GET /t/{tenant}/healthz`, answered from the tenant's cell with a fixed body.
On lgwks_bot 8.0.1 Larry's tree is 225 crates (118 before) and the cold `cargo build --bins` is 45.97 s at
load 10.4 with 998 MB peak RSS (MEASURED 2026-11-09), under 60 s, so it is in the default binary with no
`serve` feature and every test runs in the default lane. Live, 2,000 tenant probes answered 200 at p50
0.073 ms, p95 0.140 ms, p99 0.221 ms, with the host at 13 MB RSS (debug build, curl, one connection each).

Rejected: Loco for routes (544 crates, a second edge, its YAML config); `lgwks_bot::rt::serve` for the probe
(it records every request in the run store by idempotency key, so a probe would replay its first answer).

## Consequences

- A bearer key per tenant (larry#89) goes on `App::authenticate` and `App::route_as` (crates#880, lgwks_bot 8.0.0).
- The app edge's cell log is in memory (crates#815), so #90 keeps the runtime journal as the record.
- Loco stays the answer only where the estate owns nothing: mail (#91) and, if needed, ORM tables.
