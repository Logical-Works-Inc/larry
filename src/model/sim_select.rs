//! Seeded simulations of selection filters (larry#202): an explicit route wins
//! exactly as the configured resolver answers it, a route the resolver would
//! refuse (plaintext, another tenant's key) never ranks, and the residency
//! rule and the budget filter before any ranking. Ranking itself lives in
//! `sim_select_rank.rs`.
//!
//! Source: larry#202 (named wins, transport and binding gates, filters before
//! ranking, the tenant default without the `rerank` feature).
//! Requirement proved: a named route wins past the rule, the best-ranked
//! route the rule refuses loses to the next eligible one, an exhausted budget
//! filters before the engine runs, the eligible default wins with no ranker,
//! and an empty table or a refused host never reaches the engine.

use super::{Budget, Method, Policy, Residency, Selection, select};
use crate::model::route::{Draft, ENV_ROUTE, Kind, Route, RouteError, Table};
use crate::rerank::{Candidate, Deadline, RerankError, Scorer};

/// Failures propagate instead of panicking.
pub(super) type Check<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The tenant text the sim routes bind their key variables under.
pub(super) const TENANT: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A seeded engine: fixed scores per call, counting calls and rows, refusing on demand.
pub(super) struct Stub {
    /// The scores it returns, in candidate order.
    pub(super) scores: Vec<f64>,
    /// How many times it ran.
    pub(super) calls: usize,
    /// The queries it saw.
    pub(super) queries: Vec<String>,
    /// The candidate texts it saw per call.
    pub(super) texts: Vec<Vec<String>>,
    /// When set, the scores it returns instead of `scores`.
    pub(super) force: Option<Vec<f64>>,
}

impl Stub {
    /// A stub returning `scores`.
    pub(super) fn scored(scores: Vec<f64>) -> Self {
        Self {
            scores,
            calls: 0,
            queries: Vec::new(),
            texts: Vec::new(),
            force: None,
        }
    }
}

impl Scorer for Stub {
    fn score(
        &mut self,
        query: &str,
        rows: &[Candidate],
        _batch: usize,
        _deadline: Deadline,
    ) -> Result<Vec<f64>, RerankError> {
        self.calls = self.calls.saturating_add(1);
        self.queries.push(query.to_owned());
        self.texts
            .push(rows.iter().map(|row| row.text().to_owned()).collect());
        let scores = match self.force.clone() {
            Some(forced) => forced,
            None => self.scores.clone(),
        };
        Ok(scores.into_iter().take(rows.len()).collect())
    }
}

/// The stub as the ranker the choice scores with.
pub(super) fn engine(stub: &mut Stub) -> &mut dyn Scorer {
    stub
}

/// Ranks `table` for `query` under `policy` with `stub`: the unnamed-task path
/// every ranking sim drives, so the call shape lives once.
pub(super) fn choose(
    table: &Table,
    query: &str,
    policy: &Policy,
    stub: &mut Stub,
) -> Result<Selection, RouteError> {
    select(
        TENANT,
        table,
        None,
        None,
        Some(query),
        policy,
        Some(engine(stub)),
    )
}

/// The unconstrained policy most sims share: no residency rule, no budget
/// recorded, the default ranking deadline.
pub(super) fn open_policy() -> Check<Policy> {
    Ok(Policy {
        residency: Residency::parse(&|_: &str| None)?,
        budget: Budget::Unbounded,
        rank_seconds: 300,
    })
}

/// A stored route under the sim tenant with `residency` and `describe`: https
/// so the transport gate never fires, one shared key variable since the seal
/// never names it.
pub(super) fn stored(id: &str, residency: Option<&str>, describe: &str) -> Check<Route> {
    Ok(Route::stored(&Draft {
        id,
        kind: Kind::OpenAiChat,
        base_url: "https://gateway.test:1/v1",
        model: "model-1",
        key_env: "LARRY_KEY_0123456789ABCDEF_TEST",
        temperature: None,
        request_bytes: None,
        residency,
        describe,
    })?)
}

/// An explicit route always wins: a named route outside the tenant rule still
/// resolves, and the engine never runs for it.
#[test]
fn sim_a_named_route_wins_past_the_residency_rule() -> Check {
    let de = stored("de", Some("DE"), "german route")?;
    let ca = stored("ca", Some("CA"), "canadian route")?;
    let table = Table::new(vec![de, ca], Some(String::from("ca")));
    let policy = Policy {
        residency: Residency::parse(&|name: &str| {
            (name == "LARRY_RESIDENCY").then(|| String::from("CA"))
        })?,
        budget: Budget::Unbounded,
        rank_seconds: 300,
    };
    let mut stub = Stub::scored(vec![0.1, 0.9]);
    let choice = select(
        TENANT,
        &table,
        Some("de"),
        None,
        Some("a task"),
        &policy,
        Some(engine(&mut stub)),
    )?;
    assert_eq!(choice.route().id(), "de", "the named route wins");
    assert_eq!(choice.method, Method::Named, "the record says named");
    assert_eq!(stub.calls, 0, "no ranking for an explicit route");
    Ok(())
}

/// The residency filter runs before the ranking: the best score outside the
/// rule loses to the next eligible route, and the ranking names only eligible
/// routes.
#[test]
fn sim_a_best_ranked_route_outside_the_rule_loses_to_the_next_eligible() -> Check {
    let alpha = stored("alpha", Some("DE"), "the best route for every task")?;
    let beta = stored("beta", Some("CA"), "a worse route for every task")?;
    let table = Table::new(vec![alpha, beta], Some(String::from("alpha")));
    let policy = Policy {
        residency: Residency::parse(&|name: &str| {
            (name == "LARRY_RESIDENCY").then(|| String::from("CA"))
        })?,
        budget: Budget::Unbounded,
        rank_seconds: 300,
    };
    let mut stub = Stub::scored(vec![0.9, 0.1]);
    let choice = choose(&table, "a task", &policy, &mut stub)?;
    assert_eq!(
        choice.route().id(),
        "beta",
        "the eligible route wins past the better ineligible one"
    );
    assert_eq!(choice.method, Method::Sole, "one eligible route is sole");
    assert_eq!(stub.calls, 0, "a sole route never reaches the engine");
    let sealed = choice.seal().to_string();
    assert!(
        !sealed.contains("alpha"),
        "the ranking names no ineligible route: {sealed}"
    );
    Ok(())
}

/// The budget filter runs before the ranking too: an exhausted budget leaves
/// nothing eligible and the engine never runs; any remainder admits.
#[test]
fn sim_an_exhausted_budget_filters_before_the_engine_runs() -> Check {
    let route = stored("alpha", Some("CA"), "a route")?;
    let table = Table::new(vec![route], Some(String::from("alpha")));
    for (budget, admits) in [
        (
            Budget::Remaining {
                tokens: 0,
                micros: 0,
            },
            false,
        ),
        (
            Budget::Remaining {
                tokens: 7,
                micros: 0,
            },
            true,
        ),
        (
            Budget::Remaining {
                tokens: 0,
                micros: 3,
            },
            true,
        ),
        (Budget::Unbounded, true),
    ] {
        let policy = Policy {
            residency: Residency::parse(&|_: &str| None)?,
            budget,
            rank_seconds: 300,
        };
        let mut stub = Stub::scored(vec![0.5]);
        let outcome = choose(&table, "a task", &policy, &mut stub);
        assert_eq!(
            outcome.is_ok(),
            admits,
            "exhausted denies, any remainder admits: {budget:?}"
        );
        assert_eq!(
            stub.calls, 0,
            "one route never reaches the engine: {budget:?}"
        );
    }
    Ok(())
}

/// An unnamed task with no text to rank is refused before the engine runs: a
/// missing query is a caller bug, never a silent ranking of nothing.
#[test]
fn sim_an_unnamed_task_with_no_text_is_refused_before_the_ranker_runs() -> Check {
    let table = Table::new(
        vec![
            stored("alpha", Some("CA"), "the first route")?,
            stored("beta", Some("CA"), "the second route")?,
        ],
        None,
    );
    let policy = open_policy()?;
    let mut stub = Stub::scored(vec![0.9, 0.1]);
    let outcome: Result<Selection, RouteError> = select(
        TENANT,
        &table,
        None,
        None,
        None,
        &policy,
        Some(engine(&mut stub)),
    );
    assert_eq!(
        outcome,
        Err(RouteError::NoRanker {
            detail: String::from("the task names no route and carries no text to rank"),
        }),
        "a missing query is refused, never ranked empty"
    );
    assert_eq!(stub.calls, 0, "the engine never runs without text");
    Ok(())
}

/// Without a ranker there is no ranking: the eligible default wins and the
/// record says `default`; a default the rule refuses is refused, never routed
/// around in silence.
#[test]
fn sim_without_a_ranker_the_eligible_default_wins_and_the_record_says_so() -> Check {
    let de = stored("de", Some("DE"), "german route")?;
    let ca = stored("ca", Some("CA"), "canadian route")?;
    let table = Table::new(vec![de, ca], Some(String::from("ca")));
    let open = open_policy()?;
    let choice: Selection = select(
        TENANT,
        &table,
        None,
        None,
        Some("a task"),
        &open,
        None::<&mut dyn Scorer>,
    )?;
    assert_eq!(choice.route().id(), "ca", "the default wins");
    assert_eq!(choice.method, Method::Default, "the record says default");
    assert!(
        choice.seal().to_string().contains("\"method\":\"default\""),
        "the record says default: {}",
        choice.seal()
    );
    let constrained = Policy {
        residency: Residency::parse(&|name: &str| {
            (name == "LARRY_RESIDENCY").then(|| String::from("US"))
        })?,
        budget: Budget::Unbounded,
        rank_seconds: 300,
    };
    let refused: Result<Selection, RouteError> = select(
        TENANT,
        &table,
        None,
        None,
        Some("a task"),
        &constrained,
        None::<&mut dyn Scorer>,
    );
    assert_eq!(
        refused,
        Err(RouteError::NoEligibleRoute),
        "no ranker and no eligible default is refused"
    );
    Ok(())
}

/// The residency rule is parsed strictly: an unset or blank rule constrains
/// nothing, a malformed one is refused naming the knob, and a route that
/// declares no country is ineligible under any constraint.
#[test]
fn sim_a_malformed_residency_rule_is_refused_by_name() -> Check {
    for rule in ["", "   "] {
        let parsed =
            Residency::parse(&|name: &str| (name == "LARRY_RESIDENCY").then(|| rule.to_owned()))?;
        assert!(
            parsed.admits(&Route::stored(&Draft {
                id: "alpha",
                kind: Kind::OpenAiChat,
                base_url: "https://gateway.test:1/v1",
                model: "model-1",
                key_env: "LARRY_KEY_0123456789ABCDEF_ALPHA",
                temperature: None,
                request_bytes: None,
                residency: None,
                describe: "a route",
            })?),
            "{rule:?} constrains nothing"
        );
    }
    for rule in ["ca", "CAX", "CA,", "CA,,US", "C A", "12"] {
        let outcome =
            Residency::parse(&|name: &str| (name == "LARRY_RESIDENCY").then(|| rule.to_owned()));
        assert_eq!(
            outcome,
            Err(RouteError::BadResidencyRule),
            "{rule:?} is refused"
        );
    }
    assert_eq!(
        RouteError::BadResidencyRule.to_string(),
        "route residency rule (LARRY_RESIDENCY) must be comma-separated ISO 3166-1 alpha-2 country codes, or unset",
        "the refusal names the knob, never its value"
    );
    let nowhere = Route::stored(&Draft {
        id: "nowhere",
        kind: Kind::OpenAiChat,
        base_url: "https://gateway.test:1/v1",
        model: "model-1",
        key_env: "LARRY_KEY_0123456789ABCDEF_NOWHERE",
        temperature: None,
        request_bytes: None,
        residency: None,
        describe: "a route nowhere",
    })?;
    let rule =
        Residency::parse(&|name: &str| (name == "LARRY_RESIDENCY").then(|| String::from("CA")))?;
    assert!(
        !rule.admits(&nowhere),
        "an undeclared country is outside every constraint"
    );
    Ok(())
}

/// Without a ranker and no default anywhere, the refusal stays `NoRoute` when
/// the filters removed nothing: the tenant set no default, and no rule is to
/// blame.
#[test]
fn sim_without_a_ranker_and_no_default_the_refusal_stays_no_route() -> Check {
    let build = |id: &str| stored(id, Some("CA"), "a route");
    let table = Table::new(vec![build("alpha")?, build("beta")?], None);
    let policy = open_policy()?;
    let outcome: Result<Selection, RouteError> = select(
        TENANT,
        &table,
        None,
        None,
        Some("a task"),
        &policy,
        None::<&mut dyn Scorer>,
    );
    assert_eq!(
        outcome,
        Err(RouteError::NoRoute),
        "no default and no filter is today's refusal"
    );
    Ok(())
}

/// An empty table ranks nothing: with no environment route the refusal stays
/// `NoRoute`, and an environment route the tenant rule filters is
/// `NoEligibleRoute`; the engine never runs for either.
#[test]
fn sim_an_empty_table_refuses_before_any_ranking() -> Check {
    let table = Table::new(Vec::new(), None);
    let policy = open_policy()?;
    let mut stub = Stub::scored(Vec::new());
    let outcome: Result<Selection, RouteError> = choose(&table, "a task", &policy, &mut stub);
    assert_eq!(
        outcome,
        Err(RouteError::NoRoute),
        "an empty table with no environment route is no route"
    );
    assert_eq!(stub.calls, 0, "the engine never runs on an empty table");
    let environment = Route::environment(&Draft {
        id: ENV_ROUTE,
        kind: Kind::OpenAiChat,
        base_url: "https://gateway.test:1/v1",
        model: "model-1",
        key_env: "LARRY_MODEL_KEY",
        temperature: None,
        request_bytes: None,
        residency: Some("CA"),
        describe: "declared by the operator",
    })?;
    let constrained = Policy {
        residency: Residency::parse(&|name: &str| {
            (name == "LARRY_RESIDENCY").then(|| String::from("US"))
        })?,
        budget: Budget::Unbounded,
        rank_seconds: 300,
    };
    let mut filtered = Stub::scored(Vec::new());
    let refused: Result<Selection, RouteError> = select(
        TENANT,
        &table,
        None,
        Some(environment),
        Some("a task"),
        &constrained,
        Some(engine(&mut filtered)),
    );
    assert_eq!(
        refused,
        Err(RouteError::NoEligibleRoute),
        "an environment route the rule filters is refused, never ranked"
    );
    assert_eq!(
        filtered.calls, 0,
        "a filtered table never reaches the engine"
    );
    Ok(())
}

/// A plaintext route never ranks: the best score on an `http` host loses to
/// the next eligible route without the engine ever seeing the refused host,
/// and a plaintext default or name is refused exactly as the configured
/// resolver refuses it (larry#536).
#[test]
fn sim_a_plaintext_route_never_ranks() -> Check {
    let plain = Route::stored(&Draft {
        id: "plain",
        kind: Kind::OpenAiChat,
        base_url: "http://gateway.test:1/v1",
        model: "model-1",
        key_env: "LARRY_KEY_0123456789ABCDEF_PLAIN",
        temperature: None,
        request_bytes: None,
        residency: None,
        describe: "the best route for every task",
    })?;
    let sealed = Route::stored(&Draft {
        id: "sealed",
        kind: Kind::OpenAiChat,
        base_url: "https://gateway.test:2/v1",
        model: "model-2",
        key_env: "LARRY_KEY_0123456789ABCDEF_SEALED",
        temperature: None,
        request_bytes: None,
        residency: None,
        describe: "a worse route for every task",
    })?;
    let policy = open_policy()?;
    let table = Table::new(vec![plain.clone(), sealed.clone()], None);
    let mut stub = Stub::scored(vec![0.9, 0.1]);
    let choice = choose(&table, "a task", &policy, &mut stub)?;
    assert_eq!(
        choice.route().id(),
        "sealed",
        "the plaintext route never ranks"
    );
    assert_eq!(choice.method, Method::Sole, "one eligible route is sole");
    assert_eq!(stub.calls, 0, "the refused host never reaches the engine");
    let named: Result<Selection, RouteError> = select(
        TENANT,
        &table,
        Some("plain"),
        None,
        Some("a task"),
        &policy,
        Some(engine(&mut stub)),
    );
    assert_eq!(
        named,
        Err(RouteError::InsecureTransport {
            host: String::from("gateway.test:1"),
        }),
        "a named plaintext route is refused like the resolver refuses it"
    );
    let defaulted = Table::new(vec![plain, sealed], Some(String::from("plain")));
    let refused: Result<Selection, RouteError> = select(
        TENANT,
        &defaulted,
        None,
        None,
        Some("a task"),
        &policy,
        Some(engine(&mut stub)),
    );
    assert_eq!(
        refused,
        Err(RouteError::InsecureTransport {
            host: String::from("gateway.test:1"),
        }),
        "a plaintext default is refused before any ranking"
    );
    Ok(())
}
