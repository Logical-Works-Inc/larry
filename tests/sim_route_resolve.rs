//! Seeded adversarial simulations of choosing a route (larry#201), driven
//! through `larry route`'s own entry point and the task parser, the code the
//! shipped binary calls.
//!
//! Source: the route table design of larry#201 (named route, else the tenant
//! default, else the operator's environment route, else a typed refusal).
//! Requirement proved: the resolver matches an independent reference for every
//! generated table, a name nothing has is never swapped for the default, a
//! stored route answers even when the environment is poisoned, two tenants on
//! one route id never read each other's, and a hostile route name in a task
//! document is a typed refusal before any store or gateway is asked. Every
//! actor is a fool: a neighbour, a typo, a saboteur who writes escape bytes.

#[path = "support/route_cli.rs"]
mod route_cli;

use std::sync::Barrier;

use larry_demo::executor::routes::RouteCliError;
use larry_demo::executor::{Task, TaskError};
use larry_demo::model::RouteError;
use lgwks_std::json::{Value, json};
use lgwks_std::seeded::Seeded;
use route_cli::{GATEWAY, POOL, Scratch, TestResult, add, joined, listed, parse, route_run};

/// The id a task naming `named` resolves to by the written rule, from the
/// table's `included` routes, its `default` and whether the environment
/// declares a route; `Err` carries the expected refusal.
fn expected_route(
    included: &[&str],
    default: Option<&str>,
    env_declared: bool,
    named: Option<&str>,
) -> Result<String, RouteError> {
    match named {
        Some(id) if included.contains(&id) => Ok(id.to_owned()),
        Some("env") if env_declared => Ok(String::from("env")),
        Some(id) => Err(RouteError::Unknown { id: id.to_owned() }),
        None => match default {
            Some(id) => Ok(id.to_owned()),
            None if env_declared => Ok(String::from("env")),
            None => Err(RouteError::NoRoute),
        },
    }
}

/// Generalized: for 64 seeded tables (0 to 6 routes, a default or not, an
/// environment route or not) every name, the unknown ones and `env` among
/// them, resolves exactly as the written order says: named, else default,
/// else environment, else `NoRoute`, and a name nothing has never falls
/// through to the default. Bug caught: a typo that silently uses the default
/// and sends a prompt where the tenant did not choose.
#[test]
fn sim_resolver_order_holds_for_every_generated_table() -> TestResult {
    for seed in 0..64_u64 {
        let mut rng = Seeded::from_seed(seed);
        let state = Scratch::new()?;
        let mut included: Vec<&str> = Vec::new();
        for id in POOL {
            if rng.below(2)? == 1 {
                included.push(id);
            }
        }
        let default = if included.is_empty() || rng.below(2)? == 0 {
            None
        } else {
            let pick = usize::try_from(rng.below(u64::try_from(included.len())?)?)?;
            included.get(pick).copied()
        };
        let env_declared = rng.below(2)? == 1;
        for id in &included {
            let model = format!("model-{id}");
            add(state.path(), id, &[("--model", model.as_str())], &[])?;
        }
        if let Some(id) = default {
            route_run(state.path(), &[], &["default", id])?;
        }
        let env: Vec<(&str, &str)> = if env_declared {
            vec![
                ("LARRY_MODEL_URL", GATEWAY),
                ("LARRY_MODEL", "env-model"),
                ("LARRY_MODEL_KEY", "env-key"),
            ]
        } else {
            Vec::new()
        };
        let mut names: Vec<Option<&str>> = vec![None, Some("env"), Some("nope")];
        names.extend(POOL.iter().map(|id| Some(*id)));
        for named in names {
            let args: Vec<&str> = match named {
                Some(id) => vec!["resolve", id],
                None => vec!["resolve"],
            };
            let got = route_run(state.path(), &env, &args);
            match (got, expected_route(&included, default, env_declared, named)) {
                (Ok(text), Ok(id)) => assert_eq!(
                    parse(&text)?["id"].as_str(),
                    Some(id.as_str()),
                    "seed {seed} named {named:?}"
                ),
                (Err(RouteCliError::Route(ref got)), Err(ref want)) => {
                    assert_eq!(got, want, "seed {seed} named {named:?}");
                }
                (got, want) => {
                    return Err(format!(
                        "seed {seed} named {named:?}: got {got:?}, wanted {want:?}"
                    )
                    .into());
                }
            }
        }
    }
    Ok(())
}

/// A stored route answers even when the environment is poisoned, because the
/// environment is parsed only when no stored route does: a typo in a model
/// knob fails nothing that never uses it, and fails the task that does.
#[test]
fn sim_a_poisoned_environment_fails_only_what_needs_it() -> TestResult {
    let state = Scratch::new()?;
    add(state.path(), "alpha", &[], &["--default"])?;
    let poisoned = [
        ("LARRY_MODEL_URL", GATEWAY),
        ("LARRY_MODEL", "env-model"),
        ("LARRY_MODEL_KEY", "k"),
        ("LARRY_MODEL_TEMPERATURE", "hot"),
    ];
    let text = route_run(state.path(), &poisoned, &["resolve"])?;
    assert_eq!(
        parse(&text)?["id"].as_str(),
        Some("alpha"),
        "the stored default answers"
    );
    let other = Scratch::new()?;
    assert!(
        matches!(
            route_run(other.path(), &poisoned, &["resolve"]),
            Err(RouteCliError::Exec(_))
        ),
        "with no stored route the poisoned environment is the refusal"
    );
    Ok(())
}

/// What a tenant's route must resolve to: its model, host and residency.
type Truth = (&'static str, &'static str, &'static str);

/// One tenant's cases: the name asked for (empty for none) and the truth, or
/// `None` when the name must be unknown to this tenant.
type Cases = [(&'static str, Option<Truth>); 4];

/// Multi-tenant, the neighbour: two state directories each hold a route
/// `shared` with different models, hosts and residencies, plus one route of
/// their own. Two threads hammer both at once for 64 seeded steps each; every
/// answer is the asker's own, the other tenant's private name is unknown, and
/// removing `shared` in one leaves the other's. Bug caught: a store or a cache
/// keyed by route id alone.
#[test]
fn sim_two_tenants_resolve_only_their_own_routes() -> TestResult {
    let (one, two) = (Scratch::new()?, Scratch::new()?);
    let shared = |model: &'static str, url: &'static str, residency: &'static str| {
        [
            ("--model", model),
            ("--url", url),
            ("--residency", residency),
        ]
    };
    add(
        one.path(),
        "shared",
        &shared("one-shared", "http://one.test:1/v1", "CA"),
        &["--default"],
    )?;
    add(one.path(), "only-one", &[("--model", "one-private")], &[])?;
    add(
        two.path(),
        "shared",
        &shared("two-shared", "http://two.test:2/v1", "DE"),
        &[],
    )?;
    add(
        two.path(),
        "only-two",
        &[("--model", "two-private")],
        &["--default"],
    )?;
    let truth_one: Cases = [
        ("shared", Some(("one-shared", "one.test:1", "CA"))),
        ("only-one", Some(("one-private", "gateway.test:8443", "CA"))),
        ("only-two", None),
        ("", Some(("one-shared", "one.test:1", "CA"))),
    ];
    let truth_two: Cases = [
        ("shared", Some(("two-shared", "two.test:2", "DE"))),
        ("only-two", Some(("two-private", "gateway.test:8443", "CA"))),
        ("only-one", None),
        ("", Some(("two-private", "gateway.test:8443", "CA"))),
    ];
    let barrier = Barrier::new(2);
    std::thread::scope(|scope| -> TestResult {
        let mut handles = Vec::new();
        for (seed, state, truth) in [
            (1_u64, one.path(), &truth_one),
            (2_u64, two.path(), &truth_two),
        ] {
            let barrier = &barrier;
            handles.push(scope.spawn(move || -> Result<(), String> {
                barrier.wait();
                let mut rng = Seeded::from_seed(seed);
                for step in 0..64_u32 {
                    let pick = rng.below(4).map_err(|error| error.to_string())?;
                    let index = usize::try_from(pick).map_err(|error| error.to_string())?;
                    let entry = truth.get(index).ok_or("a case")?;
                    let name = entry.0;
                    let args: Vec<&str> = if name.is_empty() {
                        vec!["resolve"]
                    } else {
                        vec!["resolve", name]
                    };
                    match (route_run(state, &[], &args), entry.1.as_ref()) {
                        (Ok(text), Some(expect)) => {
                            let value = parse(&text).map_err(|error| error.to_string())?;
                            let got = (
                                value["model"].as_str(),
                                value["host"].as_str(),
                                value["residency"].as_str(),
                            );
                            if got != (Some(expect.0), Some(expect.1), Some(expect.2)) {
                                return Err(format!("seed {seed} step {step} {name}: got {got:?}"));
                            }
                        }
                        (Err(RouteCliError::Route(RouteError::Unknown { .. })), None) => {}
                        (other, _) => {
                            return Err(format!("seed {seed} step {step} {name}: {other:?}"));
                        }
                    }
                }
                Ok(())
            }));
        }
        for handle in handles {
            joined(handle)??;
        }
        Ok(())
    })?;
    route_run(one.path(), &[], &["remove", "shared"])?;
    let still = parse(&route_run(two.path(), &[], &["resolve", "shared"])?)?;
    assert_eq!(
        still["model"].as_str(),
        Some("two-shared"),
        "tenant two keeps its shared route"
    );
    assert!(
        matches!(
            route_run(one.path(), &[], &["resolve", "shared"]),
            Err(RouteCliError::Route(RouteError::Unknown { .. }))
        ),
        "tenant one lost its own"
    );
    assert_eq!(
        listed(one.path())?.0,
        ["only-one"],
        "tenant one keeps only its private route"
    );
    Ok(())
}

/// A task document naming a route.
fn document(route: &Value) -> String {
    json!({"repo": "r", "base": "HEAD", "acceptance": [], "route": route}).to_string()
}

/// Secure, the saboteur: a route name in a task document is a route id or a
/// typed refusal, before any store or gateway is asked. Escape bytes, a path,
/// a NUL, a 65th byte and the empty name never parse; a non-string is a wrong
/// type; the 64-byte name and `env` do parse.
#[test]
fn sim_hostile_task_route_names_are_typed_refusals() -> TestResult {
    let long = "r".repeat(65);
    for name in [
        "Up",
        "",
        "a b",
        "../x",
        "a\u{0}b",
        "a\nb",
        "\u{1b}[31m",
        "caf\u{e9}",
        "a.b",
        long.as_str(),
    ] {
        let parsed = Task::parse(document(&json!(name)).as_bytes());
        assert!(
            matches!(parsed, Err(TaskError::BadRoute)),
            "{name:?} must be BadRoute, got {parsed:?}"
        );
    }
    for value in [
        json!(7),
        json!(null),
        json!(["a"]),
        json!({"a": 1}),
        json!(true),
    ] {
        let parsed = Task::parse(document(&value).as_bytes());
        assert!(
            matches!(parsed, Err(TaskError::WrongType { key: "route" })),
            "{value} must be a wrong type, got {parsed:?}"
        );
    }
    let top = "r".repeat(64);
    for name in ["env", "a_b-9", top.as_str()] {
        Task::parse(document(&json!(name)).as_bytes())?;
    }
    Ok(())
}
