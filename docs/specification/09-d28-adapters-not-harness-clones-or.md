---
type: Design
title: specification.md, part 9 of 18
description: Part 9 of /specification.md, in order; the index lists every part.
resource: /docs/specification/09-d28-adapters-not-harness-clones-or.md
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
### D28 — adapters, not harness clones or ambient extensions

The Director selected explicit adapters. Support admitted standard tool protocols
and inspected imports of skills/config/history; do not promise arbitrary Codex or
OpenCode plugins/hooks run unchanged. Each tool advertisement binds executable
identity, schema, version, tenant and environment. Validate inputs, authorize the
resolved target, execute, then validate outputs. A replacement invalidates stale
registrations; it cannot quietly change an already approved call's target.

Untrusted extensions run in Nova, without ambient host credentials. Define load,
ready, failed, draining and disposed states, bounded initialization/teardown,
ordering, cycle rejection and removal of partial registrations. Revalidate any
permitted argument transformation before effects. Failed replacement retains the
old admitted implementation or explicitly disables the tool; never fake readiness.
No user-supplied shell spelling bypasses admission, including direct/manual tools.
Remote paths are environment-owned identities, not host-native strings. Resolve
case, symlink, mount and path rules through the owning Nova/Moo environment.
E32/E48; owner tool integration. No host hooks are installed by this specification.

### D29 — model and artifact boundaries preserve uncertainty

Provider adapters lower a canonical transcript without breaking tool-call/result
pairing or reusing incompatible opaque reasoning metadata. Provider-executed
tools are not re-executed locally. A finish reason is interpreted with outstanding
calls and observed effects, not treated as proof of task completion. Interrupted
tool exchange and absent external effect are different states.

Credential refresh, expired credentials, unavailable models and rate limits have
bounded retry/deadline policies and explicit status. Retry-After fits inside the
run deadline and budget; retry never changes provider, account, region, model or
data recipient without standing authorization. No secrets in prompts, traces or
support bundles. Redacted diagnostics retain useful typed errors.

Record producer capture completeness, canonical-result validity, preview
truncation and artifact retention separately. A stored file cannot be called full
output if producer capture already lost bytes. Stream bounded artifacts through
scoped opaque references with size, digest, type and expiry. Apply limits to
structured data and media too; artifact readback is authorized each time. Missing
or expired artifacts are explicit, not reconstructed guesses. E33/E34.

### D30 — identity continuity does not imply shared channels

External identity is scoped by tenant, connector account and provider-defined
identifier semantics. Account linking is explicit and reversible; display-name
similarity never merges identities, memory or authority. Unlinking stops future
sharing and exposes already-shared data limits honestly.

Persist initiating channel/account/thread/recipient and visible-delivery owner.
Tool sends and automatic final delivery reconcile against the same delivery
obligation. A result for thread A cannot follow the most recently active thread B.
Worker-private output never becomes a public fallback. Receiver-side ambiguity
uses the same OutcomeUnknown protocol as other external effects. E37/E38.

### D31 — unattended work is admitted, observable and bounded

Director-approved capability: scheduled and event-triggered work inside standing
authority. Every schedule records owner, trigger/timezone, occurrence key, expiry,
budget, concurrency, target and notification policy. Default missed-run policy is
coalesce to one eligible catch-up occurrence; no overlapping occurrences by
default. Calendar schedules select the first repeated wall-clock occurrence and
skip nonexistent DST times, with both choices visible and customizable. Interval
schedules use explicit anchors; time uncertainty suspends dispatch.

Catch-up shares ordinary admission limits; no outage backlog can multiply a
parent budget. Event triggers deduplicate stable source IDs and bound bursts;
events without stable IDs require a declared weaker duplicate contract. No idle
inference merely to appear alive. Recheck authority for every occurrence; stop
and expiry cancel undispatched work, not pretend committed effects vanished.
E39; owner scheduler, execution through existing compute/effect owners.

### D32 — installation, diagnosis and accessibility are product contracts

Setup is resumable and idempotent, preserving explicit choices. Distinguish
configured, dependencies available, authority established and first useful
operation verified. Non-interactive setup returns actionable missing-input
results. No automatic model download or paid fallback. Installation, updates and
removal account for services, processes and artifacts; removal is reversible.

Diagnostics include phase, owner/lease, last observation timestamp/source,
dependency state and safe recovery actions. Unreachable is not freshly running;
a watchdog cannot resend an unknown external effect. Diagnostic exports are
redacted and opt-in. Access supports keyboard/screen reader, reduced motion,
non-color status, UTF-8 and interrupted streams. Render external terminal text
as data: escape control/OSC sequences, make links explicit, and never permit
output to silently write clipboard contents or spoof trusted approval prompts.
E42/E44/E51; owner lifecycle and presentation.
