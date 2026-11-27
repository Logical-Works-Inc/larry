---
type: ADR
title: "ADR 0130: Semantic route selection on the shipped local reranker"
description: Why an unnamed task is ranked by meaning on route descriptions with the pinned Qwen3 reranker after residency and budget filters, and what the sealed record carries.
resource: /docs/adr/0130-semantic-route-selection.md
tags: [larry, adr, model, routing]
generated: {by: codex, at: 2026-11-27}
verified: {by: codex, method: lgwks-box cargo nextest and clippy on toronto, 2026-11-27}
status: accepted
stale_after: 2027-05-27
sources: [larry#202]
---

# ADR 0130

## Context

After the route table landed, a task still named its route by hand: no code
chose a model from what the task is. The Director calls routing a semantic
feature of the end user. A keyword or regex overlap would be a second system
standing in for meaning (SD-2).

## Decision

- `model::select` is pure: an explicit route resolves exactly as before and
  always wins. Without one, a route the configured resolver would refuse
  (plaintext to a non-loopback host, another tenant's key variable) never
  ranks, then the tenant's stored routes pass the residency rule
  (`LARRY_RESIDENCY`, unset constrains nothing) and the budget (always
  affordable until #204 records remainders), then the survivors rank by
  meaning on their capability descriptions. A sole survivor wins directly;
  without the `rerank` feature the eligible default wins.
- The ranker is the shipped Qwen3 reranker through the `rerank::Scorer`
  trait: the query is the goal, files and acceptance commands cut to the
  ranker's query bound, each candidate its description (or its id and model
  when undescribed). Ties keep table order, so the same table seals the same
  bytes on every replay.
- The sealed record carries `route_selection` beside `route`: the method
  (`named`, `ranked`, `sole`, `default`, `environment`), and when ranked the
  full ranking, every score and the pinned model id and revision.
- Any ranker failure (no engine, missing model, missing text, deadline,
  broken scores) is `NoRanker`: the task fails before any byte leaves,
  never silently routed.

## Consequences

- Feature builds rank; default builds keep the default. `larry route resolve`
  keeps its configured order: ranking needs the task's query.
- #203 moves the residency rule into per-tenant data and enforces it on the
  explicit path too; #204 fills the budget remainders. Neither reorders this.

## Proof

- `sim_ranked_choice_matches_the_reference_for_every_seed` (64 seeds),
  `sim_a_named_route_wins_past_the_residency_rule`,
  `sim_a_best_ranked_route_outside_the_rule_loses_to_the_next_eligible`,
  `sim_an_exhausted_budget_filters_before_the_engine_runs`,
  `sim_without_a_ranker_the_eligible_default_wins_and_the_record_says_so`,
  `sim_an_unnamed_task_with_no_text_is_refused_before_the_ranker_runs`,
  `sim_a_malformed_residency_rule_is_refused_by_name`,
  `sim_a_scorer_that_breaks_its_contract_is_unavailable`,
  `sim_the_task_query_is_bounded_and_an_empty_description_falls_back`,
  `sim_the_ranking_deadline_is_a_bounded_knob`,
  `sim_without_a_ranker_and_no_default_the_refusal_stays_no_route`,
  `sim_an_empty_table_refuses_before_any_ranking`,
  `sim_a_table_past_the_tenant_cap_still_ranks`,
  `sim_a_tied_ranking_keeps_table_order`,
  `sim_hostile_descriptions_rank_and_seal_cleanly`,
  `sim_a_plaintext_route_never_ranks` (larry#536: a stored route the resolver
  would refuse never ranks), the four `selection_*` journeys in
  `route_cli_journey.rs`.
- `ranked_choice_latency_with_the_pinned_ranker` (in `select_latency.rs`, a
  measurement control outside the `sim_*` modules, feature `rerank` only,
  toronto box): cold 68236.3 ms (weights load over 2 routes, winner `beta`,
  the database-migration route for a database-migration task, stable across
  every run), p50 55492.7 ms, p95 58698.0 ms, p99 58698.0 ms, peak RSS
  3588156 KiB. Wall times feed only the printed card, never an assertion. No
  Microsoft counterpart exposes local-reranker route-choice latency to beat
  on a matched workload; the proof is absolute latency plus the
  replay-identity sims above.
