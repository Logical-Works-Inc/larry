---
type: Log
title: log.md, part 3 of 15
description: Part 3 of /log.md, in order; the index lists every part.
resource: /docs/log/03-2026-09-22-published-estate-refresh-and.md
tags: [governance, decisions, evidence]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: explicit conversation decisions and source inspection}
status: active
stale_after: 2026-10-15
sources: [Director conversations 2026-09-15 through 2026-09-17, /specification.md, /research-ledger.md]
---
## 2026-09-22: published estate refresh and partial strict-build repair

Director direction: read the last work, load rust-skills and the anti-pattern
digest, work next, update the Logicalworks-Crates releases from Cargo, and use
applicable estate capabilities. The actual checkout was `416a8ac`, later than
the previously recorded green runtime work. Its lint/non-exhaustive sweep left
the workspace uncompilable before this change: the first `cargo check
--workspace --all-targets --locked` exited 101, including a lint-level conflict for test panic
suppression beneath `forbid`. This supersedes neither the historical receipts
below nor the still-open C02 acceptance obligations.

Cargo search and version-specific `cargo info` identified published
`lgwks_std 0.6.6`, `lgwks_deps 0.1.12`, `lgwks_bot 0.4.2`, and
`lgwks_ast 0.2.2`. The first two consumed pins and the unused bot catalog pin
are updated. `cargo update -p lgwks_std -p lgwks_deps` exited 0 and updated
only those two package versions; the lock also records the new tracing feature
closure. The MSRV remains 1.98. `lgwks-deps check .` exits 0 with one semantic
approval; no approval or lint was weakened.

Reuse assessment: UUID generation, entropy, BLAKE3 hashing, JSON/serde and
AppCUI already route through the estate and remain there on the new versions.
Deterministic collections and clock types remain Rust std. The synchronous
contract reducer needs neither the bot's asynchronous runtime nor a parser;
neither dependency is added merely to increase usage. The existing admitted
thiserror edge remains unchanged. No sibling repository was edited, no owner
adapter connected, and the Logical-DB memory hold remains in force. Broader
estate adoption and E63/E65 completion are not claimed.

The next buildable repair removes forbidden test panic/unwrap suppressions,
adds public constructors for non-exhaustive records, and replaces enum casts
in canonical identities with explicit stable tags. Regression tests pin the
same digest bytes, reject unknown wire variants, and preserve expired-authority
denial and replay divergence. Runtime consumers now use those constructors;
unknown authorization, command, event, retry-policy and disposition variants
produce typed refusals, never authorization or terminal success. The runtime's
new malformed-strategy test observes zero dispatches, authorizations and
receipts. Journal authority, stage order and BTreeMap determinism remain intact.

The demo consumer repair is **partial**. Constructor calls, missing mouse-test
imports, test error propagation and clipped rectangle bounds are repaired, but
its private documentation, layout arithmetic/casts, reference patterns and a
disallowed Vec::remove still fail the strict gate. Its new boundary tests have
not executed. The workspace must not be described as compiling or tested green.

| Traceability | Proven locally | Still open |
|---|---|---|
| C02, D01, D04, D36 | Contract construction, stable digest tags, deterministic replay and runtime refusal regressions | Full workspace acceptance, independent C02 acceptance review and proof obligations |
| E63, E65, E71 | Unsupported wire-strategy refusal; existing tenant/freshness regressions pass | Hostile-generator/display-to-dispatch acceptance and authenticated authority integration |

Final scoped receipts from the implementation runs: contract `cargo check`,
`cargo test`, strict `cargo clippy` and fmt all exit 0; 76 test executions with
defaults and 73 without defaults pass. The no-default test previously tried to
serialize unconditionally; only the JSON comparison is now feature-gated,
while semantic replay assertions still run. Runtime check, test, strict Clippy
and fmt exit 0, with 60 test executions passing. Demo test and Clippy exit 101
before tests execute. Its final Clippy run reports 371 library and 355
library-test diagnostics; counts are not a measure of progress or completeness.
The earlier whole-workspace fmt check also failed on the native import block;
that block was formatted, and all four changed demo files subsequently passed
the scoped rustfmt check. `git diff --check` exits 0. The read-only reviewer
found no actionable defect in the contract or subsequent runtime/demo diffs;
this is source review, not independent execution or full C02 acceptance.

| Axis | Evidence and limits for this partial change |
|---|---|
| Frontier | Known-root-cause compatibility repair; no superiority measurement |
| Hyperscale | No fleet/load run |
| Idiomatic | Contract/runtime strict scoped gates pass; demo remains uncompilable |
| Generalized | Default/no-default contract suites, stable-byte and unsupported-input regressions pass; demo boundary cases unexecuted |
| Decoupled | Public constructor consumers and typed unsupported-variant refusals exercised in runtime tests; full application gate remains open |
| Ephemeral | Journal-in-hand replay/recovery tests pass; no process-death durability proof |
| Portable | Local macOS ARM64 only; no other platform run |
| Multi-tenant | Existing tenant-declaration/restoration falsifiers pass; no concurrent fleet isolation measurement |
| Performance | No latency, allocation or comparative benchmark; no speed claim |

The bounded implementation stopped without a commit or PR because the workspace
is still red. Continue with `src/native.rs` and `src/native/workspace.rs`, then
run every lane in AGENTS.md, including actual CLI and PTY journeys against newly
built binaries. Do not reuse stale binaries as verification. Pre-exec receipt:
`tools/receipts/20260922-estate-build-pre-exec.json`; post-exec receipt:
`tools/receipts/20260922-estate-build-post-exec.json`. One attempted test-print
patch was blocked by the library-print rule, corrected by removing those prints,
and reapplied without bypass; compile guard reports also remained visible.
