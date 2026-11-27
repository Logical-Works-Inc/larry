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

`larry route residency <CC,...>` stores at most 64 sorted, uppercase ISO
country codes for the tenant in `executor.db` beside its routes. `--clear`
removes the row. A non-empty policy requires the resolved route to declare one
of those codes before its model configuration or key is read. `route list`
shows the policy.

The executor advertises `NetworkEgress`, includes it in the default policy,
and requires it for every briefed task: a policy that omits it is denied by
the existing durable gate before the body runs. Terminal sessions run under
the session scopes without it, and each of their model exchanges rides the
admitted session the person drives.

Every terminal prompt says `Working directory: .`. The one `process_env`
snapshot gives the executor every exact value of `LARRY_MODEL_KEY`,
`LARRY_RECEIPT_KEY` and `LARRY_KEY_*` that it read, when at least 8 bytes.
Those values and the absolute workspace path are replaced in each observation
before marker rules. The combined observation is bounded at 8 KiB. Bearer/basic
values, key assignments and common token shapes become `[REDACTED]` too.

## Consequences

Remote plaintext routes and routes outside a tenant's declared countries fail
before the model client reads the key or opens a connection. Local loopback
HTTP remains available for deterministic mocks and sidecars. Operators set the
policy through `larry route residency` and must declare matching residency on
every route.

## Proof

`sim_plaintext_non_loopback_is_refused_before_send` proves the route
refusal, and `sim_plaintext_loopback_is_allowed` its loopback exception.
`exec_denies_a_brief_task_when_the_policy_lacks_network_egress` proves the
shipped executor gate denies before the endpoint is reached.
`route_residency_policy_is_stored_and_filters_model_egress` proves the stored
policy filters resolve, and
`a_legacy_plaintext_route_lists_and_removes_but_resolution_refuses` proves a
pre-rule row lists, removes, and never resolves.
`a_terminal_command_redacts_route_key_and_workspace_path_in_model_requests`
proves exact secrets never enter the conversation.
