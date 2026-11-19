---
type: ADR
title: "ADR 0065: the help table dispatches, and the workspace holds the estate lint floor"
description: Why each help row now carries its handler, why route refusals keep one prefix by default, and why every floor lint is at deny with no allow.
resource: /docs/adr/0065-lint-floor-and-table-dispatch.md
tags: [larry, adr, cli, lints, errors]
generated: {by: claude-code, at: 2026-11-19}
verified: {by: claude-code, method: the diff read, and clippy plus nextest on Oracle on this branch, 2026-11-19}
status: accepted
stale_after: 2027-02-19
sources: [larry#295, larry#341, larry#321, larry#228, /docs/adr/0054-cli-help-from-dispatch-table.md]
---

# 0065: the help table dispatches, and the workspace holds the estate lint floor

Status: accepted, 2026-11-19 (larry#295, larry#341). Supersedes the `DISPATCHED` list and the
match-arm dispatch of ADR 0054. ADR 0054's help, form and prefix rulings stand.

## Context

The adversarial review (larry#321) found three claims the code did not keep:
- ADR 0054 kept the help table and the match arms in step through a hand-typed `DISPATCHED`
  list. A word listed in both could still take arguments it ignored: `larry mcp --extra` served.
- `larry route remove r9` printed a cause with no `route` word. The CLI dropped its prefix for
  every variant except a short list, so a new variant lost the prefix by default.
- The settings doc said 256 pending questions was the ceiling. `validate` let any value through.

The estate guard also gained a 15-axis lint floor on 2026-11-19. Larry lacked 38 of its lints.
At `deny`, they named about 650 sites. Most were unchecked indexing in tests, `pub` items no
caller outside the crate could reach, and eight `mod.rs` files.

## Decision

- Each row of `COMMANDS` holds its handler (`run: Handler`). `main` finds the row by its word
  and calls it, so no word can dispatch without a help row. Each handler owns its arity: a
  handler that takes no arguments refuses any, through `bare`. The refusal is one `Refusal`
  enum that keeps its typed cause, in place of `Result<_, String>`.
- `RouteCliError::Route` keeps its `route:` prefix by default. Only the field refusals, whose
  text already opens with `route`, print bare. `RouteError::Unknown` became `NoSuchId`, a
  typed name rather than a catch-all.
- `Runtime::new` and `restore` refuse more than `MAX_PENDING_QUESTIONS`.
- Every floor lint is in `[workspace.lints]` at `deny`, the same level as the rest of the
  rust table. No `allow` is added. `allow_attributes` is already `forbid`, so a `deny` cannot
  be lowered from source either. Each site is fixed in the code: checked `get` in place of an
  index, `pub(crate)` in place of an unreachable `pub`, `X.rs` in place of `X/mod.rs`, and
  helpers in place of functions over 100 lines.
- `src/bin/larry.rs` passed the 750-line law. `larry governance` and `larry tracker` moved to
  `src/bin/larry/governance_cli.rs`, the module of that binary.

## Consequences

- A new subcommand is one table row with its handler, plus one test list entry. A word with
  no row cannot dispatch.
- A new route refusal says `route:` unless someone adds it to the bare list on purpose.
- An index that could panic is a compile error in every target, tests included.
- The fixes add no dependency. The clippy wall time before and after is in the PR, measured on
  Oracle.

## Rejected

- Keep `DISPATCHED` and add an arity test per word. That keeps two lists, which is the drift
  larry#228 and larry#295 both found.
- Set the floor lints to `forbid`. A derive or proc macro that emits its own `allow` would stop
  compiling. `allow_attributes = "forbid"` already blocks a hand-written `allow`.
- Land the lint floor in a later PR. The guard refuses the manifest this PR must change for the
  pin bumps until the floor is present. Clippy then refuses the commit until every site is fixed.

Reversal: a floor lint moves to `forbid` once a release builds with no macro that needs to
lower it. If the floor pushes the clippy gate past its 5-minute budget on Oracle, this ADR
reopens with that measurement.
