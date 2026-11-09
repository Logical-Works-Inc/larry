---
type: Log
title: log.md, part 10 of 15
description: Part 10 of /log.md, in order; the index lists every part.
resource: /docs/log/10-2026-09-18-one-repository-the-contract.md
tags: [governance, decisions, evidence]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: explicit conversation decisions and source inspection}
status: active
stale_after: 2026-10-15
sources: [Director conversations 2026-09-15 through 2026-09-17, /specification.md, /research-ledger.md]
---
## 2026-09-18: one repository — the contract crate merges into Larry

Director order, verbatim: "NO ITS ONE REPO. FIX IT", following "Why is it not 1
repo?". The Director authorized both paths. `srinji-kaggss/rocco-runtime` was a
second repository holding `crates/larry-contracts` (C02, 3,657 LOC across nine
modules); the crate is now a member of this repository's workspace and the
second repository is no longer part of the project.

Inventory unchanged: 96/53/80/13/9. No specification item moved or was
redefined; the merge changed where code lives, not what it must do.

ONE MANIFEST. `Cargo.toml` is now both the `larry-demo` package and the
workspace root, with `members = ["crates/larry-contracts"]`. This is one
workspace with one Cargo.lock, one lint set and one dependency register. The
root package previously declared `[workspace]` alone, and the runtime manifest
declared the same crate set from the other side; both are replaced.

ONE LINT SET — the stricter of the two. `missing_docs = "deny"`,
`unsafe_code = "forbid"`, and `clippy::unwrap_used`/`expect_used`/`panic`/
`todo`/`unimplemented`/`dbg_macro` all denied, plus `all = "warn"`. Two
consequences had to be paid rather than waived. First, `missing_docs` reached
`larry-demo`, which had never been held to it: ten `Rejected` variants, the
`larry` binary, and `tests/demo.rs` had no crate or item documentation, and
each got a real doc comment naming what it means. Second, `unwrap_used` reached
`larry-demo`'s test code. Rather than lower the lint, the two allow sites are
scoped to the code that is itself the test — the `native` test module and the
`demo` integration crate — and each states why an unwrap is the right failure
there. No production path is exempted.

Cargo refused the first lint block outright: a lint group and an individual
lint at the same priority is an error, not a warning, so `all` is pinned to
priority `-1` beneath the explicit denies.

ONE REGISTER. `contract/APPROVED.toml` merged from both. `lgwks-deps check .`
now reports **1** semantic approval where the runtime repository's register
held four rows: the checker's own ladder treats `lgwks_std` as rung 2
(workspace stdlib+, self-exempt) and `lgwks_deps` as the estate storefront, so
neither needs a row, and `lgwks_bot` is a catalog entry no member depends on.
The single real third-party edge is `thiserror =2.0.20` for
`larry-contracts`, registered in the schema the tool enforces — with
`tier`, `capability`, `source`, `allowed_consumers`, `allowed_kinds` and a
sign-off naming the Director's own admission commit
(`rocco-runtime@53c7512`, 2026-09-16) rather than a freshly invented approval.

Two findings worth keeping. The runtime register was **never schema-checked**:
that repository's gate list ran only `cargo fmt`/`clippy`/`test`, so its rows
carried `features` and `default_features` keys this schema does not have and
the merge is the first time the register was validated at all. And
`lgwks_deps` moved `=0.1.7` → `=0.1.8`, because 0.1.8 is the published release
carrying the `appcui` feature the native surface builds against.

The `larry-contracts` manifest declared `rust-version = "1.85"`. That was
never satisfiable: its own dependency `lgwks_std 0.6.3` declares
`rust-version = "1.98"`. The merge sets one workspace floor of 1.98 and thereby
corrects a false claim rather than raising a real requirement.

Resolver moved 2 → 3, matching edition 2024 on the `larry-demo` side; the
runtime workspace had pinned 2.

ONE CI. `.github/workflows/ci.yml` now carries both former workflows as two
jobs, step for step — the Rust lanes widened to `--workspace`, and the
governance lanes with their pinned action SHAs and artifact upload unchanged.
Two conventions coexist in the file (tag refs in one job, SHA pins in the
other) because merging did not change either.

MEASURED on this Mac after the merge: `cargo test --workspace --all-targets`
passes 82 tests (57 from `larry-contracts`, 19 from `larry_demo`, 6 integration)
— the contracts suite runs here, in this workspace, not in a second repository.
`cargo clippy --workspace --all-targets --locked -- -D warnings` clean;
`cargo fmt --all -- --check` clean; `lgwks-deps check .` OK; 45 governance unit
tests pass; governance checker PASS; 2 native PTY and 4 JSON-driver journeys
pass. `lgwks-deps freshness` reports 151 packages checked, 41 stale, 0 errors —
every *authored* pin (lgwks_std, lgwks_deps, thiserror, blake2/thiserror-impl)
is current; the 41 are transitive platform crates under AppCUI.

`docs/dependencies/ADAPTER-MANIFEST.md` described the second repository and a
register read at its commit `1154959`, including version and key names that no
longer match. Its section is rewritten and retitled against a fresh read and a
real `lgwks-deps check .` run; the correction is marked in the document rather
than silently edited.

Limits: the merged workspace's Linux CI has never run. AppCUI's Linux closure
is pure-Rust in the lockfile as far as this machine can see, but this machine
is a Mac, and hosted CI for this repository was already unresolved (see the
GitHub verification boundary below) — so "it will build on ubuntu-latest" is
NOT RUN, not implied. The second repository's git history is not folded into
this one: the files were copied, not `filter-repo`'d, so the crate's commits
live only in the remote `srinji-kaggss/rocco-runtime`. That remote was not
touched, per the standing instruction to leave it alone without explicit
confirmation. Coverage is not re-measured for the merged workspace.
