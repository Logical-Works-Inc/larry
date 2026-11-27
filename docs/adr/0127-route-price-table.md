---
type: ADR
title: "ADR 0127: The route carries its own versioned price table"
description: Why a priced route needs no LARRY_EVAL_PRICE_* in its environment.
resource: /docs/adr/0127-route-price-table.md
tags: [larry, adr, eval, routing]
generated: {by: opencode, at: 2026-11-27}
verified: {by: opencode, method: lgwks-box nextest on the touched suites, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#204, larry#533]
---

# ADR 0127: The route carries its own versioned price table

## Context

`larry eval` priced every arm from `LARRY_EVAL_PRICE_IN` and
`LARRY_EVAL_PRICE_OUT`, so an arm over a stored route priced a model
its environment never named. The route table (larry#201) knew the
gateway, the model and the key variable, but not what a million
tokens cost there. The budget half of #204 (per-tenant limits, the
limiter, the retry wait) stays on #204; this record is the price
table it prices from.

## Decision

The price table rides the route row: `price_version`,
`price_input` and `price_output` beside it (migration 8), revision 1
with no rates on a new row. Only `larry route price` writes those
columns, bumping the revision in the same statement, so two pricings
cannot share a revision; `route add` re-adding a priced route keeps
the table it found. Rates past 10,000 dollars per million tokens are
refused at the grammar, the command and the row, naming the rate.

The arm is priced by its route's table when the route declares one
(its revision is past 1, which only `route price` writes, so even a
route priced at zero and zero counts), else by the environment: a
declared table wins, so a priced route needs no `LARRY_EVAL_PRICE_*`.
The card names the table revision. An unpriced seal is byte-identical
to before, so no sealed report changes under existing routes.

## Rejected

A second table keyed by route id: a second ownership story and a
join on every resolve, for data one row already owns. The table rides
the row it prices.

## Consequences

`larry route price <id> --in <dollars> --out <dollars>` sets the
table; `list --json` and `resolve` print it. A damaged row (a
negative or over-bound rate, an unknown revision) is refused by name,
never read. Reversal trigger: a priced arm whose card disagrees with
the table's arithmetic reopens this ADR.

## Regression

`tests/sim_route_price.rs`
(`sim_route_price_is_versioned_and_typed`,
`sim_concurrent_prices_bump_once_each`,
`sim_every_tabled_route_prices_alone`),
`tests/route_price_journey.rs`
(`a_priced_route_is_versioned_and_listed`) and
`tests/eval_journey/priced.rs`
(`a_priced_route_prices_the_arm_without_eval_price_settings`,
`a_zero_priced_route_scores_at_zero_cost_with_no_eval_price_settings`,
`an_unpriced_route_with_no_eval_price_settings_is_refused_naming_the_variable`)
fail without the table and pass with it. The ledger round trip
(`route_prices_round_trip_bump_and_refuse_damage`) and the field
check (`a_price_past_the_bound_is_refused`) pin the bounds.
