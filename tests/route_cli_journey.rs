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
#[path = "support/larry_cli.rs"]
mod larry_cli;
#[path = "support/mock_model.rs"]
mod mock_model;
#[path = "support/origin.rs"]
mod origin;

use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use larry_cli::{denial_lines, flag_of, report_of, spawn_larry, stderr_of};
use lgwks_std::json::{self, Value, json};
use mock_model::{Config, Fault, Handle};
use origin::{TempDir, commit_all, fixture};

/// What every journey returns.
type TestResult = Result<(), Box<dyn Error>>;

/// The scopes a briefed task needs, so no journey depends on the default policy.
const POLICY: &str = "file.read,file.write,process.spawn,model.invoke";

/// Runs `larry route <args>` with only PATH, the state directory and `env` set.
fn larry_route(
    state: &Path,
    env: &[(&str, &OsStr)],
    args: &[&str],
) -> Result<Output, Box<dyn Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_larry"))
        .arg("route")
        .args(args)
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .env("LARRY_STATE_DIR", state)
        .envs(env.iter().copied())
        .output()?)
}

/// The host and port a mock listens on, as a sealed report names it.
fn host_of(mock: &Handle) -> String {
    format!("127.0.0.1:{}", mock.addr().port())
}

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
    /// The variable its route reads the key from.
    key_var: &'static str,
    /// The key itself, which only the exec child's environment holds.
    key: &'static str,
}

impl<'tenant> Tenant<'tenant> {
    /// A Canadian tenant on `model-a` with the key `secret-a` in `KEY_A`.
    fn standard(state: &'tenant TempDir, origin: &'tenant TempDir, mock: &'tenant Handle) -> Self {
        Tenant {
            state,
            origin,
            mock,
            model: "model-a",
            residency: "CA",
            key_var: "KEY_A",
            key: "secret-a",
        }
    }

    /// Stores the route `id` pointing at this tenant's gateway, as the default.
    fn add_route(&self, id: &str) -> TestResult {
        let url = format!("http://{}/v1", host_of(self.mock));
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
                self.key_var,
                "--residency",
                self.residency,
                "--default",
            ],
        )?;
        assert!(output.status.success(), "route add: {}", stderr_of(&output));
        Ok(())
    }

    /// Runs `larry exec` on `task` with the tenant's key in the child's environment.
    fn exec(&self, task: &str) -> Result<Output, Box<dyn Error>> {
        self.exec_with(task, &[])
    }

    /// Runs `larry exec` on `task` with the extra variables `extra` as well.
    fn exec_with(&self, task: &str, extra: &[(&str, &OsStr)]) -> Result<Output, Box<dyn Error>> {
        let mut env: Vec<(&str, &OsStr)> = vec![
            ("LARRY_STATE_DIR", self.state.path().as_os_str()),
            ("LARRY_POLICY", OsStr::new(POLICY)),
            (self.key_var, OsStr::new(self.key)),
        ];
        env.extend_from_slice(extra);
        Ok(spawn_larry("exec", &env, task.as_bytes())?.wait_with_output()?)
    }

    /// A briefed task over this tenant's origin, naming `route` and `key` when given.
    fn briefed(&self, route: Option<&str>, key: Option<&str>) -> Result<String, Box<dyn Error>> {
        let mut task = json!({
            "repo": self.origin.path().display().to_string(),
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

    /// What a report must say about the route this tenant's task ran under.
    fn sealed_route(&self, id: &str) -> Value {
        json!({
            "id": id,
            "model": self.model,
            "host": host_of(self.mock),
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
                sent["model"].as_str(),
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
            report["authored"][0]["route"],
            self.sealed_route(id),
            "the receipt names this tenant's route, host and residency"
        );
        let printed = String::from_utf8_lossy(&output.stdout);
        for secret in [self.key, self.key_var] {
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
                key_var: "KEY_B",
                key: "secret-b",
                ..Tenant::standard(&state_b, &origin_b, mock_b)
            };
            one.add_route("main")?;
            two.add_route("main")?;
            let (task_one, task_two) = (one.briefed(None, None)?, two.briefed(None, None)?);
            let (out_one, out_two) = std::thread::scope(|scope| {
                let first = scope.spawn(|| one.exec(&task_one).map_err(|error| error.to_string()));
                let second = scope.spawn(|| two.exec(&task_two).map_err(|error| error.to_string()));
                (first.join(), second.join())
            });
            let out_one = out_one.map_err(|_panicked| "tenant one panicked")??;
            let out_two = out_two.map_err(|_panicked| "tenant two panicked")??;
            one.assert_sealed_under(&out_one, "main")?;
            two.assert_sealed_under(&out_two, "main")?;
            assert_ne!(
                host_of(mock_a),
                host_of(mock_b),
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
    let added = larry_route(
        state.path(),
        &[("ROUTE_KEY", secret)],
        &[
            "add",
            "main",
            "--url",
            "http://gateway.test:8443/v1",
            "--model",
            "model-a",
            "--key-env",
            "ROUTE_KEY",
            "--residency",
            "CA",
            "--describe",
            "cheap drafts",
            "--default",
        ],
    )?;
    assert!(added.status.success(), "route add: {}", stderr_of(&added));
    let listed = larry_route(state.path(), &[], &["list", "--json"])?;
    let value: Value = json::from_slice(&listed.stdout)?;
    assert_eq!(
        value["default"].as_str(),
        Some("main"),
        "main is the default"
    );
    assert_eq!(
        value["routes"][0]["host"].as_str(),
        Some("gateway.test:8443"),
        "the listing shows the gateway's host and port"
    );
    assert_eq!(
        value["routes"][0]["key_env"].as_str(),
        Some("ROUTE_KEY"),
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
            key_var: "ROUTE_KEY_UNSET",
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
            stderr_of(&output).contains("ROUTE_KEY_UNSET"),
            "the refusal names the variable: {}",
            stderr_of(&output)
        );
        assert_eq!(mock.served(), 0, "nothing reached the gateway");
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
