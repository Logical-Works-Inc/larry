//! Semantic route selection (larry#202): a task that names no route is sent to
//! the eligible route whose capability description the shipped local reranker
//! scores highest for the task. A keyword or regex overlap is never the ranker
//! (SD-2): the scores come from the `rerank::Scorer` the shipped `larry-rerank`
//! runs, so the same task over the same table seals the same choice and a
//! replay answers from the record without scoring again.
//!
//! The order is fixed: an explicit route always wins, then the residency rule
//! (`LARRY_RESIDENCY`) and the budget filter out what the task must not reach,
//! and only the survivors are ranked. Without a ranker (a build without the
//! `rerank` feature) the eligible default wins and the record says so.

use lgwks_std::json::{Map, Value, json};

use super::route::{ENV_ROUTE, Route, RouteError, Table, is_residency, resolve};
use crate::rerank::{self, Candidate, Scorer};

/// The variable naming the countries a task's data may be processed in:
/// comma-separated ISO 3166-1 alpha-2 codes. Unset or blank constrains
/// nothing; #203 moves the rule into per-tenant data.
pub(crate) const RESIDENCY_VARIABLE: &str = "LARRY_RESIDENCY";
/// The variable bounding one ranking run, in seconds.
pub(crate) const RANK_SECONDS_VARIABLE: &str = "LARRY_ROUTE_RANK_SECONDS";
/// The ranking deadline when the knob is unset, in seconds.
pub(crate) const DEFAULT_RANK_SECONDS: u64 = 300;
/// Most seconds one ranking run may take, mirroring the reranker's own bound.
pub(crate) const MAX_RANK_SECONDS: u64 = 3600;

/// How a selection chose its route, as the sealed record names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    /// The task named the route, so no filter and no ranker ran.
    Named,
    /// The ranker scored at least two eligible routes and the best won.
    Ranked,
    /// One route was eligible, so there was nothing to rank.
    Sole,
    /// No ranker was built, so the eligible default won.
    Default,
    /// No stored route was eligible, so the eligible environment route won.
    Environment,
}

impl Method {
    /// The method's name in the sealed record.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Named => "named",
            Self::Ranked => "ranked",
            Self::Sole => "sole",
            Self::Default => "default",
            Self::Environment => "environment",
        }
    }
}

/// The tenant's residency rule: the countries a task's data may be processed
/// in, or nothing when the operator set no constraint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Residency {
    /// The allowed countries; empty constrains nothing, so every route is
    /// eligible.
    allowed: Vec<String>,
}

impl Residency {
    /// Reads the rule from `read`: `LARRY_RESIDENCY` missing or blank
    /// constrains nothing, otherwise every comma-separated part must be two
    /// uppercase ASCII letters.
    ///
    /// # Errors
    ///
    /// [`RouteError::BadResidencyRule`] when a part is not a country code; the
    /// refusal names the knob, never its value.
    pub(crate) fn parse(read: &impl Fn(&str) -> Option<String>) -> Result<Self, RouteError> {
        let Some(text) = read(RESIDENCY_VARIABLE) else {
            return Ok(Self {
                allowed: Vec::new(),
            });
        };
        if text.trim().is_empty() {
            return Ok(Self {
                allowed: Vec::new(),
            });
        }
        let mut allowed = Vec::new();
        for part in text.split(',') {
            let code = part.trim();
            if !is_residency(code) {
                return Err(RouteError::BadResidencyRule);
            }
            if !allowed.contains(&code.to_owned()) {
                allowed.push(code.to_owned());
            }
        }
        Ok(Self { allowed })
    }

    /// Whether `route` may process this tenant's data: every route when no
    /// rule constrains the tenant, else only a route declaring an allowed
    /// country. A route that declares no country is outside every constraint.
    pub(crate) fn admits(&self, route: &Route) -> bool {
        self.allowed.is_empty()
            || route
                .residency()
                .is_some_and(|code| self.allowed.iter().any(|one| one == code))
    }
}

/// The tenant's remaining model budget. The executor builds
/// [`Budget::Unbounded`] today; #204 fills `Remaining` from its ledger and
/// un-gates the variant, and the filter already runs first so that ledger
/// plugs in here with no reordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Budget {
    /// No budget is recorded yet, so every route is affordable.
    Unbounded,
    /// The tokens and micro-dollars the tenant may still spend.
    #[cfg(test)]
    Remaining {
        /// Tokens left.
        tokens: u64,
        /// Micro-dollars left.
        micros: u64,
    },
}

/// The inputs the filters read before any ranking: the residency rule, the
/// budget and the ranking deadline. Parsed once per selection from the
/// operator environment, so the ranker never sees what the filters removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Policy {
    /// The countries a task's data may be processed in.
    pub(crate) residency: Residency,
    /// The tokens and micro-dollars the tenant may still spend.
    pub(crate) budget: Budget,
    /// Most seconds one ranking run may take.
    pub(crate) rank_seconds: u64,
}

impl Policy {
    /// Reads the policy from `read`: the residency rule and the ranking
    /// deadline; the budget is unbounded until #204 records it.
    ///
    /// # Errors
    ///
    /// [`RouteError::BadResidencyRule`] or [`RouteError::BadRankSeconds`]
    /// naming the knob, never its value.
    pub(crate) fn parse(read: &impl Fn(&str) -> Option<String>) -> Result<Self, RouteError> {
        Ok(Self {
            residency: Residency::parse(read)?,
            budget: Budget::Unbounded,
            rank_seconds: rank_seconds(read)?,
        })
    }
}

impl Budget {
    /// Whether `route` is affordable: everything while unbounded, and
    /// anything while either remainder is unspent once #204 prices routes.
    pub(crate) const fn admits(&self, _route: &Route) -> bool {
        match *self {
            Self::Unbounded => true,
            #[cfg(test)]
            Self::Remaining { tokens, micros } => tokens > 0 || micros > 0,
        }
    }
}

/// What a selection chose and how to prove it: the route, the method, and,
/// when the ranker ran, the full ranking with every score and the pinned
/// ranker identity, so a replay answers from the record.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Selection {
    /// The chosen route.
    route: Route,
    /// How it was chosen.
    method: Method,
    /// The eligible route ids, best first; non-empty only when ranked.
    ranking: Vec<String>,
    /// Every eligible route's score, parallel to `ranking`.
    scores: Vec<f64>,
    /// The ranker's model id at selection time.
    model: &'static str,
    /// The ranker's snapshot revision at selection time.
    revision: &'static str,
}

impl Selection {
    /// A choice made without the ranker: only the method is recorded.
    fn decided(route: Route, method: Method, pin: (&'static str, &'static str)) -> Self {
        Self {
            route,
            method,
            ranking: Vec::new(),
            scores: Vec::new(),
            model: pin.0,
            revision: pin.1,
        }
    }

    /// The chosen route.
    pub(crate) fn route(&self) -> &Route {
        &self.route
    }

    /// The choice as the sealed record carries it beside the route seal: the
    /// method and the ranker pin always, and the full ranking with every
    /// score when the ranker ran.
    pub(crate) fn seal(&self) -> Value {
        let mut record = Map::new();
        record.insert(String::from("method"), Value::from(self.method.name()));
        record.insert(String::from("model"), Value::from(self.model));
        record.insert(String::from("revision"), Value::from(self.revision));
        if self.method == Method::Ranked {
            record.insert(
                String::from("ranking"),
                Value::Array(
                    self.ranking
                        .iter()
                        .map(|id| Value::from(id.as_str()))
                        .collect(),
                ),
            );
            let mut scores = Map::new();
            for (id, score) in self.ranking.iter().zip(self.scores.iter()) {
                scores.insert(id.clone(), json!(score));
            }
            record.insert(String::from("scores"), Value::Object(scores));
        }
        Value::Object(record)
    }
}

/// The task text the ranker scores route descriptions against: the goal, the
/// files and the acceptance commands, cut to the ranker's query bound.
pub(crate) fn query_of(goal: &str, files: &[String], acceptance: &[Vec<String>]) -> String {
    let mut query = String::from(goal);
    for file in files {
        query.push('\n');
        query.push_str(file);
    }
    for argv in acceptance {
        query.push('\n');
        query.push_str(&argv.join(" "));
    }
    let bound = rerank::MAX_QUERY_BYTES.min(query.len());
    let mut end = bound;
    while !query.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    query.truncate(end);
    query
}

/// The text the ranker scores a route on: its capability description, or its
/// id and model when it describes nothing, so an undescribed route still
/// ranks instead of vanishing.
fn candidate_text(route: &Route) -> String {
    if route.describe().is_empty() {
        format!("{} {}", route.id(), route.model())
    } else {
        route.describe().to_owned()
    }
}

/// The whole seconds `RANK_SECONDS_VARIABLE` names, or the default when it is
/// unset.
///
/// # Errors
///
/// [`RouteError::BadRankSeconds`] when the knob is not a whole number of
/// seconds from 1 to 3600.
fn rank_seconds(read: &impl Fn(&str) -> Option<String>) -> Result<u64, RouteError> {
    match read(RANK_SECONDS_VARIABLE) {
        None => Ok(DEFAULT_RANK_SECONDS),
        Some(text) => match text.trim().parse::<u64>() {
            Ok(seconds) if (1..=MAX_RANK_SECONDS).contains(&seconds) => Ok(seconds),
            _ => Err(RouteError::BadRankSeconds),
        },
    }
}

/// Chooses the route a task that names `named` (or none) sends its model
/// calls to. An explicit name resolves exactly as before and always wins: no
/// filter and no ranker runs for it. Without a name, the stored routes the
/// tenant may not use (another tenant's key variable), the residency rule and
/// the budget filter first; then the survivors rank by meaning when `scorer`
/// runs, the sole survivor wins directly, and without a scorer the eligible
/// default (else the eligible environment route) wins. No eligible route is
/// [`RouteError::NoEligibleRoute`], except an empty table with no environment
/// route, which stays [`RouteError::NoRoute`].
///
/// # Errors
///
/// [`RouteError`] for an unknown name, a damaged default, a malformed ranking
/// knob, no eligible route, a ranking with no text to score, or a ranker that
/// is unavailable or breaks its contract; a scorer failure never falls back
/// to a silent pick.
pub(crate) fn select(
    tenant: &str,
    table: &Table,
    named: Option<&str>,
    environment: Option<Route>,
    query: Option<&str>,
    policy: &Policy,
    scorer: Option<&mut dyn Scorer>,
) -> Result<Selection, RouteError> {
    let pin = rerank::pin().map_err(|cause| RouteError::NoRanker {
        detail: format!("the pinned ranker identity is malformed: {cause}"),
    })?;
    // An explicit name resolves exactly as the configured resolver does, so a
    // later resolve filter covers the named task path with no second copy of
    // the order, and the win is recorded as named (or environment for `env`).
    if named.is_some() {
        return match resolve(tenant, table, named, environment) {
            Ok(route) => {
                let method = if route.id() == ENV_ROUTE {
                    Method::Environment
                } else {
                    Method::Named
                };
                Ok(Selection::decided(route, method, pin))
            }
            Err(refusal) => Err(refusal),
        };
    }
    if let Some(default) = table
        .default_id()
        .and_then(|id| table.routes().iter().find(|route| route.id() == id))
    {
        // The default answers exactly as the configured resolver answers it:
        // transport first, then its key binding (larry#328, larry#536).
        if let Some(refusal) = default.transport_refusal() {
            return Err(refusal);
        }
        if let Some(refusal) = default.key_env_refusal(tenant) {
            return Err(refusal);
        }
    }
    let eligible: Vec<&Route> = table
        .routes()
        .iter()
        .filter(|route| {
            // A route the configured resolver would refuse never ranks: its
            // transport or binding fails closed here, before the residency
            // rule and the budget filter run (larry#202, larry#328, larry#536).
            route.transport_refusal().is_none()
                && route.key_env_refusal(tenant).is_none()
                && policy.residency.admits(route)
                && policy.budget.admits(route)
        })
        .collect();
    if eligible.is_empty() {
        return match environment {
            Some(route) if policy.residency.admits(&route) && policy.budget.admits(&route) => {
                Ok(Selection::decided(route, Method::Environment, pin))
            }
            Some(_) => Err(RouteError::NoEligibleRoute),
            None if table.routes().is_empty() => Err(RouteError::NoRoute),
            None => Err(RouteError::NoEligibleRoute),
        };
    }
    if eligible.len() == 1 {
        // The length was just checked, so the first route exists; the refusal
        // below is unreachable and only keeps a broken index from misrouting.
        let Some(only) = eligible.first() else {
            return Err(RouteError::NoRanker {
                detail: String::from("the eligible routes vanished before ranking"),
            });
        };
        return Ok(Selection::decided((*only).clone(), Method::Sole, pin));
    }
    rank(
        table,
        environment.as_ref(),
        query,
        policy,
        scorer,
        pin,
        &eligible,
    )
}

/// Ranks at least two eligible routes by meaning and chooses the best, or,
/// without a scorer, the eligible default (else the eligible environment
/// route). A missing query where the scorer would run, a scorer failure or a
/// broken score contract is [`RouteError::NoRanker`], never a silent pick;
/// the ranking is a stable sort, so ties keep table order.
///
/// # Errors
///
/// [`RouteError::NoEligibleRoute`] when no default and no environment route
/// can take the task without a scorer; [`RouteError::NoRanker`] when the
/// scorer fails, the query is missing, or a score is missing or not finite.
fn rank(
    table: &Table,
    environment: Option<&Route>,
    query: Option<&str>,
    policy: &Policy,
    scorer: Option<&mut dyn Scorer>,
    pin: (&'static str, &'static str),
    eligible: &[&Route],
) -> Result<Selection, RouteError> {
    let Some(scorer) = scorer else {
        if let Some(default) = table
            .default_id()
            .and_then(|id| eligible.iter().find(|route| route.id() == id))
        {
            return Ok(Selection::decided((*default).clone(), Method::Default, pin));
        }
        if let Some(route) = environment
            && policy.residency.admits(route)
            && policy.budget.admits(route)
        {
            return Ok(Selection::decided(route.clone(), Method::Environment, pin));
        }
        // No ranker, no eligible default and no eligible environment. When the
        // filters removed nothing, the tenant simply set no default: today's
        // `NoRoute`, not a filter refusal.
        if table.default_id().is_none()
            && environment.is_none()
            && eligible.len() == table.routes().len()
        {
            return Err(RouteError::NoRoute);
        }
        return Err(RouteError::NoEligibleRoute);
    };
    // The scorer runs only here, so a missing query is refused typed rather
    // than ranked empty: without text there is nothing to score by meaning.
    let Some(query) = query else {
        return Err(RouteError::NoRanker {
            detail: String::from("the task names no route and carries no text to rank"),
        });
    };
    let rows: Vec<Candidate> = eligible
        .iter()
        .map(|route| Candidate::new(route.id().to_owned(), candidate_text(route)))
        .collect();
    let scores = scorer
        .score(
            query,
            &rows,
            rows.len(),
            crate::rerank::Deadline::limit(policy.rank_seconds),
        )
        .map_err(|cause| RouteError::NoRanker {
            detail: cause.to_string(),
        })?;
    if scores.len() != rows.len() {
        return Err(RouteError::NoRanker {
            detail: format!(
                "the ranker returned {} scores for {} candidates",
                scores.len(),
                rows.len()
            ),
        });
    }
    if scores.iter().any(|score| !score.is_finite()) {
        return Err(RouteError::NoRanker {
            detail: String::from("a ranker score is not finite"),
        });
    }
    // Pair each eligible route with its score; the sort is stable, so ties
    // keep table order and no index ever names a route.
    let mut ranked: Vec<(&Route, f64)> = eligible
        .iter()
        .copied()
        .zip(scores.iter().copied())
        .collect();
    ranked.sort_by(|left, right| right.1.total_cmp(&left.1));
    let mut ranking = Vec::with_capacity(ranked.len());
    let mut ordered = Vec::with_capacity(ranked.len());
    for &(route, score) in &ranked {
        ranking.push(route.id().to_owned());
        ordered.push(score);
    }
    let Some(&(winner, _)) = ranked.first() else {
        // The ranker scored at least two candidates, so the ranking is never
        // empty; the refusal below is unreachable and only keeps a missing
        // winner from misrouting.
        return Err(RouteError::NoRanker {
            detail: String::from("the ranker ranked no candidate"),
        });
    };
    Ok(Selection {
        route: (*winner).clone(),
        method: Method::Ranked,
        ranking,
        scores: ordered,
        model: pin.0,
        revision: pin.1,
    })
}

/// Seeded simulations of the choice (larry#202); the cases live in
/// `sim_select.rs` and `sim_select_rank.rs` so the choice and its proof change
/// together.
#[cfg(test)]
#[path = "sim_select.rs"]
mod sim_select;

/// Seeded simulations of ranking by meaning (larry#202); the cases live here
/// so the ranker and its proof change together.
#[cfg(test)]
#[path = "sim_select_rank.rs"]
mod sim_select_rank;

/// Route-choice latency with the pinned ranker on (larry#202); a measurement
/// control, not a seeded sim, so it lives outside the `sim_*` modules.
#[cfg(test)]
#[path = "select_latency.rs"]
mod select_latency;
