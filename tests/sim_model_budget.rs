//! Seeded model-budget journeys through the public model client and mock endpoint.

#[path = "support/http_wire.rs"]
mod http_wire;
#[path = "support/mock_model.rs"]
mod mock_model;

use std::error::Error;
use std::time::Duration;

use larry_demo::model::{Message, ModelClient, ModelConfig, ModelError, Role};
use lgwks_bot::kernel::clock::Clock;
use mock_model::Config;

type TestResult = Result<(), Box<dyn Error>>;
type Settings = Vec<(&'static str, String)>;

fn settings(base: &str) -> Settings {
    vec![
        ("LARRY_MODEL_URL", base.to_owned()),
        ("LARRY_MODEL", String::from("mock-model")),
        ("LARRY_MODEL_KEY", String::from("test-key")),
        ("LARRY_MODEL_TIMEOUT_MS", String::from("5000")),
        ("LARRY_MODEL_ATTEMPTS", String::from("3")),
        ("LARRY_MODEL_BACKOFF_MS", String::from("0")),
        ("LARRY_MODEL_DEADLINE_MS", String::from("60000")),
    ]
}

fn with(mut table: Settings, name: &'static str, value: &str) -> Settings {
    table.retain(|entry| entry.0 != name);
    table.push((name, value.to_owned()));
    table
}

fn client(table: Settings) -> Result<ModelClient, ModelError> {
    client_with(table, |_wait| true)
}

fn client_with(
    table: Settings,
    pause: impl Fn(Duration) -> bool + Send + Sync + 'static,
) -> Result<ModelClient, ModelError> {
    let config = ModelConfig::from_env(move |name| {
        table
            .iter()
            .find(|entry| entry.0 == name)
            .map(|entry| entry.1.clone())
    })?;
    Ok(ModelClient::new(
        config,
        Clock::virtual_at(Duration::ZERO),
        pause,
    ))
}

fn user(text: &str) -> Message {
    Message::new(Role::User, text)
}

fn base(addr: std::net::SocketAddr) -> String {
    format!("http://{addr}/v1")
}

#[test]
fn sim_tenant_budget_refuses_before_call() -> TestResult {
    mock_model::run(Config::seeded(0), |handle| {
        let outcome = client(with(
            settings(&base(handle.addr())),
            "LARRY_MODEL_TOKEN_BUDGET",
            "0",
        ))?
        .complete(&[user("over budget")]);
        assert!(
            matches!(
                outcome,
                Err(ModelError::BudgetExceeded {
                    tokens: 0,
                    limit: 0
                })
            ),
            "the zero token budget is the typed refusal, got {outcome:?}"
        );
        assert_eq!(handle.served(), 0, "the mock sees no request");
        assert!(
            handle.requests().is_empty(),
            "no request bytes reached the mock"
        );
        Ok::<_, Box<dyn Error>>(())
    })?;
    Ok(())
}

#[test]
fn sim_tenant_money_budget_refuses_before_call() -> TestResult {
    mock_model::run(Config::seeded(1), |handle| {
        let outcome = client(with(
            settings(&base(handle.addr())),
            "LARRY_MODEL_MICRO_DOLLAR_BUDGET",
            "0",
        ))?
        .complete(&[user("over money budget")]);
        assert!(
            matches!(
                outcome,
                Err(ModelError::MoneyBudgetExceeded {
                    micros: 0,
                    limit: 0
                })
            ),
            "the zero money budget is the typed refusal, got {outcome:?}"
        );
        assert_eq!(handle.served(), 0, "the mock sees no request");
        assert!(
            handle.requests().is_empty(),
            "no request bytes reached the mock"
        );
        Ok::<_, Box<dyn Error>>(())
    })?;
    Ok(())
}

/// Reported usage is priced in whole micro-dollars: at one million
/// micro-dollars per million tokens each way, a small completion costs its
/// token count, so two completions fit a one-dollar budget and both reach the
/// mock. A client counting picodollars as micro-dollars would refuse the
/// second call a million times too early.
#[test]
fn sim_reported_usage_is_priced_in_whole_micro_dollars() -> TestResult {
    mock_model::run(Config::seeded(2), |handle| {
        let priced = with(
            with(
                settings(&base(handle.addr())),
                "LARRY_MODEL_PRICE_IN_MICROS",
                "1000000",
            ),
            "LARRY_MODEL_PRICE_OUT_MICROS",
            "1000000",
        );
        let priced = with(priced, "LARRY_MODEL_MICRO_DOLLAR_BUDGET", "1000000");
        let shared = client(priced)?;
        let first = shared.complete(&[user("first")]);
        assert!(first.is_ok(), "the first call fits one dollar: {first:?}");
        let second = shared.complete(&[user("second")]);
        assert!(
            second.is_ok(),
            "usage under a dollar is not a million times it: {second:?}"
        );
        assert_eq!(handle.served(), 2, "both calls reach the mock");
        assert_eq!(
            handle.requests().len(),
            2,
            "the mock recorded both request bodies"
        );
        assert!(
            handle
                .authorizations()
                .iter()
                .all(|header| header.as_deref() == Some("Bearer test-key")),
            "both calls carry the client's key"
        );
        Ok::<_, Box<dyn Error>>(())
    })?;
    Ok(())
}

/// Only a reported completion debits the budget: a delayed-then-released
/// reply, a padded one and an empty one each record their usage, so the next
/// call on a one-token budget is refused before the gateway; a refused
/// gateway (hostile 401) and a cut reply (`length`) record nothing, so the
/// next call is still admitted and served clean. Catches a refusal that
/// spends and a success that does not.
#[test]
fn sim_only_reported_usage_debits_the_budget() -> TestResult {
    use mock_model::Fault;
    let cases: [(Fault, bool); 5] = [
        (Fault::Delay, true),
        (Fault::PaddedDone, true),
        (Fault::HostileRefusal { status: 401 }, false),
        (Fault::Finish { reason: "length" }, false),
        (Fault::EmptyContent, true),
    ];
    for (index, (fault, debited)) in cases.into_iter().enumerate() {
        let seed = u64::try_from(index)?.saturating_add(10);
        mock_model::run(
            Config {
                script: vec![Some(fault)],
                ..Config::seeded(seed)
            },
            |handle| {
                if matches!(fault, Fault::Delay) {
                    handle.release_delayed();
                }
                let table = with(
                    settings(&base(handle.addr())),
                    "LARRY_MODEL_TOKEN_BUDGET",
                    "1",
                );
                let shared = client(table)?;
                let first = shared.complete(&[user("budgeted")]);
                if debited {
                    assert!(first.is_ok(), "{fault:?} is served: {first:?}");
                } else {
                    assert!(first.is_err(), "{fault:?} is a refusal: {first:?}");
                }
                let second = shared.complete(&[user("again")]);
                if debited {
                    assert!(
                        matches!(second, Err(ModelError::BudgetExceeded { .. })),
                        "a recorded call refuses the next one: {second:?}"
                    );
                    assert_eq!(handle.served(), 1, "the refusal reaches no gateway");
                } else {
                    assert!(second.is_ok(), "a refusal debits nothing: {second:?}");
                    assert_eq!(handle.served(), 2, "both calls reach the mock");
                }
                assert_eq!(
                    u64::try_from(handle.requests().len())?,
                    handle.served(),
                    "the mock recorded every body it served"
                );
                assert!(
                    handle
                        .authorizations()
                        .iter()
                        .all(|header| header.as_deref() == Some("Bearer test-key")),
                    "every call carries the client's key"
                );
                Ok::<_, Box<dyn Error>>(())
            },
        )?;
    }
    Ok(())
}

/// One non-zero backoff is waited, then cancelled: the pause sees exactly the
/// configured wait, answers it `false`, and the gateway's refusal is the
/// answer after one served request. A client that slept on the wall or retried
/// past the cancel serves more than once.
#[test]
fn sim_a_non_zero_backoff_is_waited_then_cancelled() -> TestResult {
    use std::sync::{Arc, Mutex};
    let mut traces = Vec::new();
    for _ in 0..2 {
        mock_model::run(
            Config {
                script: vec![Some(mock_model::Fault::ServerError { status: 503 }); 4],
                ..Config::seeded(3)
            },
            |handle| {
                let waits: Arc<Mutex<Vec<Duration>>> = Arc::new(Mutex::new(Vec::new()));
                let seen = Arc::clone(&waits);
                let outcome = client_with(
                    with(
                        with(
                            settings(&base(handle.addr())),
                            "LARRY_MODEL_BACKOFF_MS",
                            "50",
                        ),
                        "LARRY_MODEL_ATTEMPTS",
                        "3",
                    ),
                    move |wait| {
                        if let Ok(mut record) = seen.lock() {
                            record.push(wait);
                        }
                        false
                    },
                )?
                .complete(&[user("cancelled")]);
                assert!(
                    matches!(outcome, Err(ModelError::Server { status: 503, .. })),
                    "the gateway refusal is the answer, got {outcome:?}"
                );
                assert_eq!(handle.served(), 1, "the cancel stops the retry");
                let waits = match waits.lock() {
                    Ok(record) => record.clone(),
                    Err(_) => return Err("the wait record is poisoned".into()),
                };
                assert_eq!(
                    waits,
                    vec![Duration::from_millis(50)],
                    "one non-zero backoff is waited, then cancelled: {waits:?}"
                );
                traces.push(handle.trace_hash());
                Ok::<_, Box<dyn Error>>(())
            },
        )?;
    }
    assert_eq!(
        traces.first(),
        traces.get(1),
        "one seed replays one gateway trace"
    );
    Ok(())
}
