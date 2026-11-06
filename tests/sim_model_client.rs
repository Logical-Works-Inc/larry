//! Seeded deterministic simulations of the model client (#83) against the
//! seeded mock endpoint (larry#82), driven through its public interface.
//!
//! Source: the OpenAI chat-completions contract the forge client speaks
//! (`forge-harness` `sdk/forge-providers`, `openai_compatible.rs` and
//! `completion_body.rs`), HTTP/1.1 (RFC 9110 sections 10.2.3 and 15.5.20) for
//! 429 with `Retry-After`, 5xx and status-class retry semantics, and the
//! estate's seeded-simulation rule (one seed, same trace hash). Requirement
//! proved: a configured model and temperature always reach the gateway; only
//! 429 and 5xx retry, within the policy; `Retry-After` is a floor; every
//! failure and every bad setting is a typed `ModelError`; no client sends
//! another's key. Settings come from a table, never the process environment.

#[path = "support/http_wire.rs"]
mod http_wire;
#[path = "support/mock_model.rs"]
mod mock_model;

use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use larry_demo::model::{Completion, Message, ModelClient, ModelConfig, ModelError, Role};
use lgwks_std::json::{self, Value};
use mock_model::{Config, Fault};

/// What every test here returns.
type TestResult = Result<(), Box<dyn Error>>;

/// One `LARRY_MODEL_*` table: the variables a case sets.
type Settings = Vec<(&'static str, String)>;

/// What one paced run observed: the outcome, the waits the client asked for and
/// the requests the mock served.
type Paced = (Result<Completion, ModelError>, Vec<Duration>, u64);

/// Seeds every behaviour case runs over.
const SEEDS: u64 = 64;

/// Concurrent requests per tenant in the isolation case; two tenants stay at
/// the mock's connection cap of eight.
const PER_TENANT: u32 = 4;

/// The table every case starts from: the gateway at `base`, `key`, a 5 s
/// timeout and three zero-backoff attempts within 60 s. No case waits on a clock.
fn settings(base: &str, key: &str) -> Settings {
    vec![
        ("LARRY_MODEL_URL", base.to_owned()),
        ("LARRY_MODEL_KEY", key.to_owned()),
        ("LARRY_MODEL_TIMEOUT_MS", String::from("5000")),
        ("LARRY_MODEL_ATTEMPTS", String::from("3")),
        ("LARRY_MODEL_BACKOFF_MS", String::from("0")),
        ("LARRY_MODEL_DEADLINE_MS", String::from("60000")),
    ]
}

/// `table` with each `(name, value)` of `overrides` set in place of the old value.
fn with(mut table: Settings, overrides: &[(&'static str, &str)]) -> Settings {
    for &(name, value) in overrides {
        table.retain(|entry| entry.0 != name);
        table.push((name, value.to_owned()));
    }
    table
}

/// A config read from `table`.
fn config_from(table: Settings) -> Result<ModelConfig, ModelError> {
    ModelConfig::from_env(move |name| {
        table
            .iter()
            .find(|entry| entry.0 == name)
            .map(|entry| entry.1.clone())
    })
}

/// A client for `table` whose retries go ahead at once, never waiting.
fn client_from(table: Settings) -> Result<ModelClient, ModelError> {
    Ok(ModelClient::new(config_from(table)?, |_wait| true))
}

/// One user turn.
fn user(text: &str) -> Message {
    Message::new(Role::User, text)
}

/// The base URL of a mock server bound to `addr`.
fn base_of(addr: std::net::SocketAddr) -> String {
    format!("http://{addr}/v1")
}

/// Run one completion against a fresh mock for `config` with `overrides`,
/// returning its outcome and the trace hash the run ended on.
fn run_once(
    config: Config,
    overrides: &[(&'static str, &str)],
    messages: &[Message],
) -> Result<(Result<Completion, ModelError>, String), Box<dyn Error>> {
    mock_model::run(config, |handle| {
        let table = with(settings(&base_of(handle.addr()), "test-key"), overrides);
        let outcome = client_from(table)?.complete(messages);
        Ok::<_, Box<dyn Error>>((outcome, handle.trace_hash()))
    })
}

/// A config whose script is `script` from `seed`.
fn scripted(seed: u64, script: Vec<Option<Fault>>) -> Config {
    Config {
        script,
        ..Config::seeded(seed)
    }
}

#[test]
fn a_normal_completion_returns_the_mock_reply() -> TestResult {
    for seed in 0..SEEDS {
        let (outcome, _) = run_once(Config::seeded(seed), &[], &[user("hello")])?;
        let completion = outcome.map_err(|error| format!("seed {seed}: {error}"))?;
        assert!(
            !completion.content().is_empty(),
            "seed {seed}: the reply carries content"
        );
    }
    Ok(())
}

#[test]
fn the_body_sent_carries_the_pinned_model_and_temperature() -> TestResult {
    for seed in 0..SEEDS {
        let temperature = format!("{}", f64::from(u32::try_from(seed % 5)?) / 4.0);
        mock_model::run(Config::seeded(seed), |handle| {
            let table = with(
                settings(&base_of(handle.addr()), "test-key"),
                &[("LARRY_MODEL_TEMPERATURE", temperature.as_str())],
            );
            client_from(table)?.complete(&[user("probe")])?;
            let recorded = handle.requests();
            let sent: Value = json::from_slice(recorded.first().ok_or("no recorded request")?)?;
            assert_eq!(
                sent["model"], "ocg/deepseek-v4.1-flash",
                "seed {seed}: the default pinned model reached the gateway"
            );
            assert_eq!(
                sent["temperature"],
                temperature.parse::<f64>()?,
                "seed {seed}: the configured temperature reached the gateway"
            );
            assert!(
                sent.get("stream").is_none(),
                "seed {seed}: no streaming key is sent"
            );
            Ok::<_, Box<dyn Error>>(())
        })?;
    }
    Ok(())
}

#[test]
fn a_rate_limit_then_success_completes_within_the_budget() -> TestResult {
    for seed in 0..SEEDS {
        let limited = Some(Fault::RateLimit {
            retry_after_secs: 1,
        });
        let (outcome, _) = run_once(scripted(seed, vec![limited, None]), &[], &[user("again")])?;
        let completion = outcome.map_err(|error| format!("seed {seed}: {error}"))?;
        assert!(
            !completion.content().is_empty(),
            "seed {seed}: the retried attempt succeeds"
        );
    }
    Ok(())
}

#[test]
fn a_server_error_that_exhausts_the_retries_is_a_server_error() -> TestResult {
    for seed in 0..SEEDS {
        let failing = vec![Some(Fault::ServerError { status: 503 }); 3];
        mock_model::run(scripted(seed, failing), |handle| {
            let client = client_from(settings(&base_of(handle.addr()), "test-key"))?;
            assert_eq!(
                client.complete(&[user("doomed")]),
                Err(ModelError::Server {
                    status: 503,
                    detail: String::from("upstream failure"),
                }),
                "seed {seed}: five-hundreds retry, then the last one is reported"
            );
            assert_eq!(
                handle.served(),
                3,
                "seed {seed}: a three-attempt policy sends exactly three requests"
            );
            Ok::<_, Box<dyn Error>>(())
        })?;
    }
    Ok(())
}

#[test]
fn a_client_error_is_never_retried() -> TestResult {
    for seed in 0..SEEDS {
        mock_model::run(Config::seeded(seed), |handle| {
            let wrong_route = format!("http://{}/v2", handle.addr());
            let client = client_from(settings(&wrong_route, "test-key"))?;
            assert_eq!(
                client.complete(&[user("lost")]),
                Err(ModelError::Client {
                    status: 404,
                    detail: String::from("no such route"),
                }),
                "seed {seed}: a 404 is a client error carrying the gateway's reason"
            );
            assert_eq!(handle.served(), 1, "seed {seed}: a 4xx is sent once");
            Ok::<_, Box<dyn Error>>(())
        })?;
    }
    Ok(())
}

#[test]
fn a_padded_reply_with_a_trailing_done_marker_is_a_completion() -> TestResult {
    for seed in 0..SEEDS {
        let (padded, _) = run_once(
            scripted(seed, vec![Some(Fault::PaddedDone)]),
            &[],
            &[user("framed")],
        )?;
        let (bare, _) = run_once(Config::seeded(seed), &[], &[user("framed")])?;
        assert_eq!(
            padded.map_err(|error| format!("seed {seed}: {error}"))?,
            bare.map_err(|error| format!("seed {seed}: {error}"))?,
            "seed {seed}: OpenRouter's padded framing reads as the bare document (larry#128)"
        );
    }
    Ok(())
}

#[test]
fn a_hostile_refusal_message_is_bounded_and_cannot_drive_a_terminal() -> TestResult {
    for seed in 0..SEEDS {
        let status = [401_u16, 403, 500, 503][usize::try_from(seed % 4)?];
        let config = scripted(seed, vec![Some(Fault::HostileRefusal { status }); 3]);
        let (outcome, _) = run_once(config, &[], &[user("hostile")])?;
        let detail = match outcome {
            Err(ModelError::Client {
                status: got,
                detail,
            })
            | Err(ModelError::Server {
                status: got,
                detail,
            }) => {
                assert_eq!(got, status, "seed {seed}: the status is kept");
                detail
            }
            other => return Err(format!("seed {seed}: expected a refusal, got {other:?}").into()),
        };
        assert!(
            detail.starts_with(" [31mexpired key"),
            "seed {seed}: the gateway's words survive with control characters blanked: {detail:.40}"
        );
        assert!(
            !detail.chars().any(char::is_control),
            "seed {seed}: no control character reaches the operator"
        );
        assert_eq!(
            detail.chars().count(),
            512,
            "seed {seed}: the reason is cut at 512 characters"
        );
        assert!(
            mock_model::hostile_message().chars().count() > 512,
            "the fixture is longer than the bound"
        );
    }
    Ok(())
}

#[test]
fn a_malformed_200_body_is_a_malformed_error_and_is_not_retried() -> TestResult {
    for seed in 0..SEEDS {
        let config = scripted(seed, vec![Some(Fault::MalformedJson)]);
        let (outcome, _) = run_once(config, &[], &[user("bad body")])?;
        assert!(
            matches!(outcome, Err(ModelError::Malformed(_))),
            "seed {seed}: a reply that is not JSON is malformed, got {outcome:?}"
        );
    }
    Ok(())
}

#[test]
fn a_truncated_body_is_a_transport_failure_never_a_short_reply() -> TestResult {
    for seed in 0..SEEDS {
        let config = scripted(seed, vec![Some(Fault::TruncatedBody)]);
        let (outcome, _) = run_once(config, &[], &[user("cut")])?;
        assert_eq!(
            outcome,
            Err(ModelError::Transport),
            "seed {seed}: a body cut short against its declared length is a transport failure"
        );
    }
    Ok(())
}

#[test]
fn a_delay_past_the_timeout_is_a_timeout() -> TestResult {
    for seed in 0..SEEDS {
        mock_model::run(scripted(seed, vec![Some(Fault::Delay)]), |handle| {
            let table = with(
                settings(&base_of(handle.addr()), "test-key"),
                &[
                    ("LARRY_MODEL_TIMEOUT_MS", "150"),
                    ("LARRY_MODEL_ATTEMPTS", "1"),
                ],
            );
            let outcome = client_from(table)?.complete(&[user("slow")]);
            handle.release_delayed();
            assert_eq!(
                outcome,
                Err(ModelError::Timeout),
                "seed {seed}: a reply that outlasts the timeout is a timeout"
            );
            assert_eq!(handle.served(), 1, "seed {seed}: a timeout is never resent");
            Ok::<_, Box<dyn Error>>(())
        })?;
    }
    Ok(())
}

#[test]
fn a_missing_key_is_refused_before_anything_is_sent() -> TestResult {
    for seed in 0..SEEDS {
        mock_model::run(Config::seeded(seed), |handle| {
            let base = base_of(handle.addr());
            let empty = config_from(settings(&base, ""));
            assert!(
                matches!(empty, Err(ModelError::MissingKey)),
                "seed {seed}: an empty key is refused"
            );
            let mut unset = settings(&base, "");
            unset.retain(|entry| entry.0 != "LARRY_MODEL_KEY");
            assert!(
                matches!(config_from(unset), Err(ModelError::MissingKey)),
                "seed {seed}: an unset key is refused"
            );
            assert_eq!(
                handle.served(),
                0,
                "seed {seed}: nothing reached the gateway"
            );
            Ok::<_, Box<dyn Error>>(())
        })?;
    }
    Ok(())
}

#[test]
fn hostile_settings_are_typed_refusals_and_the_key_never_shows() -> TestResult {
    let base = "http://127.0.0.1:9/v1";
    let integer = |name: &'static str, value: &str| {
        matches!(
            config_from(with(settings(base, "k"), &[(name, value)])),
            Err(ModelError::BadInteger { variable, .. }) if variable == name
        )
    };
    let range = |name: &'static str, value: &str| {
        config_from(with(settings(base, "k"), &[(name, value)]))
            .err()
            .is_some_and(|error| error == ModelError::OutOfRange(name))
    };
    assert!(
        integer("LARRY_MODEL_ATTEMPTS", "many"),
        "attempts must be a number"
    );
    assert!(
        integer("LARRY_MODEL_TIMEOUT_MS", "-1"),
        "a timeout is never negative"
    );
    assert!(
        integer("LARRY_MODEL_BACKOFF_MS", "1e3"),
        "backoff is whole milliseconds"
    );
    assert!(
        integer("LARRY_MODEL_DEADLINE_MS", ""),
        "an empty deadline is refused"
    );
    assert!(
        range("LARRY_MODEL_ATTEMPTS", "0"),
        "zero attempts sends nothing"
    );
    assert!(
        range("LARRY_MODEL_ATTEMPTS", "17"),
        "attempts are capped at 16"
    );
    assert!(
        range("LARRY_MODEL_TEMPERATURE", "NaN"),
        "NaN never reaches the wire"
    );
    assert!(
        range("LARRY_MODEL_TEMPERATURE", "inf"),
        "infinity never reaches the wire"
    );
    assert!(
        matches!(
            config_from(with(
                settings(base, "k"),
                &[("LARRY_MODEL_TEMPERATURE", "hot")]
            )),
            Err(ModelError::BadFloat {
                variable: "LARRY_MODEL_TEMPERATURE",
                ..
            })
        ),
        "a temperature that is not a number is refused"
    );
    let shown = format!("{:?}", config_from(settings(base, "sk-very-secret"))?);
    assert!(
        !shown.contains("sk-very-secret"),
        "Debug masks the key: {shown}"
    );
    Ok(())
}

#[test]
fn an_oversize_input_is_refused_before_anything_is_sent() -> TestResult {
    for seed in 0..SEEDS {
        mock_model::run(Config::seeded(seed), |handle| {
            let client = client_from(settings(&base_of(handle.addr()), "test-key"))?;
            let too_many: Vec<Message> = (0..257).map(|_| user("x")).collect();
            assert_eq!(
                client.complete(&too_many),
                Err(ModelError::BodyTooLarge),
                "seed {seed}: more than 256 messages is refused"
            );
            let huge = "y".repeat(1_048_577);
            assert_eq!(
                client.complete(&[user(&huge)]),
                Err(ModelError::BodyTooLarge),
                "seed {seed}: a request body over 1 MiB is refused"
            );
            assert_eq!(
                handle.served(),
                0,
                "seed {seed}: nothing reached the gateway"
            );
            Ok::<_, Box<dyn Error>>(())
        })?;
    }
    Ok(())
}

#[test]
fn concurrent_tenants_each_send_only_their_own_key() -> TestResult {
    for seed in 0..SEEDS {
        mock_model::run(Config::seeded(seed), |handle| {
            let base = base_of(handle.addr());
            let alpha = client_from(settings(&base, "tenant-alpha-secret"))?;
            let beta = client_from(settings(&base, "tenant-beta-secret"))?;
            let outcomes: Vec<Result<Completion, ModelError>> = std::thread::scope(|scope| {
                let mut running = Vec::new();
                for turn in 0..PER_TENANT {
                    let (alpha, beta) = (&alpha, &beta);
                    running.push(
                        scope.spawn(move || alpha.complete(&[user(&format!("alpha {turn}"))])),
                    );
                    running
                        .push(scope.spawn(move || beta.complete(&[user(&format!("beta {turn}"))])));
                }
                running
                    .into_iter()
                    .map(|worker| match worker.join() {
                        Ok(outcome) => outcome,
                        Err(_) => Err(ModelError::Transport),
                    })
                    .collect()
            });
            assert!(
                outcomes.iter().all(Result::is_ok),
                "seed {seed}: every concurrent request completes: {outcomes:?}"
            );
            let bodies = handle.requests();
            let keys = handle.authorizations();
            assert_eq!(
                bodies.len(),
                8,
                "seed {seed}: every request reached the gateway"
            );
            for (body, key) in bodies.iter().zip(&keys) {
                let text = String::from_utf8_lossy(body);
                let expected = if text.contains("\"alpha ") {
                    "Bearer tenant-alpha-secret"
                } else {
                    "Bearer tenant-beta-secret"
                };
                assert_eq!(
                    key.as_deref(),
                    Some(expected),
                    "seed {seed}: each request carries its own tenant's key and no other"
                );
                assert!(
                    !text.contains("secret"),
                    "seed {seed}: the key travels as a header, never in the body"
                );
            }
            Ok::<_, Box<dyn Error>>(())
        })?;
    }
    Ok(())
}

#[test]
fn the_same_seed_gives_the_same_trace_hash_for_every_case() -> TestResult {
    for seed in 0..SEEDS {
        let limited = Some(Fault::RateLimit {
            retry_after_secs: 1,
        });
        let script = vec![limited, None];
        let one = run_once(scripted(seed, script.clone()), &[], &[user("replay")])?;
        let two = run_once(scripted(seed, script), &[], &[user("replay")])?;
        assert_eq!(
            one.1, two.1,
            "seed {seed}: the same seed replays to one trace hash"
        );
        assert_eq!(
            one.0, two.0,
            "seed {seed}: the same seed replays to one outcome"
        );
    }
    Ok(())
}

/// One rate limit carrying `hint` seconds, then a clean reply, under
/// `overrides`, with a pause that records each wait and answers `go_on`.
fn paced(
    seed: u64,
    hint: u32,
    overrides: &[(&'static str, &str)],
    go_on: bool,
) -> Result<Paced, Box<dyn Error>> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&calls);
    let limited = Some(Fault::RateLimit {
        retry_after_secs: hint,
    });
    let (outcome, served) = mock_model::run(scripted(seed, vec![limited, None]), |handle| {
        let table = with(settings(&base_of(handle.addr()), "test-key"), overrides);
        let client = ModelClient::new(config_from(table)?, move |wait| {
            if let Ok(mut waits) = recorder.lock() {
                waits.push(wait);
            }
            go_on
        });
        let outcome = client.complete(&[user("paced")]);
        Ok::<_, Box<dyn Error>>((outcome, handle.served()))
    })?;
    let waits = calls.lock().map_err(|_| "pause recorder poisoned")?.clone();
    Ok((outcome, waits, served))
}

#[test]
fn retry_after_is_a_floor_never_shortened() -> TestResult {
    for seed in 0..SEEDS {
        let (outcome, waits, _) = paced(seed, 2, &[], true)?;
        assert!(outcome.is_ok(), "seed {seed}: the retry succeeds");
        assert_eq!(
            waits,
            [Duration::from_secs(2)],
            "seed {seed}: a zero backoff still waits the gateway's 2s hint"
        );
        let long = [("LARRY_MODEL_BACKOFF_MS", "30000")];
        let (_, waits, _) = paced(seed, 2, &long, true)?;
        assert_eq!(
            waits,
            [Duration::from_secs(30)],
            "seed {seed}: a longer policy backoff is kept, the hint only raises it"
        );
    }
    Ok(())
}

#[test]
fn a_retry_after_past_the_deadline_returns_the_rate_limit() -> TestResult {
    for seed in 0..SEEDS {
        let short = [("LARRY_MODEL_DEADLINE_MS", "10000")];
        let (outcome, waits, served) = paced(seed, 3_600, &short, true)?;
        assert_eq!(
            outcome,
            Err(ModelError::RateLimited {
                retry_after: Some(3_600)
            }),
            "seed {seed}: a hint the deadline cannot honour is the answer"
        );
        assert!(waits.is_empty(), "seed {seed}: the client never waited");
        assert_eq!(served, 1, "seed {seed}: no second request was sent");
    }
    Ok(())
}

#[test]
fn a_pause_that_declines_stops_retrying_with_the_refusal() -> TestResult {
    for seed in 0..SEEDS {
        let (outcome, waits, served) = paced(seed, 2, &[], false)?;
        assert_eq!(
            outcome,
            Err(ModelError::RateLimited {
                retry_after: Some(2)
            }),
            "seed {seed}: a cancelled or refused wait returns the gateway's refusal"
        );
        assert_eq!(
            waits,
            [Duration::from_secs(2)],
            "seed {seed}: the pause was asked once"
        );
        assert_eq!(
            served, 1,
            "seed {seed}: nothing is resent after a declined wait"
        );
    }
    Ok(())
}
