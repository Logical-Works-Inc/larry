---
type: ADR
title: "ADR 0028: the harness is a content-hashed value"
description: Why the harness Larry runs with is a bounded, canonical, BLAKE3-hashed value validated by a pure reducer in larry-contracts, and not a directory of files.
resource: /docs/adr/0028-harness-as-a-content-hashed-value.md
tags: [larry, adr, contracts, harness, epic-60]
generated: {by: claude-code, at: 2026-11-10}
verified: {by: claude-code, method: nextest run of crates/larry-contracts/tests/sim_harness.rs and the workspace}
status: accepted
stale_after: 2027-02-10
sources: [larry#61, larry#60, docs/product/README.md, crates/larry-contracts/src/harness/mod.rs, docs/adr/0024-fence-on-broker.md]
---

# 0028: the harness is a content-hashed value

Status: accepted, 2026-11-10 (larry#61 part 1, epic larry#60)

## Context

Epic #60 has Larry learn its own harness. Google Harness² treats the harness as an editable file tree
that is checked on the way in. A tree on disk can change under a running agent, has no identity of its
own and says nothing about who changed it or why. Larry already makes every effect a journal fact, so
each harness version should be one too. The acceptance is a roundtrip identity over generated trees and
a refusal with a specific error for an edit off the surface.

## Decision

- The harness is `HarnessTree` in `larry-contracts::harness`: entries of eight closed kinds (system
  prompt, guardrails, memory, tool guidance, subagents, skills, scripts, plugins), each with a bound on
  entry count and bytes per entry. The bounds table lives in the module doc and is the one source.
- The canonical form is a version byte, a big-endian entry count and entries sorted by (tag, name), each
  length prefixed. `decode` refuses anything `encode` could not write: unsorted, duplicate, trailing,
  truncated, unknown tag, wrong version, bad name or oversize. Both directions are identity.
- `HarnessHash` is `ContentDigest` (BLAKE3) over a domain tag and the canonical bytes. Insertion order
  never changes it. It covers the tree only; which tenant adopted it is a separate record.
- `HarnessVersion` is the hash, the parent hash, the author (human, proposer or composer) and a bounded
  reason. The hash is derived from the tree, so a version cannot name a tree it does not have.
- `validate_edit(current, edit, policy)` is a pure reducer. A path is `<component>/<name>`; anything
  else is `OffSurface`, a bad name is `InvalidName`. Then `Forbidden`, `Reserved`, `Oversize` and
  `TooManyEntries`. The first refusal ends the edit and nothing is applied.
- Nothing reads a disk or a clock. The active harness is never read from a file at run time.

Rejected:
- A file tree on disk like Harness²: it has no identity, drifts under a run and cannot be replayed.
- A new hash type: `ContentDigest` already is the estate's BLAKE3 identity (INV-HASH-DETERMINISTIC).
- Serde JSON as the canonical form: key order, whitespace and number forms are not fixed, so one tree
  could have several hashes.

## Consequences

- Part 2 (larry#61) adopts a hash as an authorized effect through the existing fence (ADR 0024), scoped
  by tenant, with the previous harness kept adopted on any refusal. Rollback adopts an earlier hash.
- Changing a bound or the encoding changes hashes. The pinned hash in `sim_harness.rs` makes that loud,
  and a new format takes a new version byte.
- Covered by `tests/sim_harness.rs` (`sim_harness_roundtrip_is_identity_over_seeded_trees`,
  `harness_path_rules_separate_off_surface_from_invalid_names`, `harness_non_canonical_bytes_are_refused`,
  `harness_hash_is_pinned_to_a_known_answer`). The hash known answer is MEASURED on this build only;
  no second BLAKE3 implementation checked it (NOT RUN).
