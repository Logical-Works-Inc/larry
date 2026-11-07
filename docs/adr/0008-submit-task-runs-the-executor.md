---
type: ADR
title: "ADR 0008: larry_submit_task runs the executor, and the model writes whole files"
description: How a typed MCP task becomes a sealed run, what the model may touch, and what counts as success.
resource: /docs/adr/0008-submit-task-runs-the-executor.md
tags: [larry, adr, mcp, executor, model, receipts]
generated: {by: claude-code, at: 2026-11-07}
verified: {by: claude-code, method: cargo nextest run --workspace --locked; tests/mcp_stdio_journey.rs against the mock model}
status: accepted
stale_after: 2027-02-07
sources: [larry#123, larry#83, larry#85, larry#86, ADR 0004, ADR 0006, forge#158]
---

# 0008: larry_submit_task runs the executor

Status: accepted, 2026-11-07 (larry#123)

## Context

`larry mcp` advertised `larry_submit_task` (goal, repo, files, acceptance) but refused every task with
`executor_not_connected`. The model client (#83), the workspace executor (#85) and sealed receipts (#86)
now exist; the tool must run the task through all three and return the diff, the output and a receipt
Claude can check offline, never reporting success for work that did not pass.

## Decision

- One task format: the tool builds a `larry exec` task document (`repo`, `base: HEAD`, `goal`, `files`,
  each acceptance command as `sh -c <command>`) and runs it through `executor::execute`. `larry exec`
  gains the same optional brief, so both doors run one code path and one digest names the run.
- The model writes whole files: one completion per named file, carrying the goal, the path and the current
  content; the reply (one surrounding code fence removed, a final newline ensured) replaces the file in
  the clone before any acceptance command runs. Rejected: a unified-diff protocol (cheap models emit
  malformed hunks, and a bad hunk is an unrecoverable apply), and letting the model run commands (denied
  until Nova, logicalworks-crates#846).
- A brief is 1 to 64 distinct repo-relative paths, never `.`, `..` or `.git`; an existing symlink on any
  path component refuses the task (CWE-59), so a reply never lands outside the workspace.
- The executor never blocks on a clock: a model retry runs only when its backoff is zero
  (`LARRY_MODEL_BACKOFF_MS=0`); otherwise the refusal and any `Retry-After` reach the planner at once.
  `std::thread::sleep` is disallowed in this workspace.
- Success is the sealed verdict: an accepted run is a result with the report and receipt; a rejected run
  is `isError` with `task_rejected` and its report; any refusal is `task_refused` with nothing sealed; a
  process without `LARRY_STATE_DIR` or `LARRY_MODEL_KEY` is `executor_not_connected`. The text block
  carries the same JSON as `structuredContent`, so a client without structured output can still verify.

## Consequences

- `larry mcp` reads `LARRY_STATE_DIR`, `LARRY_EXEC_TIMEOUT_MS` and `LARRY_MODEL_*` once at start.
- A tool call holds the stdio loop until its run ends (clone, completions, commands); the MCP Tasks
  extension for long calls is not used yet, so a client's tool timeout bounds the run.
- The tool's bounds now equal the executor's: 64 files and 16 commands (were 256 and 32 advertised).
- Model cost per task is one completion per file; #87 measures cost per accepted task on real models.
