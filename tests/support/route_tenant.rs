//! One tenant's route-journey harness (larry#201, larry#202): a state
//! directory, an origin repository, a gateway and a key, with the `larry
//! route` door, the `larry exec` door and the sealed-report assertions every
//! route journey drives. One home, so no journey copies it: both the
//! route-table journeys and the selection journeys run their tenants through
//! here (`route_cli_journey.rs`, `route_select_journey.rs`).

use std::error::Error;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Child, Output};

#[path = "json_at.rs"]
pub(crate) mod json_at;
#[path = "larry_cli.rs"]
pub(crate) mod larry_cli;
#[path = "mock_model.rs"]
pub(crate) mod mock_model;
#[path = "origin.rs"]
pub(crate) mod origin;
#[path = "route_key.rs"]
pub(crate) mod route_key;
#[path = "sealed_report.rs"]
pub(crate) mod sealed_report;

use json_at::At;
use larry_cli::{spawn_larry_args, stderr_of};
use lgwks_std::json::{self, Value, json};
use mock_model::Handle;
use origin::TempDir;
use sealed_report::{denial_lines, flag_of, report_of};

/// What every journey returns.
pub(crate) type TestResult = Result<(), Box<dyn Error>>;

/// The scopes a briefed task needs, so no journey depends on the default policy.
pub(crate) const POLICY: &str = "file.read,file.write,process.spawn,model.invoke";

/// One tenant: a state directory, an origin repository, a gateway and a key.
pub(crate) struct Tenant<'tenant> {
    /// The tenant's state directory.
    pub(crate) state: &'tenant TempDir,
    /// The origin repository its tasks clone.
    pub(crate) origin: &'tenant TempDir,
    /// The gateway its route points at.
    pub(crate) mock: &'tenant Handle,
    /// The model its route names.
    pub(crate) model: &'static str,
    /// The residency its route declares.
    pub(crate) residency: &'static str,
    /// The suffix of the key variable its route names, under its own tenant tag.
    pub(crate) suffix: &'static str,
    /// The key itself, which only the exec child's environment holds.
    pub(crate) key: &'static str,
}

impl<'tenant> Tenant<'tenant> {
    /// A Canadian tenant on `model-a` with the key `secret-a` in its `_A` variable.
    pub(crate) fn standard(
        state: &'tenant TempDir,
        origin: &'tenant TempDir,
        mock: &'tenant Handle,
    ) -> Self {
        Tenant {
            state,
            origin,
            mock,
            model: "model-a",
            residency: "CA",
            suffix: "A",
            key: "secret-a",
        }
    }

    /// Stores the route `id` pointing at this tenant's gateway, as the default.
    pub(crate) fn add_route(&self, id: &str) -> TestResult {
        let url = format!("http://{}/v1", host_of(self.mock));
        let key_env = route_key::key_env(self.state.path(), self.suffix)?;
        let output = larry_route(
            self.state.path(),
            &[],
            &[
                "add",
                id,
                "--url",
                &url,
                "--model",
                self.model,
                "--key-env",
                &key_env,
                "--residency",
                self.residency,
                "--default",
            ],
        )?;
        assert!(output.status.success(), "route add: {}", stderr_of(&output));
        Ok(())
    }

    /// Runs `larry exec` on `task` with the tenant's key in the child's environment.
    pub(crate) fn exec(&self, task: &str) -> Result<Output, Box<dyn Error>> {
        self.exec_with(task, &[])
    }

    /// Runs `larry exec` on `task` with the extra variables `extra` as well.
    pub(crate) fn exec_with(
        &self,
        task: &str,
        extra: &[(&str, &OsStr)],
    ) -> Result<Output, Box<dyn Error>> {
        Ok(self.start(task, extra)?.wait_with_output()?)
    }

    /// Starts `larry exec` on `task` with the extra variables `extra` as well, unwaited.
    pub(crate) fn start(
        &self,
        task: &str,
        extra: &[(&str, &OsStr)],
    ) -> Result<Child, Box<dyn Error>> {
        let variable = route_key::key_env(self.state.path(), self.suffix)?;
        let mut env: Vec<(&str, &OsStr)> = vec![
            ("LARRY_STATE_DIR", self.state.path().as_os_str()),
            ("LARRY_POLICY", OsStr::new(POLICY)),
            (variable.as_str(), OsStr::new(self.key)),
        ];
        env.extend_from_slice(extra);
        spawn_larry_args(&["exec"], &env, task.as_bytes())
    }

    /// A briefed task over this tenant's origin, naming `route` and `key` when given.
    pub(crate) fn briefed(
        &self,
        route: Option<&str>,
        key: Option<&str>,
    ) -> Result<String, Box<dyn Error>> {
        briefed_task(self.origin, route, key)
    }

    /// What a report must say about the route this tenant's task ran under.
    pub(crate) fn sealed_route(&self, id: &str) -> Value {
        json!({
            "id": id,
            "model": self.model,
            "host": host_of(self.mock),
            "residency": self.residency,
        })
    }

    /// Asserts the gateway saw this tenant's model and key, and only those.
    pub(crate) fn assert_gateway_saw_only_this_tenant(&self) -> TestResult {
        let bodies = self.mock.requests();
        assert!(!bodies.is_empty(), "the gateway served the briefed task");
        for body in &bodies {
            let sent: Value = json::from_slice(body)?;
            assert_eq!(
                sent.at("/model")?.as_str(),
                Some(self.model),
                "every request carries the route's model"
            );
        }
        let expected = Some(format!("Bearer {}", self.key));
        for header in self.mock.authorizations() {
            assert_eq!(
                header, expected,
                "only this tenant's key reached its gateway"
            );
        }
        Ok(())
    }

    /// Asserts the exec `output` is an accepted report whose authored record
    /// names this tenant's route `id`, and that no key or key variable is in it.
    pub(crate) fn assert_sealed_under(&self, output: &Output, id: &str) -> TestResult {
        let report = report_of(output)?;
        assert!(flag_of(&report, "accepted")?, "{}", stderr_of(output));
        assert!(denial_lines(&report)?.is_empty(), "no denial was sealed");
        assert_eq!(
            *report.at("/authored/0/route")?,
            self.sealed_route(id),
            "the receipt names this tenant's route, host and residency"
        );
        assert_eq!(
            report.at("/authored/0/route/model")?.as_str(),
            Some(self.model),
            "the sealed route names the model the gateway served"
        );
        let printed = String::from_utf8_lossy(&output.stdout);
        let variable = route_key::key_env(self.state.path(), self.suffix)?;
        for secret in [self.key, variable.as_str()] {
            assert!(
                !printed.contains(secret),
                "{secret} must not appear in a report"
            );
        }
        self.assert_gateway_saw_only_this_tenant()
    }
}

/// The host and port a mock listens on, as a sealed report names it.
pub(crate) fn host_of(mock: &Handle) -> String {
    format!("127.0.0.1:{}", mock.addr().port())
}

/// Runs `larry route <args>` with only PATH, the state directory and `env` set.
pub(crate) fn larry_route(
    state: &Path,
    env: &[(&str, &OsStr)],
    args: &[&str],
) -> Result<Output, Box<dyn Error>> {
    Ok(std::process::Command::new(env!("CARGO_BIN_EXE_larry"))
        .arg("route")
        .args(args)
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .env("LARRY_STATE_DIR", state)
        .envs(env.iter().copied())
        .output()?)
}

/// A briefed task over `origin`: write `note.txt`, then prove it, over the
/// route `route` and with the idempotency key `key` when given.
pub(crate) fn briefed_task(
    origin: &TempDir,
    route: Option<&str>,
    key: Option<&str>,
) -> Result<String, Box<dyn Error>> {
    let mut task = json!({
        "repo": origin.path().display().to_string(),
        "base": "HEAD",
        "goal": "write the note",
        "files": ["note.txt"],
        "acceptance": [["true"]],
    });
    let object = task.as_object_mut().ok_or("a task object")?;
    if let Some(id) = route {
        object.insert(String::from("route"), json!(id));
    }
    if let Some(text) = key {
        object.insert(String::from("idempotency_key"), json!(text));
    }
    Ok(task.to_string())
}
