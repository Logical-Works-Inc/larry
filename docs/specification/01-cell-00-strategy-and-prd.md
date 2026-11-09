---
type: Design
title: specification.md, part 1 of 18
description: Part 1 of /specification.md, in order; the index lists every part.
resource: /docs/specification/01-cell-00-strategy-and-prd.md
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
## Cell 00 — strategy and PRD

**Diagnosis.** A prompt loop makes the model repeatedly reconstruct intent,
authority, context, coordination and evidence. The human becomes the runtime's
memory, permission interpreter, supervisor and cleanup service. More tools and
longer prompts can increase this burden rather than solve it.

**Guiding policy.** One persistent relationship; disposable, specialized workers;
deterministic ownership of commitments, resources and effects. Put cognitive tax
on engineering once, rather than on every user and every model turn.

**Product sentence.** Larry is the assistant that remembers what matters, acts
within your standing agreement, coordinates work and can show what got
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
authority, browser engine, editor or container runtime inside Larry. No permanent
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
