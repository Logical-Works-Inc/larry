---
type: ADR
title: "ADR 0061: the temp-path rule parses with the estate reader"
description: Why larry governance deletes its hand-written lexer and judges one function over an lgwks_ast tree.
resource: /docs/adr/0061-temp-rule-on-lgwks-ast.md
tags: [larry, adr, governance, tests, syntax]
generated: {by: claude-code, at: 2026-11-17}
verified: {by: claude-code, method: larry-oracle nextest for the rule's tests and larry-oracle clippy on the larry#293 branch, 2026-11-17}
status: accepted
stale_after: 2027-02-17
sources: [larry#293, larry#267, larry#311, /docs/adr/0057-temp-path-token-rule.md, /docs/adr/0051-temp-path-guard.md, https://docs.rs/lgwks_ast/6.0.3/, CWE-59]
---

# 0061: the temp-path rule parses with the estate reader

Status: accepted, 2026-11-17 (larry#293)

## Context

ADR 0057 replaced text matching with a hand-written token reader, `governance::tokens`. An audit of
that reader found five defects that passed a real temp path: it judged one statement, so a temp
directory bound on one line and a pid read on a later one passed; it matched a call, so a pid read
as a value passed; it left `use std::{process::{self}}` unbound and dropped a glob; it knew only
`std::env::temp_dir`; and it read `'\''` one byte short. The estate already owns a Rust reader:
`lgwks_ast` 6.0.3 parses with its tree-sitter grammar and refuses a source past its byte, node,
depth or time bound with a typed error.

## Decision

- Delete `governance::tokens`. Parse every source with `lgwks_ast::try_parse(text, Language::Rust)`
  and judge the tree. A refused file is a typed `GovernanceError` naming it (`SourceTooLarge`,
  `Source`, `SourceNotUtf8`), never a pass.
- Judge each `function_item` as one region. A function that reaches a temp source and names
  `std::process::id`, as a call or as a value, is refused at its first use.
- Temp sources: `std::env::temp_dir`, a string literal whose content starts `/tmp` or `/var/tmp`,
  `env!("TMPDIR")` and `var("TMPDIR")`.
- Resolve names through the file's `use` declarations: a plain path, an `as` alias, a `{...}` group,
  a `self` import, a glob, `crate`, `super` and an `::std` prefix all bind the same path.
- A pattern binding, a closure argument, a field or method name (`x.id()`), a shadowing local and a
  string literal's own content are not the process id.
- The rule reads the root's `src` and `tests`. Every workspace member, a missing root and following
  `#[path]` and `include!` modules are larry#311.
- The walk skips every symlink (CWE-59) and refuses nesting past 64 levels by name. A check scans
  its files on at most four lanes and collects the verdicts in walk order.
- The submodules split by responsibility: `temp_paths` the walk, the parse and the verdict order;
  `temp_paths/syntax.rs` the per-function verdicts; `temp_paths/uses.rs` the names the `use`
  declarations bind.

## Consequences

- Each of the five defects has a seeded simulation in `governance::sim_temp_paths` that fails on
  the previous rule and passes here, and each false refusal the audit named has a passing case.
- The rule is no longer a second Rust reader; the estate's grammar and bounds are the only ones in
  the tree.
- The rule parses every source instead of scanning text. On Oracle, 2026-11-17, 100 checks of this
  tree measured p50 279.5 ms, p95 282.8 ms and p99 356.1 ms; the text rule's 100 checks measured
  p50 25.545 ms and p99 25.940 ms (the draft run's measurement on the same box, before this
  branch). The 100-check control takes 28.1 s in the suite.
- The edge adds `lgwks_ast =6.0.3` (`lang-rust`, defaults off) to the default binary. The cold
  build measured on Oracle for the draft with this same edge: 230.8 s on 214 crates before, 406.7 s
  on 228 crates after, under load 5.75 to 10.55 on 4 cores. This branch re-ran the rule's tests and
  clippy on Oracle.
- The rule is still a syntax rule: a pid a function returns under another name, or a temp path
  assembled at run time, is beyond it. larry#311 widens the read to every workspace member.

## Rejected

- Keep the hand-written lexer and fix each defect: five repairs would leave a second Rust reader
  the estate must maintain, and the next syntax form would pass again.
- `syn` or `proc-macro2`: a mature parser, but a second reader beside the estate's own, and a `gap`
  issue for a capability the estate already ships.
- Parse only files whose text mentions a temp name: a parse refusal, a byte-bound refusal and a
  non-UTF-8 refusal must fire for every source, so a text prefilter would skip the refusals the
  order asks for.
