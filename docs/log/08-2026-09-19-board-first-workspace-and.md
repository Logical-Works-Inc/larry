---
type: Log
title: log.md, part 8 of 15
description: Part 8 of /log.md, in order; the index lists every part.
resource: /docs/log/08-2026-09-19-board-first-workspace-and.md
tags: [governance, decisions, evidence]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: explicit conversation decisions and source inspection}
status: active
stale_after: 2026-10-15
sources: [Director conversations 2026-09-15 through 2026-09-17, /specification.md, /research-ledger.md]
---
## 2026-09-19: board-first workspace and visual-fidelity correction

Director correction: the earlier mockup looked like a drawing. Rocco must be a
lightweight terminal workspace for a human, Rocco the System and an AI: start
with Kanban state, open work as Continue, retain dedicated chat without making
the whole page chat, and make disconnected fixture data visibly honest.

Decision: retain AppCUI and the single terminal binary, but replace the sparse
transparent composition with an ANSI16 spatial shell: work rail, teal task
cards, green focus, purple System attention stack, and a distinct Human + AI
pane. The counterpart is an input-driven focus response, not an idle loop.
This is informed by READ-only study of local Codex/Cline/OpenCode/Forge source
and public Mirador/Euporie/Socktop/Awesome TUIs material; no external source
code was copied. The full decision and rejected alternatives are ADR-W01–W04
in `docs/workspace-experience.md`.

MEASURED failure and repair: an AppCUI 8fps frame experiment emitted more than
the existing 1MiB idle PTY bound in under 30 seconds at 120x32. It was removed
instead of loosening the limit. The independently input-driven debug binary
then completed a 300.197-second actual 120x32 macOS PTY probe: 0 idle output
bytes, sampled max RSS 7,680KiB, delta CPU 0.0%, p95 input 3.576ms, first
paint 18.716ms and quit-to-reap 75.058ms; receipt
`docs/evidence/workspace-final-metrics.json`. This is a host-specific
measurement, not a Ryzen, fleet or web-runtime assertion. Static SVG output
was converted and visually inspected locally after native terminal control was
denied; this is presentation evidence, not orchestration, human-acceptance or
cross-platform proof.

Truth contract: all board cards, activities, diffs and fixture feedback state
`DEMO DATA`; the truth view names the real future owners. The typed Continue
coordinator refuses with `Awaiting Moo` when Moo is absent; it does not create
a Running/Completed fiction. Session-only notes stay visibly local and unsent.
The remaining nine-axis limits remain those stated in the ADR; no load,
multi-tenant, Linux/Windows/Ryzen, persistence, real Moo/model or human study
has been claimed.

The bounded local post-exec reranker independently ranked the terminal-IDE
shell and truthful-colour candidates 1.0 and interaction-driven motion 0.934
(`tools/receipts/20260919-workspace-ui-post-exec.json`). It is advisory design
pressure only: PTY measurements, source owners and the acceptance table remain
the controlling evidence.

## 2026-09-19: Quiet Torque correction and query-tool recovery

Director correction: the first high-fidelity terminal pass used too much colour
and caused visual fatigue. W04's teal work, green focus and purple System
regions are superseded by ADR-W05: black baseline, gray structure, white
content, one blue selected-work surface, and amber/red only for text-labelled
operational warning/error. Porsche v3 gives the monochrome hierarchy and
purposeful-motion premise; Atlassian gives neutral Kanban/elevation roles; Apple
layout supplies adjacent-pane and grouping discipline. The accepted result is
Porsche wrapped around Jira—not a showroom and not a stock Jira board. The
input-only focus marker remains because the prior 8fps redraw experiment failed
the 1MiB idle-output constraint.

MEASURED query-tool failure and repair: the local Qwen query process first
failed because a stray empty `numpy-*.dist-info` directory made
`importlib.metadata.version("numpy")` return `None` despite imported NumPy
reporting 2.4.4. `tools/rerank_local.py` now recovers that package version only
when metadata is unusable, before Transformers imports. The regression suite
adds both fallback and healthy-metadata cases; an actual Qwen rerank then
completed with no caller metadata overlay. The overlay was task-owned and moved
to Trash after that proof; no shared Python package was changed. The reranker
is a bounded advisory selector, never a design authority or orchestration path.

MEASURED W05 UI receipt: the actual debug binary completed a 300.361-second
120x32 PTY run with 0 idle bytes, 0.0% delta CPU, 8,992KiB sampled peak RSS,
1.052ms p95 input, 153.504ms first paint, 73.695ms quit-to-reap and a clean
reaped exit. New Home/Work/Truth SVG previews were visually inspected; they are
faithful cell-render evidence, not terminal screenshots or user-study evidence.
The Qwen post-exec rank was quiet-torque 1.0, Jira-default 0.563 and showroom
0.0134; it is advisory only. The prior colourful SVG and metric files remain
historical and are not used as W05 evidence.

The first refreshed PTY pass also rejected a weak test oracle: "Workspace"
appears in a work title and was therefore not sufficient evidence that Escape
had returned Home. The native journey now waits for the Home-only strapline
"Everything else stays quiet"; no product behavior or acceptance was weakened.

## 2026-09-19: rail affordance and macOS pointer correction

Director feedback was specific: the board was intuitive without research, but
the work rail did not look clickable and its layout felt unnaturally detached.
READ source confirmed the behavioral side: rail rows had no mouse handler.
The correction gives only the selected two-line rail row a compact blue focus
surface, leading marker and `click / Enter to open` affordance; all resting
rows stay neutral. Pressing that target uses the same typed Continue route as
a board card and therefore still refuses honestly with `Awaiting Moo`.

MEASURED repair: AppCUI's macOS default is Termios, whose own capability table
excludes mouse support. The binary now chooses the already-admitted CrossTerm
backend, which enables mouse capture. A real 120x32 POSIX PTY test sends an
SGR click into rail row three and observes both its selected work and the
missing-Moo refusal. The test harness was upgraded from ANSI stripping to a
small cursor-addressed screen model because CrossTerm correctly emits delta
paints; pretending its raw escape stream was contiguous text would be a weak
oracle. Forge influenced only the quiet selected-row grammar—one focus surface
and a leading marker—not copied Ratatui code or a new dependency.

MEASURED W06 receipt: the mouse-enabled macOS debug binary completed a
300.124-second 120x32 PTY probe with 0 idle bytes, 0.0% delta CPU, 7,824KiB
sampled peak RSS, 0.892ms p95 input, 216.412ms first paint, 59.635ms
quit-to-reap and a clean reaped exit. Binary SHA-256:
`c7c2882f7a186521e6b46424bd3417f92000e54470a6d44e3905d1d9d1b47968`.
This is local debug/PTy evidence, not portable performance, live Moo, or a
human study.
