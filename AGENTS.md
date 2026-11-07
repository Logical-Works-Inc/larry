---
type: Instructions
title: Larry repository execution contract
description: Working rules for humans and agents in Larry, reconciled with the estate rulings to 2026-11-03.
resource: /AGENTS.md
tags: [governance, agents, larry]
generated: {by: claude-code, at: 2026-11-03}
verified: {by: claude-code, method: estate rulings in ~/.codex/AGENTS.md, province program issues and logic-os-kernel#850}
status: active
stale_after: 2026-12-03
sources: [Director orders 2026-09-15 to 2026-11-03, /GOVERNANCE.md, /specification.md, /docs/product/README.md]
---

# Larry execution contract

`~/.codex/AGENTS.md` governs every session here; this file adds what is specific to Larry. A newer Director
order beats every older line, including lines in this repository. Read this file, GOVERNANCE.md,
INVARIANTS.md and the plan before the first edit.

## What Larry is

- Larry (`Logical-Works-Inc/larry`, public alpha, formerly Rocco) is the always-on harness organ of Logic OS, which
  replaces Microsoft's whole stack by 2027-03-21. Larry answers Microsoft Copilot (Copilot, Copilot Studio,
  Agent 365): an agent that works for a person, asks before it acts and can account for every effect.
- Rocco names a separate embedded harness that cannot speak (the in-app dock); nothing in this repository is
  Rocco. Every crate, binary and document here says Larry (Director 2026-11-06: "Rename Rocco to Larry", larry#131).
- The plan is [the product plan](/docs/product/README.md); the next PR is the first open line of its
  [ship checklist](/docs/product/SHIP.md). specification.md owns every requirement, decision, falsifier,
  cell and axis ID; INVARIANTS.md holds the runtime rules; LICENSE is HL3-FULL plus Part B, not open source.

## One owner per capability

- Logicalworks-Crates is the root of every dependency chain. `lgwks_bot` owns the durable journal, the ECS
  world and the app SDK (`lgwks_bot::app`); Larry's own journal and fold move onto it (plan, Track 1).
- Secure-Authority owns capabilities and authority; Logical-DB owns state, sync and the memory store;
  Nova owns sandboxed code execution; Moo owns workspace claims and the revenue-slice journey (moo#184);
  Sentinel keeps its owner; Drishti owns the UI stack; logical_ci owns verdicts.
- logic-os-kernel is deprecated by absorption (logic-os-kernel#850). Larry never depends on it.
- Memory implementation waits for Logical-DB readiness: no substitute store, no pretend adapter.
- MatrAIx is one evaluation source; never modify it or pass simulated satisfaction off as human results.

## Finding and deciding

- Locate code with `codegraph`, recall estate memory with `wwfd query`, read with the Read tool.
- Before an owned edit: `design-gate.mjs research` then `design-gate.mjs plan` from the repository root.
- The local reranker mandate (`tools/rerank_local.py`, pre/post receipts) is retired by the Python ban
  (2026-10-13). Retained receipts stay as history; no new receipt is required, and `larry governance`
  still checks that they are well formed.
- Answer implementation questions from the repository's own text before asking the Director.

## Dependencies

- `lgwks_*` crates are exact-pinned to the newest release (rust-guard ESTATE-LATEST).
- Any reproducible, advisory-free OSS crate is allowed when its manifest line cites an open `gap` issue on
  `Logical-Works-Inc/logicalworks-crates` and the gates stay under 5 minutes (rust-guard GAP). Its row in
  `contract/APPROVED.toml` names that issue. Loco.rs fills web, ORM, jobs, mail and auth gaps; the
  owner of every capability is set in `docs/dependencies/cargo-sweep.md`, and a new edge updates it.
- New workspace members or crates of ours need the Director's name.
- A broken upstream edge gets a pin plus an issue (logicalworks-crates#841: appcui_proc_macro =0.3.6).

## Gates

Run from the repository root, one cargo suite on the machine at a time, each under 5 minutes:

```sh
cargo build --bins --locked
cargo nextest run --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
target/debug/larry governance && target/debug/larry tracker --check   # `larry tracker` regenerates the page
lgwks-deps check .   # cargo install lgwks_deps --version 6.1.0 --locked --bin lgwks-deps; not in CI
git diff --check
```
The checker and tracker are Rust (ADR 0010); the PTY journeys and reranker controls stay Python, in CI only,
until larry#53 ports them; agents may not run Python locally. `cargo local-ci` mirrors CI. Each CI job
picks its runner from `github.event.repository.private`: the self-hosted Mac while private, `ubuntu-latest`
once public, never self-hosted on a public repository (larry#116). Launch the demo with
`cargo run --locked --bin larry`; fixture runs are not backend, device or fleet acceptance.

## Size law

A `.md` file holds at most 120 lines; a code file at most 750 code lines and 1000 in all. specification.md,
log.md, research-ledger.md and the other over-size documents are frozen until larry#53 splits them, because
the checker reads them by name. Until then new directions land in the plan, INVARIANTS.md, this file,
GOVERNANCE.md and GitHub issues, and log.md inventory lines move only when specification.md moves.

## Self-maintenance

1. Classify a new direction: requirement, settled decision, correction or evidence.
2. Put it in its one home: an ID in specification.md, a rule in INVARIANTS.md, an order in the plan, a
   working rule here and in GOVERNANCE.md, a task in an issue. Never create a competing instruction file.
3. Preserve stable IDs and earlier acceptance obligations; supersession names the Director's order.
4. The shipping PR ticks its ship-checklist line; tracker data moves with a cell or falsifier (CI checks it).
5. Each change carries a traceability line: IDs addressed, what is proven, what is NOT RUN or ESTIMATE.

## Tests and evidence

- At least half the tests are seeded deterministic simulations (`sim_*`): one seed drives time, I/O and
  randomness; same seed, same trace hash. No sleeps, no unseeded randomness, no `#[ignore]`.
- Each new API gets an adversarial or negative case. Tests drive the shipped binary through its public
  interface under kill -9, slow owners and full disk, and report p50/p95/p99 and peak RSS.
- Label claims READ, DOC, TARGET, ESTIMATE, MEASURED, UNKNOWN or NOT RUN. A diagram, checker pass, review
  or receipt is not runtime proof.
- Workspace lints forbid `unsafe_code` and the panic family; never lower a lint to clear a failure.

## Delivery

- One task, one issue, one branch, one PR (Director 2026-11-03: "work based on each pr carries GH issues").
  Planned work is filed as issues sized to one PR, as sub-issues of the order they serve; the PR body says
  `Closes #N` (a keyword only in a commit does not link the PR, and any "close #N" left in a branch commit
  message links that issue too: write issue lists without closing verbs). Commit only on
  `rust-guard: GO COMMIT` with a design-gate plan trailer.
- `main` has no branch protection (read 2026-11-03), so the endpoint is a merged PR on a green CI run, then
  local and remote in sync. Merge each falsifier PR as it goes green; stacked PRs left open conflict.
- Subagents only on the Director's explicit order; a subagent never merges its own PR.
- Another running session on this repo is a peer: message it and agree the order; never overwrite its work.

Prohibited: claiming done without the gate output; reaching into other repositories without an order;
weakening lints, gates or pins to make a failure disappear; installing host hooks or daemons from here;
publishing anything from this repository without the Director naming it.
