---
type: Log
title: log.md, part 1 of 15
description: Part 1 of /log.md, in order; the index lists every part.
resource: /docs/log/01-2026-09-28-estate-pins-moved-to-the.md
tags: [governance, decisions, evidence]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: explicit conversation decisions and source inspection}
status: active
stale_after: 2026-10-15
sources: [Director conversations 2026-09-15 through 2026-09-17, /specification.md, /research-ledger.md]
---
## 2026-09-28: estate pins moved to the published 0.7.0 release

The Director authorized the estate release (logicalworks-crates PR #174,
merge 9658b732). lgwks_std 0.7.0, lgwks_deps 0.1.13 and lgwks_bot 0.5.0 are
published on crates.io and tagged. Larry's exact pins move to them. lgwks_bot is
catalogued but has no consumer, so the lockfile does not change for it.

One consumer break: lgwks_std 0.7.0 removes `Uuid::from_bytes` in favour of
`impl From<[u8; 16]> for Uuid`, whose behaviour is identical. The id macro in
`larry-contracts/src/ids.rs` now calls `Uuid::from(raw)`. Larry's own
`from_bytes` API is unchanged. The lockfile gains `tracing-subscriber` and its
tree transitively through the estate crates. `lgwks-deps check .` confirms that no
new authored edge was added.

Checks: `bash .wwfd/local-ci.sh` exit 0 on every lane; stable 1.98.1 strict
Clippy clean. Receipts: `tools/receipts/20260928-estate-pins-{pre,post}-exec.json`.
No spec IDs changed and no cells moved.

## 2026-09-28: main returned to green before parallel falsifier work

Main at e9c3973 failed the strict Clippy lane: `WorkItems::items` carried a
bare `#[must_use]` on an `impl ExactSizeIterator` return, which clippy
`double_must_use` denies under `-D warnings`. The attribute is removed; the
iterator type already carries the obligation. No lint was lowered.

The local-CI span check counted unittest lines ending in `... ok`. On a host
with `FORCE_COLOR` set, Python 3.14 wraps `ok` in ANSI escapes, so the check
read 0 journeys against a green PTY suite and refused the receipt. The check's
own invocation now forces colour off; the threshold of two journeys is
unchanged.

Estate pins were checked against crates.io on 2026-09-28: lgwks_std 0.6.6,
lgwks_bot 0.4.2 and lgwks_deps 0.1.12 are the latest published releases, so no
pin moved. Estate main carries unpublished 0.6.7 / 0.5.0 / 0.1.13; the
register law forbids path or git edges, so they are NOT consumed until
published. The installed `lgwks-deps` binary was 0.1.2 and is now 0.1.12,
matching the pin.

Checks: `bash .wwfd/local-ci.sh` exit 0 across every lane (build, workspace
tests, strict Clippy, fmt, unittest, governance, lgwks-deps, demo CLI, native
PTY, diff check, span check). Receipts:
`tools/receipts/20260928-baseline-green-{pre,post}-exec.json`. Hosted CI:
NOT RUN at the time of writing. No spec ID changes; no cell moves.

## 2026-09-22: native layout hardening and full strict-gate recovery

The native AppCUI path now represents terminal dimensions, text widths, layout
shares, cursor motion, panel bounds and fixture indexing through explicit
saturating or checked operations. The repair removes float layout shares,
unchecked casts and arithmetic, stale reference patterns, an unbounded
front-removal pattern, and undocumented private drawing helpers. Narrow and
extreme terminal dimensions now clip deterministically rather than relying on
implicit conversion or arithmetic behavior. SVG review helpers use the same
checked terminal-dimension boundary.

The root line driver and native launcher retain their standard-error contract
without the forbidden print macros. The executable journey tests still observe
their failures on standard error. Native layout tests now calculate the panel
floor through the production integer share function, so the layout invariant
cannot drift from the code it verifies.

This completes the previously recorded demo strict-lint repair; it does not
complete C02 or turn the fixture into an owner integration. The workspace now
has local build/test/lint evidence, while authenticated authority, E65's
trusted display-to-dispatch binding, real owner adapters, durability, hosted
CI and independent acceptance remain open.

The commit gate identified a duplicate `RetryPolicy` name. Larry's old enum
did not own delay, backoff, deadline or jitter; it selected one of three
reconciliation actions: bounded idempotent re-dispatch, observation, or a
permanent unknown. That is not representable by `lgwks_std::retry::RetryPolicy`,
which owns the timing contract. The duplicate is replaced by
`ReconciliationAction`; a real owner adapter must compose the estate retry
budget only for its bounded re-dispatch action.

Receipts: `tools/receipts/20260922-native-layout-pre-exec.json` and
`tools/receipts/20260922-native-layout-post-exec.json`. Measured local gates:
`cargo build --bins --locked`, `cargo test --workspace --all-targets --locked`
(178 passed), `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`lgwks-deps check .`, `tools/check_governance.py` (55 Python tests), both CLI
journey suites (7 tests), `cargo fmt --all -- --check`, and `git diff --check`.
