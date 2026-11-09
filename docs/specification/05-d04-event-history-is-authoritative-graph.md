---
type: Design
title: specification.md, part 5 of 18
description: Part 5 of /specification.md, in order; the index lists every part.
resource: /docs/specification/05-d04-event-history-is-authoritative-graph.md
tags: [larry, prd, architecture, continuity, efficiency, reinforcement-learning]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: bounded source inspection and external critique; not runtime verification}
status: final-proposed-v1.2-awaiting-architecture-approval
stale_after: 2026-10-15
sources:
  - Director requirements and corrections in conversations 2026-09-15 and 2026-09-16
  - /research-ledger.md
  - /Users/srinji/.okf/governance/contract/frontier-universal-assurance.md
---
### D04 — event history is authoritative; graph and context are projections

Choose causally linked, scoped events with command identity, parent references,
schema version and evidence hashes. A graph is a representation, not a demand
for a new graph database or scanning all history each turn. Context manifest
records exact selected source versions, omissions, budget, tokenizer/model and
rendered digest. Assemble stable policy/tools/pinned facts before volatile task
data where provider semantics permit. Keep tool schemas stable within a run.
Cache keys include tenant/security domain, model, tokenizer, template, adapter
and policy version; private cache sharing is prohibited. Replay authoritative
facts, not an LLM summary alone. Reject the claim that append-only storage
automatically produces cache hits. E09; owner context compiler.

### D05 — optimize cost per verified useful outcome, not raw tokens alone

Choose a constrained quality/cost Pareto objective. Record uncached input,
cache writes, cache reads, output, retries, abandoned speculation, subagents,
judges, learning and infrastructure. Report tokens and priced cost separately;
tokens from different tokenizers are not interchangeable compute units. Prefer
local parsing, indexed queries, batched tools and tested workflows over model
calls. Admit an extra worker only with an independence argument and a bounded
expected benefit; benchmark the rule. No fixed subagent minimum. Reject cached
irrelevant context kept merely to inflate a hit ratio. E03; owner scheduler and
evaluation, not a token-minimizing reward without quality constraints.

### D06 — four distinct learning mechanisms, one controlled promotion path

1. Explicit corrections/preferences update scoped records immediately, with
   provenance and an undo path. This is memory, not RL.
2. Repeated operational failures become general regression cases and, where
   sound, deterministic guards or tested procedures. Avoid endless instruction
   accumulation. A correction must name cause, detection and repair.
3. Begin genuine reward-driven adaptation with a bounded contextual bandit over
   already-admitted routing/tool/context strategies. Log action propensities,
   outcome delay, cost and policy version. Start in offline/shadow evaluation;
   exploration is opt-in within a fixed safe action set. For delayed multi-step
   decisions, compare an MDP policy against the bandit before adopting it.
4. Parametric fine-tuning/RL for local models is a separate experimental lane,
   with consent, training/evaluation separation, poisoning tests and rollback.

Use Bayesian calibration, linear models, decision trees, bandits, anomaly
detection and neural methods where matched evidence justifies them. No model
may alter authorization or evaluation truth to improve reward. Reward includes
independently measured completion, violations, correction recurrence and total
cost; constraint violations disqualify an action rather than being traded for
high utility. Causal credit assignment remains a research obligation.

Skill acquisition: discover existing skills first; obtain source/license/digest,
quarantine, run parameterized positive/negative tests, declare effects/budgets,
then promote within a preapproved scope. Revoke on drift. Generated skills are
tenant-scoped artifacts in Logical-DB, NOT automatic commits to shared foundation
crates. Shared publication is a separate governed development action. E04/E26;
owner learning/evaluation and skill registry.

### D07 — standing authority contracts, not approval fatigue or silent takeover

Choose RBAC for administration plus resource/action/context capabilities for
execution. A standing agreement covers project, allowed effect classes,
spending, duration, delegation depth, reversibility and notification preferences.
Major architectural commitments, training on personal data, new external
recipients, deployment and dependency policy changes are authority-relevant
effects too. Warn once, explain consequence, then perform an authorized lawful
non-harmful unwise action; model taste is not a veto. Unsupported operations and
missing authority return an actionable reason, never fabricated success.

Attenuation uses owner-defined semantic constraints on resource, recipient,
purpose, amount, action, expiry and evidence; generic JSON intersection is not a
permission algebra (RFC 9396 section 6.1). Descendants spend reserved portions of
their parent budget, not independent copies. Collaborators cannot pool grants
to manufacture a permission no principal holds.
Intent classifier output is a proposal, not a grant. Ambiguity requiring materially
different permissions is resolved by the human or by already approved policy.
Disallowed harmful/illegal actions cannot be laundered through delegation or
roleplay. Provider-imposed limits are disclosed rather than promised away.
An approved goal can authorize routine work without individual clicks; it cannot
authorize unrelated goals. E06; owner Secure-Authority adapter plus intent layer.

### D08 — semantic effects absorb common model mistakes

Choose typed operations such as reversible remove, scoped patch and execute in
workspace, with strict argument contracts and concise recovery hints. A known
`rm -rf` request may normalize to an advertised reversible-remove operation on
admitted user files. Do NOT claim a shell alias catches arbitrary scripts,
syscalls, redirections, database mutation or hostile binaries. Raw shell runs
inside Nova against disposable or copy-on-write workspace state; Moo integrates
authorized deltas. On host files, removal goes to the actual OS Trash; emptying
it remains human-only. No destructive operation is run in this design session.

Guarantee transparency: record requested operation, normalized semantics,
affected identities, pre/post evidence and restore handle. Test symlinks, mount
boundaries, open handles, hard links, collisions, permissions, quota exhaustion
and partial move failures. Some delete semantics cannot be faithfully mapped to
Trash; return Unsupported rather than silently claiming equivalence. Learn from
this session's false absence search: tools must return search scope and index
freshness so narrow evidence cannot become a global claim. E07; owner effect UX.
