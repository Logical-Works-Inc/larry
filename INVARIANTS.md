# Larry invariants

One line per rule: the rule, why it holds, the test that enforces it. A correction replaces its line.
Changing a runtime rule is a specification change, never a refactor.

## Runtime (crates/rocco-runtime)

- The journal is authoritative; state is a fold over recorded facts and every projection is discardable and re-derivable. Why: a live-state read as truth cannot be replayed or audited. Test: `tests/sim_replay.rs`, `tests/sim_crash_restore.rs`.
- Nothing is dispatched without a durable intent, a recorded authorization and current freshness; the intent is written before the owner is called. Why: an effect without a prior intent is unaccountable. Test: `tests/lifecycle.rs`, `tests/sim_lifecycle.rs`.
- `EventKind` is closed and has no "dispatched" fact; a crash inside the dispatch window stays ambiguous. Why: a convenience fact would let recovery publish a receipt for an effect that never ran. Test: `tests/sim_crash_restore.rs`.
- The dispatch fence re-checks security epoch, withdrawn operations and the offline exposure bound after the reservation is durable and before the owner is reached; it gates mutating reaches only. Why: a recorded decision is not a standing entitlement. Test: `tests/fence.rs`, `tests/sim_fence.rs`, `tests/sim_epochs.rs`.
- A status answer resolves only an effect the runtime actually sent; the live effect table is the witness. Why: without it a status reply can publish success for an effect that provably never happened. Test: `tests/e63_forged_claims.rs`.
- Replay is bit-identical; each authorization carries its own pinned snapshots and a divergence surfaces as a finding. Why: a silent replay pass proves nothing. Test: `tests/determinism.rs`, `tests/sim_replay.rs`.
- Decisions read no wall clock and iterate only ordered collections; the fold never runs in parallel; the fence holds counters and sets, no duration. Why: elapsed time or hash order would break replay and extend authority. Test: `tests/determinism.rs`, `tests/sim_budget.rs`.
- Duplicate and replayed inputs are idempotent under the same key. Why: retries must not double an effect. Test: `tests/sim_duplicates.rs`.
- Tenants never share state, budgets or receipts. Why: one principal's agent must not act on another's data. Test: `tests/sim_multitenant.rs`.
- Cancellation stops new work and releases owned resources. Why: the stop control in LICENSE Part B.4.3 must be real. Test: `tests/sim_cancel.rs`.
- Hostile or malformed input is refused with a typed error, never a panic. Why: the harness reads untrusted text all day. Test: `tests/hostile_input.rs`.
- Test doubles never ship: `owner::fixture` compiles only for this crate's tests or under the `fixture` feature, and only the crate's own dev-dependency turns it on. Why: a scripted owner resolves to `None` and would bypass binding in a real build (larry#68). Test: `owner::fixture::tests::an_exhausted_script_reports_unknown_never_success`, `cargo build --bins --locked`.
- Each capability has one estate owner; a module moves onto lgwks_bot row by row per larry#68 and the Larry copy is deleted in the same change. Why: kernel absorption, one owner per capability (province#4, #7). Test: review-only, owner Director, 2026-11-03, until larry#68 closes; each row's PR shows that row's suites green before and after on the same seeds, and the first row that deletes a Larry module adds the test that refuses its return.

## Contracts (crates/rocco-contracts)

- `rocco-contracts` owns no state, reads no clock and performs no I/O. Why: the algebra must stay pure to be replayable. Test: `crates/rocco-contracts/tests/construction.rs`.

## Workspace and dependencies

- One workspace, one lockfile, one lint set; no second repository. Why: the split runtime repo drifted and was deleted (2026-10-26). Test: `cargo metadata --locked`.
- Every lgwks_* pin is the newest release. Why: estate rule ESTATE-LATEST (2026-11-03). Test: rust-guard ESTATE-LATEST at Edit and pre-commit.
- A third-party crate cites an open `gap` issue on Logical-Works-Inc/logicalworks-crates on its manifest line. Why: every dependency is later replaced by the estate. Test: rust-guard GAP.
- One SQLite engine per binary: Loco's sqlx path, and `lgwks_deps/sqlite` stays off until logicalworks-crates#849 closes. Why: both link `sqlite3` at different `libsqlite3-sys` majors (docs/dependencies/cargo-sweep.md). Test: `cargo metadata --locked` refuses to resolve both.
- Loco serves and authenticates and never dispatches an effect, authorizes, or holds authority; its tables are projections the journal rebuilds. Why: a Loco job or row beside the journal is a second record (larry#70). Test: review-only, owner Director, 2026-11-03, until larry#70 lands its two-tenant kill -9 falsifier.
- `unsafe_code` is forbidden and clippy lints are `forbid`, never lowered. Why: a lowered lint hides the failure it exists to catch. Test: `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- The license is HL3-FULL plus the Licensor's Part B in `LICENSE`; the manifest says `LicenseRef-Larry-HL3-FULL`. Why: Director order 2026-11-03. Test: `cargo metadata --locked` license field.

## Product

- Synthetic data stays visibly DEMO and orchestration is never simulated. Why: a demo must not become evidence. Test: `tests/demo.rs`, `tests/e2e_journey.rs`.
- No write leaves the gate without an intent, an authority record and a receipt, for two tenants under kill -9 and replay. Why: the accountable-effect promise is the product. Test: `crates/rocco-runtime/tests/sim_crash_restore.rs`, `crates/rocco-runtime/tests/sim_multitenant.rs`.
- Memory waits for Logical-DB; no substitute store. Why: state that outlives a run lives in the estate database. Test: review-only, owner Director, 2026-11-03, until Logical-DB ships the store.
