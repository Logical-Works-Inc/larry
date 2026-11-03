---
type: Overview
title: Larry
description: Private repository for Larry, the always-on, journal-authoritative agent harness of Logic OS.
resource: /README.md
tags: [larry, specification, agent-runtime]
generated: {by: claude-code, at: 2026-11-03}
verified: {by: claude-code, method: repository read 2026-11-03; not runtime verification}
status: specification-plus-runtime-slice
stale_after: 2026-12-03
sources: [Director orders 2026-09-15 to 2026-11-03]
---

# Larry

Larry (`Logical-Works-Inc/larry`, formerly Rocco) is the always-on harness of Logic OS: a persistent
assistant with disposable workers, explicit authority, accountable effects and measurable learning. Its
counterpart is Microsoft Copilot. The name Rocco now belongs to a separate embedded harness that cannot
speak; the `rocco-*` crate and `rocco` binary names below are this repository's until they are renamed.

This private repository holds the specification, the contract crate (`crates/rocco-contracts`), the
headless runtime (`crates/rocco-runtime`) and the fixture workspace demo. The runtime has falsifier and
simulation suites; it is not yet a deployed or validated product.

Start with the [product plan](/docs/product/README.md), the [specification index](index.md), the
[visual specification](specification.md) and the [source review](research-ledger.md). Working rules are in
[AGENTS.md](AGENTS.md) and [GOVERNANCE.md](GOVERNANCE.md); runtime rules with their tests are in
INVARIANTS.md; accepted directions up to 2026-10-13 are in [log.md](log.md). The structure map lives
under [docs/](/docs/README.md).

## License

Larry is licensed under the Hippocratic License 3.0 (HL3-FULL, every module) plus Logical Works Inc.'s
Additional Ethical and Legal Conditions, both in LICENSE. The additional conditions bar, among other
things, stalkerware and coercive control, impersonation, the practices banned by Article 5 of the EU AI
Act, weapons targeting, unconsented sensitive inference, spending past a principal's budget and stripping
Larry's authority, journal and receipt safeguards; they apply Canadian law. This is not an open source
license. The repository is private and confidential; holding a copy grants no right.

## Try the workspace demo

```sh
cargo run --locked --bin rocco
```

Start on the Kanban board. Click a card, press its number, or select it and press Enter to open work and
request Continue. Escape returns Home with the draft intact. Chat occupies its own pane; Tab and `/files`,
`/agents`, `/changes` expose context. `/next` and `/inspect` exercise synthetic changes. `/continue` uses
the typed owner boundary; `/fixture-continue` only releases a demo feedback hold. `t` on Home or `/truth`
in work explains every simulation. Ctrl-C quits. Notes are unsent and session-only; no model or Moo owner
is connected, so nothing actually starts. The [workspace ADRs](/docs/workspace-experience.md) record the
truthful integration path and the unfulfilled product gates.

## Checks

```sh
cargo build --bins --locked
cargo nextest run --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
lgwks-deps check .
```

The structural governance checker and its negative controls run in CI until larry#53 ports them to Rust.
They verify specification structure, not product behaviour.

The Director selected core and TUI first, bounded unattended work, adapters rather than compatibility
clones, and user-adjustable memory decay whose implementation waits for Logical-DB readiness. Phone/app
and website remain required surfaces.
