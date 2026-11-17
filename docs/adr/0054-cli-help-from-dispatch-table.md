---
type: ADR
title: "ADR 0054: help is one dispatch table, and a refusal names its door's task form"
description: Why larry help renders from one table in the binary, why a wrong-typed acceptance names the form its door takes, and why route errors carry one prefix.
resource: /docs/adr/0054-cli-help-from-dispatch-table.md
tags: [larry, adr, cli, help, errors]
generated: {by: claude-code, at: 2026-11-16}
verified: {by: claude-code, method: the diff read, and larry-oracle all on this branch, 2026-11-16}
status: accepted
stale_after: 2027-02-16
sources: [larry#228, larry#85, larry#123, larry#201, /docs/product/SHIP.md]
---

# 0054: help is one dispatch table, and a refusal names its door's task form

Status: accepted, 2026-11-16 (larry#228)

## Context

A live journey on Oracle found a first-time user could not drive the binary. `larry --help` and
`larry help` exited 1 with a DEMO line that listed the subcommands but said nothing about the
variables each reads, and no help arm existed. Two doors read a task document: `larry exec` takes
`acceptance` as argv arrays (`[["true"]]`) and `larry_submit_task` takes shell command strings.
Both answered a wrong shape with "has the wrong type", so a client that sent the other door's form
learned nothing. `larry route add` printed `route: route key-env must ...`: the route check already
opens with `route`, and the CLI added a second prefix.

## Decision

- One table. `COMMANDS` in `src/bin/larry.rs` holds every subcommand's name, one-line summary and
  the `LARRY_*` variables it reads. `help_text` renders it, and `larry help`, `larry --help` and
  `larry -h` print it to standard output and exit 0.
- The arms and the table must agree. `main`'s first match arm refuses any first word `listed` does
  not accept, and `listed` accepts exactly the table's names plus the three help spellings, so an
  arm for a word no help row names is unreachable at run time. `DISPATCHED`, beside the arms, lists
  every word an arm handles, and `tests::every_dispatch_arm_is_in_help` fails when the two lists
  differ or a row prints twice; `tests::only_listed_words_dispatch` holds the gate itself.
- A refusal names its door's form. `TaskError::AcceptanceNotArgv` prints the argv-array form with
  the example `[["true"]]`; the MCP tool's `WrongType` carries the form its call site declares and
  prints `acceptance must be an array of shell command strings ... such as ["cargo nextest run --locked"]`.
- One prefix. `RouteCliError::Route` prints its cause without a second `route:` when the cause
  already opens with `route` or names the row; a cause that names neither, such as `NoRoute` or a
  full table, keeps the CLI's one `route:` prefix.

## Consequences

- Adding a subcommand is two edits in one file, the match arm and the table row, and one test list
  entry. An arm whose row is missing never runs, and the test names the mismatch, so the second edit
  cannot be forgotten quietly.
- The unknown-argument refusal points at `larry help` instead of listing every word, so the list
  has one home.
- The env names in help are text, not a runtime read. `larry help` needs no state directory and
  reads no variable, so it answers on any host.
- A client that sends the other door's acceptance shape gets an actionable refusal rather than a
  type error, and no request is sent.

## Rejected

- A hand-written help string: the arms and the text become two lists, which is the drift the issue
  found.
- A clap-style parser crate: a new dependency and a rewrite of every arm's argument handling for a
  twelve-word CLI, against the lean axis and the SD-5 cost rule.
- Keeping the route prefix on one variant only: the doubled word lives in the CLI layer, so the CLI
  layer is where it is dropped.
