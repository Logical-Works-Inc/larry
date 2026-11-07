---
type: Log
title: log.md, part 12 of 15
description: Part 12 of /log.md, in order; the index lists every part.
resource: /docs/log/12-2026-09-17-native-appcui-pass.md
tags: [governance, decisions, evidence]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: explicit conversation decisions and source inspection}
status: active
stale_after: 2026-10-15
sources: [Director conversations 2026-09-15 through 2026-09-17, /specification.md, /research-ledger.md]
---
## 2026-09-17: native AppCUI pass implemented through published estate

CORRECTION: the former no-commit/human-signature blocker was invalid. The
Director clarified the delegation boundary and authorized necessary estate
admission, then explicitly selected "Authorize release continuation". The
estate optional default-off AppCUI feature and register landed in commits
4c4dcb2 and 822307d, PR #7. Real `cargo publish --locked -p lgwks_deps`
published 0.1.8; fresh download confirmed its feature. Rocco now pins published
`lgwks_deps = =0.1.8` plus `lgwks_std = =0.6.3`, with no path dependency.

`src/native.rs` and `src/bin/rocco.rs` add the real input-driven AppCUI view:
quiet default conversation, compact DEMO header/footer, optional folder drawer
and vertical parent/clone rail, manual live fixture events, selectable anchors,
diff inspection, anchored feedback/hold and remembered Continue. Input, pane
toggles and fixture changes run. No provider/job/workspace effects
are connected. All simulations and future integrations remain explicit in
`docs/architecture/visual-pass.md`; D19 now records the authorized visual scope.
AGENTS/GOVERNANCE document the additional native verification commands.

MEASURED on this Mac: 9 Rust tests passed; clippy with warnings denied passed;
4 JSON subprocess journeys and 2 real native PTY journeys passed. Two of the
six journeys induce actual owned SIGKILL and restart (33.3%). Native PTY checks
exercise drawing, typed commands, panes, hold, Continue, inspection and exit.
The actual AppCUI Surface at 120x32 was inspected; smaller-size component
renders exercised 1x1, 49x11, 50x12 and 80x24 without panic. This is not a GUI
screenshot or a physical-device/human visual study. Bounded assertion mapping
is 20/20 across component and subprocess evidence, not universal acceptance.

Fresh isolated coverage: 438/544 lines (80.51%), including 118/118 model and
320/372 native lines. Binary entry paths are not included in instrumented tests;
real PTY/CLI tests are separately uninstrumented. Branch coverage is unmeasured.
The reused-target coverage attempt duplicated old model counters and was
replaced by an isolated target, retaining the old output. Initial native clippy
failed on a test initializer and was fixed; initial PTY quit checks timed out
because the harness did not drain output, then passed after bounded draining.
No limit was relaxed. Missing pyte was not worked around by installing a package.

Estate local gates passed 206 workspace tests, 38 selected-feature tests and
the macro doctest; hosted CI run 35281748262 passed at 822307d on the published
source, including Linux/macOS/Windows AppCUI feature lanes. Parent independent
review remains pending. Existing default-branch grid advisory
GHSA-38c5-483c-4qqp remains reported, not silently fixed outside scope.

Native UI launch: `cargo run --locked --bin rocco`. Remaining limits: no full
editor or real backend, no physical Ryzen/fleet/human/performance comparisons,
no production durable recovery or authenticated tenancy proof. Pre-exec receipt
is `tools/receipts/20260917-appcui-admission-pre-exec.json`; post-exec receipt
is `tools/receipts/20260917-appcui-admission-post-exec.json`. Rocco edits remain
scoped for parent integration; estate release is real, not a dry-run claim.

## 2026-09-17: visual-pass independent fixture scaffolding

Director authorized native AppCUI visual pass 1 and explicitly required actual
Logicalworks-Crates use, then directed independent work to continue despite
the admission boundary. The initial delegation limited commits; the Director
subsequently authorized necessary estate commit/push/PR workflow. Parent owns
integration and review.
D19/D21 and R24/R31/R40 remain unchanged. The Rust fixture driver now uses
published `lgwks_std = =0.6.3` JSON for every snapshot. This is not the native
view: all missing visual work and simulated behavior is inventoried in
[visual-pass evidence](/docs/architecture/visual-pass.md).

The real gate initially walked up to an unrelated home evaluation workspace
and failed (exit 2); an explicit Rocco contract diagnosis refused its missing
register (exit 2). Rocco now has an independent Cargo workspace, lockfile and
fail-closed register with no fabricated third-party approvals. The new graph
printed `OK . — 0 semantic approvals, every authored external edge is owned`.
The published foundation is self-exempt; this verdict does not admit AppCUI.
Initial Rust verification passed six fixture tests and clippy with warnings
denied. Further final verification receipts are reported after execution.

AppCUI 0.5.1 needs an optional `lgwks_deps` storefront edge, genuine signed
admission and a new published estate version. The current published 0.1.7 has
no AppCUI feature. CORRECTION: the former claim that a forbidden commit blocked
human signoff confused delegation scope with user authority. The Director has
authorized the estate admission workflow; cryptographic human signing is not
required by the inspected policy. The original fixture-only turn changed no
sibling source; the continuation now adds the owned AppCUI feature there.
No unpublished version is consumed here. Pre-exec relevance receipt:
`tools/receipts/20260917-visual-fixtures-pre-exec.json`; two scoped candidates
ranked 1.0 using the pinned local model. Runtime/provider calls remain absent.

MEASURED: `cargo build --locked` exit 0; `python3 tools/test_demo_cli.py`
exit 0, 4/4 real subprocess journeys, including 1/4 actual SIGKILL/reset
journey with source/manifest/lock digests unchanged. Six Rust tests passed,
and `cargo llvm-cov --no-clean --all-targets --locked --show-missing-lines`
reported 115/118 library lines and 0/43 main lines (115/161 total, 71.43%).
The main driver was exercised separately without coverage instrumentation;
branch/condition coverage remains unmeasured. VF1–VF14 all have bounded
falsifying checks; this is not complete native UI or product acceptance.
Governance checker passed the unchanged 96/53/80/13/9 inventory; its Python
suite passed 45/45; fmt and diff checks exited 0. The editor's initial optional
pipe-type diagnostics in the Python crash test were corrected by asserting
the configured pipes before use; no test failure was hidden or weakened.

The final post-exec relevance receipt is
`tools/receipts/20260917-visual-fixtures-post-exec.json`. The original native
visual requirements, physical Ryzen profiles, million-user evidence, human
outcomes, matched performance comparisons, production recovery and independent
review remain unmet. Owned local `target/` build/coverage artifacts are retained
for parent verification, ignored by VCS; no resident worker or provider exists.
