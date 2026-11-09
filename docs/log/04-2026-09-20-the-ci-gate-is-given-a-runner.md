---
type: Log
title: log.md, part 4 of 15
description: Part 4 of /log.md, in order; the index lists every part.
resource: /docs/log/04-2026-09-20-the-ci-gate-is-given-a-runner.md
tags: [governance, decisions, evidence]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: explicit conversation decisions and source inspection}
status: active
stale_after: 2026-10-15
sources: [Director conversations 2026-09-15 through 2026-09-17, /specification.md, /research-ledger.md]
---
## 2026-09-20: the CI gate is given a runner — it had never once run

Director direction: "LOCAL RUNNER". Both workflows requested `ubuntu-latest`, and
no run in this repository's history had ever been assigned a runner: 61 workflow
runs are recorded, all 61 concluded failure, and not one executed a step. The
jobs were created and completed in about two seconds with an empty `steps` array
while the job log returned `BlobNotFound`. Actions is enabled and the repository
is private, so this reads as an account-level runner or spending-limit
condition — not a defect in the workflow text, and not something the repository
can repair from inside.

The estate already answers this. Every sibling repository runs a self-hosted
runner on this machine, each installed as a user LaunchAgent under
`~/Library/LaunchAgents`, and their workflows ask for it with
`runs-on: [self-hosted, macOS, ARM64]`. Larry was the only repository without
one. A runner is now registered to this repository as `rocco-macos-arm64`
(labels `self-hosted, macOS, ARM64` plus the repository label), installed the
same way, and both workflows ask for it.

Semantic change, stated rather than hidden: the lane now proves the workspace on
macOS/ARM64. It does not claim Linux coverage — it never had any, because it
never ran. This is a limit, not a gain.

Two commands were also aligned with the standard they appear to enforce. The rust
job ran `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo test --workspace` — no `--locked`, no `--all-targets` — so it checked a
different thing from the lane set AGENTS.md declares and `.wwfd/local-ci.sh`
mirrors. That divergence was invisible while the job could not start, and would
become a false green the moment it could, so both commands now carry `--locked`
and `--all-targets`. This tightens the gate; nothing was loosened, and no check
was disabled to make a failure disappear.

Affected IDs: none added. R01–R96, D01–D53, E01–E80, C00–C12 and A1–A9 are
unchanged; the inventory stays 96/53/80/13/9. This is delivery tooling, not a
requirement, a decision or a falsifier, and it is not recorded as one.

Evidence: `bash .wwfd/local-ci.sh` at `e0a576f` — every declared lane passed and
the receipt names that commit; `cargo test --workspace --all-targets --locked`
164 passed, 0 failed; `lgwks-deps check .` exit 0 with no new edge;
`python3 tools/check_governance.py` PASS on 96/53/80/13/9; 55 python tests OK.
The runner reports `online` with labels `self-hosted, macOS, ARM64,
rocco-macos-arm64`, read from `gh api repos/.../actions/runners`. Reranker
receipts `tools/receipts/20260920-ci-local-runner-{pre,post}-exec.json`.

Unresolved, and not claimed: the workflow still runs a **subset** of the lanes
AGENTS.md declares. `cargo build --bins --locked`, `lgwks-deps check .`,
`tools/test_demo_cli.py`, `tools/test_native_cli.py` and `git diff --check` run in
the local mirror and not in CI, so "CI green" is weaker than "local-CI green" and
must not be read as the latter. The runner is one machine sharing this host with
the development environment: it is not a fleet, not a second platform, and not
portable evidence. Hosted runners remain unavailable and were not pursued. The
delivery protocol's "unprotected target: stop at a verified open PR" line is
unchanged by this entry; whether it should be amended is the Director's call.
