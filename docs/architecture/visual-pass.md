---
type: Design
title: Native AppCUI visual pass and fixture evidence
description: Native terminal interactions, explicit simulation inventory and published estate consumption.
resource: /docs/architecture/visual-pass.md
tags: [larry, demo, appcui, fixtures, evidence]
generated: {by: agent:rust-coder, at: 2026-09-17}
updated: {by: agent:claude-code, at: 2026-09-18, note: Forge terminal language pass}
verified: {by: agent:rust-coder, method: source inspection; execution receipts in log}
status: forge-look-implemented-visual-review-pending
stale_after: 2026-10-18
sources: [Director continuation 2026-09-17, /specification.md, /log.md, https://github.com/gdt050579/AppCUI-rs, https://crates.io/api/v1/crates/lgwks_std/0.6.3, https://crates.io/api/v1/crates/lgwks_deps]
---

# Scope and literal status

This is native pass 1 for D19/D21, R24/R31/R40 and E20/E22, with broader
product acceptance still unverified. `src/lib.rs` owns fixture transitions;
`src/theme.rs` owns the colour language; `src/native.rs` paints and handles
events through AppCUI InputApp, and `src/bin/larry.rs` launches the native
terminal. `src/main.rs` retains the separately labeled JSON-lines fixture
driver.
The published `lgwks_std = =0.6.3`, feature `json`, serializes every
JSON view; published `lgwks_deps = =0.1.8` supplies the actual AppCUI renderer
and input loop. Neither is an unused dependency or documentation-only adoption.
The repository is its own Cargo workspace so it cannot inherit the unrelated
home-directory evaluation workspace. There is no third-party approval invented
in the Larry register; the foundation is self-exempt in the inspected gate.

## Parts

1. [Launch and interaction](/docs/architecture/visual-pass/01-launch-and-interaction.md)
2. [Simulation and future integration inventory](/docs/architecture/visual-pass/02-simulation-and-future-integration.md)
3. [Excellence and open evidence](/docs/architecture/visual-pass/03-excellence-and-open-evidence.md)
