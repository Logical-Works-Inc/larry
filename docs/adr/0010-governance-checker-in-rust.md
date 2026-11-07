---
type: ADR
title: "ADR 0010: the governance checker and the tracker are larry subcommands"
description: Why the Python checker, tracker generator and controls moved into the larry binary on lgwks_std.
resource: /docs/adr/0010-governance-checker-in-rust.md
tags: [larry, adr, governance, python-ban]
generated: {by: claude-code, at: 2026-11-07}
verified: {by: claude-code, method: cargo nextest run --workspace --locked; tests/governance_journey.rs on the real tree}
status: accepted
stale_after: 2027-02-07
sources: [larry#53, GOVERNANCE.md, tools/tracker/tracker.json, CPython re and urllib.parse, CommonMark 0.30]
---

# 0010: the governance checker and the tracker are larry subcommands

Status: accepted, 2026-11-07 (larry#53 step 1)

## Context

Python is banned in product repositories (Director 2026-10-13), yet CI and `.wwfd/local-ci.sh` still ran
`tools/check_governance.py`, `tools/tracker/generate.py` and `tests/test_governance.py`, and agents could not
run the gate locally at all.

## Decision

- Two doors on the shipped binary: `larry governance [--report] [--root DIR]` and
  `larry tracker [--check] [--root DIR]`, in `src/governance/`. Rejected: a test-only checker, which cannot
  write the tracker render or emit CI's evidence file, and a new crate, which needs the Director's name.
- The estate owns every primitive: `lgwks_std::pattern` (the `pattern` feature, adding regex and three crates
  it needs), `json`, `hash` and `seeded`. The checker's regular expressions port verbatim, because Rust's regex
  engine chooses matches leftmost-first as CPython's `re` does; the one lookahead is a hand scan.
- Every rule and error message of the Python checker is kept, in the same order, with Python's `\w`, `\s`,
  `\b`, `splitlines`, `repr` and codec texts reproduced (each matched to a reference in `sim_parsers`).
  Declared changes: document digests are BLAKE3 under the key `blake3` (lgwks_std has no SHA-256); a
  boolean is not an integer in a receipt or a tracker total; a receipt that is not an object is refused,
  where CPython crashed with a `TypeError`; OS and JSON-parser error texts are Rust's; and the NFKC
  netloc check of `urlsplit` is not ported, since every link with a netloc is already refused.
- The 45 Python controls are seeded simulations: each mutates a seeded place, not one fixed row.
- The tracker page stays byte-identical: a printer of its own reproduces `json.dumps(indent=2,
  sort_keys=True, ensure_ascii=True)` and CPython's float repr, and the committed `index.html` is the test.
- Every input is bounded (2 MiB per document, 4096 documents, 1 MiB per receipt, 4 MiB tracker data and
  template), and `larry tracker` writes a temporary file and renames it, so a kill leaves a whole page.

## Consequences

- CI's governance jobs build the binary and run both doors; the controls run with the workspace tests.
- `tests/test_rerank_local.py` is the one Python control left in CI until larry#53 step 3 ports the reranker.
- The checker still checks structure only: runtime, model and human results remain NOT RUN.
