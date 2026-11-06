---
type: ADR
title: "ADR 0003: the model client runs on lgwks_std::http and retries only what the gateway refused"
description: Why Larry's chat-completions client (larry#83) uses lgwks_std's blocking HTTP, which failures it retries, and how it waits.
resource: /docs/adr/0003-model-client-on-lgwks-std-http.md
tags: [larry, adr, model, http, retry]
generated: {by: claude-code, at: 2026-11-06}
verified: {by: claude-code, method: tests/sim_model_client.rs over 64 seeds against tests/support/mock_model.rs}
status: accepted
stale_after: 2027-02-06
sources: [larry#83, larry#71, "RFC 9110 sections 10.2.3 and 15.5.20", lgwks_std 4.0.2 src/http.rs and src/retry.rs]
---

# ADR 0003: model client on lgwks_std::http

## Status

Accepted, 2026-11-06, with larry#83.

## Context

The executor (#85) runs a task on a cheap model through the estate's 9router gateway, which speaks the
OpenAI chat-completions wire. forge-harness `sdk/forge-providers` held a client; the plan (#71) moves it onto
its estate owners. lgwks_std 4.0.2 owns blocking HTTP (`http`, bounded bodies, per-phase timeout and a
whole-call deadline) and a pure retry budget (`retry::RetryPolicy`, no clock, no sleep). The candidates were
that pair, `ureq` or `reqwest` directly, and an async client on `lgwks_bot::rt`.

## Decision

- `src/model/` calls `lgwks_std::http::post_with` with the `http` feature; no second HTTP stack. A direct
  `ureq` or `reqwest` edge would duplicate the owner, and the executor is blocking, so async buys nothing yet.
- Only 429 and 5xx retry, within the `RetryPolicy` attempts and deadline. A timeout or a cut connection may
  already have spent tokens upstream, so it is returned to the caller, which decides whether to pay again.
- `Retry-After` is a floor (RFC 9110): the wait is the larger of the policy backoff and the hint; a hint past
  the remaining deadline returns the 429 instead of retrying early.
- The configured timeout is both the per-phase timeout and the whole-call deadline.
- The caller owns the clock and cancellation: the client asks a pause function it is given, which waits and
  returns whether to retry. Tests record waits and never sleep; `larry model` refuses any wait and reports the
  refusal at once; the executor (#85) waits on its cancel token. No jitter, so one seed replays one wait
  sequence; a fleet draws it from `lgwks_std::random`.
- Bounds: 256 messages, a 1 MiB request and a 4 MiB reply, each refused with `ModelError::BodyTooLarge`.
- Every setting comes from `LARRY_MODEL_*` (URL, key, model, temperature, timeout, attempts 1 to 16,
  backoff, deadline); a bad one is a typed error naming the variable, never a silent default.
- The shipped caller is `larry model`: a prompt on stdin, the reply on stdout. It is the operator's check
  that a key and a gateway work, and the executor (#85) calls the same client.
- From the first live run (larry#128): the default model is `ocg/deepseek-v4.1-flash`, the id 9router
  serves; a reply padded with whitespace and ending `data: [DONE]` on the document's line (OpenRouter) is
  read as the bare document; `Server` and `Client` carry the gateway's own reason, cut at 512 characters
  with control characters blanked so an upstream cannot drive the operator's terminal.

## Consequences

- The `http` feature adds ureq and rustls to the root crate's tree (28 lock entries).
- The executor (#85) owns timeout retries and idempotency; the model client never resends a request whose
  fate it cannot know.
- tests/sim_model_client.rs proves each rule against the seeded mock and tests/model_cli_journey.rs drives
  the shipped binary; the mock now records each request's `Authorization` header so tenant isolation is
  checked on the wire.
