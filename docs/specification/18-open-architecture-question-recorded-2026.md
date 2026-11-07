---
type: Design
title: specification.md, part 18 of 18
description: Part 18 of /specification.md, in order; the index lists every part.
resource: /docs/specification/18-open-architecture-question-recorded-2026.md
tags: [rocco, prd, architecture, continuity, efficiency, reinforcement-learning]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: bounded source inspection and external critique; not runtime verification}
status: final-proposed-v1.2-awaiting-architecture-approval
stale_after: 2026-10-15
sources:
  - Director requirements and corrections in conversations 2026-09-15 and 2026-09-16
  - /research-ledger.md
  - /Users/srinji/.okf/governance/contract/frontier-universal-assurance.md
---
### Open architecture question recorded 2026-09-20

**Is an ECS world ever authoritative, or always a view?** The Director deferred
this on 2026-09-20 rather than settling it. It is recorded here as open, not
ratified, and no implementation settled it by accident: the implemented runtime
reads state only through a projection seam and treats the journal as
authoritative, which is the D04-literal reading and is reversible if the
question is later answered the other way. The `bevy_ecs` dependency admission
that would make an ECS backend possible is likewise pending Director sign-off
and is not an edge in the dependency register.

## File-specific excellence record

Requirements R01–R96; all nine axes. Predicted outcome: explicit decisions and
falsifiers replace repeated architectural rediscovery. Verification: requirement
IDs unique/complete, decision/evaluation links resolve, Markdown structure
inspected, source/review dispositions retained. The four specification diagrams
and the governance diagram rendered successfully with Mermaid 11.4.1 in the
agent browser; component-map and DAG screenshots were inspected. GitHub's own
renderer is not independently verified by that local-browser check.
Unresolved: approval of remaining architecture,
P1–P8, all runtime/human/performance tests. No Rust files changed; no compiler,
coverage or performance result is implied by this documentation.
