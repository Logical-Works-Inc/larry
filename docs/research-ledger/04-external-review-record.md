---
type: Research
title: research-ledger.md, part 4 of 7
description: Part 4 of /research-ledger.md, in order; the index lists every part.
resource: /docs/research-ledger/04-external-review-record.md
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
## External review record

BrowserOS task session `forge strategy` opened owned Gemini and ChatGPT tabs.
Architecture summaries only, no credentials or private source were submitted.
No API model or local agent swarm was spawned for these reviews.

- [Gemini review](https://gemini.google.com/app/722981afca407aa1): initial answer
  called third-party sources primary, claimed append-only events guarantee cache
  hits, implied durable execution eliminates duplicate external effects, claimed
  held-out tests guarantee no future regression, and proposed automatic shared
  crate publication of learned skills. These claims are REJECTED. A correction
  request was sent covering all of them. Useful prompts: context manifests,
  capability attenuation, speculative leakage and self-grading risks.
- [ChatGPT review](https://chatgpt.com/c/6aa9d18f-0048-83ea-9436-82ed4413a692):
  completed and inspected. Accepted as design counterexamples: semantic rather
  than structural grant attenuation, bounded offline leases, no pooled authority,
  crash ambiguity, separate erasure/replay contracts, effect-aware speculation,
  source-bound memory, independent learning evaluation and complete cost ledgers.
  These are recommendations, not proof of runtime guarantees. RFC 9396 section
  6.1 directly supports the permission-comparison warning.
- External model agreement is not architecture validation. Changes require
  source checks, explicit rationale and falsifiers in the notebook.

Saved initial Gemini capture:
`/Users/srinji/.browseros/tool-output/evaluate-1789514223781-c103a7ce-3e0f-4d07-91f7-b690b42ae8f7.txt`.
Browser captures retain full source excerpts separately from this concise ledger.

Gemini's follow-up acknowledged the six corrections. Its further suggestion
that Sentinel can itself impose receiver-side deduplication remains rejected:
an arbitrary remote endpoint must support that contract. Its assertions
about all weak models and exact RAM needs are not accepted as measured evidence.
Its suggestion to keep acquired user skills in shared foundation crates conflicts
with the tenant-scoped promotion rule. No further review agreement is treated as
independent product acceptance.

## MatrAIx — truthful persona evaluation audit

User-supplied source: [MatrAIx-Persona-8B](https://github.com/MatrAIx-ai/MatrAIx-Persona-8B).
Pinned inspection revision `3633d8dab149a9482a71b024418a49ae828cc941`.
GitHub license API returned MIT for code (license blob
`62e4d23affd663f90c59715414ebe5764ded78ea`). The subsequently inspected
[dataset card](https://huggingface.co/datasets/MatrAIx2026/MatrAIx_Persona_1M)
states non-commercial research-only terms, including subsets/derivatives, and
retains upstream source restrictions. Code MIT does not relicense those data.
No dataset download, install, training or evaluation execution occurred.

READ README: infrastructure for heterogeneous simulated users; four environment
families; authors claim 1290 categorical dimensions and a one-million-persona
public coreset. Authors explicitly reject treating this as a replacement for
real-person evidence. Development sample of roughly 200 is smoke-only.
DOC current dataset card: 999,847 released personas, 1,290 attributes, 10 packed
Parquet shards; the 999-row viewer is a distinct sample. Counts and composition
are publisher claims, not independently decoded or validated in this task.

READ at pinned revision through `gh api .../contents/... --jq .content`:
- `docs/persona/validation.md`: 10 attributes ×4 environments, positive/negative
  pairs, LLM adherence judge; adherence deliberately separate from task reward.
- `application/task-spec/docs/reporting-and-evaluation.md`: deterministic
  aggregation, simulated self-report, optional LLM analysis. It describes optional
  LLM facets whose absence does not fail deterministic grading and whose provenance
  can become indistinguishable in reporting. Forge must preserve method/missingness.
- `application/tasks/example-chat-api_support_chatbot/tests/test_state.py`:
  `outcome_status` can derive from feedback satisfaction; fallback uses `tracking`
  plus support-message count. `_resolution_progression` treats non-repeated text
  as advanced. These are example heuristics, NOT independent task-success or
  real-human-satisfaction oracles. `PASS` establishes artifact validity there.

CANDIDATE, corrected by Director: use the repository as one external evaluation
source; do not modify it or turn it into the entire acceptance framework. Reject
wholesale adoption of example scoring. Keep separate axes
for simulator fidelity, assistant persona adherence, truthfulness, task outcome,
authority, privacy, continuity, accessibility and human comprehension. Proposed
cohort/trials and cost-approval gates live in notebook Cell 10.

## OSS harness structure review

The Director clarified that Codex means the open-source harness, not another
model's review. READ below means source inspection only, not executed behavior.
Checkouts are pinned reference snapshots, not a claim of latest upstream HEAD.
No source was changed. None of these four roots contained `.codegraph/`; per the
conditional CodeGraph rule, bounded source searches were used instead. Earlier
CodeGraph receipts for the sibling owners remain above; no duplicate clone was
created. Two research-only subagents inspected OpenCode and OpenClaw/Letta;
the implementation executor inspected Codex and reconciled findings.
