---
type: ADR
title: "ADR 0015: Over-size documents are indexes over parts, and the checker enforces 120 lines"
description: Why specification.md, log.md and research-ledger.md became indexes whose parts the checker reads as one document.
resource: /docs/adr/0015-documents-split-under-the-line-law.md
tags: [larry, adr, governance, size-law]
generated: {by: claude-code, at: 2026-11-07}
verified: {by: claude-code, method: larry governance on the split tree prints the unsplit counts; governance controls}
status: accepted
stale_after: 2027-02-07
sources: [larry#53, larry#155, Director size law 2026-10-10, governance 1.2 scope on larry#53 (2026-11-03)]
---

# 0015: Over-size documents are indexes over parts, and the checker enforces 120 lines

Status: accepted, 2026-11-07 (larry#155, step 6 of larry#53)

## Context

Seven governed documents broke the 120-line law. specification.md had 1,581 lines, log.md 1,221 and
research-ledger.md 589; four documents under `docs/` held 154 to 298 each. `larry governance` reads the
first three by name, so they were frozen, and nothing enforced the law on the rest.

## Decision

- Each over-size document keeps its path as an index: its frontmatter, its opening section and a numbered
  list of parts. The parts live under `docs/<name>/` and are split at heading boundaries, so no table or
  fence is cut. Every part carries the original frontmatter, with its own title, description and resource.
- `documents::SPLIT` names the three documents the checker reads by name. `Documents::whole` returns the
  index followed by each part's body (frontmatter removed), in load order. Every whole-document rule
  reads that text: inventories, requirement mapping, cells, log drift, crash cases, diagrams, proof IDs
  and tracker drift. Frontmatter, fences, links and references are still checked file by file.
- Moved text links root-absolute (`/docs/...`), which works in the checker and on GitHub. A fragment
  link whose heading moved now names the part that holds it.
- The checker now enforces the law itself: any governed document over 120 lines is refused with its count.

## Consequences

- The split tree prints the unsplit counts: axes 9, decisions 53, execution cells 13, falsifiers 80,
  requirements 96.
- The controls edit whichever file of a whole document holds the text, so each error names that file.
  `sim_the_line_law_holds_at_its_boundary_for_every_document` passes at 120 lines and refuses 121.
- The freeze is lifted: a new entry goes into the last part, or into a new part when that one is full.
