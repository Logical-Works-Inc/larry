---
type: ADR
title: "ADR 0110: Every error enum derives Display, Error and From from lgwks_macros"
description: Why Larry's error enums derive their impls from the estate's lgwks_macros::Error instead of hand-writing them, and which three forms stay hand-written until the derive takes them.
resource: /docs/adr/0110-errors-derive-from-lgwks-macros.md
tags: [larry, adr, errors, dependencies, lgwks_macros]
generated: {by: claude-code, at: 2026-11-24}
verified: {by: claude-code, method: cargo nextest run --workspace --locked on toronto, 1336 of 1336 passed, 2026-11-24}
status: accepted
stale_after: 2027-05-24
sources: [larry#435, larry#339, logicalworks-crates#1151]
---

# 0110: Every error enum derives Display, Error and From from lgwks_macros

Status: accepted, 2026-11-24 (larry#435).

## Context

- Larry hand-wrote `Display`, `Error` and `From` for its error enums in all three members: a second implementation of what the estate owns.
- `lgwks_macros::Error` (2.1.4) derives all three (`#[error]`, `#[from]`, `#[source]`, `#[error(transparent)]`). It was already locked through `lgwks_bot`.
- The cargo sweep's reason to refuse it ("a `#[from]` field is not the source") is false: lgwks_ast 6.1.1 `error_derive/parse.rs:205-207` makes every `#[from]` field the source.

## Decision

- `lgwks_macros` is a workspace dependency with `default-features = false`. The root crate, `larry-contracts` and `larry-runtime` each declare it. The lock gains edges only, no package.
- Every error enum derives from it, and every message is byte-identical.
- Three forms stay hand-written, each named in `REFUSED` in `tests/sim_error_derive.rs`: `RouteError::Damaged` and `RerankError::ReceiptPath` (an `Option` source), and `mutants::Fault::Read` (the source sits inside a field that is not an `Error`). The gap is logicalworks-crates#1151.

Rejected: `thiserror`. It is a second derive beside the estate's, and larry#339 removed it.

## Consequences

- `sim_no_error_enum_keeps_a_hand_written_impl` fails on any hand-written `Error` or `Display` impl outside `REFUSED`.
- Some sim and local-CI error enums now report a source where they reported none before.
- Reverse a `REFUSED` entry when crates#1151 ships the form it needs.
