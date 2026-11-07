---
type: Log
title: log.md, part 2 of 15
description: Part 2 of /log.md, in order; the index lists every part.
resource: /docs/log/02-2026-09-22-bounded-e63-hostile-input.md
tags: [governance, decisions, evidence]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: explicit conversation decisions and source inspection}
status: active
stale_after: 2026-10-15
sources: [Director conversations 2026-09-15 through 2026-09-17, /specification.md, /research-ledger.md]
---
## 2026-09-22: bounded E63 hostile-input runtime corpus

Director direction: finish a bounded core slice from the current Rocco
specification and live issues, using deep implementation research. Live issue
#4 is architecture/governance-only and explicitly leaves runtime experiments
NOT RUN, so it was not treated as authorization for a separate routing engine.
The selected C02 slice is E63 hostile input at the public runtime boundary.

The corpus submits malformed proposal bytes, a policy-forbidden scope and an
over-budget declaration through `Runtime::submit`/`Runtime::tick`. The first
produces `RuntimeError::Codec`; the latter two report exact deterministic
`PolicyRefuses` and `BudgetExhausted` reasons. Each case asserts zero durable
intent, receipt, fixture dispatch and fixture reconciliation. Malformed bytes
are decoded before journal admission, so they consume neither journal capacity
nor a command identity; the same command ID can later carry a valid proposal.
An established run's tenant is also checked before journal admission, so a
foreign cancellation leaves the journal, run state and owner-call count
unchanged. The scope and budget cases deliberately script a success at the
owner, proving the denied request cannot consume it. Existing E63 falsifiers
continue to establish that a proposal cannot declare its own tenant or
admission epoch.

Research read Temporal's deterministic history/effect separation, Cedar's
fail-closed versioned authorization input, and Kubernetes's bounded
reconciliation mechanics. We adopt Cedar's fail-closed input treatment and
Temporal's journal-before-effect boundary in the corpus. We reject a new Cedar
dependency, a substitute policy database and a model-declared E65 binding:
Secure-Authority and Logical-DB own those authorities, while an untrusted
proposal cannot safely supply a trusted resolved resource/account/payload
binding. E65 stays NOT RUN pending its canonical resolver/owner path.

| Traceability | Proven locally | Still open |
|---|---|---|
| C02, E63, R79, D42 | malformed input has a typed codec refusal with zero journal admission and same-ID recovery; a foreign cancel has zero journal/state/owner mutation; forged scope/budget have exact gate denials with zero fixture-owner reach, intent and receipt | hostile model/generative corpus, forged labels/grants, authenticated authority, real owner and independent acceptance review |
| C02, E65, R81 | unchanged | trusted resolved resource/destination/payload/adapter/schema/policy binding and a real display-to-dispatch swap journey |

Receipts: `tools/receipts/20260922-c02-effect-binding-pre-exec-retry.json` and
`tools/receipts/20260922-c02-effect-binding-post-exec.json`.
The first pre-exec attempt using the default Python failed before receipt
creation because it lacked NumPy; the approved already-provisioned
`/Users/srinji/.local/bin/python` rerun completed and wrote the retained
receipt. Scoped `cargo clippy -p rocco-runtime --test hostile_input --locked
-- -D warnings`, `cargo test -p rocco-runtime --test hostile_input --locked`
(4 passed), and `cargo check -p rocco-runtime --all-targets --locked` exit 0.
The inherited root demo strict-lint failure remains outside this runtime slice;
the final whole-workspace strict Clippy run reports 371 diagnostics in
`src/native.rs` and `src/native/workspace.rs`. It prevents
whole-workspace-green, local-CI, PR and delivery claims.
