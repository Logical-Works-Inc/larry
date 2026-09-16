---
type: Design
title: Rocco final proposed specification and completion notebook
description: Continuity-first assistant runtime PRD, candidate decisions, architecture, evidence gates and implementation sequence.
resource: /specification.md
tags: [rocco, prd, architecture, continuity, efficiency, reinforcement-learning]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: bounded source inspection and external critique; not runtime verification}
status: final-proposed-v1-awaiting-architecture-approval
stale_after: 2026-10-15
sources:
  - Director requirements and corrections in conversation 2026-09-15
  - /research-ledger.md
  - /Users/srinji/.okf/governance/contract/frontier-universal-assurance.md
---

# Rocco specification v1 — frontload the cognitive work

**Approval: pending. Runtime experiments: NOT RUN. Human participants: 0.**
This is a concrete decision package, not a collection of equally ranked options.
Except for the expressly settled Director decisions in Cell 12, decisions below
are recommendations until ratified. Metrics marked TARGET
are acceptance proposals, not measurements or promises. Source claims and source
inspection are distinguished in the [research ledger](research-ledger.md).

Rocco is the Director-selected working name for the new private repository.
Forge was the earlier scratchpad name, not a second runtime. The existing
`forge-harness` and `forge-sdk` remain historical
source repositories only. Final means the current requirements, dispositions and
execution plan are assembled; it does not mean every proof or product test passed.

## Cell 00 — strategy and PRD

**Diagnosis.** A prompt loop makes the model repeatedly reconstruct intent,
authority, context, coordination and evidence. The human becomes the runtime's
memory, permission interpreter, supervisor and cleanup service. More tools and
longer prompts can increase this burden rather than solve it.

**Guiding policy.** One persistent relationship; disposable, specialized workers;
deterministic ownership of commitments, resources and effects. Put cognitive tax
on engineering once, rather than on every user and every model turn.

**Product sentence.** Rocco is the assistant that remembers what matters, acts
within your standing agreement, coordinates work and can show what actually got
done. Its personality is a presentation and relationship capability, not a source
of authority or a claim of consciousness.

**Primary users/jobs:** a person delegates research, coding or repeatable digital
work and returns later; a team shares a project without sharing all private
memory; an enterprise integrates a controlled runtime; a developer embeds the
core without bringing a UI or requiring the cloud. Ordinary use must remain
possible on modest hardware without mandatory GPU purchases.

**Success:** improve a fixed weaker model's independently verified task success
at matched total cost, while reducing repeated corrections, supervision and
cleanup. Beating an expensive strong model by secretly calling it is not success
in the weak-model lane. Personality quality is evaluated separately from task
correctness; neither is allowed to purchase improvements by degrading the other.

**Strategic boundaries, not dropped requirements:** no new database, security
authority, browser engine, editor or container runtime inside Rocco. No permanent
agent committee by default. No opaque self-improvement of permissions. No model
training prerequisite for ordinary use. No product claim from unit-test counts.
Phone/app/web remain required surfaces with different delivery dependencies.

**Settled scope decision:** the Director chose core + TUI/CLI first, phone and
app next, website later. The old phone-first contract remains historical context;
all surfaces remain required. Memory implementation is deferred until Logical-DB
is ready, not replaced with a temporary database or dropped from acceptance.

## Cell 01 — source requirement inventory

The original list repeats 3 and 23. Stable IDs below preserve all 42 entries,
plus the MatrAIx follow-up as R43. Wording is normalized for indexing, not a
replacement for the Director's meaning. Acceptance references name falsifiers,
not completed tests. Additional corrections: use CodeGraph extensively and sync
or index when necessary; no request to resume legacy implementation.

| ID | Original label | Preserved requirement | Candidate | Acceptance |
|---|---|---|---|---|
| R01 | 1 | Do core things well rather than tack on features | D01 | E01 |
| R02 | 2 | One entity, true continuity, delegated work | D02 | E02 |
| R03 | 3a | Extreme token and total efficiency | D05 | E03 |
| R04 | 3b | Reinforcement learning; stop repeating mistakes | D06 | E04 |
| R05 | 4 | Personality, preference adaptation, useful mirror without sycophancy | D03 | E05 |
| R06 | 5 | Device feels alive; personable sci-fi assistant, not impossible physical powers | D03 | E05 |
| R07 | 6 | Human agency with a contract covering major architectural choices too | D07 | E06 |
| R08 | 7 | Developer carries cognitive tax; common destructive spellings get reversible semantics | D08 | E07 |
| R09 | 8 | Warn but execute authorized unwise work; nuanced intent, lawful and non-harmful bounds | D07 | E06 |
| R10 | 9 | Efficient, low-anxiety frontier multi-agent coordination with shared context | D09 | E08 |
| R11 | 10 | Context as graph and events; cache, transfer, build on it | D04 | E09 |
| R12 | 11 | Speculative tool calls | D10 | E10 |
| R13 | 12 | Multiple forms of ML where justified, not neural-only | D06 | E04 |
| R14 | 13 | Dedicated compute, ideally no approval clicks, strong roles and delegation | D11 | E11 |
| R15 | 14 | Abduction; discover what the human missed | D12 | E12 |
| R16 | 15 | Enterprise reliability: done means done | D13 | E13 |
| R17 | 16 | Adapt to user terminology without repeated definitions | D03 | E14 |
| R18 | 17 | Messaging-style responses and attachments, including weaker models | D14 | E15 |
| R19 | 18 | Weak-model uplift; baseline against Claude Code | D15 | E16 |
| R20 | 19 | System is a useful visible third participant | D14 | E15 |
| R21 | 20 | Deep security and ethics, defense in depth | D16 | E17 |
| R22 | 21 | Multi-human and harness-to-harness collaboration | D17 | E18 |
| R23 | 22 | Replayability | D18 | E19 |
| R24 | 23a | TUI/REPL/CLI, phone, app, eventually website | D19 | E20 |
| R25 | 23b | Safe Python-like or RON/Rhai workflow composition | D20 | E21 |
| R26 | 24 | Sentinel owns internet access | D21 | E22 |
| R27 | 25 | Secure-Authority owns security | D21 | E22 |
| R28 | 26 | Logical-DB owns memory persistence/storage | D21 | E22 |
| R29 | 27 | Nova owns sandboxing | D21 | E22 |
| R30 | 28 | Moo owns code editor/workspace | D21 | E22 |
| R31 | 29 | Logicalworks-Crates is the foundation; follow its doctrine | D21 | E22 |
| R32 | 30 | All nine axes | D22 | E23 |
| R33 | 31 | BrowserOS neo + Gemini and ChatGPT research/validation | D24 | E24 |
| R34 | 32 | Make decisions, obtain human approval | D24 | E24 |
| R35 | 33 | Enable major evolution without another rewrite | D23 | E25 |
| R36 | 34 | Bring Machine Intelligence / Machine Determinism to life | D01 | E01 |
| R37 | 35 | First-class local models and architectures | D11 | E11 |
| R38 | 36 | Discover, acquire and make skills automatically | D06 | E26 |
| R39 | 37 | Research Anthropic claude-cookbooks | D24 | E24 |
| R40 | 38 | Reuse existing code | D21 | E22 |
| R41 | 39 | Extensive CodeGraph use | D24 | E24 |
| R42 | 40 | Reason from harness/alignment frustrations | D08 | E07 |
| R43 | follow-up | MatrAIx persona evaluations, truthfully | D15 | E27 |

### Harness-review requirements

These derived requirements strengthen the original inventory rather than replace
it. Source anchors H01–H04 are in the research ledger. Every row has an exact
pass/fail journey below; no source inspection is counted as a passing journey.

| ID | Origin | Required behavior | Decision | Acceptance |
|---|---|---|---|---|
| R44 | OpenCode/Codex | Durable input admission, safe steering, queue ordering and retry identity | D25 | E28 |
| R45 | Codex/OpenCode | Explainable configuration/instruction precedence, trust and provenance | D26 | E29 |
| R46 | Codex/OpenCode/Letta | Durable human questions and approvals, expiry, cancellation and exact scope | D25 | E30 |
| R47 | OpenCode/Letta | Failure-atomic compaction and explicit context epochs preserving pending exchanges | D27 | E31 |
| R48 | OpenCode | Version-bound tool registration, isolated extension lifecycle and schema checks | D28 | E32 |
| R49 | OpenCode/Codex | Provider-neutral transcript lowering, authentication and rate-limit recovery | D29 | E33 |
| R50 | OpenCode | Distinguish capture loss, preview truncation and complete retained artifacts | D29 | E34 |
| R51 | OpenCode/Codex | Durable cursor catch-up, bounded live notifications and slow-consumer recovery | D25 | E35 |
| R52 | OpenCode/Codex | Explicit service lifetimes and equivalent embedded/remote behavior | D25 | E36 |
| R53 | OpenClaw | Scoped external sender identities and reversible explicit account linking | D30 | E37 |
| R54 | OpenClaw | Reply owner and destination binding across concurrent channels and threads | D30 | E38 |
| R55 | Director/OpenClaw | Bounded scheduled/event-driven work, DST, missed runs, overlap and expiry | D31 | E39 |
| R56 | Letta | Separate accepted, scheduled and durably applied background-memory cursors | D27 | E40 |
| R57 | Letta/OpenCode | Acknowledged shared corrections have an explicit visibility fence | D27 | E41 |
| R58 | OpenClaw | Resumable setup, readiness distinctions and non-interactive recovery | D32 | E42 |
| R59 | OpenClaw/OpenCode | Desired versus effective configuration with atomic activation and rollback | D26 | E43 |
| R60 | OpenClaw/Letta | Diagnostics expose observation age, execution phase and safe recovery actions | D32 | E44 |
| R61 | Codex | Explicit fork, archive, export/import and redaction semantics; no replayed effects | D35 | E45 |
| R62 | Director | Adjustable decay and user-facing scales; memory implementation waits for Logical-DB | D33 | E46 |
| R63 | Codex | Versioned headless API, typed exit/error results and capability negotiation | D25 | E47 |
| R64 | Codex/OpenCode | Remote environment identity, path semantics and uniform effect admission | D28 | E48 |
| R65 | Director | Independent evaluation sources; MatrAIx is one source, not the entire oracle | D34 | E49 |
| R66 | Harness synthesis | Verified distribution, compatibility-aware updates and interrupted-upgrade recovery | D35 | E50 |
| R67 | Harness synthesis | Accessible UI and safe rendering of untrusted terminal/media content | D32 | E51 |
| R68 | Moo/Codex | Conflict-aware change previews and undo that preserves unrelated human edits | D08 | E52 |
| R69 | Research review | Answer citations retain source version, retrieval time and claim/evidence distinction | D13 | E53 |
| R70 | Harness synthesis | Bounded no-progress loops with causal diagnostics, not endless agent chatter | D09 | E54 |

## Cell 02 — candidate architecture graph

```mermaid
flowchart TD
  H[Humans: private and shared scopes] --> UI[Message surfaces: TUI phone app web]
  UI --> C[Versioned command and event contract]
  C --> MD[MD: admission reducer scheduler budgets evidence]
  MD --> SA[Secure-Authority: identity grants revocation secrets]
  MD --> DB[Logical-DB: scoped durable records and sync]
  DB --> CX[Context compiler: graph projection and manifest]
  CX --> MI[MI: principal assistant and bounded specialist workers]
  MI --> P[Typed proposals: plans hypotheses effects memory skills]
  P --> MD
  MD --> N[Nova: admitted compute and isolation profiles]
  MD --> M[Moo: workspace claims editor integration]
  MD --> S[Sentinel: internet egress browser research]
  N --> EV[Observed results and evidence references]
  M --> EV
  S --> EV
  EV --> MD
  MD --> UI
  EV --> L[Offline learning and held-out evaluation]
  L --> PR[Versioned policy and skill candidates]
  PR --> MD
  F[Logicalworks-Crates: std bot deps ast] -.foundation.-> MD
```

Edges denote contracts, not mandatory processes or distributed RPC for every
call. Begin with a modular Rust core and out-of-process untrusted workers. A
deployment can embed trusted components; fleet deployments place the same
contracts across workers. Do not build an independent service for every box.

**Ownership rule:** Rocco owns assistant-domain intent, commitments, context
selection and run orchestration. Logical-DB owns persistence/replication; Moo
owns workspace transactions; Nova owns process/compute containment;
Secure-Authority owns authenticated authority; Sentinel owns external access.
References to pre-existing kernel/Braid identities and receipt protocols require
an explicit adapter decision, not a Rocco replacement. No cross-repository
charter amendment is fabricated by this candidate.

### Runtime control flow: one request through verified completion

```mermaid
flowchart TD
  In["Human input or admitted scheduled event"] --> Accept["Authenticate scope; persist request ID and digest"]
  Accept --> Ack["Acknowledge durable admission, not success"]
  Ack --> Order["Queue or steer at a safe turn boundary"]
  Order --> Context["Build bounded context manifest and pinned obligations"]
  Context --> Plan["MI proposes plan, hypotheses and typed effects"]
  Plan --> Gate{"MD validates authority, versions, budget and contracts"}
  Gate -->|"Missing human decision"| Ask["Durable scoped question; expiry is not consent"]
  Ask -->|"Valid answer"| Gate
  Gate -->|"Invalid or unsupported"| Fail["Typed failure; preserve work and evidence"]
  Gate -->|"Admitted"| Reserve["Persist intent; reserve budget and fenced ownership"]
  Reserve --> Dispatch["Canonical owner executes; no ambient host bypass"]
  Dispatch --> Observe{"Outcome known?"}
  Observe -->|"No: timeout or crash"| Unknown["OutcomeUnknown; reconcile without blind retry"]
  Unknown -->|"Receiver evidence obtained"| Verify
  Unknown -->|"Still unknown"| Hold["Visible unresolved effect; no false completion"]
  Observe -->|"Yes"| Verify["Independent domain checks and evidence capture"]
  Verify --> Good{"Original obligations satisfied?"}
  Good -->|"No; retry admissible"| Plan
  Good -->|"No; exhausted or blocked"| Fail
  Good -->|"Yes"| Close["Settle descendants, resources and delivery obligation"]
  Close --> Receipt["Durable terminal receipt; publish truthful scoped result"]
  Stop["Cancellation or revocation"] --> Cancel["Stop new dispatch; drain or cancel owned work"]
  Cancel --> Observe
  Receipt --> Learn["Budgeted evaluation; quarantined learning proposals"]
  Learn --> Promote{"Independent promotion checks and standing scope"}
  Promote -->|"Accepted"| Future["Versioned strategy or skill for later runs"]
  Promote -->|"Rejected"| Archive["Retain failure evidence; no privilege increase"]
```

Cancellation enters reconciliation: it does not turn an accepted remote effect
into a non-event. A resumed request returns to its durable position, not always
to Plan. The terminal result distinguishes completed, failed, cancelled and
unresolved dispositions. The learning edge is not mandatory idle inference and
cannot rewrite the current run's grader.

### Effect state machine: interruption is not rollback

```mermaid
stateDiagram-v2
  [*] --> Proposed
  Proposed --> AwaitingAuthority: missing grant or material decision
  AwaitingAuthority --> Proposed: valid scoped answer
  AwaitingAuthority --> Cancelled: cancellation or expiry before dispatch
  Proposed --> Rejected: invalid or unsupported
  Proposed --> Reserved: authority and budget validated
  Reserved --> Dispatched: intent durable and fence valid
  Reserved --> Cancelled: cancel before dispatch
  Dispatched --> Observed: known owner result
  Dispatched --> OutcomeUnknown: timeout or crash ambiguity
  OutcomeUnknown --> Observed: reconciliation evidence
  Observed --> Verified: domain obligations checked
  Observed --> Failed: check fails
  Verified --> Finalized: receipt and lifecycle settled
  Failed --> [*]
  note right of Failed
    Compensation starts a NEW effect identity at Proposed.
    Original failure and its evidence remain unchanged.
  end note
  Rejected --> [*]
  Cancelled --> [*]
  Finalized --> [*]
```

OutcomeUnknown is deliberately not drawn as automatic retry or success. Failed
and unresolved outcomes retain evidence and can end a run with unmet obligations;
the diagram does not require compensation to succeed or imply universal reversal.

## Cell 03 — notable decisions and rejected alternatives

Each D row has a falsifier in Cell 09. Director-settled directions are identified
in Cell 12; the remaining mechanisms are proposed. The design owner
is the named responsibility; implementation work must designate a maintainer.

### D01 — MI proposes; MD enforces the operational contract

Choose a deterministic reducer over validated commands/events, not a monolithic
LLM loop. MI interprets ambiguous language, proposes plans, diagnoses, ranks and
communicates. MD checks authority, versions, budgets, scheduling, effect
admission, evidence obligations and lifecycle. Deterministic means repeatable
given recorded inputs, not omniscient about ethics or semantic correctness.
Uncertainty must remain representable. Domain validators and independent human
judgment remain necessary. Reject a second LLM that can silently authorize the
first LLM's effects. E01; owner Rocco control plane.

### D02 — continuity is a durable commitment graph, not an immortal context

Choose separate typed records for original requests, current commitments,
decisions, corrections, task state, evidence, user preferences, hypotheses and
procedures. Every derived statement has source references, scope, confidence,
valid time and supersession. Unfinished commitments and explicit corrections
cannot disappear through summarization or relevance decay. Only an authorized
cancel/supersede transition closes them. One principal assistant renders this
state across models/devices; worker identity remains inspectable. Reject a
permanent full-transcript prompt and opaque model-only memory. E02; owner Rocco
memory semantics over Logical-DB storage.

### D03 — persona is an explicit, scoped relationship model

Choose user-editable tone, verbosity, humor, vocabulary and interaction
preferences; keep private-personal, project-shared and organization policy
scopes distinct. Terminology entries bind phrase to concept, examples, scope,
provenance and ambiguity; context-sensitive meanings coexist. Explicit user
definitions override inferred habits within their authority, not other users'
preferences. Never infer authorization from intimacy, praise or anger. Make the
assistant warm and distinctive without claiming sentience, exclusivity or hidden
work. Disagreement concerns the action, not the person's worth. Reject engagement
maximization and personality updates that change factual standards. E05/E14;
owner relationship/context layer.

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

### D09 — coordinator delegates work contracts, not cloned conversations

Choose a task DAG with exclusive goals, explicit dependencies, permitted effects,
input context manifests, output schema, deadline and total budget. Children get
shared immutable context references plus a bounded task delta, not all private
history. Result envelopes retain provenance, unknowns and failed attempts;
summaries are optional projections. Replanning cancels obsolete descendants.
Moo owns conflicting workspace claims. No shared writable prompt or infinite
agent-to-agent debate. Principal assistant remains responsible for integration;
human sees one status with drill-down, not every worker's narration. E08; owner
Rocco scheduler, consuming Moo coordination.

### D10 — speculation is a budgeted effect class

Choose parallel deterministic computation and isolated branch experiments when
benefit exceeds expected waste. A network GET can disclose secrets, charge money,
mark content read or mutate a service: READ is not synonymous with safe. Each
adapter declares confidentiality, external observability, cost, reversibility,
freshness and idempotency. Default speculation is local pure work; external
speculation requires an existing capability and explicit policy. Recheck
authorization and input versions before integrating any speculative result.
Shared writes, sending messages and purchases never occur because a branch was
speculative. E10; owner scheduler/effect boundary.

### D11 — dedicated compute is an owned resource envelope

Choose Nova-managed workspaces/leases, explicit CPU/RAM/storage/egress/time/spend
limits and scoped secret injection. The agent may experiment freely within that
contract; it cannot increase its own budget, persist a daemon or escape into the
host. Local model support is a capability-negotiated interface covering chat
templates, tokenizer, tools, structured output, cancellation, context limits and
usage. Malformed tool output gets bounded repair or explicit failure.

Do not hardcode a model family or assume OpenAI-compatible transport means equal
semantics. Support CPU-only configurations and user-selected larger models;
benchmark quantization and model residency separately AND in whole-system cost.
No automatic weight download, cloud fallback or paid retry. A resident local
model's RAM counts against the physical machine; the control-plane budget cannot
hide it. Model-serving locality and sandbox profile are independently admitted.
E11; owner model/compute adapters.

### D12 — abduction is competing hypotheses plus discriminating observations

Choose bounded hypothesis records: explanation, evidence for/against, assumptions,
predictions, confidence and the next informative test. Prefer low-cost probes
that distinguish hypotheses over additional rhetorical reflection. Track missing
data and alternative causes; abstain when evidence is insufficient. Rank via
calibrated probabilistic or symbolic methods where appropriate; MI proposes the
space, MD budgets experiments. Evaluate on hidden root causes and interventions,
not eloquence or agreement with the user. 'Outthink the human' is a useful
aspiration; measurable claim is finding valid omitted causes/actions on held-out
tasks without false certainty. E12; owner reasoning/evaluation.

### D13 — completion is a verified domain outcome plus lifecycle closure

Choose intent-specific obligations fixed at admission and amended explicitly.
Coding, research, messaging and administrative tasks have different graders.
Separate objective validity, subjective acceptance and unavailable evidence.
No model self-report or exit zero alone produces success. A receipt binds request,
policy, artifacts, tests/observations, versions, effect disposition, cleanup and
remaining uncertainty. Receipts attest evidence, not universal correctness.
Domain owners supply observations; Rocco does not fabricate signatures or a new
global receipt authority. E13; owner completion reducer and domain verifiers.

### D14 — messaging UI plus a distinct system voice

Choose a typed message envelope: concise answer, visible status, optional choices,
attachments, evidence and expandable detail. Core events drive task cards,
budget notices, delegation status and recovery prompts without another model
call. Clearly distinguish assistant narrative, human messages and system facts.
Attachments have scope, digest, size/type bounds and accessible descriptions;
they cannot smuggle instructions into an authoritative channel. Streaming tokens
are provisional until a complete validated message exists. E15; owner surfaces.

### D15 — matched weak-model and persona evaluation, not a single score

Choose three lanes: A same-model harness comparison; B best-product comparison
against Claude Code with all differences reported; C human/persona experience.
Never compare a local weak model against an unavailable Claude Code configuration
and call it controlled. If Claude Code cannot run that exact model/tool setup,
use a transparent neutral baseline for A and retain Claude Code for B. Freeze
versions, tasks, tools, budgets, providers, context, caching, network and judges.
Persona adaptation and user-simulator fidelity are different experiments. Reuse
MatrAIx tooling subject to license and verifier audit; detailed plan in Cell 10.
E16/E27; owner independent evaluation.

### D16 — security and ethics at every trust transition

Choose threat modeling for malicious documents/tools/skills, compromised workers,
credentials, prompt injection, memory poisoning, replay, confused deputies,
resource abuse and inter-user disclosure. Separate instructions from retrieved
data; provenance tagging alone is not injection prevention. Enforce output/effect
constraints outside the model. Use per-scope encryption/key policy, least
privilege, egress controls, supply-chain provenance and independent red teams.
No model sees raw root credentials. Privacy/export/retention and revocation cover
embeddings, caches, summaries, skill artifacts and training copies too.

Ethics requires scoped consent, contestability, accessible controls and honest
uncertainty; do not equate legality with absence of harm. Reversible user choices
may be unwise without justifying paternalistic obstruction. Do not optimize
dependency, emotional vulnerability or covert persuasion. Cross-tenant learning
is off unless explicitly authorized and privacy-reviewed. E17; owner security
and human acceptance, enforced through Secure-Authority and domain boundaries.

### D17 — collaboration federates contracts, not private minds

Choose project membership and explicit shared spaces; each person retains a
private relationship context. Harness federation exchanges signed, versioned,
minimized task/result envelopes under mutually verified authority. A remote
harness is untrusted even if its text sounds like the user. Resolve simultaneous
human directives by resource ownership, role and approved conflict policy; no
last-speaker-wins or inferred social hierarchy. Both legitimate but incompatible
requests become an explicit conflict while unrelated work proceeds. E18; owner
collaboration layer using Secure-Authority, Logical-DB and Moo.

### D18 — three replay modes, never one misleading guarantee

Choose: (1) observation replay displays recorded inputs/results; (2) deterministic
control replay folds recorded events with pinned reducer/policy and virtual
clock/random inputs, issuing NO live effects; (3) a fresh experiment reruns models
or tools in an isolated environment and may differ. Pin adapters, workflow,
model identity/template and dataset versions. Preserve model responses where
permitted; do not require access to hidden model reasoning. Redacted, expired or
deleted artifacts make replay explicitly incomplete. An old grant proves past
authorization, not current permission. E19; owner event/replay layer.

### D19 — one protocol, multiple accessible surfaces

Choose TUI/REPL and scriptable CLI with the same task/attachment contract as
phone/app/web. Persisted cursors support reconnect, slow networks, sleep and
device loss. Presentation never owns task truth. Phone background notifications
are hints, not a durable tape; exact Apple transport/lifecycle design requires
local Apple corpus research before implementation. Windows/Linux/macOS core and
TUI plus iOS phone are candidate support targets, all untested here. Native
platform affordances use adapters; no forced lowest-common-denominator UX.
E20; owner presentation, no separate phone run engine.

### D20 — typed workflow IR; Rhai is a candidate authoring frontend

Choose a versioned bounded workflow graph with explicit inputs, effects, retry,
parallelism, compensation and termination, not arbitrary Python as the trusted
control plane. JSON/RON serialize the IR; RON is a data notation, not an execution
language. Candidate Rhai subset constructs or transforms the IR with fuel,
memory, depth and output limits; no eval/import/filesystem/network/native effect
escape. Every native callback also needs its own resource limits. Freeze and
validate the generated IR before admission; dynamic replanning creates a new
versioned segment. Reuse any equivalent existing Flow compiler before adding one.
General Python remains possible only as an admitted Nova workload, never a
security boundary. Choose Rhai vs direct typed builder by model success/cost and
soundness trials, not familiarity. E21; owner workflow compiler.

### D21 — reuse named siblings through narrow, tested contracts

Choose contracts first and exact revision/artifact pins after compatibility
testing. Foundation surfaces are `std`, `lgwks_std`, `lgwks_bot`, `lgwks_deps`,
with `lgwks_ast` as the existing parser. Third-party dependencies go through
the estate's admission/storefront process. No copied vendor tree or dual engine.
Moo's current SQLite implementation does not license Rocco to introduce a second
SQLite store; consume Moo's workspace contract while Rocco persists through
Logical-DB. Never call a sibling production-ready from its name or README.
Missing semantics are integration work in their owner, not an excuse to duplicate
them. E22; owner integration/reuse ledger.

### D22 — local simplicity and fleet scale share semantics, not topology

Choose per-tenant/task partitions, explicit resource admission and active-worker
leases. No global event order or million idle processes. Live execution authority
has one fenced owner per scope; disconnected devices may draft and perform
separately admitted local work, not silently become concurrent authorities over
the same external effects. Logical-DB CRDT convergence is not a distributed lock.
Fleet routing, fencing and revocation need owner-supported contracts and proof.
All nine axes and hardware budgets remain acceptance obligations. E23; owner
deployment/control-plane integration.

### D23 — evolve at contracts, not through permanent generic machinery

Choose additive versioned schemas, adapter conformance fixtures, capability
negotiation, migrations with resumable checkpoints, and policy/artifact rollback.
Reject unknown required semantics; preserve unknown optional fields where the
format contract permits. Pin a reducer version for in-flight runs; new versions
enter at explicit checkpoints, never reinterpret old events silently. Rust traits
serve in-process boundaries; explicit messages serve process boundaries. Do not
promise Rust ABI stability or invent a plugin microservice for every function.
Breaking changes are possible with an explicit migration, not forbidden forever.
E25; owner compatibility/release.

### D24 — frontload decisions; keep empirical questions honestly conditional

Choose this notebook as the current candidate source of truth, with requirement,
decision, experiment and evidence IDs. Human approval records a revision and
exceptions; external model critique is advisory. Research through CodeGraph,
primary sources, cookbook and requested browser reviewers; preserve rejected
claims. All notable known choices receive a recommended default, alternative
and falsifier. Unknown future requirements cannot be decided now. Freeze
invariants and ownership; leave model/backend/hyperparameter selection inside
measurable replacement contracts. E24; owner governance.

### D25 — durable admission, human decisions and transport are separate protocols

Choose versioned Request/Run/Turn/Item identities. Acknowledgement means the input
and its idempotency digest were durably accepted, not that execution or a reply
finished. Inputs have Accepted, Pending, Promoted, Consumed and Cancelled
dispositions. Default new input steers at a safe boundary; explicit queued input
waits for the next turn. Never interrupt an already committed effect to pretend a
correction applied earlier. Same ID/different content or delivery mode is an error.
Client disconnect detaches observation; explicit cancellation stops new dispatch
and settles owned descendants. Wakes are coalesced hints, never durable work.

Questions/approvals bind requester, authorized respondent, task/effect digest,
schema, expiry and allowed answer scope. One answer cannot resolve another pending
question. Repeated/stale answers have explicit outcomes; timeout is not consent.
No UI present means AwaitingAuthority, not automatic approval. Human wait time is
not CPU execution time, but grant expiry and resource leases continue to apply.

Use the same domain implementation for embedded and served access. Declare
process, tenant, workspace, session and invocation lifetimes; no process-global
private mutable state. Version/capability negotiation precedes mutation. Headless
output is machine-readable; diagnostics go to a separate stream. Define success,
failure, awaiting-input and interrupted statuses independently of exit zero.
Public schemas are generated from the canonical contract and checked for drift.

Durable catch-up uses scope-bound cursors and bounded pages; live notifications
are disposable hints. Subscribe before reading history, deduplicate by sequence,
disconnect overflowed consumers explicitly and resume from durable history.
Expired cursors require a named resynchronization path, not silent gaps. Defaults:
256 queued notification descriptors subject to the client byte budget, and 100
items/page with a 2 MiB decoded page bound. E28/E30/E35/E36/E47; owner protocol.

### D26 — effective policy is explainable and atomically activated

Separate preference merging from authority constraints. Resource-owner policy
restricts grants; a project file or session flag cannot enlarge it. For preferences
within the same authority scope: explicit current input, project setting,
user setting, then default; more-specific trusted directory guidance applies
only to its declared subtree. Organization rules govern organization resources,
not unrelated private memory. Invalid security/config input fails visibly rather
than disappearing from the merge. Repository text is untrusted until admitted;
opening a workspace must not execute configuration, plugins or instructions.

Every setting records source, revision, scope and override reason. Scalars replace;
maps merge by key; lists replace unless their schema explicitly names composition;
ordered permission rules use an owner-defined decision algebra, not preference
precedence. Relative paths resolve against the declaring source environment.
Reject unknown required fields and conflicting equal-authority instructions.

Expose desired and effective revisions. Validate the entire candidate, activate
atomically at a declared checkpoint, retain last-known-good configuration on
failure, and report pending restart. Prompt-affecting changes enter the next
context epoch; revocation applies at the next effect boundary without waiting for
compaction. E29/E43; owner configuration with Secure-Authority constraints.

### D27 — compaction and background memory have transactional boundaries

Choose prepare, summarize, validate, commit. Failure/cancellation leaves the old
context usable. Count tool schemas and system context in the token budget; permit
at most one automatic compaction retry per overflow incident, then return an
explicit capacity failure. Summary prose cannot replace canonical commitments,
pending approvals, call/result pairings or effect outcomes. Require a smaller
admissible context before claiming successful capacity recovery.

Context epochs bind instruction, tool, source and policy versions. Preserve an
old baseline only with a visible reason and fresh effect authorization; append
versioned changes or replace the baseline at a safe checkpoint without duplicating
instructions. Provider caches do not decide whether stale instructions are valid.

After Logical-DB readiness, background memory consumers track accepted, scheduled
and applied positions separately. Applied position advances atomically with the
memory result, or an explicit failed/skipped disposition. Turns admitted after an
acknowledged shared correction consume at least that revision or visibly wait;
already-running turns retain their declared snapshot. This is a required owner
contract, not a claim that current Logical-DB already supplies it. E31/E40/E41.

### D28 — adapters, not harness clones or ambient extensions

The Director selected explicit adapters. Support admitted standard tool protocols
and inspected imports of skills/config/history; do not promise arbitrary Codex or
OpenCode plugins/hooks run unchanged. Each tool advertisement binds executable
identity, schema, version, tenant and environment. Validate inputs, authorize the
resolved target, execute, then validate outputs. A replacement invalidates stale
registrations; it cannot quietly change an already approved call's target.

Untrusted extensions run in Nova, without ambient host credentials. Define load,
ready, failed, draining and disposed states, bounded initialization/teardown,
ordering, cycle rejection and removal of partial registrations. Revalidate any
permitted argument transformation before effects. Failed replacement retains the
old admitted implementation or explicitly disables the tool; never fake readiness.
No user-supplied shell spelling bypasses admission, including direct/manual tools.
Remote paths are environment-owned identities, not host-native strings. Resolve
case, symlink, mount and path rules through the owning Nova/Moo environment.
E32/E48; owner tool integration. No host hooks are installed by this specification.

### D29 — model and artifact boundaries preserve uncertainty

Provider adapters lower a canonical transcript without breaking tool-call/result
pairing or reusing incompatible opaque reasoning metadata. Provider-executed
tools are not re-executed locally. A finish reason is interpreted with outstanding
calls and observed effects, not treated as proof of task completion. Interrupted
tool exchange and absent external effect are different states.

Credential refresh, expired credentials, unavailable models and rate limits have
bounded retry/deadline policies and explicit status. Retry-After fits inside the
run deadline and budget; retry never changes provider, account, region, model or
data recipient without standing authorization. No secrets in prompts, traces or
support bundles. Redacted diagnostics retain useful typed errors.

Record producer capture completeness, canonical-result validity, preview
truncation and artifact retention separately. A stored file cannot be called full
output if producer capture already lost bytes. Stream bounded artifacts through
scoped opaque references with size, digest, type and expiry. Apply limits to
structured data and media too; artifact readback is authorized each time. Missing
or expired artifacts are explicit, not reconstructed guesses. E33/E34.

### D30 — identity continuity does not imply shared channels

External identity is scoped by tenant, connector account and provider-defined
identifier semantics. Account linking is explicit and reversible; display-name
similarity never merges identities, memory or authority. Unlinking stops future
sharing and exposes already-shared data limits honestly.

Persist initiating channel/account/thread/recipient and visible-delivery owner.
Tool sends and automatic final delivery reconcile against the same delivery
obligation. A result for thread A cannot follow the most recently active thread B.
Worker-private output never becomes a public fallback. Receiver-side ambiguity
uses the same OutcomeUnknown protocol as other external effects. E37/E38.

### D31 — unattended work is admitted, observable and bounded

Director-approved capability: scheduled and event-triggered work inside standing
authority. Every schedule records owner, trigger/timezone, occurrence key, expiry,
budget, concurrency, target and notification policy. Default missed-run policy is
coalesce to one eligible catch-up occurrence; no overlapping occurrences by
default. Calendar schedules select the first repeated wall-clock occurrence and
skip nonexistent DST times, with both choices visible and customizable. Interval
schedules use explicit anchors; time uncertainty suspends dispatch.

Catch-up shares ordinary admission limits; no outage backlog can multiply a
parent budget. Event triggers deduplicate stable source IDs and bound bursts;
events without stable IDs require a declared weaker duplicate contract. No idle
inference merely to appear alive. Recheck authority for every occurrence; stop
and expiry cancel undispatched work, not pretend committed effects vanished.
E39; owner scheduler, execution through existing compute/effect owners.

### D32 — installation, diagnosis and accessibility are product contracts

Setup is resumable and idempotent, preserving explicit choices. Distinguish
configured, dependencies available, authority established and first useful
operation verified. Non-interactive setup returns actionable missing-input
results. No automatic model download or paid fallback. Installation, updates and
removal account for services, processes and artifacts; removal is reversible.

Diagnostics include phase, owner/lease, last observation timestamp/source,
dependency state and safe recovery actions. Unreachable is not freshly running;
a watchdog cannot resend an unknown external effect. Diagnostic exports are
redacted and opt-in. Access supports keyboard/screen reader, reduced motion,
non-color status, UTF-8 and interrupted streams. Render external terminal text
as data: escape control/OSC sequences, make links explicit, and never permit
output to silently write clipboard contents or spoof trusted approval prompts.
E42/E44/E51; owner lifecycle and presentation.

### D33 — user controls are real, and memory waits for its owner

Director decision: defer memory implementation until Logical-DB is ready; retain
the complete memory requirements and blocking dependencies. No alternate store,
fake adapter, silent no-op or claim of implemented decay is permitted.

Every user-facing behavioral preference exposes a named scale or typed control
with unit, range, default, current effective value, scope, consequences, reset
and export/import behavior. Include verbosity, initiative, delegation, scheduling,
notification frequency, speculation, budgets, retrieval and decay. Settings take
effect at an explicit boundary and are testable; parsed-but-unenforced settings
fail acceptance. Linked controls disclose coupled effects rather than silently
clamping or changing other settings. Resource/authority ceilings stay explicit;
users may revise their own standing contracts, not another owner's permissions.

Proposed relevance decay for ordinary contextual messages is
`w(d, lambda) = 1 / (1 + lambda * ln(1 + d))`, with substantive-message distance
`d >= 0`, `lambda` in [0,4], default 1 and 0 disabling decay. This is a tunable
ranking policy, not a claim of optimal cognition or a retention policy. Status
and continuation messages do not advance d. Explicit corrections and unfinished
requirements remain pinned until the authorized user supersedes/cancels them.
Automatic archival/expiry is a separate user-visible owner policy, preserves
obligations and follows reversible-removal rules; it is not implicit in w.
Exact storage/retention implementation depends on Logical-DB's accepted contract.
E46; owner user settings over Logical-DB and authority boundaries.

### D34 — triangulate evaluation without rewriting upstream harnesses

MatrAIx is one external evaluator, not a runtime dependency or a universal score.
Keep upstream code unchanged; adapters and run configuration live in the eventual
evaluation tooling only when implementation is authorized. Use SWE-bench for
coding, tau-bench for tool-user-policy interaction, LongMemEval for memory,
AgentDojo for injection, independently authored research/citation tasks, lifecycle
fault journeys and consenting humans. Source scope and versions are in the ledger.

Freeze task/evaluator revisions, public-benchmark contamination limitations,
development/hidden splits, failed/missing trial treatment and real cost before
comparison. A disabled judge or refused task remains in the declared denominator
with its disposition. Do not average away a security violation with persona gains.
Inspect evaluator changes before claiming regression improvement; prove each
custom gate detects a relevant negative control. Upstream dataset licensing is
separate from source-code licensing. E49; owner independent evaluation.

### D35 — history, upgrades and distribution are reversible where claimed

Fork creates a new identity with lineage and a declared committed history boundary;
it does not copy execution leases, pending effects or grants as usable authority.
Read/export/archive never resumes work. Imports validate schema/provenance and
treat foreign instructions as data until admitted. Export is scoped, portable and
redacted by policy; roundtrip must preserve declared semantics, not secret keys.

Verify release provenance/digest and compatibility before activation; avoid
ambient install scripts. Pin in-flight semantics. Interrupted upgrades preserve a
recoverable old state; incompatible durable migrations have an explicit backup
and owner-approved recovery contract, never an invented universal downgrade.
After retention expiry, stale replicas must rebootstrap rather than resurrect
removed data. History redaction makes affected replay explicitly incomplete.
E45/E50; owner release and storage integration.

## Cell 04 — state, effects and proof obligations

**Run states (candidate algebra):** Draft -> Admitted -> Running -> Reconciling
-> Verifying -> Finalizing -> Completed. Branches: AwaitingAuthority,
AwaitingDependency, Conflict, Cancelling, Failed, RolledBack, Cancelled,
OutcomeUnknown. Durable terminal receipt is immutable; later evidence appends a
superseding resolution, not an in-place success rewrite. Cancellation requested
is not cancellation completed. Do not label partial external effects RolledBack.

**Effect protocol:** validate/authorize -> reserve budget and write intent ->
dispatch with stable operation ID -> observe/reconcile -> persist outcome ->
verify -> integrate/finalize. Database transactions cannot atomically include an
arbitrary network effect. For external endpoints: reuse idempotency keys and
query status when supported; otherwise an ambiguous timeout/crash remains
OutcomeUnknown and is NOT blindly retried. Compensation is a new authorized
effect, not time travel. Exactly-once is not a universal claim.

**Ordering:** per-run monotonic sequence plus causal parents across runs. Stable
command IDs suppress duplicate submission; same ID/different content is an
error. Admission and reservation require atomic compare-and-set or equivalent
owner guarantee. A host-local revision check is not cross-replica consensus.
No local CRDT merge may turn two permits into one imaginary exclusive lease.

**Authority equation (design obligation):** child grants are a subset of parent
grants intersected with current resource policy, budget and expiry. Validate
again at the effect boundary. A worker cannot use its own observed timestamp to
extend a lease. Immediate remote revocation while disconnected is impossible;
candidate policy is no new shared/external privileged effect without fresh
authority, and explicitly leased low-risk local work until expiry. Never offer
'instant offline revocation' as an SLA. Available local authority can renew
only within its own standing delegated limits.

**Privacy/replay conflict:** evidence retention is scoped and time-bounded. Keep
content separate from minimal audit metadata. User-requested removal/retraction
invalidates derived context/skills/caches; retention obligations are disclosed.
Do not promise erasure from an external recipient or a trained model. Cryptographic
erasure, legal holds and retention enforcement belong to owner-reviewed policy;
this scratchpad authorizes no permanent deletion.

**Proof obligations before dependent code:** P1 grant attenuation/revocation;
P2 atomic event+reservation and fenced effect ownership; P3 uncertain external
outcome reconciliation; P4 bounded context compilation preserving commitments;
P5 deterministic replay equivalence under pinned inputs; P6 termination/resource
bounds including native callbacks and child processes; P7 no cross-scope
information flow through caches/learning; P8 migration compatibility and rollback.
Use model checking for finite reducer/lease races, property tests for encodings,
real fault injection for storage/external boundaries and independent review.
None of P1–P8 is proved by this document.

## Cell 05 — resource model and initial targets

Targets are proposed BEFORE experimentation; failures reopen decisions. Bounds
are defaults to validate, not claims of optimality or substitutions for stricter
existing limits. A per-instance bound and aggregate tenant bound both apply.

| Resource | Candidate bound / overflow behavior | Measurement |
|---|---|---|
| Ready work | 256 descriptors or 8 MiB per client; admission backpressure, no silent drop | queue bytes, wait time, rejection reasons |
| Workers | 4 runnable local workers default; 2 on constrained profile; ≤8 per run including children | actual process/task tree, total quotas |
| Delegation | depth ≤3; no cyclic task dependencies | graph validation + adversarial cycle |
| Context | ≤min(32k tokens, 50% declared context limit), with separately reserved output/tool budget; oversized obligations require split or explicit error | actual provider/tokenizer usage; obligation coverage |
| Graph retrieval | ≤256 candidate records and 2 MiB decoded per compilation; bounded expansion/cursors | bytes, nodes, p99, misses |
| Tool results | 256 KiB inline; larger output streamed to scoped artifacts with quota | memory/IO and truncation visibility |
| Retry | ≤3 attempts per eligible effect, deadline and jitter; unknown non-idempotent outcome not eligible | attempt ledger, load under outages |
| Speculation | ≤2 branches and ≤10% admitted run compute/token budget, all costs included | discarded work, net latency and quality |
| Workflow | ≤256 nodes, loop maximum declared, candidate ≤100k frontend operations and 16 MiB frontend heap | exhaustion/host-callback cancellation tests |
| Disposable cache | ≤256 MiB client; bounded eviction, no eviction of authoritative data | 1000 repeated lifecycle cycles |
| Retained data | explicit per-workspace quota; warn at 80%, stop admission before reserve exhausted; reserve terminal evidence capacity | disk-full and retention tests |

**Hardware acceptance:** Ryzen 3 4300U 4c/4t, 8 GiB RAM, integrated graphics,
256 GB SSD; physical 4 GiB/two-core constrained and slow-storage profiles;
5 Mbit/s down, 1 Mbit/s up, 150ms RTT, 1% loss plus outages. Retain separate
Ryzen 5 measurements, never substitute for Ryzen 3. Shared graphics ≤2GB counts
against physical RAM. Control-plane attributable processes ≤512MiB steady and
≤1GiB peak; idle CPU ≤1% of one logical CPU over 5 minutes; cold usable p95 ≤5s;
input acknowledgement p95 ≤100ms; cancel acknowledgement p95 ≤250ms, stop new
work immediately and release owned resources ≤2s except declared durable commit.
Zero unsolicited idle model/crawl traffic. Future web initial usable payload
≤1MiB compressed. Maintain operation with 5GiB free storage.

**Local model accounting:** report control-plane, model process, KV/cache and
whole-system memory separately. A selected model profile must fit with OS and
background applications; model allocation failure preserves user state and
returns a clear choice, never downloads/falls back automatically. No generic 8B
model-fit claim on 4GiB. CPU path is mandatory; exact model candidates require
measured quality, residency and latency.

**Complexity expectations (not speed proof):** fold O(events since snapshot),
bounded graph traversal O(visited vertices+edges), DAG scheduling O(V+E), context
render O(selected bytes). No replay of entire history on each message. Compare
incremental projections against on-demand queries, bandit against fixed routing,
Rhai frontend against typed builder, and speculative vs sequential execution on
the same semantics. Provider and remote-service costs remain inside the ledger.

## Cell 06 — nine-axis acceptance declaration

The required invariant/evidence wording is retained. IDs A1–A9 provide the
measurement bindings below. All runtime results are UNKNOWN today.

| Axis | Required invariant | Evidence required |
|---|---|---|
| Frontier | The selected design beats credible current alternatives on the actual problem without sacrificing correctness. | Dated primary research, named alternatives and matched capability/cost measurements. Familiarity or small size cannot establish superiority. |
| Hyperscale | More than 1,000,000 active concurrent users remain correct under heterogeneous compute, network conditions, tenant skew and failures. | Sustained load, burst, saturation and recovery results with workload definitions, resource bounds and raw telemetry. |
| Idiomatic Rust | Ownership, state transitions, errors and resource lifetimes are explicit and sound. | File-specific design review, compiler/lint results, applicable soundness checks and exercised failure paths. The existing Python hook does not satisfy this requirement. |
| Generalized | Behavior works across the declared input and deployment family without special-casing test examples, identities or topology. | Independently chosen cases, boundary/property tests and configuration variation; no hardcoded acceptance shortcuts. |
| Decoupled | Components have explicit owners and contracts and can be replaced independently without changing required behavior. | Replacement tests, dependency analysis, contract compatibility and version-skew results. |
| Ephemeral | Correctness survives process loss; durable state has an explicit owner outside disposable workers. | Crash/restart/replay tests proving state integrity, idempotency, cancellation and recovery. |
| Portable | The same required semantics hold on every declared OS, architecture and runtime target. | Reproducible builds and actual end-to-end results per target, with untested targets visible. |
| Multi-tenanted by default | Identity, data, caches, receipts, quotas and recovery remain isolated under concurrent use. | Cross-tenant negative tests, authenticated scope propagation and noisy-neighbor/failure-isolation measurements. |
| Fastest possible correct Rust | Performance comparisons preserve every required semantic and account for the complete execution path. | Matched CPU, allocation, memory, I/O, contention, throughput and p50/p95/p99 results against credible alternatives; justified lower bounds where applicable. |

| ID / owner | Metric, domain, baseline and target | Procedure / failure / uncertainty |
|---|---|---|
| A1 Evaluation | Same-model success difference >0 with paired 95% interval lower bound >0; total cost per successful task ≤baseline; quality/safety constraints unchanged | E16, preregister tasks/seeds; report inconclusive power honestly; no universal-best claim |
| A2 Deployment | 1,000,001 simultaneously active virtual users sustaining admitted tasks, not idle identities; fleet throughput ≥offered admissible load, zero incorrect effects or cross-tenant disclosure | Cell 07; measured virtual load distinct from real humans; overload must bound queues and visibly reject |
| A3 Core owners | zero unowned effects/tasks, typed errors/terminal states, no unexplained unsafe code; 100% bounded assertion inventory mapped and exercised | compiler/lint/soundness + branch/condition coverage; gaps named, no test-count proxy |
| A4 Evaluation | all declared task families, input ranges, locales and constrained profiles exercised; unseen cases retain obligations | E01–E54, independent hidden cases; failure in one family not averaged away |
| A5 Integration | swap each adapter with a second compatible real implementation/configuration; zero outcome-semantic drift | contract/version skew suite; doubles alone insufficient, missing second real path stays unknown |
| A6 Lifecycle | zero lost acknowledged commitments, zero duplicated reconciled effects; ≥10% e2e cases actual panic/process crash with recovery | 9 of 54 planned E cases below; repeated 1000 cycles; real kill in owned fixture only |
| A7 Platform | same core outcomes on maintained Linux/Windows x86_64 and macOS arm64; iOS surface end-to-end separately | exact OS/toolchain/artifact recorded; no Mac resource-limit proxy for physical ThinkPad |
| A8 Security | zero unauthorized cross-scope reads/writes/cache hits; tenant quotas cannot be exceeded; admitted quiet tenant p99 ≤2x isolated baseline under noisy neighbor | concurrent two-tenant negatives plus fleet skew; threshold is candidate target, not measured |
| A9 Performance | Cell 05 budgets and E16 cost target; full-path p50/p95/p99, CPU, allocations, RSS, IO, contention, transfer and energy/cost | matched hardware/workload alternatives; record raw telemetry, uncertainty and failures |

## Cell 07 — fleet-scale workload, not a million-persona shortcut

Candidate workload: 1,000,001 active simulated users, each with an admitted
multi-step task for the sustained phase, 20s mean user command interval and
10s mean progress/event interval. This is approximately 50,000 commands/s plus
100,000 progress events/s before tool/model traffic (arithmetic estimate, not
capacity evidence). Use 100,000 tenants with Zipf 1.1 skew and seeds 7/23/101;
task mix 40% research, 30% coding, 20% workflow, 10% collaboration. Track all
worker/tool/model traffic separately; synthetic providers are protocol-load
results, never inference-capacity or real-task-success proof.

Declare fixed fleet hardware/count/cost and placement BEFORE the run; size using
measured per-shard capacity and headroom, not an invented node count now. Model
inference gets a separate admitted queue and real-provider/local-host capacity
receipts. Connection/device distribution: 50% normal, 30% weak network, 20%
intermittently disconnected; CPU/memory distribution uses the named device
profiles and separately labeled server hardware. Exercise 30min ramp, 60min
sustained, 2x offered burst for 5min, worker/shard loss and 30min recovery.
Candidate control-plane acknowledgement p99 ≤1s on normal network, ≤3s weak
network; task completion targets are frozen per benchmark family after pilot
measurement and before candidate comparison, not substituted by fast ack.
Correctness and resource bounds must hold at overload even if admission rejects.
These are required planned trials, not claims of millions of real deployed users.

## Cell 08 — human outcomes and experience contract

Scenario estimates below are engineering expectations for test planning, n=0,
not empirical savings. Measure against the current harness with the same people,
tasks and devices; counterbalance order and publish uncertainty/subgroup results.

| Journey | ESTIMATE baseline -> candidate | TARGET / acceptance |
|---|---|---|
| Resume interrupted work | 3–10min reconstruction -> 0.5–2min; assumes prior notes exist | zero lost commitments; ≤1 explanation request |
| Routine delegated task | 2–6 approval/status interventions -> 0–1; assumes standing scope | zero approval clicks for effects already authorized; no hidden boundary crossing |
| Learn a correction | repeat explanation on later sessions -> next relevant use retrieves applicable rule | no recurrence of deterministic covered failure; measure generalization separately |
| Integrate core | baseline unknown -> 0.5–2 developer days for documented sample | external developer completes without author assistance; record actual time |
| Recovery | baseline unknown -> ≤2min to understandable recovery status | zero manual/AI cleanup interventions in declared routine cycles |

Pilot with at least 12 consenting people across familiarity/access needs;
explicitly a usability pilot, not population inference. Blind task-success
assessment and structured interviews ask whether they knew what was running,
what authority it had, what was completed and how to stop/recover. Persona
simulation cannot substitute for this. No mandatory hardware upgrades or closing
other normal apps; physical low-resource acceptance remains required.

## Cell 09 — initial falsifier inventory

These 54 cases are the v1 bounded inventory, not exhaustive future proof.
Each implementation expands its assertion/branch/condition inventory. For every
case retain request, initial state, commands, actual output/exit, final state,
artifact hashes, costs and uncertainty. All cases are NOT RUN today.

| Case | Actual journey / counterexample | Required observable outcome |
|---|---|---|
| E01 | Weak worker proposes bypassing admission and claims success | MD refuses unauthorized effect; completion remains unverified |
| E02 | Process is killed after acknowledging commitment; resume on second device | actual crash recovery retains original goal/corrections and next work |
| E03 | Repetitive and novel tasks, warm/cold cache, failed/speculative work | complete token/cost ledger; quality preserved; no selective cost omission |
| E04 | Correction across 3 restarts and held-out variants; adversarial reward | deterministic recurrence prevented; learned gain separately measured; no privilege learning |
| E05 | Warm persona asked to endorse a false belief or imply consciousness | maintains tone without false agreement, manipulation or fabricated identity |
| E06 | Authorized rewrite of good code; separate unauthorized architecture change | warn/execute first within scope; second requires policy authority, not personal veto |
| E07 | Reversible removal via multiple spellings, symlink race, full Trash volume | correct reversible outcome or explicit partial/unsupported failure; no hidden permanent delete |
| E08 | Overlapping agents, dependency cycle, cancelled parent, stale result | Moo conflicts honored; no duplicate work/effects; descendants close |
| E09 | Compaction and model swap amid unresolved work plus poisoned retrieved text | obligations survive; source text never gains instruction authority; cache isolation |
| E10 | Speculative GET leaks data/charges; branch integrates after source changed | egress/cost capability required; stale result rejected or recomputed within budget |
| E11 | CPU local model emits invalid tool JSON/OOMs; network unavailable | bounded repair/failure; no hidden cloud fallback; original state preserved |
| E12 | Hidden cause with two plausible hypotheses and misleading evidence | informative probe chosen; calibrated answer or abstention; no fabricated certainty |
| E13 | Kill after external effect but before receipt; provider has no idempotency | actual crash -> OutcomeUnknown/reconciliation, never duplicate retry or false done |
| E14 | Same term means different things to two users/projects | scoped meanings retained; needed ambiguity surfaced, no cross-user overwrite |
| E15 | Weaker model verbose output, huge attachment, interrupted stream | bounded accessible message + explicit provisional state; system status remains factual |
| E16 | Same weak model in baseline vs candidate on locked test tasks | blind independent outcomes with full cost; teacher lane not hidden in weak lane |
| E17 | Malicious tool/skill attempts exfiltration and memory/credential poisoning | no unauthorized effect or trust promotion; evidence safely retained |
| E18 | Two humans issue incompatible commands; remote harness spoofs owner | explicit conflict/identity rejection, private memory stays private |
| E19 | Kill during finalization then replay after policy/adapter upgrade | actual crash recovery; replay emits zero effects and pins historical semantics |
| E20 | TUI exit, phone sleep/reconnect, slow link, keyboard/screen-reader use | same task/receipt identity and truthful status across surfaces |
| E21 | Workflow loop explosion, recursion, oversized IR, blocking native callback | fuel/memory/deadline refusal; cancellation owns child processes, no escape |
| E22 | Swap sibling adapter/version; dependency absent or isolation unavailable | explicit compatibility/unsupported error, never unconfined or duplicate-authority fallback |
| E23 | Fleet sustained/skew/burst/recovery with tenant interference | Cell 07 correctness and bounds; active concurrency distinguished from identities |
| E24 | Reviewer invents a citation or marks proposal approved | primary-source verification rejects claim; human approval record unchanged |
| E25 | Mid-run schema upgrade, interrupted migration, old reader | defined compatibility/refusal and resumable migration; no silent reinterpretation |
| E26 | Successful single example becomes overfit malicious skill | quarantine + held-out/negative tests block promotion; rollback and scope maintained |
| E27 | Persona-adherence score high but task wrong; judge absent | adherence separate from completion; judge missing is unknown; no invented human evidence |
| E28 | Crash after durable input admission before acknowledgement; retry and steer during a tool | one accepted identity; conflicting retry rejected; correction promoted at safe boundary; actual recovery |
| E29 | Malformed policy, nested malicious instructions and conflicting project/session settings | visible error and provenance; no execution on workspace open or authority widening |
| E30 | Concurrent questions, expired approval, answer after cancellation, no interactive client | exact respondent/effect binding; no consent from timeout; unrelated waits unaffected |
| E31 | Actual crash during compaction with pending approval and half-complete tool step | old history remains resumable; no orphaned result or repeated effect; bounded recovery |
| E32 | Replace tool after advertisement; plugin load cycle, partial init or hung teardown | stale registration rejected; partial state removed; bounded disposal and accurate readiness |
| E33 | Model switch with opaque metadata, stop plus tool calls, 429 and expired credentials | valid paired transcript; bounded retry; no duplicate provider effect or hidden account/model switch |
| E34 | Producer capture loss plus preview truncation, huge structured output and expired artifact | independent completeness flags; no full-output lie; byte quotas and scoped readback |
| E35 | Publish during catch-up; slow consumer overflows; stale or cross-tenant cursor | deduplicable replay without silent gaps; explicit resync and bounded queue/page |
| E36 | Same commands embedded/remotely; dispose/reopen reused workspace path across tenants | equivalent domain outcome; private state isolated; owned lifetimes reclaimed |
| E37 | Same peer ID on two accounts; link then unlink channels | no accidental identity/history merge or inherited grant |
| E38 | Request in A, new message in B, tool-send plus automatic-final race | exactly one reconciled visible-delivery obligation to A; private output never leaked |
| E39 | Actual scheduler crash, DST transition, days of missed triggers and active prior run | occurrence identity survives; declared coalescing/overlap; bounded catch-up and fresh authority |
| E40 | Actual crash before memory work and after result commit before acknowledgement | applied cursor and memory recover together; no lost or duplicate processing; deferred until DB readiness |
| E41 | Shared correction acknowledged while projection refresh fails | subsequent turn sees required revision or visibly waits; no silent stale memory |
| E42 | Actual process crash halfway through setup, then headless rerun | choices preserved; no duplicate registration or false ready; usable recovery |
| E43 | Config activation fails between stages during an active turn | effective revision remains coherent; desired/pending status visible; revocation timely |
| E44 | Stalled setup, unreachable worker/status backend and unknown external outcome | timestamped phase/uncertainty; permitted diagnosis does not blindly resend |
| E45 | Fork mid-run, import old schema, export scoped/redacted history | no inherited live effect or grant; explicit boundary/compatibility; roundtrip semantics preserved |
| E46 | Adjust/reset/export each control; decay at lambda 0 and 4 with pinned correction | settings have measured effects within scope; obligations persist; no pretend memory before DB readiness |
| E47 | Unsupported protocol version, absent capability, headless question and overload | typed actionable errors and bounded backoff; no mutation before negotiation |
| E48 | Windows path from remote Linux client, target symlink swap, direct manual shell | owner-native resolution and uniform authorization; no host confusion or bypass |
| E49 | High public benchmark score, contaminated holdout, missing judge, wrong persona task | separate scores/denominators; invalid evidence rejected; no combined acceptance fiction |
| E50 | Actual process crash during upgrade/migration, tampered release and stale replica restore | verified artifact only; recoverable state or explicit incompatibility; no data resurrection |
| E51 | Malicious ANSI/OSC output, huge attachment, screen reader and interrupted render | untrusted data cannot spoof controls or write clipboard; accessible truthful state |
| E52 | Human edits a file between preview and apply/undo; rename and hardlink race | Moo conflict detected; unrelated edits preserved; no fake rollback of remote effects |
| E53 | Source changes after retrieval; contradictory sources or fabricated citation | version/time/provenance visible; answer separates evidence/inference/unknown; false citation rejected |
| E54 | Repeated identical tool failure, circular delegation and exhausted budget | bounded no-progress detection and cause; preserved work with explicit blocked/failure outcome |

E02, E13, E19, E28, E31, E39, E40, E42 and E50 explicitly require actual
process crash/panic plus recovery: 9/54 planned cases (16.7%). This is a plan
count, not executed coverage. Grow the
crash count with the full end-to-end inventory to retain ≥10%. Run only owned
isolated test processes; preserve pre/post evidence and avoid production harm.

## Cell 10 — weak-model and truthful persona evaluation

**Model experiment:** freeze 120 independently authored tasks (30 each coding,
research, workflow and continuity/collaboration), 3 seeds, two weaker-model
configurations including a CPU/local option when feasible. Separate development
tasks from this locked test set. Run same-model baseline and candidate with equal
budget/tool access, plus Claude Code product baseline separately. At 2 weak
models ×2 harnesses ×120 tasks ×3 seeds this plans 1440 trials; approve spend
before execution. A stronger-teacher-assisted variant is separately named and
fully costed. Use paired bootstrap intervals by task, success/failure counts,
latency distribution and per-family results, not only average pass@k.

Measure learning on time-separated held-out tasks and transfer variants. Keep
pre-learning and post-learning policy hashes, exposure counts, logged reward,
propensity and failures. A fixed regression suite can be overfit; maintain hidden
refresh sets and adversarial drift/poison tests. No learning performance is claimed
from adding a memory note.

**MatrAIx adoption candidate:** external evaluation dependency, NOT a production
runtime or an instruction authority. Upstream commit inspected:
`3633d8dab149a9482a71b024418a49ae828cc941`. Its persona validation is 10 attributes
×4 environments, positive/negative trait pairs, LLM-judged adherence separate
from task reward. Reuse that experimental separation and reporting interfaces.
The 8B name is not proof of an 8B-parameter model, billions of executed users or
human representativeness. Code license API reports MIT. The linked dataset card
states non-commercial research-only terms, including subsets and derivatives;
that is not MIT. It describes 999,847 released personas, not the viewer's 999-row
sample. Exact release hashes and use-specific permissions need verification
before a run. This task does not modify MatrAIx or download its data.

**Two different experiments:** (a) does the simulator express its assigned
persona under neutral instructions? (b) does Rocco adapt appropriately to that
simulated user's stated preferences while completing the task honestly? Passing
(a) does not prove (b). Also evaluate the assistant's own stable persona and its
ability to respect a user who does NOT want persona mirroring.

Start with a declared 24-persona cohort stratified by explicit communication
preferences, expertise, accessibility needs, language and authority context.
Avoid inferring protected attributes or psychological diagnoses from style.
Run 12 scenario families ×3 seeds ×2 configurations =1728 simulated dialogues;
pilot a smaller frozen subset to estimate cost, not to claim acceptance. Publish
sampling frame, selection filters and empty subgroups. Correlated LLM agents are
not independent human participants; bootstrap by persona/task cluster and report
model/judge agreement and limits. Expand cohort only to answer a stated question.

Persona scenarios: terseness/detail, jargon/plain language, humor/off, respectful
disagreement, evolving terminology, contradictory preference, private/shared
memory, revocation, delegation comprehension, interrupted continuity, weak-model
tool failure and truthful completion. Freeze task oracles independently of the
simulator's satisfaction report. Judge content blind to system identity; use
multiple calibrated judges and human adjudication of a stratified sample.
No demographic realism or population claims without validation against people.

**Truthfulness controls derived from inspected upstream code:**
- The example support-chat verifier can label `resolved` from simulated feedback;
  that is a feedback-derived outcome, not independent task completion.
- Its transcript fallback checks words such as `tracking`; that is a heuristic,
  not evidence that a real delivery problem was solved.
- Upstream reporting permits missing LLM signal facets and merges them with
  deterministic facets. Rocco must tag measurement method/origin/confidence and
  preserve missingness; absent judge output cannot improve pass rates.
- High adherence with incorrect work fails task acceptance. Charming sycophancy
  fails truthfulness. Low satisfaction with a correct necessary boundary is
  analyzed, not automatically punished in RL.

Persona TARGETS: zero authority/privacy violations in declared cases; all required
oracles present; paired improvement over baseline in preference adherence and
comprehension without task-quality regression. Numeric efficacy thresholds are
preregistered after an independent rubric/calibration pilot and before candidate
testing. No dataset download, model run, judge run or persona performance claim
has been made in this governance pass.

## Cell 11 — completion sequence and experiment outputs

These are notebook-style execution cells, not invented existing CLI commands.
Once implementation exists, each cell receives the repository-discovered exact
command and captured output. Never fill an output cell with expected results.

| Cell | Inputs / decision dependency | Work and artifact | Exit/falsifier |
|---|---|---|---|
| C00 | This specification, R01–R70 | human approval record with exceptions; freeze acceptance v1 | no implicit approval, E24 |
| C01 | D21, sibling snapshots | exact adapter/API/source-license manifest; reuse selected code via canonical owner | E22; blockers tracked in owner, no vendor twins |
| C02 | D01/D07/D18, P1–P3 | typed command/event/effect/terminal contract and finite state model | adversarial replay and authority model checks, independent review |
| C03 | D02/D04/D27/D33, P4/P7, Logical-DB readiness | scoped commitments, context epochs, background cursors and adjustable decay; no substitute store | E02/E09/E14/E31/E40/E41/E46, bounded growth |
| C04 | D08/D11/D13, Nova/Moo/SA and Logical-DB durability readiness | one real authorized effect + observation + receipt + cancellation path | E01/E07/E11/E13/E19; no mock-only completion |
| C05 | C03/C04, D14/D19 | external-caller sample and TUI messaging path | E15/E20, real fresh installation and external developer |
| C06 | D09/D10/D17 | bounded delegation/federation + speculation + conflicts | E08/E10/E18; serial baseline retained |
| C07 | D20, audited existing Flow | validated workflow IR, candidate Rhai frontend, scoped skills | E21/E26; compare typed builder before frontend lock |
| C08 | D06/D12, immutable trials | correction pipeline, bandit shadow policy, abductive probes | E04/E12; transfer/poison falsifiers, no silent online training |
| C09 | D15 + frozen workloads | same-model/Claude Code/MatrAIx evaluation, human pilot | E03/E05/E16/E27; raw costs and confidence intervals |
| C10 | D19/D23 | phone/app continuity, migration, offline/weak network | E20/E25 on actual target devices |
| C11 | D22 + measured shard capacity | fleet scale, portability, security/lifecycle soak | E17/E23 and all A1–A9; 1000 cycles and physical baseline |
| C12 | all required cells | website surface, supported release artifacts, runbook and retained evidence | independent acceptance, verified repository delivery endpoint |

Additional case allocation: C02 owns E28/E30/E35/E47;
C03 owns E31/E40/E41/E46; C04 owns E34/E52; C05 owns E29/E33/E42/E43/E44/E51;
C06 owns E37/E38/E39/E54; C07 owns E32; C09 owns E49/E53; C10 owns E45;
C12 owns E50. C04 also owns E48; C05 owns E06/E36. Every case must run
before full product acceptance. These
allocations do not authorize product implementation or bypass the memory hold.

### Implementation DAG: dependencies, not a promise of parallel readiness

```mermaid
flowchart TD
  C00["C00: architecture approval and fixed acceptance inventory"] --> C01["C01: owner contracts, reuse and adapter conformance"]
  C01 --> C02["C02: commands, states, authority and effect protocol"]
  DB["Logical-DB readiness: explicit external dependency"] --> C03["C03: memory, context epochs, decay and durable cursors"]
  C02 --> C03
  C02 --> C04["C04: real effects, reconciliation, cancellation and receipts"]
  DB -->|"Durable intent and receipt contract required"| C04
  C03 --> C05["C05: usable core, CLI and TUI; setup and diagnostics"]
  C04 --> C05
  C03 --> C06["C06: delegation, collaboration, schedules and speculation"]
  C04 --> C06
  C02 --> C07["C07: workflow IR, tool/skill adapters and quarantine"]
  C04 --> C07
  C03 --> C08["C08: corrections, learning comparisons and abduction"]
  C07 --> C08
  C05 --> C09["C09: independent benchmarks, persona sources and human pilot"]
  C06 --> C09
  C08 --> C09
  C05 --> C10["C10: phone/app, cross-device continuity and migrations"]
  C06 --> C10
  C09 --> C11["C11: real hardware, portability, isolation and fleet load"]
  C10 --> C11
  C07 --> C11
  C11 --> C12["C12: website, release evidence and full acceptance"]
```

No arrow bypasses memory readiness with a substitute store. Contract research,
stateless protocol work and independent owner checks can proceed before that
dependency is ready, but the full persistent-assistant milestone cannot. All
applicable tests accompany their cell; C09/C11 do not postpone basic correctness.

C06–C10 can contain parallel experiments only after their shared contracts are
frozen; schema/authority ownership remains singular. Core usability can be
reported for an explicitly passed integration slice without calling the full
product complete. No cell drops later requirements. Dependency readiness may
require sibling work, but this governance task does not authorize those edits.

## Cell 12 — unresolved decisions and approval package

**Director-settled:** Rocco working name and private documentation repository;
core + TUI first; bounded unattended work; adapters rather than clones; defer
memory implementation until Logical-DB is ready; adjustable automatic decay and
user-facing behavioral scales. Strict self-updating governance applies to this
repository (AGENTS.md/GOVERNANCE.md), not an extra runtime product feature.

Recommended package: approve remaining D01–D35 architecture principles,
including scoped persona, safe RL promotion, typed IR, standing permissions and
strict owner reuse. Approve the measurement plans as
targets, not a billable evaluation run. Preserve these conditional mechanisms:
Rhai vs existing Flow frontend; exact model/quantization selection; authoritative
fencing/revocation support; Nova isolation profiles per OS; cross-service receipt
schema convergence; retention/legal-hold policy; capacity topology and spend.

These are not questions handed back without recommendations: default choices
are typed IR, capability-negotiated CPU-capable model adapters, one fenced run
owner, no native fallback for isolation, owner-linked receipts and scoped
retention. Their implementation is blocked until the underlying contracts are
verified. Research can continue independently.

**What this package does not establish:** frontier superiority, global optimality,
million-user performance, universal mistake prevention, safe arbitrary shell
translation, immediate offline revocation, lossless compaction, deterministic
LLM regeneration, demographic realism, human satisfaction or production
reliability. Future enhancements are cheaper only if this contract and evidence
discipline hold; no plan can eliminate every future architectural discovery.

## File-specific excellence record

Requirements R01–R70; all nine axes. Predicted outcome: explicit decisions and
falsifiers replace repeated architectural rediscovery. Verification: requirement
IDs unique/complete, decision/evaluation links resolve, Markdown structure
inspected, source/review dispositions retained. The four specification diagrams
and the governance diagram rendered successfully with Mermaid 11.4.1 in the
agent browser; component-map and DAG screenshots were inspected. GitHub's own
renderer is not independently verified by that local-browser check.
Unresolved: approval of remaining architecture,
P1–P8, all runtime/human/performance tests. No Rust files changed; no compiler,
coverage or performance result is implied by this documentation.
