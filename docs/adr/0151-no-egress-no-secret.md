---
type: ADR
title: "ADR 0151: no byte leaves without net.egress, and none that leaves holds a key"
description: "A brief, a session and larry model need net.egress; a session's observations are masked and its host directories renamed before they are cut and sent; a brief holding a key is refused."
resource: /docs/adr/0151-no-egress-no-secret.md
tags: [larry, adr, model, egress, redaction]
generated: {by: claude-code, at: 2026-11-28}
verified: {by: claude-code, method: lgwks-box cargo nextest run, 2026-11-28}
status: accepted
stale_after: 2027-05-28
sources: [larry#589, larry#203, larry#568, ADR 0019, ADR 0046, INVARIANTS.md]
---

## Status

Accepted. Supersedes the egress and redaction parts of PR #568.

## Context

`required_scopes` never asked for network egress, so a policy that granted none
still sent repository files to the gateway. A session told the model its absolute
directory and sent command output and files as they were (CWE-200).

## Decision

**One gate.** A brief and every session need `net.egress`, and so does
`larry model`, through `executor::admit_egress`, before the key is read. The
default policy holds it.

**One redactor.** `executor::redact` is the only code that decides what a model
does not see. `Settings::redactor` gives it `LARRY_MODEL_KEY` and the key
variable of each route this tenant stored. It masks those keys, the documented
token shapes (`sk-`, `ghp_`, `github_pat_`, `xoxb-`, `AKIA`, `AIza`, `eyJ` and
their siblings) and PEM private key blocks, byte for byte. It names the folder
and its confined copy `.`, the scratch `$TMPDIR`, the state directory `<state>`
and the home directory `~`.

**Before the cut.** A stream or file is redacted whole and then cut to 4 KiB, so
a cut never leaves the head of a key. The report and the person's approval keep
the real bytes.

**A brief is refused, not masked.** Its files are rewritten from the reply, so a
masked key would be written back masked. A key or token in the goal or in a named
file is `ExecError::BriefSecret`, naming the part, with nothing sent.

## What this does not do

- Masking stops an accidental disclosure. A model that asks for a key encoded or
  split (`base64`, `cut`) receives it in that form.
- A configured key shorter than 8 bytes is not masked: it is a local gateway's
  placeholder, and masking it would blank ordinary words.
- A brief's files keep the host paths written inside them.
- An edit whose `old` text spans a masked or renamed part does not match, and is
  refused as any wrong `old` is.
- `larry model` sends the operator's own words as they are.

## Rejected

- Redact each message as it enters the conversation, after the cut (PR #568). A
  key cut at the bound leaks its head, and read steps and the copy's path were
  missed.
- Heuristics on words such as `key:` or `token=`. They mangle source code the
  model must edit by exact text.
- Take every `LARRY_KEY_*` variable in the environment. It reads other tenants'
  keys.

## Cost

One redaction of 1 MiB takes p50 29 ms, p99 42 ms (dev profile, Oracle). A
session observation is at most 1 MiB for each stream.

## Consequences

A `LARRY_POLICY` or a tenant policy that lists scopes must now list `net.egress`,
or its briefs, sessions and `larry model` are refused by name.

## Reverse when

A kernel-level egress control (Secure-Authority) grants or refuses each
destination, so the scope moves there. Or a measured session spends more time in
redaction than in waiting for its model.
