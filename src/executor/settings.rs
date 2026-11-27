//! The door's settings: what the executor reads from its environment. One
//! [`Settings`] per door (`larry exec`, bare `larry`, the remote doors) names
//! the state directory, the timeout, the path, the policy, the secrets and the
//! reader the task's route and key are read through. The task run itself lives
//! in the parent module; this one only configures the door it runs behind
//! (larry#164, larry#203).

use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use larry_contracts::authority::Scope;

use crate::model::{ModelConfig, Route};

use super::DEFAULT_TIMEOUT_MS;
use super::ExecError;
use super::Ledger;
use super::MAX_TIMEOUT_MS;
use super::OBSERVATION_SECRETS;
use super::gate;
use super::routes;
use super::state_dir;
use super::tenant_key;
use super::workspace;

/// The explicit empty secret snapshot used by test readers without a process environment.
fn empty_secret_snapshot() -> String {
    String::new()
}

/// Reads one environment variable: the process environment in the binary, a
/// table in a test. A route's key is read through it when a task reaches the
/// model, not before, so a key rotated between two tasks is the next task's.
type EnvReader = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Settings the executor reads from its environment.
#[derive(Clone)]
#[non_exhaustive]
pub struct Settings {
    /// Directory holding the ledger and the workspaces.
    state_dir: PathBuf,
    /// Longest an acceptance command may run before its group is killed.
    timeout: Duration,
    /// The PATH the acceptance commands and git run under.
    path: String,
    /// The scopes the gate authorizes tasks against. Local settings read
    /// `LARRY_POLICY`; remote settings read the tenant ledger.
    policy: BTreeSet<Scope>,
    /// Exact secrets read from this process snapshot, for terminal observation redaction.
    secrets: Arc<[String]>,
    /// Whether this door is local (`larry exec`, bare `larry`): with no
    /// provisioned root row it allows any absolute path outside the state
    /// directory, while a remote door denies. Set once in `from_env_mode`.
    local: bool,
    /// Where a task's route reads its key and the `LARRY_MODEL_*` knobs from.
    read: EnvReader,
}

impl fmt::Debug for Settings {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("Settings")
            .field("state_dir", &self.state_dir)
            .field("timeout", &self.timeout)
            .field("path", &self.path)
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl Settings {
    /// Reads the settings through `read`, which yields one variable or `None`.
    /// The model is not read here: a task's route is resolved, and its key and
    /// `LARRY_MODEL_*` knobs are read, only when the task reaches a model.
    pub fn from_env(
        read: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> Result<Settings, ExecError> {
        Self::from_env_mode(read, true)
    }

    /// Reads executor settings for a remote door. Remote policy never falls back
    /// to `LARRY_POLICY`, and a missing ledger row is deny-all.
    pub(crate) fn remote_from_env(
        read: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> Result<Settings, ExecError> {
        Self::from_env_mode(read, false)
    }

    /// Reads the common settings and selects the policy owner for the door.
    fn from_env_mode(
        read: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
        local: bool,
    ) -> Result<Settings, ExecError> {
        let state_dir = state_dir(&read)?;
        let timeout_ms = match read("LARRY_EXEC_TIMEOUT_MS") {
            None => DEFAULT_TIMEOUT_MS,
            Some(value) => match value.parse::<u64>() {
                Ok(milliseconds) if (1..=MAX_TIMEOUT_MS).contains(&milliseconds) => milliseconds,
                _ => return Err(ExecError::BadTimeout { value }),
            },
        };
        let path = read("PATH").ok_or(ExecError::PathUnset)?;
        let policy = if local {
            gate::policy(&read)?
        } else {
            Ledger::open(&state_dir)?.tenant_policy(&tenant_key(&state_dir))?
        };
        let secret_text = match read(OBSERVATION_SECRETS) {
            Some(values) => values,
            None => empty_secret_snapshot(),
        };
        let secrets = secret_text
            .split('\0')
            .filter(|value| value.len() >= 8)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        Ok(Settings {
            state_dir,
            timeout: Duration::from_millis(timeout_ms),
            path,
            policy,
            secrets: Arc::from(secrets.into_boxed_slice()),
            local,
            read: Arc::new(read),
        })
    }

    /// Whether this door is local: remote doors deny a tenant with no root row.
    pub(crate) const fn is_local(&self) -> bool {
        self.local
    }

    /// The directory holding the ledger, the workspaces and `larry eval`'s origins.
    pub(crate) fn state_dir(&self) -> &std::path::Path {
        self.state_dir.as_path()
    }

    /// The exact secret values in this process snapshot, never formatted or logged.
    pub(crate) fn secrets(&self) -> &[String] {
        &self.secrets
    }

    /// Refreshes the remote door's policy from the durable tenant ledger. The process may
    /// stay open while an operator provisions or withdraws a tenant, so a cached snapshot
    /// must not outlive the prompt that uses it.
    pub(crate) fn refresh_remote_policy(&mut self) -> Result<(), ExecError> {
        self.policy = Ledger::open(&self.state_dir)?.tenant_policy(&tenant_key(&self.state_dir))?;
        Ok(())
    }

    /// The `PATH` acceptance commands and git run under.
    pub(crate) fn path_env(&self) -> &str {
        self.path.as_str()
    }

    /// The scopes the gate authorizes tasks against.
    pub(crate) fn policy(&self) -> &BTreeSet<Scope> {
        &self.policy
    }

    /// Longest an acceptance command may run before its group is killed.
    pub(crate) const fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The route a task naming `named` (or none) sends its model calls to: the
    /// named route, else the tenant's default, else the environment's, else a
    /// refusal. Nothing is sent to resolve it.
    pub(crate) fn route(&self, ledger: &Ledger, named: Option<&str>) -> Result<Route, ExecError> {
        routes::resolve_for(ledger, &self.state_dir, named, &|name: &str| {
            (self.read)(name)
        })
    }

    /// The route `larry eval` scores: what a task naming none resolves to.
    pub(crate) fn arm_route(&self) -> Result<Route, ExecError> {
        self.route(&Ledger::open(&self.state_dir)?, None)
    }

    /// The model configuration for calls over `route`: its key is read now, from
    /// the variable the route names.
    pub(crate) fn model_config(&self, route: &Route) -> Result<ModelConfig, ExecError> {
        ModelConfig::for_route(route, &|name: &str| (self.read)(name))
            .map_err(ExecError::ModelConfig)
    }

    /// Makes `dir`, whose files are written, a one-commit repository under these settings.
    pub(crate) fn seed_origin(&self, dir: &std::path::Path) -> Result<(), ExecError> {
        workspace::seed_origin(&self.state_dir, &self.path, dir)
    }

    /// Removes the origins a dead `larry eval` left under the state directory. An origin
    /// that cannot be settled is reported and kept, never a refusal of the eval (larry#334).
    pub(crate) fn sweep_origins(&self) -> Result<(), ExecError> {
        for fault in workspace::sweep_origins(&self.state_dir)? {
            fault.report();
        }
        Ok(())
    }
}
