---
type: ADR
title: "ADR 0097: Larry builds with Rust 1.100.0 on every box and in every job"
description: Why rust-toolchain.toml, the workspace rust-version and every CI toolchain step name Rust 1.100, held by one test.
resource: /docs/adr/0097-rust-1-100-everywhere.md
tags: [larry, adr, toolchain, ci, portability]
generated: {by: claude-code, at: 2026-11-23}
verified: {by: claude-code, method: the_toolchain_is_the_mandated_rust red on Oracle then green; clippy 1.100 clean on Oracle; cold builds on Oracle with 1.98.0 and 1.100.0, 2026-11-23}
status: accepted
stale_after: 2027-05-23
sources: [larry#406, logical-erp ADR 0020, NIST SP 800-53 rev. 5 CM-2]
---

# 0097: Rust 1.100.0 everywhere

Status: accepted, 2026-11-23 (larry#406). Director: "rust 1.100 upgrade mandated for all repos".

## Context

Larry had no `rust-toolchain.toml`. The Mac built it with 1.100.0, and Oracle with
its default 1.98.0. Every workflow installed `stable`, which moves on its own date.
The workspace claimed `rust-version = "1.98"`. The x86_64 build box holds only 1.100.0.

## Decision

- `rust-toolchain.toml` pins channel 1.100.0 with rustfmt and clippy, profile minimal.
- The workspace `rust-version` is 1.100.
- Every `dtolnay/rust-toolchain` step in ci.yml, governance.yml, mutants.yml and
  sim.yml installs 1.100.0.
- `the_toolchain_is_the_mandated_rust` refuses a checkout where one of them drifts.
- clippy 1.100 is clean at the unchanged lint floor (ADR 0065), with no code change.

Rejected: keep `toolchain: stable` in CI and pin only the toolchain file. rustup
then switches every step to the file's channel, so a job installs two toolchains,
and the day `stable` passes 1.100 the step installs a compiler no job uses.

## Consequences

- A toolchain move misses every cached crate once, as a `Cargo.lock` move does. The
  first run of each lane on main rebuilds its cache.
- MEASURED on Oracle (4 cores), cold `cargo build --bins --locked`, a fresh target
  and build directory, no sccache: 1.98.0 took 454 s at load 8 to 9, and 1.100.0
  took 270 s at load 4 to 5. The loads differ, so this records cost, not a speed-up.
  Both are over the 60 s budget, which is still open.
- The next move changes the four places and the test's constant in one commit.
- Falsifier: `tests/ci_fallback_job.rs` (`the_toolchain_is_the_mandated_rust`).
