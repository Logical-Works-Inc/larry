---
type: ADR
title: "ADR 0152: no command is approved unread"
description: "A command too tall for the question scrolls, and y or a is refused until every row was painted."
resource: /docs/adr/0152-no-command-is-approved-unread.md
tags: [larry, adr, tui, approval]
generated: {by: claude-code, at: 2026-11-28}
verified: {by: claude-code, method: lgwks-box cargo nextest run, 2026-11-28}
status: accepted
stale_after: 2027-05-28
sources: [larry#521, larry#475, larry#325, larry#337, INVARIANTS.md]
---

## Status

Accepted. Number 0152: `main` took 0149 (larry#591) and 0150 (larry#627), and
an open pull request holds 0151.

## Context

The question cropped a long command to its title, first row, one "rows cut" row
and the end of the block. PgUp and PgDn scrolled only the artifact pane. A 40-row
command on an 80x24 screen could be approved with most of it never shown
(larry#521, CWE-451). The word wrap also dropped runs of spaces at a row break,
so the text on screen was not the command that runs.

## Decision

`tui::ask` owns the question's view.

- `command_rows` cuts the sanitized command by character. No space is dropped. A
  combining mark with no character before it, and a wide character on a one-cell
  row, show as `⟨U+XXXX⟩`.
- A command too tall for the screen is a window. Up, Down, PgUp, PgDn, Home and
  End move it. One counted `… N rows cut …` row says how many rows are unread.
- The paint records which rows it showed. `y`, `a` and Enter on Yes are refused
  with a visible line until every row was painted. `n` and Esc always decline.
- A screen with no room for a command row never approves it.
- Once a command is read, a resize does not unread it. Before that, a new width
  numbers the rows again and the count starts over.
- A command that fits leaves PgUp and PgDn to the artifact pane, as before.

## Rejected

- Show the whole command in the artifact pane. The pane is absent below 108
  columns, so an 80x24 screen would still approve unread.
- Keep the crop and add a pager screen. That is a second owner of the question
  and of its answer keys.
- Keep the word wrap. It drops spaces, so 2,000 spaces and a second command
  after them drew as no rows at all.

## Consequences

A long command takes extra keys before a yes. While one is on screen, PgUp and
PgDn move the command and not the pane; Tab still changes the pane's tab. The
pinned frame wire advanced once.

## Reverse when

A journey shows a person who read a command and could not approve it, or a
command row that the terminal did not draw but the count took as read.
