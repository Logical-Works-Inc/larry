//! Seeded simulations of ranking by meaning (larry#202): once the filters
//! leave at least two eligible routes, the shipped local reranker scores
//! their capability descriptions against the task and the best wins. A
//! keyword or regex overlap is never the ranker (SD-2): the scores come from
//! the `Scorer` the shipped `larry-rerank` runs, here a seeded stub, so the
//! same table seals the same winner on every replay. (The pinned-ranker
//! latency control lives in `select_latency.rs`: it reads the wall clock, so
//! it cannot live in a replay-identical sim.)
//!
//! Source: larry#202 (rank after filters, stable order, the rank/scores/pin
//! hash seal the choice). Requirement proved: the choice matches an
//! independent reference over 64 seeded tables; a broken scorer is
//! unavailable, never a silent pick; the task text stays inside the ranker's
//! query bound; equal scores keep table order; hostile text ranks and seals
//! cleanly; a table past the tenant cap still ranks.

use super::sim_select::{Check, Stub, choose, open_policy};
use super::{Budget, Method, Policy, Residency, Selection, query_of};
use crate::model::route::{Draft, Kind, Route, RouteError, Table};
use lgwks_std::seeded::Seeded;

/// Seeds each control runs.
const SEEDS: u64 = 64;

/// The words seeded describes and queries draw from.
const WORDS: [&str; 8] = [
    "drafts", "review", "migrate", "database", "frontend", "cache", "auth", "audit",
];

/// A seeded non-empty text from the word pool.
fn text(rng: &mut Seeded) -> Check<String> {
    let mut out = String::new();
    for _ in 0..=rng.index(3)? {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(WORDS.get(rng.index(WORDS.len())?).ok_or("a word")?);
    }
    Ok(out)
}

/// One generated choice input: the table, its default, the policy, the task
/// query and the stub scores for one seed.
type Fixture = (Table, Option<String>, Policy, String, Vec<f64>);

/// A seeded table of 1 to 6 routes, its default and the tenant rule, budget,
/// query and stub scores for one seed.
fn fixture(seed: u64) -> Check<Fixture> {
    let mut rng = Seeded::from_seed(seed);
    let count = rng.index(6)?.saturating_add(1);
    let mut routes = Vec::new();
    for index in 0..count {
        let id = format!("route-{}", index);
        let describe = if rng.index(4)? == 0 {
            String::new()
        } else {
            text(&mut rng)?
        };
        let residency = match rng.index(4)? {
            0 => None,
            1 => Some(String::from("CA")),
            2 => Some(String::from("US")),
            _ => Some(String::from("DE")),
        };
        let key_env = format!("LARRY_KEY_0123456789ABCDEF_{index}");
        routes.push(Route::stored(&Draft {
            id: &id,
            kind: Kind::OpenAiChat,
            base_url: "https://gateway.test:1/v1",
            model: "model-1",
            key_env: &key_env,
            temperature: None,
            request_bytes: None,
            residency: residency.as_deref(),
            describe: &describe,
        })?);
    }
    let default = if rng.index(2)? == 0 {
        None
    } else {
        let pick = rng.index(count)?;
        Some(format!("route-{pick}"))
    };
    let table = Table::new(routes, default.clone());
    let residency = match rng.index(3)? {
        0 => Residency::parse(&|_: &str| None)?,
        1 => {
            Residency::parse(&|name: &str| (name == "LARRY_RESIDENCY").then(|| String::from("CA")))?
        }
        _ => Residency::parse(&|name: &str| {
            (name == "LARRY_RESIDENCY").then(|| String::from("CA, US"))
        })?,
    };
    let budget = match rng.index(3)? {
        0 => Budget::Remaining {
            tokens: 0,
            micros: 0,
        },
        1 => Budget::Remaining {
            tokens: u64::try_from(rng.index(100)?)?,
            micros: 0,
        },
        _ => Budget::Unbounded,
    };
    let policy = Policy {
        residency,
        budget,
        rank_seconds: 300,
    };
    let query = text(&mut rng)?;
    let mut scores = Vec::new();
    for _ in 0..count {
        scores.push(f64::from(u8::try_from(rng.index(5)?)?) / 4.0);
    }
    Ok((table, default, policy, query, scores))
}

/// The reference: the eligible routes in table order, then the stable rank by
/// score, highest first with ties in table order. The scores align with the
/// candidates the ranker sees, in order, exactly as the stub scores them.
fn reference(table: &Table, policy: &Policy, scores: &[f64]) -> Check<Vec<String>> {
    let eligible: Vec<&str> = table
        .routes()
        .iter()
        .filter(|&route| policy.residency.admits(route) && policy.budget.admits(route))
        .map(crate::model::route::Route::id)
        .collect();
    if eligible.len() > scores.len() {
        return Err("the reference scores every eligible route".into());
    }
    let mut ranked: Vec<(&str, f64)> = eligible.into_iter().zip(scores.iter().copied()).collect();
    ranked.sort_by(|left, right| right.1.total_cmp(&left.1));
    Ok(ranked.into_iter().map(|(id, _)| id.to_owned()).collect())
}

/// Ranks `table` for `query` twice with `scores` and proves both seals are
/// the same bytes: the replay identity every ranking sim asserts.
fn seals_twice(
    table: &Table,
    query: &str,
    policy: &Policy,
    scores: Vec<f64>,
    message: &str,
) -> Check<Selection> {
    let mut stub = Stub::scored(scores.clone());
    let first = choose(table, query, policy, &mut stub)?;
    let mut again = Stub::scored(scores);
    let second = choose(table, query, policy, &mut again)?;
    assert_eq!(
        first.seal().to_string(),
        second.seal().to_string(),
        "{message}"
    );
    Ok(first)
}

/// Proves `choice` is the ranked `winner` the seal names: the id, the ranked
/// method and the sealed ranking opening with `ranking_head`, the verdict
/// every ranking sim asserts.
fn seals_ranked(choice: &Selection, winner: &str, ranking_head: &str, message: &str) {
    assert_eq!(
        choice.route().id(),
        winner,
        "{message}: the best route wins"
    );
    assert_eq!(
        choice.method,
        Method::Ranked,
        "{message}: the record says ranked"
    );
    let sealed = choice.seal().to_string();
    assert!(
        sealed.contains(&format!("\"ranking\":[{ranking_head}")),
        "{message}: the sealed ranking keeps table order: {sealed}"
    );
}

/// Generalized: for 64 seeded tables the choice matches the reference, the
/// same seed seals the same bytes twice, and the seal carries the method, the
/// ranking, every score and the pinned ranker identity.
#[test]
fn sim_ranked_choice_matches_the_reference_for_every_seed() -> Check {
    for seed in 0..SEEDS {
        let (table, _default, policy, query, scores) = fixture(seed)?;
        let eligible = reference(&table, &policy, &scores)?;
        let mut stub = Stub::scored(scores.clone());
        let first = choose(&table, &query, &policy, &mut stub);
        match first {
            Ok(choice) => {
                let winner = eligible
                    .first()
                    .ok_or(format!("seed {seed}: nothing eligible chose {choice:?}"))?;
                assert_eq!(
                    choice.route().id(),
                    *winner,
                    "seed {seed}: the best eligible route wins"
                );
                assert_eq!(
                    choice.method,
                    if eligible.len() == 1 {
                        Method::Sole
                    } else {
                        Method::Ranked
                    },
                    "seed {seed}: one eligible route is sole, more are ranked"
                );
                let sealed = choice.seal().to_string();
                if choice.method == Method::Ranked {
                    assert!(
                        sealed.contains("\"method\":\"ranked\""),
                        "seed {seed}: the record says ranked: {sealed}"
                    );
                    for id in &eligible {
                        assert!(
                            sealed.contains(id),
                            "seed {seed}: every eligible route ranks: {sealed}"
                        );
                    }
                }
                assert!(
                    sealed.contains("\"model\":\"Qwen/Qwen3-Reranker-0.6B\""),
                    "seed {seed}: the record names the ranker: {sealed}"
                );
                assert!(
                    sealed.contains("\"revision\":\"e61197ed45024b0ed8a2d74b80b4d909f1255473\""),
                    "seed {seed}: the record names the pin: {sealed}"
                );
                seals_twice(
                    &table,
                    &query,
                    &policy,
                    scores,
                    &format!("seed {seed}: the same seed seals the same bytes"),
                )?;
                assert_eq!(
                    stub.calls,
                    usize::from(choice.method == Method::Ranked),
                    "seed {seed}: the engine runs once per ranking, never for a sole route"
                );
            }
            Err(got) => {
                assert!(
                    eligible.is_empty(),
                    "seed {seed}: eligible {eligible:?} but refused with {got:?}"
                );
                assert!(
                    matches!(got, RouteError::NoEligibleRoute | RouteError::NoRoute),
                    "seed {seed}: nothing eligible is refused, not {got:?}"
                );
                assert_eq!(stub.calls, 0, "seed {seed}: the engine never ran");
            }
        }
    }
    Ok(())
}

/// One stored route with `describe` for `id`: https and the sim tenant's key
/// variable, so neither the transport gate nor the binding gate fires and the
/// sims measure only what they name.
fn build(id: &str, describe: &str) -> Check<Route> {
    Ok(Route::stored(&Draft {
        id,
        kind: Kind::OpenAiChat,
        base_url: "https://gateway.test:1/v1",
        model: "model-1",
        key_env: "LARRY_KEY_0123456789ABCDEF_RANK",
        temperature: None,
        request_bytes: None,
        residency: Some("CA"),
        describe,
    })?)
}

/// A scorer that breaks its contract leaves the task unranked: fewer scores
/// than candidates, or a score that is not finite, is `NoRanker`, never a
/// silent pick.
#[test]
fn sim_a_scorer_that_breaks_its_contract_is_unavailable() -> Check {
    let table = Table::new(
        vec![build("alpha", "a route")?, build("beta", "a route")?],
        None,
    );
    let policy = open_policy()?;
    for (scores, detail) in [
        (vec![0.5], "the ranker returned 1 scores for 2 candidates"),
        (vec![0.5, f64::NAN], "a ranker score is not finite"),
        (vec![0.5, f64::INFINITY], "a ranker score is not finite"),
    ] {
        let mut stub = Stub {
            force: Some(scores),
            ..Stub::scored(Vec::new())
        };
        let outcome = choose(&table, "a task", &policy, &mut stub);
        assert_eq!(
            outcome,
            Err(RouteError::NoRanker {
                detail: detail.to_owned(),
            }),
            "a broken scorer is unavailable, never a silent pick"
        );
    }
    assert_eq!(
        RouteError::NoRanker {
            detail: String::from("the pinned model is missing"),
        }
        .to_string(),
        "no route ranker is available to choose a route: the pinned model is missing",
        "the refusal carries why, never a silent pick"
    );
    Ok(())
}

/// The task query carries the goal, the files and the acceptance commands
/// inside the ranker's query bound, and a route with no description is scored
/// on its id and model, never skipped.
#[test]
fn sim_the_task_query_is_bounded_and_an_empty_description_falls_back() -> Check {
    let goal = "migrate the database".to_owned();
    let files = vec![
        String::from("src/db.rs"),
        String::from("migrations/001.sql"),
    ];
    let acceptance = vec![
        vec![String::from("cargo"), String::from("test")],
        vec![String::from("true")],
    ];
    let query = query_of(&goal, &files, &acceptance);
    assert!(query.contains("migrate the database"), "the goal: {query}");
    assert!(query.contains("src/db.rs"), "the files: {query}");
    assert!(query.contains("cargo test"), "the acceptance: {query}");
    let long = query_of(&"x".repeat(20 * 1024), &files, &acceptance);
    assert_eq!(long.len(), 16 * 1024, "the query bound: {}", long.len());
    let plain = Route::stored(&Draft {
        id: "plain",
        kind: Kind::OpenAiChat,
        base_url: "https://gateway.test:1/v1",
        model: "model-7",
        key_env: "LARRY_KEY_0123456789ABCDEF_PLAIN",
        temperature: None,
        request_bytes: None,
        residency: None,
        describe: "",
    })?;
    let other = Route::stored(&Draft {
        id: "other",
        kind: Kind::OpenAiChat,
        base_url: "https://gateway.test:2/v1",
        model: "model-8",
        key_env: "LARRY_KEY_0123456789ABCDEF_OTHER",
        temperature: None,
        request_bytes: None,
        residency: None,
        describe: "something else",
    })?;
    let table = Table::new(vec![plain, other], None);
    let policy = open_policy()?;
    let mut stub = Stub::scored(vec![0.2, 0.8]);
    let choice = choose(&table, "a task", &policy, &mut stub)?;
    assert_eq!(choice.route().id(), "other", "scores decide");
    let texts = stub.texts.first().ok_or("a scoring run")?;
    assert_eq!(
        texts.first().map(String::as_str),
        Some("plain model-7"),
        "an empty description falls back to id and model: {texts:?}"
    );
    Ok(())
}

/// The ranking deadline is a knob with a bound: unset means 300 seconds, and
/// anything but a whole number of seconds from 1 to 3600 is refused.
#[test]
fn sim_the_ranking_deadline_is_a_bounded_knob() -> Check {
    let unset = Policy::parse(&|_: &str| None)?;
    assert_eq!(unset.rank_seconds, 300, "unset means 300 seconds");
    let blank = Policy::parse(&|name: &str| {
        (name == "LARRY_ROUTE_RANK_SECONDS").then(|| String::from("  "))
    });
    assert_eq!(
        blank,
        Err(RouteError::BadRankSeconds),
        "a blank deadline is refused"
    );
    for seconds in ["1", "60", "3600"] {
        let parsed = Policy::parse(&|name: &str| {
            (name == "LARRY_ROUTE_RANK_SECONDS").then(|| seconds.to_owned())
        })?;
        assert_eq!(
            parsed.rank_seconds,
            seconds.parse::<u64>().map_err(|cause| cause.to_string())?,
            "{seconds} passes"
        );
    }
    for seconds in ["0", "3601", "hot", "1.5", "-7"] {
        let outcome = Policy::parse(&|name: &str| {
            (name == "LARRY_ROUTE_RANK_SECONDS").then(|| seconds.to_owned())
        });
        assert_eq!(
            outcome,
            Err(RouteError::BadRankSeconds),
            "{seconds} is refused"
        );
    }
    assert_eq!(
        RouteError::BadRankSeconds.to_string(),
        "route rank seconds (LARRY_ROUTE_RANK_SECONDS) must be 1 to 3600",
        "the refusal names the knob, never its value"
    );
    Ok(())
}
/// Past the tenant's 256-route cap the selection still ranks: 257 stored
/// routes all reach the engine in table order and the best score wins.
#[test]
fn sim_a_table_past_the_tenant_cap_still_ranks() -> Check {
    let mut routes = Vec::new();
    for index in 0..257_u32 {
        routes.push(Route::stored(&Draft {
            id: &format!("route-{index:03}"),
            kind: Kind::OpenAiChat,
            base_url: "https://gateway.test:1/v1",
            model: "model-1",
            key_env: &format!("LARRY_KEY_0123456789ABCDEF_{index:03}"),
            temperature: None,
            request_bytes: None,
            residency: Some("CA"),
            describe: "a route past the cap",
        })?);
    }
    let table = Table::new(routes, Some(String::from("route-000")));
    let policy = open_policy()?;
    let mut scores = vec![0.0; 257];
    let last = scores.last_mut().ok_or("257 scores")?;
    *last = 1.0;
    let mut stub = Stub::scored(scores);
    let choice = choose(&table, "a task past the cap", &policy, &mut stub)?;
    seals_ranked(
        &choice,
        "route-256",
        "\"route-256\"",
        "the best score wins past the cap",
    );
    assert_eq!(stub.calls, 1, "one scoring run over the whole table");
    let seen = stub.texts.first().ok_or("a scoring run")?;
    assert_eq!(seen.len(), 257, "every candidate reaches the engine");
    Ok(())
}

/// A tied ranking keeps table order: equal scores never reorder the routes,
/// so the same table seals the same winner on every replay.
#[test]
fn sim_a_tied_ranking_keeps_table_order() -> Check {
    let table = Table::new(
        vec![
            build("alpha", "a tied route")?,
            build("beta", "a tied route")?,
        ],
        None,
    );
    let policy = open_policy()?;
    let first = seals_twice(
        &table,
        "a tied task",
        &policy,
        vec![0.4, 0.4],
        "a tie seals the same bytes twice",
    )?;
    seals_ranked(
        &first,
        "alpha",
        "\"alpha\",\"beta\"",
        "a tie keeps table order, so the first route wins",
    );
    Ok(())
}

/// Hostile description bytes that pass the field check still rank and seal:
/// newlines, quotes, backslashes, non-Latin text and a full-KiB description
/// choose by score and seal the same bytes on replay.
#[test]
fn sim_hostile_descriptions_rank_and_seal_cleanly() -> Check {
    let table = Table::new(
        vec![
            build("newline", "line one\nline two \"quoted\" \\ backslash")?,
            build("unicode", "café naïve ☃ smiley")?,
            build("full", &"x".repeat(1024))?,
        ],
        None,
    );
    let policy = open_policy()?;
    let first = seals_twice(
        &table,
        "a hostile task",
        &policy,
        vec![0.1, 0.9, 0.2],
        "hostile text seals the same bytes twice",
    )?;
    seals_ranked(
        &first,
        "unicode",
        "\"unicode\",\"full\",\"newline\"",
        "scores decide over hostile text",
    );
    Ok(())
}
