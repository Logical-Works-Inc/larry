---
type: Evidence
title: docs/dependencies/ADAPTER-MANIFEST.md, part 1 of 2
description: Part 1 of /docs/dependencies/ADAPTER-MANIFEST.md, in order; the index lists every part.
resource: /docs/dependencies/adapter-manifest/01-purpose-and-authority.md
tags: [larry, adapters, licenses, dependencies, evidence, c01]
generated: {by: opencode, at: 2026-09-16}
updated: {by: agent:claude-code, at: 2026-09-18, note: one-repo consolidation corrected the consumption section}
verified: {by: opencode, method: read sibling and lgwks Cargo.toml license fields, contract/APPROVED.toml and pinned-revision presence checks, ledger receipts and specification Cell 11 allocation lines; no integration executed}
status: active
stale_after: 2026-10-18
sources: [/specification.md, /research-ledger.md, /docs/dependencies/README.md, sibling Cargo.toml manifests read 2026-09-16, contract/APPROVED.toml read 2026-09-18, `lgwks-deps check .` run 2026-09-18]
---
## Purpose and authority

This manifest is the C01 artifact required by Cell 11 of
[/specification.md](/specification.md): "exact adapter/API/source-license
manifest; reuse selected code via canonical owner" with falsifier E22 and
blockers tracked in the owner, no vendor twins. It is evidence, not authority:
[/specification.md](/specification.md) owns every requirement (R), decision
(D), falsifier (E) and execution cell (C) referenced below; /research-ledger.md
owns source claims; this manifest complements
[/docs/dependencies/README.md](/docs/dependencies/README.md) without replacing
or redefining any canonical section. Evidence labels follow /AGENTS.md:
READ = bytes inspected this session; DOC = recorded by a canonical repo
document; UNKNOWN = insufficient evidence.

## Sibling adapter manifest

All six ledger-pinned revisions were confirmed present as commits in their
local repositories this session (`git cat-file -t`, output "commit" for each);
READ-level inspection findings are quoted from
[/research-ledger.md](/research-ledger.md). Readiness is UNKNOWN throughout:
per D21, no sibling may be called production-ready from its name or README,
and no integration has been exercised.

| Order | Canonical owner | Ledger-pinned revision | Larry consumes | License evidence | Readiness |
|---|---|---|---|---|---|
| 1 | Logicalworks-Crates | `30ae69deca86eb25c95e113296eb2d08f50526a7` (local HEAD, READ) | std/bot/deps foundation, lgwks_ast parser, dependency storefront | Apache-2.0 — READ, all four `crates/*/Cargo.toml` | UNKNOWN |
| 2 | Logical-DB | `e4e1ead86042cc917e959c3f5fb2206ff61f9319` (commit present, READ) | durable records, sync, restore; memory persistence | Apache-2.0 — READ, workspace Cargo.toml | UNKNOWN |
| 3 | Secure-Authority | `ab417c9f8f664271cf5966e24c51bdc337aa4bed` (commit present, READ) | capability issue/verify (Biscuit, TTL ceiling 300s) | MIT — READ, workspace Cargo.toml plus LICENSE-MIT file | UNKNOWN |
| 4 | Nova | `34fd34aa757c4d4fce941beb92cc297f075be80c` (commit present in nova-container-runtime, READ) | compute leases, isolation profiles; NativeTrusted is not hostile-code isolation | Apache-2.0 — READ, nova-container-runtime workspace Cargo.toml | UNKNOWN |
| 5 | Moo | `a4e64252368226913f92a1453c8b435bd1efbede` (commit present, READ) | workspace claims/release_claims, editor integration; no second SQLite store | MIT — READ, workspace Cargo.toml | UNKNOWN |
| 6 | Sentinel | `a3d62f765352636a4f930d0ad3d1f222c13be9f7` (commit present, READ) | SearchProvider/MetaSearch/EgressClient egress boundary, pinned fetch | UNKNOWN at workspace root — no `license` field in sentinel Cargo.toml; crates/anydoc declares MIT (READ) | UNKNOWN |

## License audit

READ this session from `/Users/srinji/logicalworks-crates/crates/*/Cargo.toml`
(pinned revision equals local HEAD `30ae69deca86…`):

| Crate | Version | License field observed |
|---|---|---|
| lgwks_std | 0.6.3 | Apache-2.0 |
| lgwks_bot | 0.3.2 | Apache-2.0 |
| lgwks_ast | 0.1.3 | Apache-2.0 |
| lgwks_deps | 0.1.7 | Apache-2.0 |

All four versions match the controlled-dep releases named by the workspace
manifest and, where registered, by the in-repo admission gate (see In-workspace
consumption). Foundation surfaces
per D21: `std`, `lgwks_std`, `lgwks_bot`, `lgwks_deps`, with `lgwks_ast` as
the existing parser. No vendored twin, no dual engine, no copied tree; missing
sibling semantics are integration work in the sibling owner.

## Ecosystem protocol adapter contracts

- **Agent Skills — UNKNOWN, pin pending.** The ledger records the
  agentskills.io integration page as FETCH FAILED (404) with the llms.txt
  pointer retained; the exact package spec has not been inspected, and the
  ledger lists pinning it before C01 as an open follow-up. No package-format
  assumptions are made here; the skill frontmatter boundary is governed by
  E57 below.
- **MCP — pinned schema revision 2026-07-28 (DOC, ledger receipt).** The
  ledger's protocol receipt table records the MCP specification at latest
  schema `2026-07-28`: JSON-RPC with per-request capability negotiation and
  Resources/Prompts/Tools features. D37 pins one schema revision per adapter;
  elicitation and sampling receive explicit decisions (E57).
- **A2A — compatibility/refusal boundary (DOC, ledger receipt).** The ledger
  records the A2A protocol's opaque agents, agent-card discovery and tiered
  extension governance. Unmapped A2A features are declared refusals, never
  improvised behavior, per E57.

## E57 obligations and owning cells

E57 (specification.md line 1152, READ): "Malicious skill frontmatter, MCP
sampling/elicitation request mid-run, foreign A2A agent card | untrusted
format fields rejected; sampling passes the MD authority gate; unmapped
protocol features are declared refusals". Obligations:

1. Malicious Agent Skills frontmatter — untrusted format fields rejected.
2. MCP sampling/elicitation mid-run — sampling passes the MD authority gate;
   elicitation gets an explicit decision.
3. Foreign A2A agent card — unmapped protocol features are declared refusals.

Allocation verification, Cell 11 lines 1259–1264 (READ, verbatim): "Additional
case allocation: C02 owns E28/E30/E35/E47; C03 owns E31/E40/E41/E46; C04 owns
E34/E52; C05 owns E29/E33/E42/E43/E44/E51; C06 owns E37/E38/E39/E54; C07 owns
E32; C09 owns E49/E53; C10 owns E45; C12 owns E50. C04 also owns E48; C05 owns
E06/E36; **C01 also owns E57**; C02 also owns E55; C03 also owns E60; C04 also
owns E56; C05 also owns E58; C06 also owns E59; C09 also owns E61/E62." C01
therefore owns E57; the same note assigns E32 to C07 and keeps every
allocation subordinate to the rule that these allocations do not authorize
product implementation or bypass the memory hold.
