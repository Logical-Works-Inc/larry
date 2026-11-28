---
type: ADR
title: "ADR 0146: a pasted CR, CR LF or LF is one line break"
description: "The composer turns CR LF and a lone CR into LF before it sanitizes pasted text."
resource: /docs/adr/0146-pasted-line-breaks.md
tags: [larry, adr, tui, composer, paste]
generated: {by: claude-code, at: 2026-11-28}
verified: {by: claude-code, method: lgwks-box cargo nextest run, 2026-11-28}
status: accepted
stale_after: 2027-05-28
sources: [larry#605, larry#576, larry#239, INVARIANTS.md]
---

## Status

Accepted. Number 0146: `main` took 0143 (larry#602), larry#618 holds 0144 and
the larry#598 work holds 0145.

## Context

A terminal sends the newlines of a bracketed paste (xterm mode 2004) as carriage
returns. `Composer::insert` passed the text to `style::sanitize`, which drops a
CR as a control character. The lines of a pasted task joined. The person sent
text that was not what they pasted (larry#605).

## Decision

`Composer::insert` turns CR LF and then a lone CR into LF, then sanitizes. It is
the one place pasted text is normalized. Every other control byte is dropped as
before.

## Rejected

- Keep CR as LF inside `style::sanitize`. Command output and the approval
  question use it. A progress bar would fill the pane with lines, and a CR hidden
  in a command would no longer show as `⟨U+000D⟩` (larry#337).
- Normalize in `App::handle` on `Input::Paste`. That is a second normalizer
  beside the composer's, and a later caller of `insert` would join lines again.

## Consequences

A copied line that ends in a newline keeps it. A pasted `/quit` with that newline
is a task, not a command, as it already was for a pasted LF. The pinned frame
wire advanced once, because its seeded pastes hold a CR.

## Reverse when

A terminal in use sends a lone CR inside a bracketed paste that is not a line
break, shown by a PTY journey with its bytes.
