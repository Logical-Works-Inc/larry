---
type: Research
title: research-ledger.md, part 2 of 7
description: Part 2 of /research-ledger.md, in order; the index lists every part.
resource: /docs/research-ledger/02-2026-09-19-board-first-workspace-and.md
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
## 2026-09-19 board-first workspace and selective backend reuse

READ: bounded Codex, Cline, OpenCode, AppCUI and Forge/Moo source inventory and
design dispositions are recorded in [workspace ADRs](/docs/workspace-experience.md).
Codex separates thread resume from turn start; this led to identity-bound
Continue without claiming execution. Cline's welcome prompt was rejected for
this entry point; OpenCode's independently scoped context panel was retained.
The backend agent adapted Forge's baseline/fence guard with MIT notice, not
its synthetic Moo allocator. CodeGraph source receipts and test boundaries are
in `crates/larry-contracts/WORKSPACE-EVIDENCE.md`. No sibling was edited.

The local query layer was used before implementation and again at the routing
pivot. Initial Python failures and the isolated metadata repair are retained
in the ADRs; a model's relevance rank never overrides source or scope. Native
computer-use terminal control was refused, so it was not bypassed. The emitted
SVG was instead converted locally and visually inspected as a static render;
that establishes presentation only. Actual POSIX PTY rendering, input and
mouse events remain separately testable.

### 2026-09-19 Porsche-wrapped Jira correction and local query repair

READ: Porsche Design System v3 palette migration, typography and motion pages;
Atlassian foundations, elevation and motion pages; and Apple HIG layout guidance.
Their common design consequence is not a visual brand transplant: use neutral
surfaces for structure, reserve one focal elevation/selection, apply colour to
meaningful feedback, and separate adjacent jobs spatially. W05 therefore
replaces the prior coloured-region terminal pass with "Quiet Torque": ANSI16
black/gray/white structure, blue selection, and text-labelled amber/red only.
Porsche showroom mimicry and untouched Jira were rejected as respectively too
ornamental and too generic for the return-to-work job. Source URLs are retained
in ADR-W05; source claims are design inputs, not user-study evidence.

MEASURED: the local Qwen reranker initially failed in its own import path when a
stray empty NumPy metadata directory yielded no declared version. Imported NumPy
was 2.4.4. `rerank_local.py` now falls back to that imported package version only
for unusable NumPy metadata, before Transformers validates dependencies. Six
unit tests and an actual no-overlay rerank passed; pre/post receipts are
`20260919-porsche-jira-pre-exec.json` and
`20260919-query-tool-fix-post-exec.json`. This repair makes the query tool
portable across that metadata inconsistency; it does not make Qwen an authority
on product truth or visual quality.

MEASURED after the W05 implementation: the final local post-exec comparison
ranked quiet-torque 1.0, Jira-default 0.5625 and Porsche-showroom 0.0134
(`20260919-porsche-jira-post-exec.json`). The actual macOS debug binary also
completed its five-minute 120x32 PTY measurement with 0 idle output bytes and
0.0% delta CPU; exact timing/RSS and binary hash are in the final metrics
receipt. Neither result proves visual acceptance, portable performance or
frontier superiority.

### 2026-09-19 rail affordance and terminal backend correction

READ: local AppCUI 0.5.1 `backend.rs` documents Termios as the macOS default
and limits mouse support to Windows/NCurses; its CrossTerm implementation
enables mouse capture and decodes terminal pointer events. The user-facing
consequence was independently reproduced: injected SGR pointer bytes rendered
as input under Termios. Larry now selects CrossTerm from the already-admitted
AppCUI package, rather than claiming an event-handler unit test made the macOS
terminal clickable.

READ: local Forge `forge-tui/src/theme.rs` and `render.rs` place selection on
one small slate-light list surface with a leading highlight symbol. Larry
reimplements that calm control grammar in AppCUI with an ANSI blue focus row;
no Ratatui code, dependency, or broad colour palette was copied. MEASURED: the
native 120x32 PTY journey sends an SGR click to rail row three and reaches its
work plus the honest missing-Moo refusal. Its cursor-addressed output is
emulated in the test harness so delta painting is interpreted as a screen, not
mistaken for contiguous text. This remains local terminal evidence, not a
cross-terminal or human study result.

MEASURED W06: the mouse-enabled macOS debug binary completed the fresh
300.124-second 120x32 PTY probe with 0 idle bytes, 0.0% CPU delta, 7,824KiB
sampled peak RSS, 0.892ms p95 input, a clean reaped exit, and SHA-256
`c7c2882f7a186521e6b46424bd3417f92000e54470a6d44e3905d1d9d1b47968`.
The exact input distribution and scope limits are retained in
`docs/evidence/workspace-final-metrics.json`; no result transfers to Ryzen,
other terminals, fleet load, or human preference.

## 2026-09-17 native visual pass receipts

READ: published AppCUI 0.5.1 README lists Tab, TreeView, ListView, splitters,
TextArea and Canvas. Its `src/system/builders/input_app_builder.rs:7–17`
documents input-driven custom drawing without periodic ticks. This selected
InputApp for quiet chrome rather than a dense stock-window layout. The same
source defines key repaint and close semantics; real PTY journeys exercise it.
No full editor is inferred from TextArea. Local RAG returned no AppCUI hits;
Context7 returned unrelated libraries, not usable AppCUI evidence.

READ/MEASURED: canonical estate admission in PR #7 records the explicit
Director AppCUI choice and estate-use instruction. No cryptographic human
signature is required by the inspected policy; the earlier extra-signoff claim
was wrong. After explicit release-continuation authorization, publish uploaded
`lgwks_deps` 0.1.8 from 822307da2c7adefccd09b449af2998c75deea42b. Fresh Cargo
download confirmed `appcui` and Larry pins that published artifact, not a path.
Source: https://crates.io/crates/lgwks_deps/0.1.8 and
https://github.com/srinji-kaggss/logicalworks-crates/pull/7.
The native view and tests use that real dependency; backend and workspace
effects remain simulated/absent as inventoried in the visual-pass document.
