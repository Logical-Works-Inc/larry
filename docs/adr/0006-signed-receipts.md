---
type: ADR
title: "ADR 0006: every executor report carries an ed25519 receipt, and a denied run is sealed as rejected"
description: How `larry exec` seals its report (larry#86), where the key lives, how `larry verify` checks it, and why a denied call fails the task.
resource: /docs/adr/0006-signed-receipts.md
tags: [larry, adr, executor, receipts, ed25519]
generated: {by: claude-code, at: 2026-11-07}
verified: {by: claude-code, method: tests/receipt_journey.rs on the shipped larry binary, unit sims in src/executor}
status: accepted
stale_after: 2027-02-07
sources: [larry#86, larry#114, forge#158, RFC 8032, RFC 8785, lgwks_std 4.0.2 src/sign.rs and src/json/map.rs]
---

# ADR 0006: signed receipts

## Status

Accepted, 2026-11-07, with larry#86.

## Context

Claude hands Larry a task and lands only work it can trust. Re-running the acceptance commands is the
trust it had; a receipt makes it a signature check. Forge's audit chain was SipHash, a keyed MAC: only a
holder of the secret can check it, so no third party can. Microsoft Purview's Copilot audit log is held
and unsigned by the vendor. lgwks_std 4.0.2 owns ed25519 (`sign`: `tag || BLAKE3(record)`,
`verify_strict`), and its JSON `Map` keeps keys sorted by byte, so compact output over ASCII keys and
integers is RFC 8785. Forge#158: a denied tool call was reported as success.

## Decision

- The body `larry exec` prints (`task_id`, `task_digest`, `results`, `diff`, `denied`, `accepted`) is the
  run's journal. It is sealed once at the end of the run: ed25519 over `larry.receipt.v1` and the BLAKE3
  of its compact JSON, in `receipt: {alg, key, signature}` (hex).
- `accepted` is derived, never set: no denial, a diff, every command exit 0 and not timed out. A verifier
  re-derives it after the signature check, so a seal over a success the results contradict is refused.
- The key is a 32-byte seed in `executor.db` (`receipt_key`, one row, schema version 2), drawn from OS
  entropy on first use, `INSERT OR IGNORE` so racing first runs keep one key. The store and its WAL files
  are created and kept mode 0600. One state directory is one tenant and one key.
- `larry verify` reads a report (256 MiB bound) and trusts `LARRY_RECEIPT_KEY` (64 hex) when set, else
  the state directory's key; `larry key` prints that key. Exit 0 genuine and accepted, 2 genuine and
  rejected, 1 refused with one typed line (`ReceiptError`).
- `larry exec` exits 0 accepted, 2 sealed but rejected, 1 refused with no report.
- A task with `model_commands` is denied while Nova isolation is unavailable (logicalworks-crates#846):
  nothing runs and the sealed report carries the denial, so it is never accepted. It replaces the bare
  `IsolationUnavailable` refusal.

## Consequences

- lgwks_std gains `sign` in the root crate: 17 crates (`ed25519-dalek` 3 and its RustCrypto stack), cold
  `cargo build --bins` measured in the PR.
- No per-entry prefix proof yet: a reader checks the whole report. When the journal moves onto
  lgwks_bot `FileJournal` (larry#75), receipts seal the journal head.
- The seed sits in a 0600 SQLite file; Secure-Authority owns keys once it ships a store.
