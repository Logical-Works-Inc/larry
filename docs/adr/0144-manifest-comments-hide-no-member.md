---
type: ADR
title: "ADR 0144: a comment in the Larry members array hides no member"
description: "The temp-path rule reads the workspace members array past its TOML comments."
resource: /docs/adr/0144-manifest-comments-hide-no-member.md
tags: [larry, adr, governance, workspace]
generated: {by: claude-code, at: 2026-11-28}
verified: {by: claude-code, method: design-gate red then green on the workspace simulation, 2026-11-28}
status: accepted
stale_after: 2027-05-28
sources: [larry#611, INVARIANTS.md, src/governance/temp_paths.rs]
---

## Context

The temp-path rule reads `[workspace] members` from the root `Cargo.toml` to find the sources it
scans. `string_items` read every quote in the array body as the start of a string, a quote inside a
`#` comment too. A comment with one apostrophe opened a string that ran into the next entry, so a
member could drop out of the scan and its leak passed. The rule failed open.

## Decision

`string_items` skips a `#` comment to the end of its line when it is outside a string, as TOML 1.0.0
reads it. A `#` inside a quoted string stays part of the string. This is the lexical rule
`array_items` already applies when it finds the end of the array.

A TOML crate was rejected: it adds a dependency edge to read one array of strings the reader
already lexes. It would be taken up if the rule needed a second TOML construct.

## Consequences

A comment never changes which members are scanned. The reader still reads only the forms it did
before: it does not decode TOML escapes in a member path.

## Proof

- `sim_a_comment_in_the_members_array_never_hides_a_member` failed before the change with
  `MissingRoot` naming comment text, and passes after it.

## Traceability

Issue #611. Invariant: the temp-path roots are the workspace members of the root `Cargo.toml`.
