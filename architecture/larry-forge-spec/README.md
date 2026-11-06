---
type: Specification
title: "Larry / Forge: source atlas and churn-resistant machine specs"
description: "Reviewable proposal, diagrams, schemas and executable specification checks."
resource: /architecture/larry-forge-spec/README.md
tags: [larry, forge, architecture, machine-specification]
generated: {by: ChatGPT, at: 2026-09-16}
verified: {by: ChatGPT, method: "Pinned source inspection; package checks recorded separately"}
status: proposed-not-ratified-runtime-not-run
stale_after: 2026-10-16
sources:
  - "https://github.com/srinji-kaggss/Larry/pull/1"
  - "https://github.com/srinji-kaggss/forge-harness/blob/11e9bbc2f3de9fa97d38acf47736628e530cff20/Cargo.toml"
---

# Larry / Forge — source atlas and machine specifications

This package refines Larry PR #1 with source-grounded implementation contracts intended to prevent architectural churn. **Larry PR #1 remains the product canon.** The package adds machine-focused contract detail; it does not ratify implementation or claim runtime readiness.

## The finding that determines the rest

In the inspected Forge snapshot, the CLI/TUI run path still builds `HarnessRuntime` from a file explicitly marked deprecated. The newer `RunCoordinator` path exists separately, with a `printf` execution backend and output-string protocol decisions. A replacement's type names, documentation and local tests therefore cannot establish that users execute through its intended semantics. The source evidence and limits are in `docs/01-source-review.md`.

The proposed remedy is not another broad framework. Freeze owner boundaries, request/effect identity, atomic admission, resource conservation, cancellation settlement, evidence semantics and change compatibility. Require every replacement to reach the real user route and pass the counterexample that motivated it.

## Pinned review

| Repository | Reviewed revision | Scope |
|---|---|---|
| Larry | PR #1 head `82f0172cb10300005524e6e7bcd6386a1c7cb86f` | Product specification, governance, instructions, log and research ledger |
| forge-harness | `11e9bbc2f3de9fa97d38acf47736628e530cff20` | Targeted current code; vendored `sdk/` is authoritative at this snapshot |

This is bounded source inspection, not a complete repository or compiler CFG audit.

## Package map

| Path | Purpose |
|---|---|
| `docs/01-source-review.md` | Source observations, inferred churn mechanisms and explicit coverage |
| `docs/02-contracts.md` | MC01–MC28 contracts with canonical references |
| `docs/03-protocol-and-proof.md` | Identity types, atomic boundaries, state predicates and proof hierarchy |
| `docs/04-fleet-efficiency.md` | Capacity equations, client/fleet targets, weak-model and cost acceptance |
| `docs/05-change-and-delivery.md` | Machine change packets, promotion evidence and C00–C12 sequence |
| `docs/06-evolution-envelope.md` | Semantic waist for adopting future breakthrough mechanisms without rewrites |
| `machine/` | Authoring JSON: grouped contract families, findings, graphs and protocol schema |
| `examples/` | Non-authorizing wire records and PLANNED change packet |
| `tools/` | Package validators and bounded reference-model tests |

## Machine representation note

`docs/02-contracts.md` contains **28 human-readable MC contracts**. `machine/contracts.json` contains **9 grouped machine contract families** that organize those obligations for tooling. These counts describe two representations, not conflicting inventories.

## Validation

The supplied package previously ran 36 local specification checks plus bounded reference-model interleavings. Those results establish package consistency only. They do **not** establish Forge runtime correctness, crash durability, sibling integration, hyperscale capacity or production readiness.

## Non-claims and preserved holds

No runtime implementation is authorized by this package. Preserve PR #1's R01–R70, D01–D35, E01–E54, C00–C12, A1–A9 and P1–P8 inventories. Preserve Sentinel, Secure-Authority, Logical-DB, Nova, Moo and Logicalworks-Crates ownership. **Memory implementation remains held until Logical-DB is ready.**

The governing principle is: **stabilize semantics once; permit mechanisms to change aggressively behind explicit contracts.**
