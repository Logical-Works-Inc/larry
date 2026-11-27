//! Journeys of the per-tenant route table (larry#201) as an operator lives them:
//! `larry route add` stores where a tenant's model calls go, `larry exec` sends
//! a briefed task there, and the sealed report names the route, the gateway's
//! host and the residency it ran under. Every run drives the shipped binary with
//! a cleared environment against the seeded mock model endpoint, so no test
//! makes a paid model call.
//!
//! Source: the Director's 2026-11-14 ruling that routing is a semantic feature
//! of the end user, not a compiled-in choice; the product rule that customer
//! data never takes a path to a host outside the tenant's residency, so the
//! residency a run used is sealed in its receipt; RFC 6750 section 2.1 (the
//! bearer key travels only in the `Authorization` header); POSIX exit status.
//! Requirement proved: two tenants running at once each reach their own gateway
//! with their own key and each receipt names its own route; an unknown route, no
//! route and an unset key variable send nothing; a key is never stored; a repo
//! file cannot redirect a tenant; a keyed retry answers with the route it sealed.

#[path = "support/http_wire.rs"]
mod http_wire;
#[path = "support/json_at.rs"]
mod json_at;
#[path = "support/larry_cli.rs"]
mod larry_cli;
#[path = "support/mock_model.rs"]
mod mock_model;
#[path = "support/origin.rs"]
mod origin;
#[path = "support/route_key.rs"]
mod route_key;
#[path = "support/sealed_report.rs"]
mod sealed_report;

use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{Child, Output};

use json_at::At;
use larry_cli::stderr_of;
use lgwks_deps::storefront::rusqlite::Connection;
use lgwks_std::json::{self, Value, json};
use mock_model::{Config, Fault, Handle};
use origin::{TempDir, commit_all, fixture};
use sealed_report::{denial_lines, flag_of, report_of};

#[path = "support/route_journey.rs"]
mod route_journey;
use route_journey::{NewRoute, TestResult, exec_with_key, larry_route, start_with_key};

/// One tenant: a state directory, an origin repository, a gateway and a key.
struct Tenant<'tenant> {
    /// The tenant's state directory.
    state: &'tenant TempDir,
    /// The origin repository its tasks clone.
    origin: &'tenant TempDir,
    /// The gateway its route points at.
    mock: &'tenant Handle,
    /// The model its route names.
    model: &'static str,
    /// The residency its route declares.
    residency: &'static str,
    /// The suffix of the key variable its route names, under its own tenant tag.
    suffix: &'static str,
    /// The key itself, which only the exec child's environment holds.
    key: &'static str,
}

impl<'tenant> Tenant<'tenant> {
    /// A Canadian tenant on `model-a` with the key `secret-a` in its `_A` variable.
    fn standard(state: &'tenant TempDir, origin: &'tenant TempDir, mock: &'tenant Handle) -> Self {
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
    fn add_route(&self, id: &str) -> TestResult {
        let url = format!("http://{}/v1", route_journey::host_of(self.mock));
        let key_env = route_key::key_env(self.state.path(), self.suffix)?;
        route_journey::add_route(
            self.state,
            &NewRoute {
                url: &url,
                id,
                model: self.model,
                key_env: &key_env,
                residency: self.residency,
                kind: None,
                temperature: None,
            },
        )
    }

    /// Runs `larry exec` on `task` with the tenant's key in the child's environment.
    fn exec(&self, task: &str) -> Result<Output, Box<dyn Error>> {
        self.exec_with(task, &[])
    }

    /// Runs `larry exec` on `task` with the extra variables `extra` as well.
    fn exec_with(&self, task: &str, extra: &[(&str, &OsStr)]) -> Result<Output, Box<dyn Error>> {
        let variable = route_key::key_env(self.state.path(), self.suffix)?;
        exec_with_key(
            self.state.path(),
            &variable,
            OsStr::new(self.key),
            task,
            extra,
        )
    }

    /// Starts `larry exec` on `task` with the extra variables `extra` as well, unwaited.
    fn start(&self, task: &str, extra: &[(&str, &OsStr)]) -> Result<Child, Box<dyn Error>> {
        let variable = route_key::key_env(self.state.path(), self.suffix)?;
        start_with_key(
            self.state.path(),
            &variable,
            OsStr::new(self.key),
            task,
            extra,
        )
    }

    /// A briefed task over this tenant's origin, naming `route` and `key` when given.
    fn briefed(&self, route: Option<&str>, key: Option<&str>) -> Result<String, Box<dyn Error>> {
        route_journey::briefed_task(self.origin, route, key)
    }

    /// What a report must say about the route this tenant's task ran under.
    fn sealed_route(&self, id: &str) -> Value {
        json!({
            "id": id,
            "kind": "openai-chat",
            "model": self.model,
            "host": route_journey::host_of(self.mock),
            "residency": self.residency,
        })
    }

    /// Asserts the gateway saw this tenant's model and key, and only those.
    fn assert_gateway_saw_only_this_tenant(&self) -> TestResult {
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
    fn assert_sealed_under(&self, output: &Output, id: &str) -> TestResult {
        let report = report_of(output)?;
        assert!(flag_of(&report, "accepted")?, "{}", stderr_of(output));
        assert!(denial_lines(&report)?.is_empty(), "no denial was sealed");
        assert_eq!(
            *report.at("/authored/0/route")?,
            self.sealed_route(id),
            "the receipt names this tenant's route, host and residency"
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

/// Two tenants who both call their route `main` run at once. Each report's
/// authored record names the route id, its own model, its own gateway's host and
/// its own residency; each gateway saw only its own tenant's model and key; no
/// report carries a key or the name of a key variable.
#[test]
fn two_tenants_at_once_each_receipt_names_its_own_route_and_host() -> TestResult {
    let (state_a, origin_a) = fixture()?;
    let (state_b, origin_b) = fixture()?;
    mock_model::run(Config::seeded(1), |mock_a| {
        mock_model::run(Config::seeded(2), |mock_b| -> TestResult {
            let one = Tenant::standard(&state_a, &origin_a, mock_a);
            let two = Tenant {
                model: "model-b",
                residency: "DE",
                suffix: "B",
                key: "secret-b",
                ..Tenant::standard(&state_b, &origin_b, mock_b)
            };
            one.add_route("main")?;
            two.add_route("main")?;
            let (task_one, task_two) = (one.briefed(None, None)?, two.briefed(None, None)?);
            let (child_one, child_two) = (one.start(&task_one, &[])?, two.start(&task_two, &[])?);
            let (out_one, out_two) = std::thread::scope(|scope| {
                let first = scope.spawn(move || child_one.wait_with_output());
                let second = scope.spawn(move || child_two.wait_with_output());
                (first.join(), second.join())
            });
            let Ok(waited_one) = out_one else {
                return Err("tenant one panicked".into());
            };
            let Ok(waited_two) = out_two else {
                return Err("tenant two panicked".into());
            };
            let (out_one, out_two) = (waited_one?, waited_two?);
            one.assert_sealed_under(&out_one, "main")?;
            two.assert_sealed_under(&out_two, "main")?;
            assert_ne!(
                route_journey::host_of(mock_a),
                route_journey::host_of(mock_b),
                "the two tenants reached two different gateways"
            );
            Ok(())
        })
    })
}

/// A task naming a route the tenant does not have is refused before any byte
/// leaves the host, with the id named, even though the tenant has a default: a
/// typo never falls through to it.
#[test]
fn an_unknown_route_is_refused_and_the_mock_counts_zero_requests() -> TestResult {
    let (state, origin) = fixture()?;
    mock_model::run(Config::seeded(3), |mock| -> TestResult {
        let tenant = Tenant::standard(&state, &origin, mock);
        tenant.add_route("main")?;
        let output = tenant.exec(&tenant.briefed(Some("typo"), None)?)?;
        assert!(!output.status.success(), "an unknown route fails the task");
        assert!(
            stderr_of(&output).contains("no route has the id typo"),
            "the refusal names the id: {}",
            stderr_of(&output)
        );
        assert_eq!(mock.served(), 0, "nothing reached the gateway");
        Ok(())
    })
}

/// With no stored route and no `LARRY_MODEL_*` environment, a briefed task is
/// refused naming `larry route add` and sends nothing, while a task with no
/// brief never reaches a model and runs. Bug caught: a compiled-in default
/// gateway that quietly takes the prompt.
#[test]
fn no_route_refuses_a_brief_task_and_runs_a_task_without_one() -> TestResult {
    let (state, origin) = fixture()?;
    mock_model::run(Config::seeded(4), |mock| -> TestResult {
        let tenant = Tenant::standard(&state, &origin, mock);
        let refused = tenant.exec(&tenant.briefed(None, None)?)?;
        assert!(
            !refused.status.success(),
            "a briefed task with no route fails"
        );
        assert!(
            stderr_of(&refused).contains("larry route add"),
            "the refusal says how to add a route: {}",
            stderr_of(&refused)
        );
        assert_eq!(mock.served(), 0, "nothing reached any gateway");
        let plain = json!({
            "repo": origin.path().display().to_string(),
            "base": "HEAD",
            "acceptance": [["true"]],
        })
        .to_string();
        let ran = tenant.exec(&plain)?;
        assert!(
            flag_of(&report_of(&ran)?, "accepted")?,
            "a task with no brief needs no route: {}",
            stderr_of(&ran)
        );
        Ok(())
    })
}

/// Whether `haystack` holds `needle` as a run of bytes.
fn holds(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// A route is added, listed, resolved and removed through the binary; the key
/// sits in the child's environment while the route is added, and no file in the
/// state directory holds a byte of it. Bug caught: a route store that keeps the
/// key it was given a name for.
#[test]
fn route_add_list_remove_round_trips_and_no_key_bytes_appear() -> TestResult {
    let state = TempDir::new()?;
    let secret = OsStr::new("topsecret-key-bytes");
    let key_env = route_key::key_env(state.path(), "ROUTE")?;
    let added = larry_route(
        state.path(),
        &[(&key_env, secret)],
        &[
            "add",
            "main",
            "--url",
            "https://gateway.test:8443/v1",
            "--model",
            "model-a",
            "--key-env",
            &key_env,
            "--residency",
            "CA",
            "--describe",
            "cheap drafts",
            "--default",
        ],
    )?;
    assert!(added.status.success(), "route add: {}", stderr_of(&added));
    let saved: Value = json::from_slice(&added.stdout)?;
    assert_eq!(
        saved.at("/key_env")?.as_str(),
        Some(key_env.as_str()),
        "route add prints the exact variable name to export"
    );
    assert_eq!(saved.at("/saved")?, &json!(true), "the add reports saved");
    let listed = larry_route(state.path(), &[], &["list", "--json"])?;
    let value: Value = json::from_slice(&listed.stdout)?;
    assert_eq!(
        value.at("/default")?.as_str(),
        Some("main"),
        "main is the default"
    );
    assert_eq!(
        value.at("/routes/0/host")?.as_str(),
        Some("gateway.test:8443"),
        "the listing shows the gateway's host and port"
    );
    assert_eq!(
        value.at("/routes/0/key_env")?.as_str(),
        Some(key_env.as_str()),
        "the listing names the key's variable, not its value"
    );
    let text = larry_route(state.path(), &[], &["list"])?;
    assert!(
        String::from_utf8_lossy(&text.stdout).contains("\tCA\tdefault"),
        "the plain listing marks the default and its residency"
    );
    let mut files = 0_u32;
    for entry in fs::read_dir(state.path())? {
        let path = entry?.path();
        if path.is_file() {
            files = files.saturating_add(1);
            assert!(
                !holds(&fs::read(&path)?, secret.as_encoded_bytes()),
                "{} must hold no byte of the key",
                path.display()
            );
        }
    }
    assert!(
        files > 0,
        "the store is on disk, so the scan looked at something"
    );
    for output in [&listed, &text] {
        assert!(
            !holds(&output.stdout, secret.as_encoded_bytes()),
            "no listing shows the key"
        );
    }
    let removed = larry_route(state.path(), &[], &["remove", "main"])?;
    assert!(
        removed.status.success(),
        "route remove: {}",
        stderr_of(&removed)
    );
    let after = larry_route(state.path(), &[], &["resolve"])?;
    assert!(
        !after.status.success(),
        "with the route gone nothing resolves"
    );
    assert!(
        stderr_of(&after).contains("no model route"),
        "the refusal says there is no route: {}",
        stderr_of(&after)
    );
    Ok(())
}

/// A keyed task is run, the tenant then makes another route its default, and the
/// same document is sent again as a fresh process after a restart. The retry
/// answers with the bytes the first run sealed, naming the first route, and the
/// gateway serves nothing more.
#[test]
fn a_keyed_retry_after_restart_seals_the_same_route() -> TestResult {
    let (state, origin) = fixture()?;
    mock_model::run(Config::seeded(5), |mock| -> TestResult {
        let tenant = Tenant::standard(&state, &origin, mock);
        tenant.add_route("main")?;
        let task = tenant.briefed(None, Some("retry-me"))?;
        let first = tenant.exec(&task)?;
        tenant.assert_sealed_under(&first, "main")?;
        let served = mock.served();
        tenant.add_route("other")?;
        let retry = tenant.exec(&task)?;
        assert_eq!(
            retry.stdout, first.stdout,
            "the retry answers with the sealed bytes of the first run"
        );
        assert_eq!(mock.served(), served, "the retry reached no gateway");
        Ok(())
    })
}

/// A repository can carry any file it likes; none of them is a route. The origin
/// gains a commit holding decoy route files that point at a dead address, and the
/// task, run from an empty state directory with an empty environment, still goes
/// to the tenant's stored route.
#[test]
fn a_route_runs_from_a_clean_directory() -> TestResult {
    let (state, origin) = fixture()?;
    fs::write(
        origin.path().join("routes.toml"),
        "url = \"http://127.0.0.1:1/v1\"\nmodel = \"decoy\"\n",
    )?;
    fs::write(
        origin.path().join(".env"),
        "LARRY_MODEL_URL=http://127.0.0.1:1/v1\nLARRY_MODEL=decoy\nLARRY_MODEL_KEY=decoy\n",
    )?;
    commit_all(origin.path(), "decoy route files")?;
    mock_model::run(Config::seeded(6), |mock| -> TestResult {
        let tenant = Tenant::standard(&state, &origin, mock);
        tenant.add_route("main")?;
        let output = tenant.exec(&tenant.briefed(None, None)?)?;
        tenant.assert_sealed_under(&output, "main")
    })
}

/// A route names a key variable the child does not have: the task is refused
/// naming the variable (never a value) and sends nothing.
#[test]
fn a_route_whose_key_variable_is_unset_is_refused_naming_it() -> TestResult {
    let (state, origin) = fixture()?;
    mock_model::run(Config::seeded(7), |mock| -> TestResult {
        let tenant = Tenant {
            suffix: "UNSET",
            key: "",
            ..Tenant::standard(&state, &origin, mock)
        };
        tenant.add_route("main")?;
        let output = tenant.exec(&tenant.briefed(None, None)?)?;
        assert!(
            !output.status.success(),
            "an unset key variable fails the task"
        );
        assert!(
            stderr_of(&output).contains(&route_key::key_env(state.path(), "UNSET")?),
            "the refusal names the variable: {}",
            stderr_of(&output)
        );
        assert_eq!(mock.served(), 0, "nothing reached the gateway");
        Ok(())
    })
}

/// A tenant's route is data, so it cannot make the executor send a secret the
/// operator never set aside for routes. `larry route add` naming the operator's
/// `LARRY_MODEL_KEY`, a cloud token or the bare prefix is refused (exit 1) naming
/// the `LARRY_KEY_` rule and never the value, nothing is stored, and a task
/// naming that route afterwards is refused with the operator's secrets in the
/// child's environment: the gateway counts zero requests and sees no
/// `Authorization` header. Bug caught: a stored `--key-env GH_TOKEN` whose value
/// is sent as the bearer token to the tenant's chosen host (CWE-200).
#[test]
fn a_stored_route_cannot_name_a_variable_outside_its_namespace() -> TestResult {
    let (state, origin) = fixture()?;
    mock_model::run(Config::seeded(9), |mock| -> TestResult {
        let tenant = Tenant::standard(&state, &origin, mock);
        let url = format!("http://{}/v1", route_journey::host_of(mock));
        let secret = OsStr::new("operator-secret-bytes");
        for variable in ["LARRY_MODEL_KEY", "GH_TOKEN", "LARRY_KEY_"] {
            let refused = larry_route(
                state.path(),
                &[(variable, secret)],
                &[
                    "add",
                    "x",
                    "--url",
                    &url,
                    "--model",
                    "model-a",
                    "--key-env",
                    variable,
                    "--residency",
                    "CA",
                    "--default",
                ],
            )?;
            assert_eq!(
                refused.status.code(),
                Some(1),
                "{variable} is refused: {}",
                stderr_of(&refused)
            );
            assert!(
                stderr_of(&refused).contains("must start with LARRY_KEY_"),
                "{variable}: the refusal names the namespace: {}",
                stderr_of(&refused)
            );
            assert!(
                !holds(stderr_of(&refused).as_bytes(), secret.as_encoded_bytes()),
                "{variable}: the refusal shows no value"
            );
        }
        let stored = larry_route(state.path(), &[], &["list"])?;
        assert!(stored.stdout.is_empty(), "no refused route was stored");
        let task = tenant.briefed(Some("x"), None)?;
        let output = tenant.exec_with(
            &task,
            &[
                ("LARRY_MODEL_KEY", secret),
                ("GH_TOKEN", secret),
                ("LARRY_KEY_", secret),
            ],
        )?;
        assert!(!output.status.success(), "a route nobody stored fails");
        assert!(
            stderr_of(&output).contains("no route has the id x"),
            "the task names no stored route: {}",
            stderr_of(&output)
        );
        assert_eq!(mock.served(), 0, "nothing reached the gateway");
        assert!(
            mock.authorizations().is_empty(),
            "no bearer header left the host"
        );
        Ok(())
    })
}

/// A tenant's route may name only its own `LARRY_KEY_<TAG>_*` variable: A's add
/// naming B's variable is refused typed and stores nothing, and a task naming it
/// sends nothing even with B's key in the child's environment (CWE-200, larry#328).
#[test]
fn a_route_cannot_name_another_tenants_key_variable() -> TestResult {
    let (state_a, origin_a) = fixture()?;
    let state_b = TempDir::new()?;
    mock_model::run(Config::seeded(10), |mock| -> TestResult {
        let tenant = Tenant::standard(&state_a, &origin_a, mock);
        let theirs = route_key::key_env(state_b.path(), "A")?;
        let url = format!("http://{}/v1", route_journey::host_of(mock));
        let mut args = vec!["add", "steal", "--url", url.as_str()];
        args.extend(["--model", "model-a", "--key-env", theirs.as_str()]);
        args.extend(["--residency", "CA", "--default"]);
        let refused = larry_route(state_a.path(), &[], &args)?;
        let named = stderr_of(&refused);
        assert_eq!(refused.status.code(), Some(1), "refused: {named}");
        assert!(named.contains(&theirs), "the refusal names it: {named}");
        assert!(named.contains("must start with LARRY_KEY_"), "{named}");
        let stored = larry_route(state_a.path(), &[], &["list"])?;
        assert!(stored.stdout.is_empty(), "nothing was stored");
        let secret = OsStr::new("tenant-bs-secret-bytes");
        let task = tenant.briefed(Some("steal"), None)?;
        let output = tenant.exec_with(&task, &[(theirs.as_str(), secret)])?;
        assert!(!output.status.success(), "a route nobody stored fails");
        assert_eq!(mock.served(), 0, "nothing reached the gateway");
        assert!(mock.authorizations().is_empty(), "no header left the host");
        let shown = stderr_of(&output);
        assert!(!holds(shown.as_bytes(), secret.as_encoded_bytes()));
        Ok(())
    })
}

/// Rewrites every route row's key variable in `state`'s store, the way a row
/// stored before the tenant binding, or a saboteur with the file, leaves it.
fn set_key_env(state: &Path, name: &str) -> Result<(), Box<dyn Error>> {
    Connection::open(state.join("executor.db"))?
        .execute("UPDATE routes SET key_env = ?1", [name])?;
    Ok(())
}

/// A route stored before the tenant binding names a variable no tenant tag
/// bounds: `larry route resolve` refuses it typed before the variable is read,
/// and a task that would use it sends nothing (larry#328).
#[test]
fn a_route_stored_before_the_binding_is_refused_at_resolve() -> TestResult {
    let (state, origin) = fixture()?;
    mock_model::run(Config::seeded(11), |mock| -> TestResult {
        let tenant = Tenant::standard(&state, &origin, mock);
        tenant.add_route("main")?;
        set_key_env(state.path(), "LARRY_KEY_LEGACY")?;
        let secret = OsStr::new("legacy-secret-bytes");
        let resolved = larry_route(state.path(), &[("LARRY_KEY_LEGACY", secret)], &["resolve"])?;
        let named = stderr_of(&resolved);
        assert_eq!(resolved.status.code(), Some(1), "refused: {named}");
        assert!(named.contains("LARRY_KEY_LEGACY"), "{named}");
        let task = tenant.briefed(None, None)?;
        let output = tenant.exec_with(&task, &[("LARRY_KEY_LEGACY", secret)])?;
        assert!(!output.status.success(), "a task over it fails");
        for shown in [&named, &stderr_of(&output)] {
            assert!(!holds(shown.as_bytes(), secret.as_encoded_bytes()));
        }
        assert_eq!(mock.served(), 0, "nothing reached the gateway");
        assert!(mock.authorizations().is_empty(), "no header left the host");
        Ok(())
    })
}

/// A briefed task over a fresh tenant whose gateway answers its first request
/// with `fault`; the exec child also gets `extra`. Returns its output and how
/// many requests the gateway served.
fn under_fault(fault: Fault, extra: &[(&str, &OsStr)]) -> Result<(Output, u64), Box<dyn Error>> {
    let (state, origin) = fixture()?;
    let config = Config {
        script: vec![Some(fault)],
        ..Config::seeded(8)
    };
    mock_model::run(config, |mock| {
        let tenant = Tenant::standard(&state, &origin, mock);
        tenant.add_route("main")?;
        let output = tenant.exec_with(&tenant.briefed(None, None)?, extra)?;
        mock.release_delayed();
        Ok((output, mock.served()))
    })
}

/// The model settings an operator exports still apply to a route's calls, and a
/// gateway's fault ends the task in a refusal sent once: a 401 whose message
/// carries escape bytes prints none of them, a reply slower than
/// `LARRY_MODEL_TIMEOUT_MS` times out and is not resent, and a reply padded the
/// way OpenRouter pads it is read.
#[test]
fn a_route_keeps_the_model_settings_and_refuses_each_gateway_fault_once() -> TestResult {
    let (hostile, served) = under_fault(Fault::HostileRefusal { status: 401 }, &[])?;
    assert!(!hostile.status.success(), "a 401 fails the task");
    assert_eq!(served, 1, "a 401 is never retried");
    assert!(
        !stderr_of(&hostile).trim_end().chars().any(char::is_control),
        "no escape or bell reaches the terminal"
    );
    let timeout = OsStr::new("200");
    let (slow, served) = under_fault(Fault::Delay, &[("LARRY_MODEL_TIMEOUT_MS", timeout)])?;
    assert!(
        !slow.status.success(),
        "a gateway slower than the timeout fails"
    );
    assert_eq!(served, 1, "a timeout is never resent");
    let (padded, _) = under_fault(Fault::PaddedDone, &[])?;
    assert!(
        flag_of(&report_of(&padded)?, "accepted")?,
        "a padded reply is read: {}",
        stderr_of(&padded)
    );
    // larry#396: a reply cut at the token limit, or one with no content, refuses the task
    // naming why, and is never resent.
    for (fault, why) in [
        (Fault::Finish { reason: "length" }, "finish_reason length"),
        (Fault::EmptyContent, "reply is empty"),
    ] {
        let (cut, served) = under_fault(fault, &[])?;
        assert!(!cut.status.success(), "{fault:?} fails the task");
        assert!(
            stderr_of(&cut).contains(why),
            "{fault:?}: {}",
            stderr_of(&cut)
        );
        assert_eq!(served, 1, "{fault:?} is never resent");
    }
    Ok(())
}

/// One seeded run over the stored route `main`, and the gateway's trace hash.
fn traced(state: &TempDir, origin: &TempDir, seed: u64) -> Result<String, Box<dyn Error>> {
    mock_model::run(Config::seeded(seed), |mock| {
        let tenant = Tenant::standard(state, origin, mock);
        tenant.add_route("main")?;
        tenant.assert_sealed_under(&tenant.exec(&tenant.briefed(None, None)?)?, "main")?;
        Ok(mock.trace_hash())
    })
}

/// Deterministic: the same seed over the same tenant gives the gateway the same
/// requests and so one trace hash, and another seed gives another.
#[test]
fn the_same_seed_over_a_route_replays_one_trace() -> TestResult {
    let (state, origin) = fixture()?;
    for seed in 0..4_u64 {
        let first = traced(&state, &origin, seed)?;
        assert_eq!(
            first,
            traced(&state, &origin, seed)?,
            "seed {seed}: one seed, one trace"
        );
        assert_ne!(
            first,
            traced(&state, &origin, seed.saturating_add(100))?,
            "seed {seed}: another seed, another trace"
        );
    }
    Ok(())
}
