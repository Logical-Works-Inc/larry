---
type: ADR
title: "ADR 0057: the temp-path rule reads names, not text"
description: Why larry governance follows a split let, a use alias, a shadow and a macro through parsed tokens, and why the walk never follows a symlink.
resource: /docs/adr/0057-temp-path-token-rule.md
tags: [larry, adr, governance, tests, syntax]
generated: {by: claude-code, at: 2026-11-17}
verified: {by: claude-code, method: larry-oracle all on the larry#293 branch, 2026-11-17}
status: superseded
stale_after: 2027-02-17
sources: [larry#293, larry#267, The Rust Reference path expressions and use declarations, CWE-59, /docs/adr/0051-temp-path-guard.md]
---

# 0057: the temp-path rule reads names, not text

Status: superseded by ADR 0061, 2026-11-17 (larry#293): the hand-written token reader it chose
passed five audited forms and read only the root's `src` and `tests`; the rule now parses with the
estate reader `lgwks_ast`.

## Context

ADR 0051 refused a process id in a test temp path by matching source text. The audit of the
DeepSeek-built PRs found four ways the text rule passed a real path:

- `let pid = std::process::id();` on one line and a later `env::temp_dir().join(...)` that names
  `pid`, because the old statement window held only one line.
- `use std::process;` then `process::id()`, because the rule looked for the literal
  `std::process::id()`.
- `use std::env::temp_dir;` then `temp_dir()`, for the same reason.
- A symlink cycle under `src/` or `tests/`, because the walk tested `Path::is_dir`, which follows
  the link.

A rule that matches text cannot tell a binding from a lookalike, so a fixture could name a temp
directory with the process id and pass the gate.

## Decision

- One token reader, `governance::tokens`, reduces Rust source to names, `::`, punctuation, string
  literals and other literals. Comments, lifetimes and char literals are skipped; a string's
  content stays one token, so data never becomes code.
- The rule resolves the names a file binds before it reads a statement:
  - `use` trees, including `as` aliases and `{...}` groups, map `std::process`, `std::env`, `id`
    and `temp_dir` to the local names they give.
  - A `let` of a plain name binds it as the process id when its initializer calls one, otherwise
    as something else; a scope stack makes a later `let` shadow an earlier one and an inner block
    drop its bindings.
  - A statement that calls the temp directory and names the process id, by call or by a bound
    name, is refused at the temp call's line. Inside a macro, a `{name}` capture of a bound name
    counts; a string literal's own text does not.
- The walk uses `DirEntry::file_type`, skips every symlink and refuses nesting past 64 levels by
  name.
- The allowlist stays the two guards of ADR 0051.

## Consequences

- The four passes the audit found are refusals, and a shadowed name, a string literal that looks
  like code and the guard route still pass.
- The rule owns a lexer of its own. It is bounded (one file, 2 MiB; one walk, 64 levels) and adds
  no dependency, so the cold build and the dependency count do not move.
- The rule is a conservative approximation, not a compiler: a pattern binding, a method call that
  returns the process id, or a path assembled across a struct field is out of its reach. Those
  remain the reviewer's question, and the guard's own test is the control that refusals still fire.

## Rejected

- More text patterns over lines: a binding, an alias or a split statement still passes.
- `lgwks_ast` or `syn`: an exact syntax tree, but it adds a dependency edge, an approval row and a
  grammar subtree to the cold build, which this branch may not do.
- Following symlinks with a depth or inode limit: a cycle is answered by not following it, which
  is also what CWE-59 asks of a reader that does not own the tree.
