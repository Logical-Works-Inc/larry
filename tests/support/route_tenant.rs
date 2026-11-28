//! One tenant's route-journey harness (larry#201, larry#202): a state
//! directory, an origin repository, a gateway and a key, with the `larry
//! route` door, the `larry exec` door and the sealed-report assertions every
//! tenant journey drives. One home, so no journey copies it: both the
//! route-table journeys and the selection journeys run their tenants through
//! here (`route_cli_journey.rs`, `route_select_journey.rs`). The `larry
//! route` and `larry exec` doors it stands on live in the shared
//! `route_journey` harness, which the journeys without a tenant
//! (`provider_kinds_journey.rs`) use on its own. Test support, never linked
//! into a shipped binary (INVARIANTS: test doubles never ship).

use std::error::Error;
use std::ffi::OsStr;
use std::process::Output;

use crate::larry_guard::Larry;
use crate::route_journey::json_at::At;
use crate::route_journey::larry_cli::stderr_of;
use crate::route_journey::mock_model::Handle;
use crate::route_journey::origin::TempDir;
use crate::route_journey::route_key::key_env;
use crate::route_journey::sealed_report::{denial_lines, flag_of, report_of};
use crate::route_journey::{
    NewRoute, TestResult, add_route, briefed_task, exec_with_key, host_of, start_with_key,
};
use lgwks_std::json::{self, Value, json};

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
        let variable = key_env(self.state.path(), self.suffix)?;
        add_route(
            self.state,
            &NewRoute {
                url: &url,
                id,
                model: self.model,
                key_env: &variable,
                residency: self.residency,
                kind: None,
                prompt: None,
                temperature: None,
            },
        )
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
        let variable = key_env(self.state.path(), self.suffix)?;
        exec_with_key(
            self.state.path(),
            &variable,
            OsStr::new(self.key),
            task,
            extra,
        )
    }

    /// Starts `larry exec` on `task` with the extra variables `extra` as well, unwaited.
    pub(crate) fn start(
        &self,
        task: &str,
        extra: &[(&str, &OsStr)],
    ) -> Result<Larry, Box<dyn Error>> {
        let variable = key_env(self.state.path(), self.suffix)?;
        start_with_key(
            self.state.path(),
            &variable,
            OsStr::new(self.key),
            task,
            extra,
        )
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
            "kind": "openai-chat",
            "model": self.model,
            "host": host_of(self.mock),
            "residency": self.residency,
            "prompt_hash": null,
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
            let expected = Some(format!("Bearer {}", self.key));
            for header in self.mock.authorizations() {
                assert_eq!(
                    header, expected,
                    "only this tenant's key reached its gateway"
                );
            }
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
            "the receipt names this tenant's route, kind, host and residency"
        );
        assert_eq!(
            report.at("/authored/0/route/model")?.as_str(),
            Some(self.model),
            "the sealed route names the model the gateway served"
        );
        let printed = String::from_utf8_lossy(&output.stdout);
        let variable = key_env(self.state.path(), self.suffix)?;
        for secret in [self.key, variable.as_str()] {
            assert!(
                !printed.contains(secret),
                "{secret} must not appear in a report"
            );
        }
        self.assert_gateway_saw_only_this_tenant()
    }
}
