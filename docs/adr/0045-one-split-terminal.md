---
type: ADR
title: "ADR 0045: one split terminal frontend that asks before every command"
description: Why bare `larry` is a phone-ratio chat column plus an artifact pane, drawn by Larry's own cell frame on crossterm, and why each model command waits for a person.
resource: /docs/adr/0045-one-split-terminal.md
tags: [larry, adr, tui, frontend, approval, crossterm, appcui]
generated: {by: claude-code, at: 2026-11-16}
verified: {by: claude-code, method: cargo build --bins --locked and a 120x34 script(1) capture on Oracle, 2026-11-16}
status: accepted
stale_after: 2027-02-16
sources: [larry#238, larry#239, logicalworks-crates#1067, docs/adr/0044-session-auto-compaction.md]
---

# 0045: one split terminal frontend

Status: accepted, 2026-11-16 (larry#239, supersedes the AppCUI view of larry#238)

## Context

The Director, 2026-11-16: "all changes will be routed through a completed human frontend (in tui,
then Mac app then iOS and android apps)". Also: "the tui needs to simple and Claude code/codex esq",
and "the chat space to be condensed and similar aspect ratio to phone ... with the rest of the computer
real estate being used for artifact like visualizations".

The AppCUI view streamed badly. Its crossterm backend sends SGR 0 after the colour, so bold cells lose
their colour. It brought 82 crates. Bare `larry` showed a fixture, not a session.

Codex draws with its own fork of the ratatui terminal. OpenCode draws with OpenTUI, a Zig cell diff
with DEC 2026. OpenCode keeps the chat 44 to 64 columns wide and splits at 108 columns.

## Decision

- Bare `larry` runs the real `run_session` in a worker thread. The UI talks to it over two bounded
  channels (64 events, 1 answer).
- Larry owns the frame: a cell buffer, a diff against the last frame, a full reset-first SGR per run,
  and DEC 2026 around each frame. crossterm only gives raw mode, keys, paste and resize.
- The chat column is 44 to 64 columns. At 108 columns or more the rest is the artifact pane. Below,
  the chat takes the whole width.
- `Observer::approve` asks before every model command: yes, always or no. A declined command never
  runs. The model is told, and the sealed report counts it as `session.declined`. `larry acp` has no
  person present and approves under the same `model.command` gate.
- The app state takes keys and events and returns effects. It reads no terminal and no clock, so the
  Mac, iOS and Android shells reuse it.
- The UI lives in Larry until Drishti owns a terminal runtime. Drishti is docs and GPUI today.

## Consequences

- AppCUI and its subtree leave the lock (713 lines). crossterm was in the tree already. unicode-width
  has no dependencies. Both cite logicalworks-crates#1067.
- Esc stops the session at its next step. A running command ends at its timeout, because
  `run_command` has no cancel hook yet.
- Model and command output are sanitized: no escape, control or bidi character reaches the terminal.

## Rejected

- AppCUI: the SGR bug, 82 crates, and a widget model that fights a streaming chat.
- cursive: a dialog toolkit that owns the event loop; a session must post callbacks into it.
- iocraft: declarative and credible, but a layout engine and component tree for two rectangles and a
  composer. It is the swap to measure if the component code outgrows one file per part.
- ratatui: Codex had to fork its terminal to control inserts and diffs; the frame is the part we must own.
