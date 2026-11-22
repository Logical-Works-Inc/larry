---
type: ADR
title: "ADR 0087: the contract errors hand-write their Display and Error impls"
description: Why Larry's eight contract error enums leave thiserror and the estate derive behind for hand-written Display, source and From impls.
resource: /docs/adr/0087-hand-written-contract-errors.md
tags: [larry, adr, dependencies, errors, thiserror]
generated: {by: claude-code, at: 2026-11-22}
verified: {by: claude-code, method: cargo nextest run --locked -p larry-contracts; cargo clippy --workspace --all-targets --locked; cargo deny check advisories}
status: accepted
stale_after: 2027-02-22
sources: [larry#339, Logical-Works-Inc/logicalworks-crates#842, https://doc.rust-lang.org/std/error/trait.Error.html]
---

# 0087: the contract errors hand-write their Display and Error impls

Status: accepted, 2026-11-22 (larry#339)

## Context

`larry-contracts` carried one third-party crate: `thiserror =2.0.21`, whose
manifest line cited Logical-Works-Inc/logicalworks-crates#842. That issue is
closed. The estate shipped its replacement, `lgwks_macros::Error` (through
`lgwks_ast`'s `error_derive`), and a closed issue is not the open gap
rust-guard GAP requires.

The estate derive reads `#[error(...)]`, `#[error(transparent)]`, `#[from]`
and `#[source]`, but it emits `source()` only for a field marked `#[source]`.
`thiserror` treats `#[from]` as implying `source`. `IdError`'s two `#[from]`
variants would silently lose their cause, and `RuntimeError::source` walks the
contract error's own source.

## Decision

Every error enum in `larry-contracts` writes `Display`, `std::error::Error`
(with `source`) and `From` by hand, matching the messages `thiserror`
generated. The enums are `error.rs`'s six (`IdError`, `TransitionError`,
`SequenceError`, `OrderingError`, `CommandError`, `AuthorityError`) and the
two in their own modules (`WorkspaceError`, `BindingError`).

- `#[from]` fields keep their cause: `IdError::Entropy` and `IdError::Parse`
  return it from `source`.
- `OrderingError::Sequence` stays transparent: `Display` forwards and
  `source` forwards the inner error's source, as `thiserror` did.
- The `thiserror` manifest line, its `contract/APPROVED.toml` row and its row
  in `docs/dependencies/cargo-sweep.md` are deleted in the same change.

The crate takes no new dependency: the estate derive was rejected because a
derive that drops a `#[from]` cause changes behaviour a caller reads.

## Consequences

- The Rust lines are hand-written boilerplate, more than the derive's input
  for these types, and a new variant needs its own `Display` arm. The
  compiler refuses a missing arm, so drift is loud.
- The `thiserror` crate stays in the lock through `ast-grep-core` and
  `bevy_ecs`; removing this edge removes the workspace's own dependency on it,
  not the crate from the tree.
- The advisory gate ignores `RUSTSEC-2024-0436` (`paste`, unmaintained, reached
  only by the optional `rerank` feature), and `heapless` moved to 0.9.4, which
  resolves its nine unsound advisories.
- Test: `cargo nextest run --locked -p larry-contracts` (78 tests) and
  `cargo clippy --workspace --all-targets --locked` pass. The messages and
  `IdError`'s cause are asserted in `crates/larry-contracts/tests/errors.rs`.

## Rejected

- Cite an open gap issue. #842 is closed because the gap was filled, so the
  only honest citation would be false.
- Keep `thiserror` and cite nothing. rust-guard GAP refuses a third-party
  manifest line without an open issue, and the closed citation is the defect
  this ADR answers.
- Derive with `lgwks_macros::Error`. It loses the `#[from]` cause, so the
  change would be behaviour, not cleanup.
