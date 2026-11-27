//! The route a briefed task sends its model calls to (larry#202, larry#203).
//! `select_for` chooses it: the named route when the task names one, else the
//! best-ranked eligible route over the tenant's table, with the transport and
//! key binding gates, then the residency rule and the budget filter before the
//! shipped local ranker scores the survivors by meaning. The tenant's stored
//! residency policy (`larry route residency`) is checked after the choice
//! exactly as `routes::resolve_for` checks it, so a briefed task cannot reach
//! a route outside it either. `task_select` builds the ranker's query from the
//! brief. Nothing is sent to choose it.

use std::path::Path;

use super::routes::tenant_text;
use super::task::Task;
use super::{ExecError, Ledger, Settings};
#[cfg(feature = "rerank")]
use crate::model::RouteError;
use crate::model::{Policy, Selection, env_route, query_of, select};
use crate::rerank::Scorer;
#[cfg(feature = "rerank")]
use crate::rerank::{Candidate, Deadline, RerankError};

/// Builds the shipped ranker once, or names why it cannot.
#[cfg(feature = "rerank")]
type MakeScorer = Box<dyn FnMut() -> Result<Box<dyn Scorer>, RouteError>>;

/// A scorer that builds the shipped local ranker on its first scored
/// candidate, never on construction: a tenant whose choice needs no ranking
/// never fails for a missing model or home directory, and a missing engine
/// fails only the ranking that needs it, exactly as if no engine had been
/// built (larry#202).
#[cfg(feature = "rerank")]
struct LazyScorer {
    /// Builds the engine; taken on first use.
    make: Option<MakeScorer>,
    /// The engine once built.
    engine: Option<Box<dyn Scorer>>,
}

#[cfg(feature = "rerank")]
impl LazyScorer {
    /// A scorer that builds the shipped local ranker on first use.
    fn local() -> Self {
        Self {
            make: Some(Box::new(|| {
                let scorer: Box<dyn Scorer> =
                    Box::new(crate::rerank::local_scorer().map_err(|cause| {
                        RouteError::NoRanker {
                            detail: format!(
                                "{cause}; fetch the pinned model and retry, or name the route"
                            ),
                        }
                    })?);
                Ok(scorer)
            })),
            engine: None,
        }
    }
}

#[cfg(feature = "rerank")]
impl Scorer for LazyScorer {
    fn score(
        &mut self,
        query: &str,
        rows: &[Candidate],
        batch: usize,
        deadline: Deadline,
    ) -> Result<Vec<f64>, RerankError> {
        match self.engine {
            Some(ref mut engine) => engine.score(query, rows, batch, deadline),
            None => {
                let mut make = self.make.take().ok_or_else(|| {
                    RerankError::Engine(String::from("the route ranker is unavailable"))
                })?;
                let engine = make().map_err(|cause| match cause {
                    RouteError::NoRanker { detail } => RerankError::Engine(detail),
                    _ => RerankError::Engine(cause.to_string()),
                })?;
                self.engine
                    .insert(engine)
                    .score(query, rows, batch, deadline)
            }
        }
    }
}

/// The route a briefed task sends its model calls to: the named route when the
/// task names one (resolved exactly as the configured resolver does), else the
/// best-ranked eligible route. An explicit name always wins; without one the
/// transport and key binding gates, then the residency rule and the budget
/// filter before the shipped local ranker scores the survivors by meaning,
/// and without the `rerank` feature the eligible default wins (larry#202).
/// The tenant's stored residency policy (`larry route residency`) is checked
/// after the choice exactly as `resolve_for` checks it, so a briefed task
/// cannot reach a route outside it either (larry#203). Nothing is sent to
/// choose it.
///
/// # Errors
///
/// [`ExecError::Route`] for an unknown name, a malformed ranking knob, no
/// eligible route, an unavailable ranker or a route outside the stored
/// residency policy; [`ExecError::ModelConfig`] for an environment route that
/// cannot be read.
pub(super) fn select_for(
    ledger: &Ledger,
    state: &Path,
    named: Option<&str>,
    query: Option<&str>,
    read: &impl Fn(&str) -> Option<String>,
) -> Result<Selection, ExecError> {
    let tenant = tenant_text(state);
    let table = ledger.routes(&tenant)?;
    let environment = if table.lookup(named).is_some() {
        None
    } else {
        env_route(read).map_err(ExecError::ModelConfig)?
    };
    let policy = Policy::parse(read).map_err(ExecError::Route)?;
    // The engine builds on its first scored candidate, never on construction:
    // a tenant whose choice needs no ranking never fails for a missing model
    // or home directory, and a missing engine fails only the ranking that
    // needs it, exactly as if no engine had been built (larry#202).
    #[cfg(feature = "rerank")]
    let mut engine: Option<Box<dyn Scorer>> = if named.is_none() {
        let scorer: Box<dyn Scorer> = Box::new(LazyScorer::local());
        Some(scorer)
    } else {
        None
    };
    #[cfg(not(feature = "rerank"))]
    let mut engine: Option<Box<dyn Scorer>> = None;
    // The scorer borrows the engine only for the call below, so each arm
    // ends the borrow before the choice returns. The query stays an option
    // into `select`: a named task resolves before the ranker runs, and an
    // unnamed one ranks only on the text `task_select` built from its brief.
    let choice = match engine.as_mut() {
        Some(held) => {
            let scorer: &mut dyn Scorer = &mut **held;
            select(
                &tenant,
                &table,
                named,
                environment,
                query,
                &policy,
                Some(scorer),
            )
        }
        None => select(&tenant, &table, named, environment, query, &policy, None),
    };
    let selection = choice.map_err(ExecError::Route)?;
    if let Some(allowed) = ledger.residency(&tenant)?
        && let Some(refusal) = selection.route().residency_refusal(&allowed)
    {
        return Err(ExecError::Route(refusal));
    }
    Ok(selection)
}

/// The route a briefed task sends its model calls to: the named route when
/// the task names one, else the best-ranked eligible route, with the
/// residency and budget filters before the ranker and the choice sealed
/// beside the route for the replay to answer from (larry#202).
///
/// # Errors
///
/// [`ExecError::Route`] for an unknown name, a malformed ranking knob, no
/// eligible route or an unavailable ranker; [`ExecError::ModelConfig`] for an
/// environment route that cannot be read.
pub(crate) fn task_select(
    settings: &Settings,
    ledger: &Ledger,
    task: &Task,
) -> Result<Selection, ExecError> {
    let query = match (task.route(), task.brief()) {
        (None, Some(brief)) => Some(query_of(brief.goal(), brief.files(), task.acceptance())),
        _ => None,
    };
    select_for(
        ledger,
        settings.state_dir(),
        task.route(),
        query.as_deref(),
        &|name: &str| (settings.read)(name),
    )
}

#[cfg(test)]
mod tests {
    use super::select_for;
    use crate::executor::routes::tenant_text;
    use crate::executor::{ExecError, Ledger};
    use crate::model::{Draft, Kind, Route, RouteError};

    /// A stored route with one residency is refused after the choice when the
    /// tenant's stored policy allows only another country: the refusal names
    /// the route, and no request leaves the host (larry#203). Without the
    /// stored check the selection succeeds, so this test fails on it.
    #[test]
    fn select_for_refuses_a_route_outside_the_stored_policy()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = crate::executor::sim_tree::scratch("larry-route-select-policy")?;
        let ledger = Ledger::open(&directory)?;
        let tenant = tenant_text(&directory);
        let namespace = crate::model::tenant_namespace(&tenant);
        let key_env = format!("{namespace}MAIN");
        let route = Route::new(&Draft {
            id: "main",
            kind: Kind::OpenAiChat,
            base_url: "http://127.0.0.1:9/v1",
            model: "mock-model",
            key_env: key_env.as_str(),
            temperature: None,
            request_bytes: None,
            residency: Some("US"),
            prompt: None,
            describe: "a test route",
        })
        .map_err(ExecError::Route)?;
        ledger.put_route(&tenant, &route, true)?;
        ledger.set_residency(&tenant, &["CA".to_owned()])?;
        let read = |_: &str| -> Option<String> { None };
        let refusal = select_for(&ledger, &directory, None, None, &read);
        assert!(
            matches!(
                refusal,
                Err(ExecError::Route(RouteError::ResidencyNotAllowed { .. }))
            ),
            "the stored policy refuses the choice: {refusal:?}"
        );
        Ok(())
    }
}
