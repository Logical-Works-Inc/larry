---
type: Index
title: Larry documentation tree
description: Final repository structure; navigation over the canonical specification, ledger, log and governance documents.
resource: /docs/README.md
tags: [larry, structure, navigation, okf]
generated: {by: opencode, at: 2026-09-16}
verified: {by: opencode, method: structural gate admission and link validation; content review against canonical owners}
status: final-structure-v1.2
stale_after: 2026-12-09
updated: {by: claude-code, at: 2026-11-09, note: the ADR and split-part directories added to the layout}
sources: [Director conversation 2026-09-16, /GOVERNANCE.md, /specification.md]
---

# Larry documentation tree

This tree navigates the specification; it never competes with a canonical
owner. Authority is unchanged: /specification.md owns requirements, decisions
and acceptance; /research-ledger.md owns source claims; /log.md owns settled
directions and change evidence; /AGENTS.md and /GOVERNANCE.md own working
rules; /INVARIANTS.md owns runtime rules; /LICENSE owns the terms of use. Every document here carries the full OKF frontmatter set and is
validated by the same structural gate as the root documents.

## Layout

| Path | Purpose | Canonical anchors |
|---|---|---|
| [/docs/architecture/README.md](/docs/architecture/README.md) | Final component and runtime structure, ownership rule, state machines | Cells 02/04, D01 |
| [/docs/dependencies/README.md](/docs/dependencies/README.md) | Dependency root and sibling adapter order; Logicalworks-Crates first | D21, C01 |
| [/docs/evaluation/README.md](/docs/evaluation/README.md) | Evaluation portfolio, lanes and falsifier organization | D15/D34/D41, Cell 09/10 |
| [/docs/product/README.md](/docs/product/README.md) | Product plan v2: tracks against Copilot, Muse, Duet, Dots and Harness², reconciled with the estate 2026-11-03 | None; order of work only |
| [/docs/product/SHIP.md](/docs/product/SHIP.md) | Ship checklist: every remaining PR in order with its proof, ticked by the PR that ships it | None; state of work only |
| `/docs/adr/` | Architecture decision records 0001 to 0030, one decision each (Status, Context, Decision, Consequences) | The IDs each record names |
| `/docs/specification/`, `/docs/log/`, `/docs/research-ledger/` | The parts of /specification.md, /log.md and /research-ledger.md, read as one document each (ADR 0015) | The index document |

## Structure rules

- New documents enter only with full OKF frontmatter, unique stable IDs where
  they define inventory, and the same checker admission as root documents.
- Local links are repository-root-relative ("/specification.md"); no escape
  above the repository root, no duplicate canonical sections.
- Requirements, decisions, falsifiers and execution cells are defined exactly
  once, in /specification.md; other documents may reference those IDs but
  never redefine or renumber them.

## File-specific excellence record

Requirement: final navigable structure without competing authority. Axes:
decoupled and generalized documentation obligations. Predicted outcome: one
tree where every path resolves and every claim points at a canonical owner.
Verification: structural gate with docs-tree admission, adversarial negative
controls, root-relative link checks. Unresolved: architecture ratification and
all runtime acceptance results remain NOT RUN.
