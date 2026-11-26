---
type: ADR
title: "ADR 0122: Docs and scorecards match the shipped run"
description: Larry's dependency, probe and eval documents describe the pins and paths that the binary ships.
resource: /docs/adr/0122-docs-match-code.md
tags: [larry, adr, docs, eval, healthz, measurements]
generated: {by: codex, at: 2026-11-26}
verified: {by: codex, method: targeted box tests and larry governance}
status: accepted
stale_after: 2027-05-26
sources: [larry#217, larry#70, larry#89, larry#90, larry#91]
---

# 0122: Docs and scorecards match the shipped run

Status: accepted, 2026-11-26 (larry#217).

## Context

- The dependency sweep carried measurements from older estate pins and described Loco rows in the present tense.
- `/healthz` exposed the package version on an unauthenticated route.
- The matched set covered only single-file eval work. Its scorecard did not name peak resident memory.
- At the shipped pins, the box printed 217 normal crates and a fresh-target build of 124.94 s with peak RSS 682,440 KiB.

## Decision

- Measure the workspace crate count and a fresh `cargo build --bins --locked` on the build box. The 2026-11-26
  command printed `217` and `elapsed_seconds=124.94 peak_rss_kib=682440` with the box cache enabled.
- Keep `/healthz` public and status-only. Version discovery stays out of the unauthenticated probe.
- Add `session-path` with the `larry acp` path in its brief and `multi-file` with two writable files to `matched-v1`.
- The eval card prints `peak_rss_kib` from Linux `VmHWM`. It prints `null` where the standard library has no equivalent.
- Loco capabilities remain planned: #70 owns the host rows, #89 human sign-in and tenancy, #90 operational tables,
  and #91 mail and side jobs.

Rejected: a new memory crate or `/usr/bin/time` inside the binary. The first adds a dependency for one metric. The
second measures a wrapper rather than the shipped eval process. Linux's proc status is bounded and has no new edge.

## Proof

- `cargo tree --workspace --locked -e normal --prefix none | sort -u | wc -l` prints the shipped crate count.
- `/usr/bin/time -f 'elapsed_seconds=%e peak_rss_kib=%M' cargo build --bins --locked` prints the fresh build numbers.
- The full commands were run through `lgwks-box` on Toronto and are recorded in `docs/dependencies/cargo-sweep.md`.
- `serve::tests::health_probe_omits_version` and the matched eval journey exercise the changed run paths.
- `larry governance` checks the document links, inventories and line bounds.

## Consequences

- An unauthenticated health check reveals liveness and service identity, but not the package version.
- Eval cards carry a Linux resident-memory measurement and a truthful null on unsupported hosts.
- The matched set now exercises both the ACP session brief and a two-file write.
- The dependency table distinguishes measured shipped pins from historical rows and names the owner issue for every planned Loco row.
