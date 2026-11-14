---
type: Overview
title: Larry
description: Larry, the agent harness from Logical Works that asks before it acts and accounts for every effect. Public alpha.
resource: /README.md
tags: [larry, specification, agent-runtime]
generated: {by: claude-code, at: 2026-11-09}
verified: {by: claude-code, method: repository read and src/bin/larry.rs subcommands 2026-11-09; not runtime verification}
status: public-alpha
stale_after: 2026-12-09
sources: [Director orders 2026-09-15 to 2026-11-09]
---

# Larry

> **ALPHA. ZERO SECURITY GUARANTEES. ZERO CONTRIBUTIONS.**
> Larry is alpha-stage software from Logical Works Inc. It carries no security guarantee of any kind: do not
> point it at data, accounts, credentials or machines you cannot afford to lose, and do not expose `larry mcp`
> to an untrusted network or peer. Interfaces, wire formats and stored data change without notice. This
> repository accepts no contributions: outside pull requests and issues are closed unread. Report a
> vulnerability privately as `SECURITY.md` describes.

Larry is an always-on agent harness: a persistent assistant with disposable workers, explicit authority,
accountable effects and measurable learning. Every effect passes one journal: a durable intent before the call,
a recorded authorization, a fence re-check at dispatch and a receipt after it. Replay is bit-identical, tenants
never share state, and cancellation stops new work. A question to a person survives a restart, and its
expiry is never consent.

## What is here

- `crates/larry-contracts`: the pure typed contracts (commands, events, effects, authority, replay).
- `crates/larry-runtime`: the headless journal-authoritative runtime on `lgwks_bot` (clock, journal file, fence,
  gate, retry), with falsifier and seeded simulation suites.
- `larry`: the workspace demo (fixture data; nothing executes), and the executor's doors:
  - `larry mcp`: a stateless MCP 2026-07-28 server. Its `larry_submit_task` tool runs a typed task through the
    executor and reports success only when the sealed run is accepted (ADR 0008).
  - `larry acp`: an Agent Client Protocol agent, the way Harbor runs Terminal-Bench 2.0. Each prompt is a terminal
    session, which runs only under `LARRY_POLICY` naming `model.command` (ADR 0035).
  - `larry exec`: runs one task document in a fresh clone. `acceptance` is a list of argv lists. First run:
    1. Set `LARRY_STATE_DIR` to a directory you own.
    2. Pipe a task to `larry exec > report.json`, where the task is
       `{"repo":"<path>","base":"HEAD","acceptance":[["cargo","test"]]}`.
    3. Run `larry verify < report.json`.
  - `larry verify` and `larry key`: check a sealed receipt offline (ADR 0006).
  - `larry model`: sends one completion through the configured gateway.
  - `larry route`: `add`, `list`, `remove`, `default` and `resolve` the routes a tenant owns: where its model
    calls go, which `LARRY_KEY_*` variable holds the key, and the country where the provider processes the data (ADR 0039).
  - `larry eval`: scores one model arm over a matched task set, such as `eval/matched-v1.json` (ADR 0009).
  - `larry serve`: the HTTP host on `LARRY_SERVE_ADDR` (default `127.0.0.1:8787`) over `LARRY_STATE_DIR`;
    `larry serve key <tenant>` prints a tenant's bearer key once, `GET /healthz` is open liveness,
    `GET /t/{tenant}/healthz` the keyed tenant probe, and SIGTERM drains it. A non-loopback bind is refused
    unless `LARRY_SERVE_PLAINTEXT=1`, `larry serve revoke <tenant>` revokes a key, and startup refuses past
    65,536 keys (ADR 0021, ADR 0022, ADR 0040).
  - `larry governance`, `larry tracker` and `larry mascot`: the repository's own document, tracker and
    mascot checks (ADR 0010, ADR 0014).

## Install

```sh
cargo install --git https://github.com/Logical-Works-Inc/larry --tag v0.1.0 --locked
```
`larry mcp` reads `LARRY_STATE_DIR` and a route: add one per tenant, for example a local 9router gateway:

```sh
larry route add local --url http://127.0.0.1:20128/v1 --model my-model --key-env LARRY_KEY_ROUTER --residency CA --default
```
A route names its key's variable, which must start with `LARRY_KEY_`; export the key there and it is never stored.
Or export `LARRY_MODEL_URL`, `LARRY_MODEL` and `LARRY_MODEL_KEY` together as the route `env`. With neither, a task
with a goal is refused as `executor_not_connected`.

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
cargo run --locked --bin larry
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
