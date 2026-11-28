---
type: ADR
title: "ADR 0136: anchored file edits with a content-hash precondition"
description: "A session model reads a file with its BLAKE3 digest and edits one anchored place against that digest, each after a yes."
resource: /docs/adr/0136-anchored-edit.md
tags: [larry, adr, executor, session]
generated: {by: claude-code, at: 2026-11-27}
verified: {by: claude-code, method: the tests under Proof run on a build box, 2026-11-28}
status: accepted
stale_after: 2027-05-27
sources: [larry#277, larry#586, larry#606, INVARIANTS.md, src/executor/edit.rs, src/executor/session_turn.rs]
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

`executor::edit` takes only a writable repo-relative path, through no symlink the
tree carries, that names a regular UTF-8 file of at most 262,144 bytes. Each other
case is its own typed refusal, and the model is shown it. The open and the bound
are the `larry governance` reader's: it stops one byte past the bound, so a file
that grows after its length was checked is still refused (CWE-367).
The model step of a brief reads each of the brief's files through the same
reader and the same bound (larry#617). It had checked the length and then read
the whole file.

Part 2 (larry#606) adds the reply `{"edit": {"path", "old", "new",
"expect_digest"}}`, approved like a command. A digest that no longer matches is
refused with the digest on disk and nothing is written. `old` must sit in one
place, tried as exact bytes, then as whitespace-trimmed lines, then as the block
its first and last lines bound; no place, or more than one, is refused.

The edit takes a lock on the file and checks the path still names it, so of two
edits that saw one digest at most one lands. The new file is written to a
temporary name through the held directory and renamed over the old one, so it is
whole or absent and keeps the mode.

A read is sealed in `changes` with its path, digest and length, an edit with both
digests, and either with its refusal. Neither stores the file or enters
`results`, so a session's verdict still rests on its commands alone.

The Codex patch shape is a later part. It is not in the binary yet.

## Consequences

The model can learn a file's digest without a shell command. A read of a longer
file shows its first 4 KiB and the digest still covers the whole file. A declined
step reads and writes nothing.

The lock is advisory: it orders edits, not a program that does not take it. The
rename is by path (the held directory has no rename, logicalworks-crates#1141); a
directory swapped away before it fails the edit and leaves the temporary file in
the moved directory. Off Unix there is no file identity to compare, so an edit is
refused.

Rejected: reading with a `cat` command. It gives no digest, no typed path refusal
and no record of what was read. Rejected: naming the digest only in a later stale
refusal, because the model would retry blind. Rejected: a separate MCP tool, which
has no session tree and no approval path. Rejected: `sed` or a whole-file rewrite
for edits: no precondition, so a stale read overwrites a newer file. Rejected:
`metadata` then `fs::read`, which takes the whole of a file grown in between.
Rejected: a digest check with no lock, which two edits can both pass.

## Proof

- `sim_a_read_shows_the_file_and_its_digest_for_every_seed`, 512 seeds.
- `sim_a_path_outside_the_snapshot_is_refused`.
- `sim_a_link_in_the_tree_is_refused`.
- `sim_each_read_bound_admits_its_limit_and_refuses_past_it`.
- `sim_an_approved_read_shows_the_digest`.
- `a_read_reply_parses_and_a_mistyped_one_is_refused`.
- `tui_journey_approved_read_shows_the_digest`, the shipped binary on a PTY.
- `sim_a_file_grown_after_the_stat_is_refused_too_large`, 64 seeds.
- `sim_an_edit_applied_then_reverted_gives_the_original_bytes`, 512 seeds.
- `sim_a_stale_digest_is_refused_and_the_file_is_unchanged`, 512 seeds.
- `sim_each_anchor_level_replaces_its_one_place`.
- `sim_an_anchor_that_matches_nothing_is_refused`.
- `sim_an_ambiguous_anchor_is_refused_naming_its_lines`.
- `sim_an_edit_keeps_the_mode_and_replaces_the_file_whole`.
- `sim_two_edits_that_saw_one_digest_never_both_land`.
- `sim_a_hundred_edits_at_once_on_one_digest_apply_one`.
- `sim_a_directory_swapped_for_a_link_before_the_replace_writes_nothing`.
- `sim_an_approved_edit_records_both_digests`.
- `an_edit_reply_parses_and_a_malformed_one_is_refused`.
- `tui_journey_approved_edit_lands_with_its_receipt`, the shipped binary on a PTY.
- `sim_a_source_grown_after_the_stat_is_refused_too_large`, 64 seeds, for the brief.
- `sim_each_unreadable_source_gets_its_exact_fault`.
