---
type: Plan
title: Larry product plan, reconciled with the estate on 2026-11-03
description: What Larry ships against Copilot, Muse, Duet, Dots and Harness², in what order, on which estate owners, and how each step is falsified.
resource: /docs/product/README.md
tags: [larry, product, competitors, copilot, operator, executor, learning]
generated: {by: claude-code, at: 2026-11-03}
verified: {by: claude-code, method: plan-v1 of 2026-10-22 reconciled against ~/.codex/AGENTS.md, province#3 to #10 and logic-os-kernel#850; track state re-read from git log and gh 2026-11-09; no runtime run}
status: plan-v2
stale_after: 2026-12-03
sources:
  - https://claude.com/resources/articles/claude-duet
  - https://openai.com/index/introducing-dots/
  - https://www.cnn.com/2026/09/23/tech/meta-muse-ai-agent
  - https://github.com/google-research/harness2
  - https://api-docs.deepseek.com/quick_start/pricing
---

# Larry product plan

Larry is the always-on harness organ of Logic OS: an agent that works for a person, asks before it acts and
can account for every effect. Logic OS replaces Microsoft's whole stack by 2027-03-21, so Larry's
counterpart is Microsoft Copilot (Copilot, Copilot Studio, Agent 365). Each track ships the moment it is
proven; the date is the worst case. Requirement and decision IDs stay owned by /specification.md.

## What changed since plan-v1 (2026-10-22)

| Ruling | Effect on this plan |
|---|---|
| Rocco renamed to Larry; Rocco now names an embedded harness that cannot speak (2026-10-26; repository renamed 2026-11-06, ADR 0005) | Face 1, the no-model dock, is Rocco's. Larry keeps Faces 2 and 3 and takes in the dock's low-confidence sessions. |
| Repos moved to Logical-Works-Inc (2026-11-03); public alpha (2026-11-06) | Larry is `Logical-Works-Inc/larry`, public, HL3, no contributions. |
| Kernel deprecated by absorption; one owner per capability (logic-os-kernel#850, province#4, #7) | Larry's own journal and fold move onto `lgwks_bot`; authority comes from secure-authority. |
| Revenue gate (province#5): nothing off the revenue slice's critical path | Face 2 is the agent gateway of the revenue slice (moo#184) and moves ahead of Face 3. |
| Newest estate pins and `gap` issues (2026-11-03) | Every change moves to the newest release: lgwks_std 4.2.0, lgwks_bot 8.4.0 and lgwks_deps 7.2.0 (2026-11-14); thiserror cites crates#842. |
| Python banned in product repos (2026-10-13) | The governance tooling ports to Rust (larry#53) before more tooling is added. |
| Harness² published (larry#60) | Measurable learning is a track with a matched benchmark, not a README promise. |
| HL3 ordered (2026-11-03) | LICENSE is HL3-FULL plus Part B; Part B's Safeguards are product requirements, not just terms. |
| Cargo sweep, Loco where lgwks_* is insufficient (2026-11-03) | [The sweep](/docs/dependencies/cargo-sweep.md) sets each owner; Track 3 goes before Track 2 (larry#71, #70); harness benchmark larry#72. |

## Competitors (DOC: vendor and press claims, not reproduced)

| | Meta Muse | Claude Duet | OpenAI Dots | Microsoft Copilot / Agent 365 |
|---|---|---|---|---|
| Where it runs | Meta cloud | Own VM and browser | Own cloud computer; laptop with permission | Microsoft cloud over Graph |
| Approval | "ask for some actions" | Shows the action, waits on send, pay, sign, delete | Custom Rules: allow, approve, block | Tenant admin policy; Dots plug into Agent 365 |
| Audit | Not stated | Not stated | Auto-review | Purview logs of what the vendor chose to log |

All four keep the agent in the vendor's cloud, on the vendor's model, judged by the vendor's classifier,
reaching the company through connectors into systems someone else owns. Larry differs on four points:
the company's own system of record (Logical-DB, the workspace), typed manifest actions instead of
open-ended browsing, a journal the company can replay bit-identically, and model neutrality.

## Tracks, in order

[The ship checklist](/docs/product/SHIP.md) is each track's issues in ship order; a shipping PR ticks its line.

**Track 0, estate alignment.** Shipped: HL3 license, newest pins, governance 1.2 to 1.4, INVARIANTS.md,
this plan, larry#53 (Rust governance checker, tracker, PTY journeys, reranker on lgwks_deps ML features,
pixel tool, over-size documents split) and the rename (larry#131, #178). The falsifier PRs are merged or
ported and closed: #50 merged; #20, #21, #22 and #25 ported as #166, #174, #175 and #176.

**Track 1, the runtime on the estate.** Move the journal, deterministic tick and projection onto
`lgwks_bot`'s journal and ECS world through `lgwks_bot::app`, keeping every rule in INVARIANTS.md and every
existing falsifier green before and after. Authority checks consume secure-authority. What lgwks_bot lacks
(dispatch fence, ambiguous-crash semantics, offline exposure bound) is filed as `gap` issues on
logicalworks-crates and ported there with its tests, never kept as a second copy here. The crate's
2026-10-26 deprecation is lifted (Director 2026-11-03: "Larry-runtime can be edited and it should be
decoupled"); larry#68 is the port map, one row per PR, and its first row keeps the test doubles out of
every shipped build. Clock, journal file, fence, gate and retry run on lgwks_bot (#74 to #78); the
projection (#79) and the tick and consumer (#80) are the rows left.
- larry#182 defect 1: the executor's live clock is anchored at the Unix epoch (`runtime::instant::live_clock`),
  so its 60 s lease expires; 3 falsifier tests, PR #193, issue #194.

**Track 2, Face 2: the operator agent (the Copilot answer).** The revenue slice's agent gateway (moo#184).
A model limited to each app's manifest of typed actions with a risk class. Reads run in the client;
writes and anything that leaves the company pass the gate, wait in the approval queue and leave a receipt.
Code runs in Nova; state and memory live in Logical-DB; receipts follow experience-as-code. Multi-tenant
from the first run. The human-view issues (#27 to #46 errands and threats, #54, #55, #58, #59 legacy
estates) are its acceptance cases.

**Track 3, Face 3: the executor for power users (the cost router).** The Director plans with Claude; Larry
executes on a cheaper model that the tenant's route table names (ADR 0039), DeepSeek V4.1 Flash first. No
gateway or model is compiled in; 9router is one possible route the operator adds. Claude hands a typed
task (goal, repo, files, acceptance commands) through an MCP tool; Larry returns a diff, the command output
and a receipt; only Claude-approved work lands. It runs on the Director's own code and uses the same gate
as Track 2, so it exercises the gate on real work every day. The per-tenant route and its declared
residency have shipped; refusal by residency and egress is open in #203. DeepSeek's first-party API is
hosted outside Canada, so customer data never takes this path; Face 2 picks a host per tenant.

**Track 4, learning (larry#60 to #66).** The harness becomes a versioned, content-hashed journal value;
propose, probe and compose run as journalled steps; no grader verdict reaches the improver; lessons live
in Logical-DB per tenant. Never a fork or wrapper of Harness² (Python, file-tree state).

**Rocco handoff.** Rocco's dock (page-transition Markov chain plus BM25 over walkthroughs, extending
lgwks-frontend `src/lib/rocco-rank.js`) hands a low-confidence session to Larry's Face 2 with its context.

## Falsifiers

- Track 0: the CI governance lane and the Rust gates green on main with the new pins; LICENSE Part A
  byte-identical to the canonical HL3-FULL text.
- Track 1: every runtime falsifier passes on lgwks_bot; replay p50/p95/p99 and peak RSS printed before
  and after the move on the same seed set.
- Track 2: no write leaves the gate without an intent, an authority record and a receipt, under kill -9
  and replay, for two tenants at once; one matched journey against Copilot Studio with p50/p95/p99, peak
  RSS and the approval count a user sees.
- Track 3: one matched task set, cost per accepted task and acceptance rate for Claude alone against Claude
  planning plus Flash executing, numbers printed.
- Track 4: larry#65, matched against Harness² on JobBench and LAB: score, cost, p50/p95/p99, peak RSS.
- License: every Part B prohibited use that the runtime can see (spend over budget, Safeguards bypassed,
  impersonation without authorization) is refused by the gate with a typed error in a test.
