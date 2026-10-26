---
type: Plan
title: Rocco product plan, three faces against Muse, Duet and Dots
description: Where Rocco sits after Meta Muse, Claude Duet and OpenAI Dots shipped the operator agent, and the planner/executor cost split for power users.
resource: /docs/product/README.md
tags: [rocco, product, competitors, cost, dock, operator, executor]
generated: {by: claude-code, at: 2026-10-22}
verified: {by: claude-code, method: primary-source fetches through design-gate on 2026-10-22; no runtime run}
status: plan-v1
stale_after: 2026-11-22
sources:
  - https://claude.com/resources/articles/claude-duet
  - https://openai.com/index/introducing-dots/
  - https://www.engadget.com/2256577/how-to-get-started-with-meta-s-new-ai-agent-muse/
  - https://www.cnn.com/2026/09/23/tech/meta-muse-ai-agent
  - https://api-docs.deepseek.com/quick_start/pricing
  - https://venturebeat.com/technology/deepseek-v4-1-flash-debuts-with-0-003-1m-off-peak-cached-input-rate-and-benchmarks-eclipsing-gpt-5-6-sol-claude-opus-5
---

# Rocco product plan

The harness before Forge, then Forge, then Rocco were one idea: an agent that
works for a person, asks before it acts, and can account for what it did. In
September 2026 three labs shipped that idea as a consumer product. This plan
records what they shipped, where Rocco still differs, and the three faces Rocco
ships as. Requirement and decision IDs stay owned by /specification.md.

## What shipped (DOC: vendor and press claims, not reproduced)

| | Meta Muse | Claude Duet | OpenAI Dots |
|---|---|---|---|
| Launch | 2026-09-08, own iOS, Android and web app | Research preview, waitlist, US Pro and Max | 2026-09-29, Pro and Business Premium |
| Model | Meta's own | Claude Opus 5.5 | GPT-6 Astra |
| Where it works | Meta cloud; can "build its own software" | Own VM and browser; morning brief; checks apps every few hours | Own cloud computer and browser; your laptop with permission |
| Reach | Connectors plus any public API | 6,000+ connectors and plugins | 4,000+ apps; ChatGPT, Slack, Teams |
| Approval | Default "ask for some actions"; "always ask" option | Shows the exact action and waits before send, pay, sign, delete; some actions off-limits | Built-in rules plus Custom Rules (allow, require approval, block); auto-review |
| Money | Stripe wallet, card number hidden | Stripe Link virtual card, Shop Pay | Not stated |
| Organisation | Personal | Professionals and small business | Specialist dots with their own identity; Microsoft Agent 365 governance |

What all three share: the agent lives in the vendor's cloud, runs the vendor's
model, is judged by the vendor's classifier, and reaches the company through
connectors into systems someone else owns. Dots going into Microsoft Agent 365
makes Microsoft's governance the control plane for OpenAI's agents.

## Where Rocco differs

- **The company's own system of record.** Logic OS data (Logical-DB, the
  workspace) is the ground truth, not a connector into Graph or Gmail.
- **Typed actions, not open-ended browsing.** Each app publishes a manifest of
  action ids with parameters and a risk class. The model can only pick from it.
- **Accountable effects.** rocco-runtime records the intent before acting,
  checks authority and freshness, writes a receipt and replays bit-identically.
  The vendors offer approvals; none offers a journal a company can audit.
- **Model-neutral.** Planner and executor are chosen per task by cost and
  capability, which is Face 3.

## Face 1: the dock (embedded, no model)

The Rocco dock inside every Logic OS app predicts the next page and matches
walkthroughs. On-device counts: a page-transition Markov chain blended with a
shipped all-users prior, plus BM25 over walkthrough titles and example
phrasings; day 0 lists the walkthroughs. Kilobytes, not a language model. It
extends lgwks-frontend `src/lib/rocco-rank.js`. Low confidence hands off to Face 2.

## Face 2: the operator agent (the Copilot answer)

The chat shape of the same dock, backed by a model limited to the app
manifest's actions. Read-only actions run in the client. Writes and anything
that leaves the company go through the rocco-runtime gate, wait in the approval
queue and leave a receipt. Code execution, when a task needs it, runs in Nova.

## Face 3: the executor for power users (the cost router)

The Director plans with Claude in Claude Code; Rocco executes on a cheaper
model, so a Claude subscription buys judgment, not typing.

- **Contract.** Claude hands Rocco a typed task (goal, repo, files, acceptance
  commands) through an MCP tool. Rocco returns a diff, the command output and a
  receipt. Claude reviews; only Claude-approved work lands.
- **Executor model.** DeepSeek V4.1 Flash first. Its API speaks the OpenAI and
  Anthropic formats, so the provider is a config value, not code. Its open
  weights are MIT, so other hosts can serve it.
- **Price (DOC, VentureBeat 2026-09-10).** Flash off-peak: $0.003 per million
  cached input, $0.15 uncached, $0.60 output; peak doubles. Claude Opus 5: $5
  input, $0.50 cached, $25 output. Peak is Mon to Fri 01:00 to 04:00 and 06:00
  to 10:00 UTC, so schedulable work runs off-peak.
- **Where Flash is weaker (DOC, DeepSeek-run).** It edges Opus 5 on DeepSWE
  (74.2 against 74.0) but trails on Terminal-Bench 3.0 (30.0 against 43.3).
  Long terminal work is its weak spot, so every run ends in the task's own
  acceptance commands, and a failure returns to Claude, never to the user as
  success.
- **Metric.** Cost per accepted task and cache-hit ratio on a matched task set,
  against Claude doing the same tasks end to end. Headline token price is not
  the metric.
- **Data.** DeepSeek's first-party API is hosted outside Canada. Face 3 runs on
  the Director's own code; Face 2 customer data uses a host chosen per tenant.

## Order

1. Face 3: smallest surface, fastest saving, and it exercises the gate, the
   journal and receipts on real work every day.
2. Face 1: no backend, extends code already in lgwks-frontend.
3. Face 2: the same gate, once Logical-DB and the workspace handlers exist.

## Falsifiers

- Face 3: one matched task set, cost per accepted task and acceptance rate for
  Claude alone against Claude planning plus Flash executing, numbers printed.
- Face 1: replayed sessions, hit rate of the clicked page in the dock's top 3
  against authored links alone and the current ranker.
- Face 2: no write leaves the gate without an intent, an authority record and a
  receipt, under kill -9 and replay, for two workspaces at once.
