---
type: ADR
title: "ADR 0020: commands enter the runtime through protocol negotiation"
description: Why every executor command passes Runtime::submit_protocol (E47), which negotiates version and capabilities and checks tenant and queue before the displayed command is queued.
resource: /docs/adr/0020-protocol-negotiation-before-ingress.md
tags: [larry, adr, runtime, protocol, ingress, e47]
generated: {by: claude-code, at: 2026-11-08}
verified: {by: claude-code, method: full nextest run on the port}
status: accepted
stale_after: 2027-02-08
sources: [larry#101, PR #20, larry#68, MCP lifecycle 2025-03-26, RFC 7301 ALPN]
---

# 0020: commands enter the runtime through protocol negotiation

Status: accepted, 2026-11-08 (larry#101, E47)

## Context

The runtime had no version negotiation at its command ingress (PR #20): an incompatible client, a client
without the needed capability and a headless human question had no typed refusal before the queue.

## Decision

`Runtime::submit_protocol(offer, command, display)` negotiates the highest overlapping version and the
required capabilities, checks the offer's tenant against the command's and refuses a full queue with a
4-tick delay, all before anything is queued or journaled. A displayed command then rides
`submit_displayed`, so its identity check has one home. The executor offers its own range, `SubmitCommand`
only, headless, and seals a protocol refusal as a rejected report like a policy refusal.

Rejected: exact version equality (cannot pick an overlapping range or report bounds); wall-clock retry jitter
(backoff is part of the deterministic runtime boundary).

## Consequences

`negotiate`, `NegotiatedProtocol` and the backoff cap are crate-private; the HTTP API (#90) is the next caller.
