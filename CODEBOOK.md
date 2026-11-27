---
type: Codebook
title: Larry codebook
description: Module map, owners and invariants for the Larry workspace.
resource: /CODEBOOK.md
tags: [larry, codebook, modules]
generated: {by: claude-code, at: 2026-11-07}
verified: {by: claude-code, method: read of each crate root and module declaration, 2026-11-19}
status: public-alpha
stale_after: 2026-12-06
sources: [crates/larry-contracts/src/lib.rs, crates/larry-runtime/src/lib.rs, src/lib.rs, INVARIANTS.md]
---

# Codebook

Rules live in `INVARIANTS.md` with their tests; decisions in `docs/adr/`. Each module has one owner.

## crates/larry-contracts (pure algebra; no state, clock or I/O)

| Module | Owns |
|---|---|
| `ids` | Typed identifiers and BLAKE3 content digests |
| `state` | The run-state algebra and its one transition function |
| `event` | Recorded events and per-run sequencing |
| `command` | Command envelopes and duplicate suppression |
| `effect` | Intents, reconciliation strategies, terminal receipts |
| `authority` | Grants, attenuation, boundary re-validation |
| `binding` | Resolution of an intent to its external target |
| `replay` | The pure authorization reducer and pinned replay; expiry on `lgwks_bot::skew::lease_holds`, coverage on `lgwks_bot::gate::GrantSet` (larry#77) |
| `error` | Every typed contract failure |
| `workspace` | Bounded workspace projections and the Continue boundary |
| `fixture` (test only) | Pinned instants shared by the unit tests |

## crates/larry-runtime (the deterministic tick)

| Module | Owns |
|---|---|
| `journal` | The bounded append-only authoritative record; `journal::durable` its `RecordJournal` file, group-committed (larry#75, ADR 0023) |
| `payload` | The typed bodies of the closed event vocabulary, including each outcome's dispatch count (larry#182, ADR 0036) |
| `projection` | The fold, operation-to-run and receipt indexes, effect phases, each effect's recorded dispatch count and the replay report (larry#182, larry#216) |
| `protocol` | E47 version and capability negotiation: the offer, its typed refusals |
| `gate` | Admission and the `cap::Auth` proof every mutating reach presents (larry#77, ADR 0018) |
| `fence` (private) | Dispatch-time freshness on one `lgwks_bot::broker` environment, and `effect_key`, the one key an intent derives (larry#76, ADR 0024) |
| `owner` | The effect-owner trait; each mutating call carries and re-reads the checked `cap::Auth` with its exact required capabilities; unbound support is explicit; test doubles under `fixture` only (ADR 0120) |
| `consumer` | Bounded, tenant-scoped journal catch-up with explicit resync (E35) |
| `error` | Every refused runtime operation |
| `runtime` | `TickReport`, stage 2 (admit, reserve, dispatch), indexed cancellation and per-tenant run counts, `restore`; `runtime::settings` the `RuntimeSettings` every capacity, payload ceiling, skew bound, attempt bound, protocol range and lease, with their Cell 05 defaults and the one validation `Runtime::new` and `restore` apply (larry#212, ADR 0056, larry#216) |
| `runtime::instant` | The tick's one pinned instant and the live clock's epoch anchor (`live_clock`): `lgwks_bot::clock` read as time since the Unix epoch (larry#74, #182) |
| `runtime::admission` | E28 durable admission before the acknowledgement, the pending set, safe-boundary promotion, nothing in memory before the sync (ADR 0025, ADR 0036) |
| `runtime::approval` | E30 durable questions and answers, the question book folded from the journal, the approval gate (ADR 0027) |
| `runtime::display` | E65 display and admission on the real command path |
| `runtime::ingress` | `submit_protocol`: negotiate, tenant check and bounded backoff before the queue (E47) |
| `runtime::live` | Run and effect witnesses the journal does not carry |
| `runtime::reconcile` | Stage 3 reconciliation and stage 4 cancellation |
| `runtime::reach` | The one owner reach: the warrant, the checked `cap::Auth` proof and required capabilities, the handoff re-check, owner proof read, call and count (larry#76, #77, #461) |
| `runtime::retry` | The retry decision on `lgwks_bot::retry` over `fence::effect_key` (larry#78, ADR 0017) |
| `runtime::writers` | Every journal writer and the run-state `advance` |

## Root crate (the `larry` binary)

| Module | Owns |
|---|---|
| `lib` | The effect-free DEMO session state, `replace_file` (the whole-or-nothing write) and `diagnose` (one stderr line) |
| `executor`, `executor::error` | `larry exec`: settings, the run of one task, its JSON report (ADR 0004); `ExecError`, its message and its source chain (ADR 0100); `process_env` the one environment snapshot every door reads, a non-UTF-8 variable refused by name and an unset `PATH` refused, simulated in `executor::sim_env` (larry#213, ADR 0107) |
| `executor::identity` | The identities a run derives: the state directory's tenant and worker, its run, operation and command ids, and the run id itself, a fresh UUID version 7 or the idempotency key's (larry#182, ADR 0037) |
| `executor::gate` | Every task as a displayed proposal on `larry_runtime`; `LARRY_POLICY`; the sealed refusal; the canonical owner boundary with resolved target identity, tenant check, exact owner-side `cap::Auth` read and one gated task body; a retry answered from its evidence (ADR 0019, 0037); the admission lease starts at one live clock's reading, `lease_start`, simulated in `executor::gate::sim_clock` (larry#436) |
| `executor::recovery`, `executor::sim_*` | The task journals a killed executor left: their path, liveness by the journal's own lock, and the recovery that sets each aside as `<task>.recovered` or `<task>.refused` without reaching the world; the seeded simulations of the identity, the recovery, the report rows and the sweep: `executor::sim_ledger` the sealed-report rows and the live-group rows, `executor::sim_workspace` the workspace and origin sweeps (larry#75, larry#182 defect 3, larry#214, ADR 0023, ADR 0037) |
| `executor::task`, `executor::task::confine` | The bounded task document, its optional `idempotency_key`, and its typed refusals; whether acceptance runs confined: a brief, or the MCP door's flag, because a remote peer drove the run (larry#324, ADR 0088) |
| `executor::author` | The brief (goal, files) and the model step that writes each file before acceptance, every prompt on one byte-stable prefix per brief (ADR 0008, ADR 0043) |
| `executor::run`, `executor::run::group`, `executor::run::sim_drain` | One command in its own process group: capture, timeout, the observed clip; `group` the group kill and wait, the kill repeated each slice until the leader's status arrives when `setsid` had not made the group yet (larry#442), all ending on one deadline from an injected `lgwks_bot::clock::Clock` (larry#268) and pausing on a 100 us to 10 ms backoff on a wall clock, never a yield loop (larry#427), simulated in `group::sim_group`; the output drain ends on the command's status and a `DRAIN_GRACE` read from an injected clock (larry#334); the seeded virtual-clock simulations of both bounds |
| `executor::session`, `executor::compact`, `executor::sim_refusal` | A terminal session: one gated proposal, one `command` or `done` reply per step, its stops (ADR 0035); its auto-compaction: old output cleared, old steps folded into a record, the window learned from the limit a gateway refusal names (ADR 0044); the seeded simulations of that refusal, its parse and its shrink (larry#236) |
| `executor::ledger`, `executor::ledger_routes`, `executor::routes` | The SQLite record of live groups, the reap after a kill -9, the receipt key, the per-tenant route table and residency policy (`ledger_routes`: at most 256 routes and 64 residency codes per tenant, one default, every statement names the tenant; the versioned price table `larry route price` bumps, migration 7); `larry route` and the route a task resolves to (larry#201, #203, ADR 0039, ADR 0117, ADR 0118) |
| `executor::receipt` | The ed25519 seal over a report, its verification and derived verdict (ADR 0006) |
| `executor::workspace`, `executor::workspace::snapshot`, `executor::workspace::mode` | The throwaway `git clone --local` workspace, its diff and its removal; the bounded snapshot of a person's folder and the checked `git apply` back (larry#243); the mode reset before removal, walked through held `lgwks_std::fs::capability::Dir` descriptors with a no-follow reset and one typed event per link kept (larry#334, larry#366, ADR 0082) |
| `executor::confine`, `executor::island` | The kernel wrapper (Seatbelt; `setsid` then Landlock on Linux, a host without `setsid` refused typed, larry#386, ADR 0091, the simulation in `executor::sim_confine_session`) and its probe, denying the state directory and the executor's own process entry to a command while its own copy stays readable (larry#322, ADR 0081), the Linux read walk bounded by the estate's `LandlockRules::MAX_RULES`, a file beside it granted the file rights (larry#382, ADR 0092, the simulations in `executor::sim_confine_walk`), and on macOS denying a Mach launch service, a signal outside the command's own group and the `open`, `launchctl` and `osascript` launchers (larry#326, ADR 0089, the simulations in `executor::sim_confine_macos`); a session's isolated copy, its review diff and its handoff (larry#243, ADR 0046) |
| `eval`, `eval::sim_eval` | `larry eval`: one arm over a matched set, origins, outcomes, the scorecard priced by the route's table when it declares one else by `LARRY_EVAL_PRICE_*` (ADR 0009, ADR 0118), and `EvalError::Stopped`, the partial card of an arm a fault stopped (ADR 0101); the seeded simulations of its scoring and set checks |
| `eval::set` | The bounded task set and its typed refusals, checked before any task runs |
| `eval::score` | Whole-number prices and costs, nearest-rank percentiles, the card; an incomplete cost prints `unknown` per accepted task (larry#340) |
| `model` | The blocking chat-completions client, built per task over its route; no gateway or model is compiled in (ADR 0003, 0039) |
| `model::client` | One exchange per attempt, retried within the `RetryPolicy`, its deadline read on the injected `lgwks_bot::kernel::clock::Clock` the caller passes to `ModelClient::new`, never the wall (larry#436); the bounded tenant-route token and micro-dollar budget is checked before the exchange and reported usage is priced in whole micro-dollars, with estate-seeded jitter from the tenant-route key (ADR 0118), simulated in `tests/sim_model_budget.rs` |
| `model::config`, `model::route` | Key read from the route's variable (never shown; a stored route's is `LARRY_KEY_<TAG>_*` under its own tenant's tag and no other's, refused at add and again at resolve, larry#328), timeout, retry with an unset backoff of zero like the executor that never waits, a declared route table pricing the calls else the environment overrides, and tenant limits; the `env` route; the route (id, URL, model, key variable, residency), HTTPS-or-loopback transport, the versioned price table `route price` bumps, its checks and the resolver (larry#201, ADR 0117, ADR 0118) |
| `model::wire`, `model::wire::sim` (`sim_wire.rs`) | The request body and the reply parser, cached and written tokens in every provider shape; the seeded usage round trips (larry#230, ADR 0043) |
| `tui` | Bare `larry`: the terminal guard, key mapping and the drive loop over a real session (ADR 0045) |
| `tui::app`, `tui::ask` | The pure state: keys and session events in, effects and one frame out; no terminal, no clock; the question's title, answers, highlighted marker and the fit that keeps its title and first command row (larry#325) |
| `tui::worker`, `tui::sim_tui::worker_panics` | `run_session` on its own thread; bounded event and answer channels; approve, decline, cancel, join; a thread that panics ends its task as `Failed` naming the panic (larry#412) |
| `tui::buffer`, `tui::style` | The cell buffer, its diff and DEC 2026 frame, a cell holding a character and a mark tail only a combining mark allocates, its wire pinned by a digest in `tui::sim_tui::cells` (larry#415); reset-first SGR, widths, wrap, sanitize |
| `tui::layout`, `tui::pane`, `tui::sim_tui::chat` | The phone-ratio chat column (44..64) and the artifact pane at 108+ columns; the pane's bounded Output, Diff and Receipt tabs and their paint (larry#244, ADR 0047); the chat/pane column sim reads the frame and holds that no chat text paints right of the pane's rule (larry#340) |
| `tui::cells`, `tui::composer` | Transcript entries drawn the Codex way; the multi-line composer, history and slash popup |
| `mcp` | The `larry mcp` server; `larry_submit_task` runs the executor (ADR 0008); `mcp::wire` the stdio framing and envelope; `mcp::modern` the stateless 2026-07-28 era; `mcp::legacy` the initialize lifecycle of the four earlier revisions; `mcp::tools` the one tool, `larry_submit_task` |
| `acp` | `larry acp`: the Agent Client Protocol agent Harbor drives; each prompt a terminal session (ADR 0035); `acp::tests` its protocol surface with no executor behind it; `acp::sim_acp` the seeded adversarial simulations of that surface; `session/close` frees a session's slot (ADR 0102), simulated in `acp::sim::close` |
| `serve`, `serve::error` | `larry serve`: the address, tenant keys, the snapshot a SIGHUP reloads, the routes on `lgwks_bot::app`, the stop signals; its typed refusals (ADR 0021, 0022, 0040) |
| `governance`, `governance::temp_paths`, `governance::temp_paths::syntax`, `governance::temp_paths::uses` | `larry governance`: the structural checker over the governed documents and its report (ADR 0010); the temp-path rule: a process id names a temp directory only in the two test guards (`executor::sim_tree::scratch` and `tests/support/temp_dir.rs`), refused elsewhere in test code by parsing every source with `lgwks_ast` and judging one function item at a time, through any `use` path, alias, `self` import, glob or `::std` prefix; the roots are the workspace members read from the root `Cargo.toml` (`[workspace] members`, a trailing `*` glob, and the root `[package]`), so every member's `src` and `tests` are read, a missing root is a typed refusal naming it, and `#[path]` and `include!` modules are followed and inherit the test code that names them; a reader budget is refused by name and the walk never follows a symlink; the checked parse runs on every source, the verdict pass only on a text holding `process` and a temp token and the follow pass only on one holding `#[path` or `include`, proved equal to the ungated report in `governance::sim_temp_gate` (larry#267, larry#293, larry#311, larry#415, ADR 0051, ADR 0061, ADR 0063, ADR 0109); `temp_paths::syntax` the per-function verdicts over the parsed tree, `temp_paths::uses` the names the file's `use` declarations bind (larry#293) |
| `governance::documents` | The bounded load of the top-level documents and the `docs/` tree; split documents read whole (ADR 0015) |
| `governance::markdown` | Frontmatter, fences, local links and GitHub heading ids |
| `governance::inventory` | The R, D, E, C and axis inventories, traceability, cells, log drift, crash cases |
| `governance::error` | Every refusal that stops a check, printed in the Python checker's wording |
| `governance::receipts` | The reranker receipts' shape, SHA-256 (retained) or BLAKE3 (ADR 0013) |
| `mascot`, `mascot::webp`, `mascot::sim` | `larry mascot`: the one pixel grid, its SVG, preview and pet package (ADR 0014); the lossless VP8L encoder of the pet spritesheet (RFC 9649), seeded round trips, the SVG check |
| `mutants`, `mutants::read` | `larry mutants verdict`: the mutation lane's one verdict, run by `mutants-verdict.yml` on `workflow_run` from main's build; main's floor and caps, the floor per architecture, parity and its typed fail-closed refusals; `mutants::read` the plan, outcomes, jobs and artifact directories, each file through `governance::read` (larry#387, ADR 0098) |
| `rerank` | `larry-rerank`: the request, its bounds, the ranking and the create-exclusive receipt, whole by link; its revision is the snapshot label, not a weights digest (larry#340, ADR 0013) |
| `rerank::qwen`, `rerank::sim` | The pinned Qwen3 reranker on Candle and its tokenizer, feature `rerank` only; the seeded ports |
| `governance::tracker` | `larry tracker`: the tracker page rendered from `tools/tracker/tracker.json` |
| `governance::pyjson`, `governance::pystr`, `governance::read` | JSON printed as CPython's `json.dumps`; the CPython `str` semantics the messages need; the one bounded reader every governance read calls, holding the root and every directory with `lgwks_std::fs::capability::Dir`, opening the final component no-follow and non-blocking, judging the opened descriptor (regular, within the bound) and reading through `take(limit + 1)`, so a link or FIFO swapped in after the check is refused (larry#336, larry#359, ADR 0069, ADR 0074) |
| `governance::patterns` | Every checker regex, compiled once, searched as Python's `\w`, `\s` and `\b` read |
| `governance::sim_*`, `governance::tests`, `governance::fixture` | Seeded ports of the Python controls, parser references, bounds and two-root checks; the controls over a governed fixture tree the tests write for themselves, also included by path into `tests/governance_journey.rs`; `sim_governance` mutates documents in memory only (larry#270) |
| `src/main.rs` | The demo binary entry |
| `src/bin/larry.rs`, `src/bin/larry/governance_cli.rs`, `src/bin/larry/eval_cli.rs` | The `larry` launcher: the one `COMMANDS` table each word dispatches from (ADR 0065), the split terminal, `mcp`, `acp`, `model`, `route`, `exec`, `verify`, `key`, `mascot`, `mutants`, `serve`; `governance_cli` the `governance` and `tracker` runs and their typed refusal; `eval_cli` the `eval` run, its card and its typed refusal (ADR 0101) |
| `src/bin/larry-local-ci.rs` | `cargo local-ci`: every gate in AGENTS.md order, then an atomic receipt under `--receipts`, `LARRY_STATE_DIR` or `target/local-ci`; invalid state paths are typed |
| `src/bin/larry-rerank.rs` | `larry-rerank` (feature `rerank`): one ranked request and its printed receipt |
| `eval/matched-v1.json`, `eval/errands-v1.json` | The shipped matched task set, 12 POSIX sh tasks; 12 Rust errands with hidden tests, the cache benchmark's set (larry#230) |

## Invariants the module map carries

- Only `runtime::reconcile` completes a cancellation, and only with no owned effect open (ADR 0001).
- Only `runtime::writers` appends events, and every event folds the projection after it; commands enter by `Journal::accept_command`; `tick`, `runtime::reach` and `Runtime::admit` are the only syncs. `runtime::live` witnesses are private to `runtime`; `restore` rebuilds them from the journal.
- `model` retries only 429 and 5xx and never shortens a `Retry-After` (ADR 0003).
- `executor::ledger` records a group's leader before the wait and removes it only once the group is gone, and opens and migrates `executor.db` on the store's own busy wait, with no lock of its own (ADR 0100).
- Only `executor::receipt` derives `accepted`; sealing and verifying both call it (ADR 0006).
- Shared test fixtures live in `tests/support`, `tests/sim_kit` and `tests/fence_kit`, each with a test that exercises every helper: `support/origin.rs` the journeys' origin repository and its one `git_command` builder that keeps every test git call off the host (larry#269), `support/ps.rs` the probe's `ps` RSS sample, `support/mcp_task.rs` the operator session behind `larry_submit_task`, `support/larry_cli.rs` the `larry` child, `support/stdin_feed.rs` the one stdin write that takes a broken pipe as the child having stopped reading, included by `larry_cli.rs` and `eval_arm.rs` (larry#444), `support/larry_child.rs` the long-running stdio child (`larry mcp`, `larry acp`), `support/serve_cli.rs` `larry serve`, `support/route_cli.rs` `larry route` and its state directory, `support/route_key.rs` the key variable name a tenant's tag gives (larry#328), `support/exec_cli.rs` `larry exec`, `larry verify` and `larry key` with their sealed report, `support/held_group.rs` the held command a kill -9 leaves running and the ledger row count, `support/governed_tree.rs` the governed tree copy and the one `larry` run over it (larry#340), `support/deadline.rs` one failure deadline and the event waits that use it (larry#268), `support/http_wire.rs` the bounded HTTP/1.1 framing of the mock, `support/mock_model.rs` the seeded OpenAI-compatible mock endpoint, `support/eval_arm.rs` the `larry eval` arm, its card and its origin listing (larry#399), `support/json_at.rs` the one RFC 6901 pointer read whose missing member is a typed `Missing` (also included by path into `src/lib.rs` for the unit tests, larry#341), `support/acp_agent.rs` the `larry acp` driver, `support/sealed_report.rs` the sealed report's flags and denials, `support/tree_digest.rs` the work tree's BLAKE3 digest a confinement journey compares (larry#324); the in-crate `sim_*` modules share the test-only `sim_error::SimError` (`src/sim_error.rs`), whose variants a caller can match; every seed sweep runs on the test-only `sim_lanes` (`src/sim_lanes.rs`, also included by path into `tests/sim_exec_gate.rs` and `tests/sim_model_client.rs`): at most 8 scoped lanes (cap 16) for a sweep that waits, one lane for a CPU-bound one, contiguous chunks, outputs in seed order, a panicked lane named by `LaneError` (larry#246).
