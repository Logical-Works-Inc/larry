---
type: ADR
title: "ADR 0117: Residency and transport are checked before model egress"
description: Why model routes require HTTPS or loopback, tenant residency is checked before configuration, and model work needs network egress.
resource: /docs/adr/0117-residency-transport.md
tags: [larry, adr, model, residency, transport]
generated: {by: codex, at: 2026-11-26}
verified: {by: cargo nextest, method: focused box run, 2026-11-26}
status: accepted
stale_after: 2027-05-26
sources: [larry#203, ADR 0039, OWASP ASVS 4.0.3]
---

# ADR 0117: Residency and transport are checked before model egress

## Context

The model route is the last local owner before a request body and bearer key
reach a gateway. A route could still use plaintext for a remote host. A brief
could also pass the executor gate without `network.egress`. Session prompts
showed an absolute working directory. Command observations could carry tokens
and key-shaped values into the next model request.

## Decision

`Route` refuses `http` unless its authority is loopback. All other routes need
`https`. The refusal is typed and carries only the host.

`Settings` reads a bounded, comma-separated `LARRY_RESIDENCY_ALLOWLIST` of
uppercase ISO country codes. A non-empty list requires the resolved route to
declare one of those codes before its model configuration or key is read.

The executor advertises `NetworkEgress`, includes it in the default policy,
and requires it for every briefed task and terminal session. A policy that
omits it is denied by the existing durable gate before the body runs.

Every terminal prompt says `Working directory: .`. Observation text is scanned
at the session boundary. Bearer/basic values, key assignments and common token
shapes become `[REDACTED]` before the observer or conversation receives them.

## Consequences

Remote plaintext routes and routes outside a tenant's declared countries fail
before the model client reads the key or opens a connection. Local loopback
HTTP remains available for deterministic mocks and sidecars. Operators that
set `LARRY_RESIDENCY_ALLOWLIST` must declare matching residency on every route.

## Proof

`sim_plaintext_non_loopback_is_refused_before_send` proves the route refusal.
`exec_denies_a_brief_task_when_the_policy_lacks_network_egress` proves the
shipped executor gate and zero endpoint reach. The session redaction test
proves secret-shaped values do not enter the conversation.
