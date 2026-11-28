---
type: ADR
title: "ADR 0137: the lag past an expiry is judged against the skew bound"
description: "A past expiry is authorized only while the observer's lag is under the declared clock-skew bound."
resource: /docs/adr/0137-expiry-lag-against-the-skew-bound.md
tags: [larry, adr, contracts, authority, time]
generated: {by: claude-code, at: 2026-11-27}
verified: {by: claude-code, method: lgwks-box cargo nextest run -p larry-contracts -p larry-runtime, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#580, larry#74, larry#212, INVARIANTS.md, crates/larry-contracts/tests/replay_expiry.rs]
---

## Status

Accepted. Corrects the call ADR 0016 and larry#212 left in `replay::decide`.

## Context

`decide` turned a past expiry into zero remaining and passed the observer as
zero. `skew::lease_holds(0, 0, bound)` is true for every non-zero bound, so a
lease a year stale was `Authorized` under 1 ms (CWE-613). The default bound is
zero, so only a host that set one was exposed.

## Decision

An expiry still ahead is passed as what remains. An expiry already behind is
passed as zero, with the lag past it as the observer reading. The estate judge
then holds a lease only while `observed_at < valid_until + skew_bound`.

Rejected: a comparison written in Larry, a second copy of the estate judge.
Rejected: refusing every past expiry, which drops the declared allowance.
Reversed by: an estate judge that takes two `SystemTime` readings.

## Consequences

An approval older than the bound is `AuthorityExpired`. A replay of a record
authorized outside its bound now reports `Diverged`, which is the truth about it.
No format, setting or dependency changes.

## Proof

`sim_an_expiry_past_the_skew_bound_is_denied_for_every_seed` returned
`Authorized` for the year-old case before the change. 644 contracts and runtime
tests pass on toronto after it.
