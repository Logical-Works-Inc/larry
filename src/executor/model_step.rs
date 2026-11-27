//! A task's model step: the route its calls go to and a client built from it.
//! Split from the executor root so the root stays under the size law; the step
//! is one job (route resolution plus client construction) beside the task body.

use std::sync::Arc;

use super::author;
use super::backoff;
use super::digest_hex;
use super::identity;
use super::{ExecError, Ledger, Settings, Task};
use crate::model::{ModelClient, Route};

/// What a task with a brief needs before anything is cloned: the route its model
/// calls go to and a client built from it.
pub(super) struct ModelStep<'task> {
    /// The route the task resolved to; the sealed report records it.
    pub(super) route: Route,
    /// The brief the model writes files for.
    pub(super) brief: &'task author::Brief,
    /// The client over `route`, holding the key read for this task alone.
    pub(super) client: ModelClient,
}

/// Resolves the task's route and builds its client, only when the task has a
/// brief: a task with none never reaches a model, so its route is only checked
/// when it names one, and a typo in a model setting fails nothing it does not use.
/// An unknown name is refused here, before any byte leaves the host.
pub(super) fn model_step<'task>(
    task: &'task Task,
    settings: &Settings,
    ledger: &Ledger,
) -> Result<Option<ModelStep<'task>>, ExecError> {
    let Some(brief) = task.brief() else {
        if let Some(named) = task.route() {
            settings.route(ledger, Some(named))?;
        }
        return Ok(None);
    };
    let route = settings.route(ledger, task.route())?;
    // The one timeline the retry budget reads, built the way the gate builds its own (larry#436).
    let clock = larry_runtime::live_clock().map_err(ExecError::Gate)?;
    let tenant = digest_hex(identity::tenant_of(&settings.state_dir).as_bytes());
    let client = ModelClient::with_limiter(
        settings.model_config(&route)?,
        clock,
        Arc::clone(&settings.limiter),
        tenant,
        route.id(),
        backoff,
    );
    Ok(Some(ModelStep {
        route,
        brief,
        client,
    }))
}
