---
type: Overview
title: Larry
description: Larry, the agent harness from Logical Works that asks before it acts and accounts for every effect. Public alpha.
resource: /README.md
tags: [larry, specification, agent-runtime]
generated: {by: claude-code, at: 2026-11-06}
verified: {by: claude-code, method: repository read 2026-11-06; not runtime verification}
status: public-alpha
stale_after: 2026-12-06
sources: [Director orders 2026-09-15 to 2026-11-06]
---

# Larry

> **ALPHA. ZERO SECURITY GUARANTEES. ZERO CONTRIBUTIONS.**
> Larry is alpha-stage software from Logical Works Inc. It carries no security guarantee of any kind: do not
> point it at data, accounts, credentials or machines you cannot afford to lose, and do not expose `rocco mcp`
> to an untrusted network or peer. Interfaces, wire formats and stored data change without notice. This
> repository accepts no contributions: outside pull requests and issues are closed unread. Report a
> vulnerability privately as [SECURITY.md](SECURITY.md) describes.

Larry is an always-on agent harness: a persistent assistant with disposable workers, explicit authority,
accountable effects and measurable learning. Every effect passes one journal: a durable intent before the call,
a recorded authorization, a fence re-check at dispatch and a receipt after it. Replay is bit-identical, tenants
never share state, and cancellation stops new work. The `rocco-*` crate and `rocco` binary names are Larry's
until they are renamed.

## What is here

- `crates/rocco-contracts`: the pure typed contracts (commands, events, effects, authority, replay).
- `crates/rocco-runtime`: the headless journal-authoritative runtime with falsifier and seeded simulation suites.
- `rocco`: the workspace demo (fixture data; nothing executes) and `rocco mcp`, a stateless MCP 2026-07-28
  server whose `larry_submit_task` tool validates a typed task and answers `executor_not_connected` until the
  executor lands.

Start with the [product plan](/docs/product/README.md), the [specification index](index.md), the
[visual specification](specification.md) and the [source review](research-ledger.md). Working rules are in
[AGENTS.md](AGENTS.md) and [GOVERNANCE.md](GOVERNANCE.md); runtime rules with their tests are in INVARIANTS.md;
accepted directions up to 2026-10-13 are in [log.md](log.md). The structure map lives under [docs/](/docs/README.md).

## License

Larry is licensed under the Hippocratic License 3.0 (HL3-FULL, every module) plus Logical Works Inc.'s
Additional Ethical and Legal Conditions, both in LICENSE. The additional conditions bar, among other things,
stalkerware and coercive control, impersonation, the practices banned by Article 5 of the EU AI Act, weapons
targeting, unconsented sensitive inference, spending past a principal's budget and stripping Larry's
authority, journal and receipt safeguards; they apply Canadian law. This is not an open source license.

## Try the workspace demo

```sh
cargo run --locked --bin rocco
```
Start on the Kanban board. Click a card, press its number, or select it and press Enter to open work and
request Continue. Escape returns Home with the draft intact. Chat occupies its own pane; Tab and `/files`,
`/agents`, `/changes` expose context. `/next` and `/inspect` exercise synthetic changes. `/continue` uses the
typed owner boundary; `/fixture-continue` only releases a demo feedback hold. `t` on Home or `/truth` in work
explains every simulation. Ctrl-C quits. Notes are unsent and session-only; no model is connected, so nothing
starts. The [workspace ADRs](/docs/workspace-experience.md) record the integration path.

## Checks

```sh
cargo build --bins --locked
cargo nextest run --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```
