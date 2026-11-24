---
type: ADR
title: "ADR 0104: The Loco boundary, measured on loco-rs 1.3.1"
description: Why Larry keeps Loco out of the default build after the 2026-11-24 deps audit, what each Loco feature costs on Larry's tree, and why the one-SQLite-engine rule no longer blocks Loco's db-sqlite.
resource: /docs/adr/0104-loco-boundary-measured-on-1-3-1.md
tags: [larry, adr, dependencies, loco, sqlite, estate]
generated: {by: claude-code, at: 2026-11-24}
verified: {by: claude-code, method: cargo generate-lockfile and cargo tree on scratch manifests with Larry's estate set, 2026-11-24}
status: accepted
stale_after: 2027-05-24
sources: [larry#433, larry#91, logicalworks-crates#849, logicalworks-crates#848, logicalworks-crates#1138, logicalworks-crates#1139, logicalworks-crates#1140]
---

# 0104: The Loco boundary, measured on loco-rs 1.3.1

Status: accepted, 2026-11-24 (larry#433). Extends ADR 0021 and ADR 0004.

## Context

- The Director's rule (2026-10-24): the estate owns runtime, JSON, hashing and time; Loco fills web,
  ORM, jobs, mail and auth where the estate owns nothing.
- Larry's tree is 209 crate lines (`cargo tree -e normal --prefix none | sort -u | wc -l`) on
  lgwks_std 5.0.0, lgwks_bot 10.0.2, lgwks_deps 8.0.0, lgwks_ast 6.1.1, crossterm and unicode-width.
- Loco 1.3.1 on that set, defaults off: no feature 344, `auth` 360, `mailer` 359, `worker` with
  Postgres 388, `with-db` with SQLite 416, `with-db` with Postgres 429.
- `loco-rs[db-sqlite]` now reaches `sqlx-sqlite` 0.9.1, which links `libsqlite3-sys` 0.40, the same
  crate `lgwks_deps[sqlite]` reaches through rusqlite 0.43. The clash in crates#849 is gone.

## Decision

- Larry ships no Loco today. No Larry journey reaches a Loco capability yet: the edge is
  `lgwks_bot::app`, the record is the journal, and the ledger is `lgwks_deps` SQLite.
- Approval mail (#91) is the first Loco row. It lands behind a cargo feature, never in the default
  build: `mailer` alone adds 150 crates, more than half again Larry's whole tree.
- When a Loco table lands, it takes `db-sqlite` on the ledger's own engine, one `libsqlite3-sys`.
  A bump that splits the two is refused at resolve, not answered with a second engine.
- The rows Loco may fill stay as ADR 0021 and `docs/dependencies/cargo-sweep.md` set them: mail,
  side jobs, operational tables rebuilt from the journal, human sign-in. Loco never serves a route,
  dispatches an effect, authorizes or holds authority.

Rejected: Loco `with-db` and `auth` in the default build now, to close the framework axis on paper.
It adds 151 to 220 crates and no journey calls it. Rejected too: Postgres through sqlx beside the
ledger's SQLite. It is a second store for one product state.

## Consequences

- The larger cuts are in the estate: `edge` pulls the bevy ECS world (crates#1138, about 50 crates),
  `lang-rust` pulls ast-grep's matcher and walker (crates#1139, 21), and `kernel` lacks
  `KernelError` and the `app` stop types (crates#1140).
- Reverse this when #91 ships its falsifier on Loco's mailer and the feature build stays under the
  60 s cold-build budget, or when an estate crate ships a mailer, which then replaces Loco's.
