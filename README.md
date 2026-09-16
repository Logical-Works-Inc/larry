---
type: Overview
title: Rocco
description: Private documentation repository for a continuity-first agent runtime.
resource: /README.md
tags: [rocco, specification, agent-runtime]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: repository bootstrap; not runtime verification}
status: documentation-only
stale_after: 2026-10-15
sources: [Director decisions in conversation 2026-09-15]
---

# Rocco

Rocco is the working name for a persistent assistant with disposable workers,
explicit authority, accountable effects and measurable learning. This private
repository holds its specification, not an implemented or validated runtime.

Start with the [specification index](index.md), the [visual specification](specification.md)
and [source review](research-ledger.md). Repository rules are in [AGENTS.md](AGENTS.md)
and [GOVERNANCE.md](GOVERNANCE.md); accepted directions and changes are in [log.md](log.md).
No application, model, service, hook or benchmark runner is installed by this repository.

## Deterministic repository checks

Requires Python 3.11+ and its standard library only:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/check_governance.py
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests -v
git diff --check
```

The checker emits the current inventory and source digests; CI refreshes its
report on every push/PR. It verifies specification structure, not product behavior.
Tests deliberately corrupt in-memory document fixtures to prove rejection. They
do not launch models or modify the specification.

The Director selected core and TUI first, bounded unattended work, adapters rather
than compatibility clones, and user-adjustable memory decay whose implementation
waits for Logical-DB readiness. Phone/app and website remain required surfaces.

## Change evidence

Requirement: establish the explicitly requested private Rocco repository without
claiming implementation. Applicable axes: all nine remain product obligations;
decoupling prevents this bootstrap from introducing another runtime authority.
Predicted outcome: one named specification repository. Verification: Git and
GitHub visibility/branch/PR inspection plus document validation. Unresolved:
architecture approval and every runtime acceptance result. No Rust changed.
