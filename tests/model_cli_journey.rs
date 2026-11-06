//! Journeys against the shipped `rocco model` command (larry#83), as an
//! operator lives them: set `LARRY_MODEL_*`, pipe a prompt, read the reply.
//!
//! Source: the OpenAI chat-completions contract the 9router gateway speaks and
//! POSIX process conventions (exit status, stdout for the answer, stderr for
//! the diagnostic). Requirement proved: the shipped binary sends the prompt and
//! the operator's key to the configured gateway, prints the reply, retries a
//! 5xx, and refuses a missing key, an empty or oversize prompt, a bad setting
//! and an unreachable gateway with a typed message and a failing exit, sending
//! nothing it should not. Each child runs with a cleared environment, so the
//! developer's own key can never reach a test.

#[path = "support/http_wire.rs"]
mod http_wire;
#[path = "support/mock_model.rs"]
mod mock_model;

use std::error::Error;
use std::io::{self, Write};
use std::net::TcpListener;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use lgwks_std::json::{self, Value};
use mock_model::{Config, Fault};

/// What every test here returns.
type TestResult = Result<(), Box<dyn Error>>;

/// Round trips the latency case samples.
const SAMPLES: u64 = 32;

/// Run `rocco model` with only `env` set and `prompt` on stdin.
fn rocco_model(env: &[(&str, String)], prompt: &[u8]) -> Result<Output, Box<dyn Error>> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rocco"))
        .arg("model")
        .env_clear()
        .envs(env.iter().map(|entry| (entry.0, entry.1.as_str())))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("child stdin was not piped")?;
    stdin.write_all(prompt)?;
    drop(stdin);
    Ok(child.wait_with_output()?)
}

/// The settings an operator exports for the gateway at `addr`.
fn operator(addr: std::net::SocketAddr) -> Vec<(&'static str, String)> {
    vec![
        ("LARRY_MODEL_URL", format!("http://{addr}/v1")),
        ("LARRY_MODEL_KEY", String::from("cli-key")),
        ("LARRY_MODEL_BACKOFF_MS", String::from("0")),
    ]
}

/// The child's stderr as text.
fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The sample at percentile `per_cent` of `sorted`, nearest rank.
fn percentile(sorted: &[Duration], per_cent: usize) -> Duration {
    let rank = sorted.len().saturating_mul(per_cent).div_ceil(100);
    match sorted.get(rank.saturating_sub(1)) {
        Some(&sample) => sample,
        None => Duration::ZERO,
    }
}

#[test]
fn a_prompt_on_stdin_prints_the_gateway_reply() -> TestResult {
    let mut latencies = Vec::new();
    for seed in 0..SAMPLES {
        mock_model::run(Config::seeded(seed), |handle| {
            let started = Instant::now();
            let output = rocco_model(&operator(handle.addr()), b"summarise the plan\n")?;
            latencies.push(started.elapsed());
            assert!(
                output.status.success(),
                "seed {seed}: exit {:?}, stderr {}",
                output.status,
                stderr_of(&output)
            );
            let reply = String::from_utf8(output.stdout)?;
            assert!(
                !reply.trim().is_empty(),
                "seed {seed}: the reply is printed"
            );
            let sent: Value = json::from_slice(handle.requests().first().ok_or("no request")?)?;
            assert_eq!(
                sent["messages"][0]["content"], "summarise the plan",
                "seed {seed}: the prompt reached the gateway as one user turn"
            );
            assert_eq!(
                handle.authorizations(),
                [Some(String::from("Bearer cli-key"))],
                "seed {seed}: the operator's key, and only it, reached the gateway"
            );
            Ok::<_, Box<dyn Error>>(())
        })?;
    }
    latencies.sort_unstable();
    let line = format!(
        "rocco model round trip over {SAMPLES} runs: p50 {:?} p95 {:?} p99 {:?}\n",
        percentile(&latencies, 50),
        percentile(&latencies, 95),
        percentile(&latencies, 99)
    );
    let mut report = io::stderr().lock();
    report.write_all(line.as_bytes())?;
    Ok(())
}

#[test]
fn a_server_error_then_success_retries_through_the_binary() -> TestResult {
    let config = Config {
        script: vec![Some(Fault::ServerError { status: 502 }), None],
        ..Config::seeded(11)
    };
    mock_model::run(config, |handle| {
        let output = rocco_model(&operator(handle.addr()), b"retry me")?;
        assert!(output.status.success(), "stderr {}", stderr_of(&output));
        assert_eq!(handle.served(), 2, "one 502, then the retry");
        Ok(())
    })
}

#[test]
fn a_rate_limit_is_reported_at_once_never_waited_out() -> TestResult {
    let config = Config {
        script: vec![
            Some(Fault::RateLimit {
                retry_after_secs: 30,
            }),
            None,
        ],
        ..Config::seeded(14)
    };
    mock_model::run(config, |handle| {
        let started = Instant::now();
        let output = rocco_model(&operator(handle.addr()), b"busy?")?;
        assert!(
            !output.status.success(),
            "a 429 the check will not wait out fails"
        );
        assert!(
            stderr_of(&output).contains("retry after 30s"),
            "the diagnostic carries the gateway's hint: {}",
            stderr_of(&output)
        );
        assert_eq!(handle.served(), 1, "the request is not resent early");
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the check returned without waiting out the hint"
        );
        Ok(())
    })
}

#[test]
fn a_missing_key_exits_nonzero_and_sends_nothing() -> TestResult {
    mock_model::run(Config::seeded(12), |handle| {
        let mut env = operator(handle.addr());
        env.retain(|entry| entry.0 != "LARRY_MODEL_KEY");
        let output = rocco_model(&env, b"hello")?;
        assert!(!output.status.success(), "a missing key fails");
        assert!(
            stderr_of(&output).contains("LARRY_MODEL_KEY"),
            "the diagnostic names the variable"
        );
        assert!(output.stdout.is_empty(), "nothing is printed as a reply");
        assert_eq!(handle.served(), 0, "nothing reached the gateway");
        Ok(())
    })
}

#[test]
fn an_empty_or_oversize_prompt_is_refused_before_sending() -> TestResult {
    mock_model::run(Config::seeded(13), |handle| {
        let empty = rocco_model(&operator(handle.addr()), b" \n\n")?;
        assert!(!empty.status.success(), "an empty prompt fails");
        assert!(
            stderr_of(&empty).contains("empty"),
            "the diagnostic says why"
        );
        let huge = vec![b'y'; 1_048_577];
        let oversize = rocco_model(&operator(handle.addr()), &huge)?;
        assert!(!oversize.status.success(), "an oversize prompt fails");
        assert!(
            stderr_of(&oversize).contains("size bound"),
            "the diagnostic names the bound"
        );
        assert_eq!(handle.served(), 0, "neither reached the gateway");
        Ok(())
    })
}

#[test]
fn a_bad_setting_or_a_dead_gateway_is_a_typed_diagnostic() -> TestResult {
    let closed = TcpListener::bind(("127.0.0.1", 0))?.local_addr()?;
    let dead = rocco_model(&operator(closed), b"hello")?;
    assert!(!dead.status.success(), "an unreachable gateway fails");
    assert!(
        stderr_of(&dead).contains("could not be reached"),
        "the diagnostic says the gateway is unreachable: {}",
        stderr_of(&dead)
    );
    let mut env = operator(closed);
    env.push(("LARRY_MODEL_ATTEMPTS", String::from("many")));
    let bad = rocco_model(&env, b"hello")?;
    assert!(!bad.status.success(), "a bad setting fails");
    assert!(
        stderr_of(&bad).contains("LARRY_MODEL_ATTEMPTS is not a whole number"),
        "the diagnostic names the variable: {}",
        stderr_of(&bad)
    );
    Ok(())
}

#[test]
fn a_gateway_slower_than_the_timeout_is_a_timeout_through_the_binary() -> TestResult {
    let config = Config {
        script: vec![Some(Fault::Delay)],
        ..Config::seeded(15)
    };
    mock_model::run(config, |handle| {
        let mut env = operator(handle.addr());
        env.push(("LARRY_MODEL_TIMEOUT_MS", String::from("200")));
        let output = rocco_model(&env, b"slow")?;
        handle.release_delayed();
        assert!(!output.status.success(), "a timed-out exchange fails");
        assert!(
            stderr_of(&output).contains("timed out"),
            "the diagnostic says the gateway timed out: {}",
            stderr_of(&output)
        );
        assert_eq!(handle.served(), 1, "a timeout is never resent");
        Ok(())
    })
}

#[test]
fn a_padded_openrouter_reply_prints_its_content() -> TestResult {
    let padded = Config {
        script: vec![Some(Fault::PaddedDone)],
        ..Config::seeded(16)
    };
    let framed = mock_model::run(padded, |handle| {
        rocco_model(&operator(handle.addr()), b"framed")
    })?;
    let bare = mock_model::run(Config::seeded(16), |handle| {
        rocco_model(&operator(handle.addr()), b"framed")
    })?;
    assert!(framed.status.success(), "stderr {}", stderr_of(&framed));
    assert_eq!(
        framed.stdout, bare.stdout,
        "the padded framing prints the same reply as the bare document (larry#128)"
    );
    Ok(())
}

#[test]
fn a_refusal_shows_the_gateway_reason_without_terminal_control() -> TestResult {
    let config = Config {
        script: vec![Some(Fault::HostileRefusal { status: 401 })],
        ..Config::seeded(17)
    };
    mock_model::run(config, |handle| {
        let output = rocco_model(&operator(handle.addr()), b"expired?")?;
        assert!(!output.status.success(), "a 401 fails");
        let shown = stderr_of(&output);
        assert!(
            shown.contains("refused with status 401:  [31mexpired key"),
            "the operator sees the gateway's reason: {shown:.120}"
        );
        assert!(
            !shown.trim_end().chars().any(char::is_control),
            "no escape or bell reaches the terminal"
        );
        assert_eq!(handle.served(), 1, "a 401 is never retried");
        Ok(())
    })
}

#[test]
fn the_same_seed_through_the_binary_replays_one_trace() -> TestResult {
    let trace = |seed: u64| {
        mock_model::run(Config::seeded(seed), |handle| {
            let output = rocco_model(&operator(handle.addr()), b"replay")?;
            assert!(output.status.success(), "stderr {}", stderr_of(&output));
            Ok::<_, Box<dyn Error>>((handle.trace_hash(), output.stdout))
        })
    };
    for seed in 0..8 {
        let one = trace(seed)?;
        let two = trace(seed)?;
        assert_eq!(
            one, two,
            "seed {seed}: one seed, one trace and one printed reply"
        );
    }
    Ok(())
}
