---
type: ADR
title: "ADR 0077: a question keeps its title and first command row, and a key typed before it never answers"
description: Why the asking block is cropped by its middle, why Enter starts at No, and why a key read before the question's frame is dropped.
resource: /docs/adr/0077-a-question-keeps-its-title-and-a-typed-key-never-answers-it.md
tags: [larry, adr, tui, approval, frame]
generated: {by: claude-code, at: 2026-11-21}
verified: {by: claude-code, method: larry#325 simulations and PTY journey failing on main, then green, 2026-11-21}
status: accepted
stale_after: 2027-02-21
sources: [larry#325, /INVARIANTS.md, /docs/adr/0076-tui-invisible-characters-and-transcript-bound.md]
---

# 0077: a question keeps its title and first command row, and a key typed before it never answers

Status: accepted, 2026-11-21 (larry#325).

## Context

The frame drew its bottom block and, when it did not fit, skipped its first
rows. For a question that meant the title and the head of the command were
cut, and the person read the command from its tail without the keys that say
what to press.

`Event::Ask` also opened with `Decision::Run`, so Enter alone ran the command.
And a key read from the terminal sat in the driver's queue while the model
worked; when the question arrived, that key could answer it, so a key typed
before anything was shown could consent to a command nobody had read.

## Decision

- An asking block is fit to the screen by its middle: the first rows (the
  title and the command's first row) stay, the last rows stay, and one
  `… N rows cut …` row between them names every row the cut took. A screen
  too short for the head keeps the head's own first rows.
- The narrow screen's one-line diff summary is not painted while a question is
  open: the question's title and first command row outrank it.
- `Event::Ask` opens with `Decision::Decline`; Enter confirms the highlighted
  answer, which is No until the person moves it.
- The app counts the frame that painted the question. A key read while the
  question is open but before that frame answers nothing: it is dropped and
  counted, and the footer says how many keys were ignored. A key read from
  then on answers as before.
- The question's title, answers, marker and fit live in `src/tui/ask.rs`; the
  state machine in `src/tui/app.rs` calls them. `app.rs` stood at the size law
  line, and the question's presentation is one responsibility.

## Consequences

- The person always sees what is asked and how to answer, however long the
  command.
- A long command is read from its head; the exact number of hidden rows is on
  screen, never a silent crop.
- Enter alone runs nothing. A key typed while the model worked can never
  answer a question the person has not seen.
- A key dropped this way costs one footer line and no other state; the next
  key after the paint answers normally.

## Rejected

- Letting the person scroll the question. It adds a scroll mode and a second
  owner of the answer keys for a block that is usually four rows.
- Cutting the command's tail instead of its middle. The tail hides the first
  row, which is the one the model wrote first and the one the person needs.
- Swallowing every key while a question is open. The question must be
  answerable; only the keys read before its frame are refused.
- Flushing the terminal queue in the driver. The queue is crossterm's, and the
  rule belongs to the machine that paints the question, not to one driver.

Reversal: if the terminal stack can stamp an input with the frame it was read
at, the app compares that stamp and drops the counter.
