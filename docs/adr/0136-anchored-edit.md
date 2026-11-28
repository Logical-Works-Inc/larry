---
type: ADR
title: "ADR 0136: anchored file edits with a content-hash precondition"
description: "A session model reads a file and its BLAKE3 digest after a yes; later parts of larry#277 edit against that digest."
resource: /docs/adr/0136-anchored-edit.md
tags: [larry, adr, executor, session]
generated: {by: claude-code, at: 2026-11-27}
verified: {by: claude-code, method: the tests under Proof run on a build box, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#277, larry#586, INVARIANTS.md, src/executor/edit.rs, src/executor/session_turn.rs]
---

# ADR 0136: anchored file edits with a content-hash precondition

## Context

A session model changed files only by shell command. A shell write has no
precondition, so a stale read silently overwrites a newer file, and it leaves no
record of what the file was. An edit can name what it expects only if the model
was first shown a digest of the file.

## Decision

larry#277 ships in parts, each wired into the session. This record covers the
whole decision; the text below states only what has shipped.

Part 1 (larry#586) adds the reply `{"read": path}`. The person approves it like a
command. It reads in the session tree (the folder, or its isolated copy) and shows
the model the BLAKE3 digest of the whole file, its length and its first 4 KiB.

`executor::edit` reads only a writable repo-relative path, through no symlink the
tree carries, that names a regular UTF-8 file of at most 262,144 bytes. Each other
case is its own typed refusal, and the model is shown it.

A read is sealed in `changes` with its path and its digest and length, or its
refusal. It never stores the file and never enters `results`, so a session's
verdict still rests on its commands alone. `executor::session_turn` holds the turn
loop.

The later parts add an edit that refuses a stale digest, anchored matching and the
Codex patch shape. None of them is in the binary yet.

## Consequences

The model can learn a file's digest without a shell command. A read of a longer
file shows its first 4 KiB and the digest still covers the whole file. A declined
read reads nothing.

Rejected: reading with a `cat` command. It gives no digest, no typed path refusal
and no record of what was read. Rejected: naming the digest only in a later stale
refusal, because the model would retry blind. Rejected: a separate MCP tool, which
has no session tree and no approval path.

## Proof

- `sim_a_read_shows_the_file_and_its_digest_for_every_seed`, 512 seeds.
- `sim_a_path_outside_the_snapshot_is_refused`.
- `sim_a_link_in_the_tree_is_refused`.
- `sim_each_read_bound_admits_its_limit_and_refuses_past_it`.
- `sim_an_approved_read_shows_the_digest`.
- `a_read_reply_parses_and_a_mistyped_one_is_refused`.
- `tui_journey_approved_read_shows_the_digest`, the shipped binary on a PTY.
