---
type: Research
title: research-ledger.md, part 1 of 7
description: Part 1 of /research-ledger.md, in order; the index lists every part.
resource: /docs/research-ledger/01-2026-09-20-hardening-pass-what-the-rules.md
tags: [larry, research, codegraph, evidence, persona]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: CodeGraph current source and browser or GitHub API reads}
status: specification-v1.2-source-review-complete-runtime-unverified
stale_after: 2026-10-15
sources:
  - Director conversations 2026-09-15 and 2026-09-16
  - https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
  - https://www.anthropic.com/engineering/multi-agent-research-system
  - https://github.com/MatrAIx-ai/MatrAIx-Persona-8B
  - https://arxiv.org/abs/2609.00006
  - https://modelcontextprotocol.io/specification/latest
  - https://a2a-protocol.org/latest/
---
## 2026-09-20 hardening pass: what the rules required of a witness

READ: the estate's Rust rule corpus at `~/.agents/skills/rust-skills/rules/`.
The rules that shaped this pass, and what each changed:
`anti-unwrap-abuse` and `err-typed-domain-error` (no new panic path: every new
refusal is a typed value, and the new projection accessor returns `Option`
rather than panicking on an absent run); `proj-pub-crate-internal` (the new
`LiveEffect::sent` witness and the `RunView` tenant stay private, reachable
only through the read-only `ProjectionBackend` accessor); `type-newtype-ids`
and `api-newtype-safety` (the run's tenant is a `TenantId` throughout — the
type is what made "which tenant did we actually record" a question the
compiler could answer, and the answer was the defect); `mem-avoid-alloc` (the
`run_tenant` accessor borrows from the existing `BTreeMap` instead of
collecting a parallel tenant map, and recovery still allocates nothing
per-tick). One rule was consciously traded against another:
`mem-bounded-collections` argues every collection needs a hard ceiling, and the
fence's `revoked` set has none. The trade is deliberate and recorded rather
than silently accepted — evicting a withdrawn operation re-authorizes it, so
the bound is the host advancing the epoch, which is D49's anti-rollback
territory, not a local eviction policy.

READ: the OSS prior art this pass leaned on is not a repository but a pattern
the runtime already used, applied twice more. The estate's own admission-epoch
rule — a security fact the caller declares is made an *argument* the runtime
supplies, never a field the proposal is trusted for — was generalised to the
tenant, and the same shape ("the journal cannot carry this fact, so the live
process must") produced the `sent` witness. The alternative considered and
rejected for the tenant was a typed refusal of a mismatched body: it would
have surfaced the hostile input loudly but left the declaration consulted,
where the argument form removes it from the trusted path entirely, which is
what D42 asks for. The alternative considered and rejected for the witness was
exhausting a retry budget on a fenced first dispatch; that fails *open* in the
opposite direction, because a genuine first dispatch after re-authentication
would then be suppressed.

The local query layer was run before the work and again after. It was used to
order inspection, never as evidence that a passage was correct — and in this
pass the oracle's ranked candidates agreed with the code reading on all three
defects, while the defects themselves were established by failing tests, not by
the ranking.

The three findings and their repairs are recorded in `log.md`. The one worth
carrying forward as a lesson rather than a diff is the third: an earlier pass
concluded, in this same file, that a fenced first dispatch was
indistinguishable from a crash in the dispatch window, and that conclusion was
recorded as a *known limit* and pinned by a test. It was wrong, and the error
was not in the reasoning about `EventKind` — which is closed and has no
"dispatched" fact, exactly as stated — but in stopping there. "The journal
cannot tell" is not "the system cannot tell". The live table had the witness
the whole time; the cost of not noticing was a path that receipted a success
for an effect that provably never happened.

## 2026-09-20 dispatch-time freshness: rules loaded and oracle read

READ: the estate's Rust rule corpus at `~/.agents/skills/rust-skills/rules/`
(265 rule files) was loaded for this change rather than cited. The rules that
shaped it, and what each changed: `proj-pub-crate-internal` (the new module is
private with three curated re-exports, and the fence's two mutating operations
are `pub(crate)`, so a caller cannot pre-clear a reach or assert that a dispatch
happened); `num-overflow-explicit` (`checked_add` for the epoch, so exhaustion
is a typed refusal rather than a wrap that would alias a withdrawn generation
with a live one; `saturating_add` for the exposure counter, so a wrapped count
cannot hand back authority already spent); `pat-exhaustive-enum` (every match on
the refusal enums is exhaustive inside the crate); `err-lowercase-msg` and
`type-display-vs-debug` (hand-written `Display`, lowercase, no trailing
punctuation, never delegating to `Debug`); `type-newtype-ids` (the epoch is a
private-field newtype). One rule was deliberately not applied as an attribute:
`type-newtype-ids` recommends `#[serde(transparent)]`, which would be a no-op
here because a tuple newtype already serializes as its inner value — so the
property was *proved* by a round-trip test asserting the exact string `"7"`
instead of asserted by attribute, which is the stronger form.

As before, the local query layer ran on both sides of the work. Pre-exec ranked
D48 (1.0) and E71 (0.992) first among six scope-filtered candidates, which
agreed with the code reading; the post-exec query ranked the tracker gate and
this file's own change record first. Per the standing caveat, the ranking was
used to order inspection and never as evidence that a passage was correct — the
two design errors this change made were both found by tests, not by the oracle.

READ: the sibling repositories were **not** read for code this turn, on explicit
Director direction. The prior reuse survey stands unrevised and nothing from it
was imported.

The two findings worth carrying forward are recorded in `log.md`: a fenced first
dispatch is indistinguishable from a crash in the dispatch window because the
event vocabulary still has no "not dispatched" fact, and consulting the fence
inside the reconciliation-open predicate would have made the refusal silent.
