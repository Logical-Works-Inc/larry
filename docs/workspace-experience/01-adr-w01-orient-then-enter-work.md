---
type: ADR
title: docs/workspace-experience.md, part 1 of 3
description: Part 1 of /docs/workspace-experience.md, in order; the index lists every part.
resource: /docs/workspace-experience/01-adr-w01-orient-then-enter-work.md
tags: [architecture, experience, workspace, evidence]
generated: {by: Codex, at: 2026-09-19}
verified: {by: Codex, method: source inspection and final local PTY receipt}
status: implementation-verified-locally-owner-integration-pending
stale_after: 2026-10-19
sources: [/specification.md, /research-ledger.md, /log.md, Director instructions 2026-09-19]
---
## ADR-W01 — Orient, then enter work

Decision: stable Kanban lanes, compact next-action cards and recent activity
precede chat. Enter/click work restores its own draft, selection and inspection
state and invokes the SAME typed Continue coordinator as the explicit command.
Escape returns Home without discarding the draft. No automatic execution claim
on navigation. Completed work opens for inspection and refuses a continuation
through the same terminal-state guard.

Rejected: Cline's hero/prompt landing because it requires remembering what to
ask; a single full-screen chat because context vanishes into chronology; a
full Jira clone because maintenance controls bury the next action. Accepted
tradeoff: fewer simultaneous controls; deeper inspection is one step away.

## ADR-W02 — Stable attention, contextual system cards

Decision: chat is a dedicated central area, with a separate system/context
panel. The highest-priority system fact changes with actual local state
(dependency missing, feedback held, change selected), in a FIXED location.
No auto-rotating carousel, timer, notification count inflation or invented
cost/latency. Narrow terminals expose the same context through panel focus.
Text labels carry state independently of color; ordinary keys work without
function keys. The raccoon identity remains available without becoming the
landing page's main content. Human ADHD benefit is an ESTIMATE, not a clinical
or measured usability claim. Human aesthetic acceptance remains unknown.

## ADR-W03 — Truth is part of the interface

Decision: persistent DEMO DATA labeling plus a discoverable truth view lists
which information is synthetic and what supplies it later. Cards, recent
activity, source diffs and fixture feedback are synthetic. Continue dispatch
is real typed validation, with MissingMooAdapter returning AwaitingDependency;
it never creates a fake Running/Completed state. Chat without a model records
a local unsent note, not a pretend AI response. Drafts are session-only, lost
on process exit, and visibly described that way.

Truth path: replace the fixture projection with authenticated Moo workspace
summary/event/check snapshots; revalidate tenant, workspace, baseline, fence
and revision at Moo's atomic claim/admission boundary; retain operation and
command identities durably in their real owner before enabling effects.
Replace unsent notes with a bounded model transport only when admission,
cancellation and provenance work. Logical-DB owns future durable memory.
No generic filesystem/process tool is introduced by this surface change.

## ADR-W04 — A terminal workspace needs depth, not decorative telemetry (superseded visually by W05)

Decision: Rocco uses a dark spatial shell, a persistent work rail, large
coloured work cards, an independent purple System attention stack, and a
separate Human + AI work pane. Green means selected focus, teal means work
surface, amber marks synthetic/disconnected data, and every one of those
states also has explicit text. Card/panel colour is presentation, not a source
of orchestration truth. The AppCUI admission is ANSI16, so the palette names
its actual terminal colours instead of pretending to deliver 24-bit RGB.

Opening, selecting, and returning home change an input-driven focus marker;
there is no idle animation. A first implementation used AppCUI's 8fps frame
loop for six focus frames. It failed the pre-existing 1MiB idle-output bound in
under 30 seconds on the actual 120x32 PTY, because the framework redraws the
whole surface every tick. That implementation was removed rather than raising
the bound. The retained feedback is local response to human input; it neither
cycles status nor depicts a model, Moo, or any process as working.

Rejected: the prior sparse transparent board (it read as an annotated drawing,
not a place to work); a full monitoring dashboard (would manufacture telemetry
Rocco does not have); the measured frame-loop animation and ambient
shimmer/rotating cards (would turn a glanceable workspace into an attention
demand). The accepted tradeoff is ANSI16 depth and static density over web-like
gradients and continuous animation.

## ADR-W05 — Quiet Torque: Porsche hierarchy wrapped around Jira work

Decision: correct W04's initially over-coloured terminal translation. Rocco now
uses a black baseline, gray structural lines, white reading text, and exactly
one blue selected-work surface. Amber and red retain only warning/error meaning
and remain paired with words. System and unselected work are outlined neutral
surfaces, not coloured regions. The board remains Jira-like where it earns its
place: lanes name current state, compact cards expose a next action, the rail
keeps project context, and the fixed System stack supplies a nearby operational
fact. It is not a Porsche showroom and it is not stock Jira.

Porsche v3's monochrome semantic palette, typography hierarchy and deliberate
motion establish the visual grammar; Atlassian supplies the Kanban/elevation
discipline: a neutral backdrop, bordered resting cards, one raised/focused
object, and compact textual status recognition. Apple layout guidance supplies
the spatial rule: adjacent panes have clear roles and unrelated controls do not
compete for the same glance. In an ANSI16 terminal, surface separation comes
from borders, whitespace, density and type—not gradients, shadow theatre or
colour volume. Motion stays input-triggered because the measured 8fps redraw
trial exceeded the existing 1MiB idle-output bound; `▌` records focus changed
by the human and never implies work is progressing.

Rejected: W04's teal/magenta/green region fills because they made every panel
ask for attention; a literal Porsche recreation because automotive luxury
would erase operational density; unmodified Jira because enterprise defaults
make the next decision visually ordinary. Accepted tradeoff: less ornamental
colour and less ambient motion in exchange for a faster visual hierarchy,
honest state and a quieter return-to-work surface.

Sources: Porsche [palette migration](https://designsystem.porsche.com/v3/news/migration-guide/utilities/),
[motion](https://designsystem.porsche.com/v3/styles/motion/) and
[typography](https://designsystem.porsche.com/v3/styles/typography/); Atlassian
[foundations](https://atlassian.design/foundations),
[elevation](https://atlassian.design/foundations/elevation) and
[motion](https://atlassian.design/foundations/motion/); Apple
[layout guidance](https://developer.apple.com/design/human-interface-guidelines/layout).
