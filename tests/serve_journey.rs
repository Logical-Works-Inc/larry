//! Journeys against the shipped `larry serve` (larry#88, larry#89), as an
//! operator, an orchestrator and a service tenant live them: issue a key, start
//! the host, probe it with and without the key, stop it, kill it, start it again.
//!
//! Source: the Kubernetes liveness and readiness probe contract (an HTTP `GET`
//! answered `200` while serving), the SIGTERM-then-drain shutdown every
//! orchestrator sends before SIGKILL, RFC 6750 section 3 (a missing or wrong
//! bearer token is `401` with `WWW-Authenticate: Bearer`), OWASP's API key
//! guidance (store a digest, show the key once, rotate by reissuing), RFC 9110
//! sections 15.5.1 and 15.5.14 (`400`, `413`), and POSIX exit-status
//! conventions. Requirement proved: the binary, started with a cleared
//! environment from a directory that is not the repository, admits a tenant
//! request only with that tenant's current key, answers liveness with none,
//! refuses an oversize body at the edge, exits 0 on SIGTERM or SIGINT and stops
//! accepting, leaves its port free after a kill -9 with its keys intact, and
//! refuses a malformed or taken address, a missing state directory or a bad
//! tenant name with a message naming it and exit 1.

#[path = "support/serve_cli.rs"]
mod serve_cli;

use std::error::Error;
use std::io::{self, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use lgwks_std::json::{self, Value};
use serve_cli::{
    Serving, exchange, fill_key_store, fresh_state_dir, get, issue_key, probe_body, revoke,
};

/// Keyed probes each latency column of the revoke journey samples.
const REVOKE_PROBES: usize = 1_000;

/// The edge's key cap, `MAX_TENANT_KEYS`: a store holds at most this many rows.
const KEY_CAP: usize = 65_536;

/// What every test here returns.
type TestResult = Result<(), Box<dyn Error>>;

/// Probes the latency case samples.
const SAMPLES: usize = 256;

/// The edge's body ceiling, 1 MiB; one byte past it is refused.
const BODY_CEILING: usize = 1024 * 1024;

/// The edge's answer to a request with no valid key for its tenant.
const UNKNOWN: &str = "name yourself";

/// The probe path of the one tenant most journeys key.
const ACME_PROBE: &str = "/t/acme/healthz";

/// A host on `addr` over the store in `state`.
fn start_at(addr: &str, state: &Path) -> Result<Serving, Box<dyn Error>> {
    let dir = state.display().to_string();
    Serving::start(&[
        ("LARRY_SERVE_ADDR", addr),
        ("LARRY_STATE_DIR", dir.as_str()),
    ])
}

/// A host on a free loopback port over `state`.
fn start_on(state: &Path) -> Result<Serving, Box<dyn Error>> {
    start_at("127.0.0.1:0", state)
}

/// The message of a start that must be refused; an error if the host was served.
fn refusal(started: Result<Serving, Box<dyn Error>>, what: &str) -> Result<String, Box<dyn Error>> {
    match started {
        Ok(_) => Err(format!("{what} was served").into()),
        Err(error) => Ok(error.to_string()),
    }
}

/// A fresh state directory with a key for `acme`, a host over it, and the key.
fn start() -> Result<(Serving, String, PathBuf), Box<dyn Error>> {
    let state = fresh_state_dir();
    let key = issue_key(&state, "acme")?;
    Ok((start_on(&state)?, key, state))
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
fn a_health_probe_answers_on_production_defaults() -> TestResult {
    let (mut serving, key, _state) = start()?;
    assert_eq!(
        get(serving.addr(), "/healthz", None)?,
        (200, String::from("ok")),
        "the edge answers liveness with no tenant and no key"
    );
    let mut latencies = Vec::with_capacity(SAMPLES);
    for round in 0..SAMPLES {
        let started = Instant::now();
        let (status, body) = get(serving.addr(), ACME_PROBE, Some(&key))?;
        latencies.push(started.elapsed());
        assert_eq!(status, 200, "probe {round} answers 200");
        assert_eq!(body, probe_body(), "probe {round} answers the fixed body");
    }
    let (status, stderr) = serving.signal_and_wait("TERM")?;
    assert!(status.success(), "SIGTERM exits 0: {status}, {stderr}");
    assert!(stderr.is_empty(), "a clean stop prints nothing: {stderr}");
    latencies.sort_unstable();
    let line = format!(
        "larry serve keyed probe over {SAMPLES} requests: p50 {:?} p95 {:?} p99 {:?}\n",
        percentile(&latencies, 50),
        percentile(&latencies, 95),
        percentile(&latencies, 99)
    );
    io::stderr().lock().write_all(line.as_bytes())?;
    Ok(())
}

#[test]
fn a_tenant_request_needs_that_tenants_own_key() -> TestResult {
    let state = fresh_state_dir();
    let acme = issue_key(&state, "acme")?;
    let beta = issue_key(&state, "beta")?;
    let mut serving = start_on(&state)?;
    let addr = serving.addr();
    let wrong = format!("{acme}x");
    for (key, why) in [
        (None, "no key"),
        (Some(beta.as_str()), "another tenant's key"),
        (Some(wrong.as_str()), "a near key"),
        (Some("lk_"), "the bare prefix"),
    ] {
        assert_eq!(
            get(addr, ACME_PROBE, key)?,
            (401, String::from(UNKNOWN)),
            "{why} is refused at the edge"
        );
    }
    assert_eq!(
        get(addr, "/t/acme/whoami", Some(&acme))?,
        (200, String::from("{\"principal\":\"acme\"}")),
        "the key names its tenant to the route"
    );
    assert_eq!(
        get(addr, "/t/unkeyed/healthz", Some(&acme))?.0,
        401,
        "a tenant with no key admits no one"
    );
    let (status, stderr) = serving.signal_and_wait("TERM")?;
    assert!(status.success(), "stops clean after refusals: {stderr}");
    Ok(())
}

#[test]
fn issuing_again_rotates_the_key() -> TestResult {
    let state = fresh_state_dir();
    let first = issue_key(&state, "acme")?;
    let second = issue_key(&state, "acme")?;
    assert_ne!(first, second, "every issue draws a new key");
    assert!(
        second.starts_with("lk_") && second.len() == 67,
        "a key is lk_ and 64 hex: {second}"
    );
    let mut serving = start_on(&state)?;
    assert_eq!(
        get(serving.addr(), ACME_PROBE, Some(&first))?.0,
        401,
        "the old key is dead"
    );
    assert_eq!(
        get(serving.addr(), ACME_PROBE, Some(&second))?.0,
        200,
        "the new key serves"
    );
    let (status, stderr) = serving.signal_and_wait("TERM")?;
    assert!(status.success(), "stops clean: {stderr}");
    Ok(())
}

#[test]
fn sigterm_and_sigint_drain_and_exit_zero() -> TestResult {
    for signal in ["TERM", "INT"] {
        let (mut serving, key, _state) = start()?;
        assert_eq!(
            get(serving.addr(), ACME_PROBE, Some(&key))?.0,
            200,
            "serving before SIG{signal}"
        );
        let addr = serving.addr();
        let (status, stderr) = serving.signal_and_wait(signal)?;
        assert!(status.success(), "SIG{signal} exits 0: {status}, {stderr}");
        assert!(
            TcpStream::connect(addr).is_err(),
            "after SIG{signal} nothing accepts on {addr}"
        );
    }
    Ok(())
}

#[test]
fn a_kill_9_leaves_the_port_and_the_keys_for_the_next_start() -> TestResult {
    let (mut first, key, state) = start()?;
    let addr = first.addr();
    assert_eq!(
        get(addr, ACME_PROBE, Some(&key))?.0,
        200,
        "the first host serves"
    );
    let (status, _) = first.signal_and_wait("KILL")?;
    assert!(!status.success(), "kill -9 is not a clean exit: {status}");
    let pinned = addr.to_string();
    let dir = state.display().to_string();
    let mut second = Serving::start(&[
        ("LARRY_SERVE_ADDR", pinned.as_str()),
        ("LARRY_STATE_DIR", dir.as_str()),
    ])?;
    assert_eq!(
        second.addr(),
        addr,
        "the second host binds the same address"
    );
    assert_eq!(
        get(addr, ACME_PROBE, Some(&key))?,
        (200, probe_body()),
        "the restarted host serves the same key"
    );
    let (status, stderr) = second.signal_and_wait("TERM")?;
    assert!(status.success(), "the second host stops clean: {stderr}");
    Ok(())
}

#[test]
fn a_body_past_the_ceiling_is_refused_at_the_edge() -> TestResult {
    let (mut serving, key, _state) = start()?;
    let oversize = vec![b'x'; BODY_CEILING.saturating_add(1)];
    let (status, body) = exchange(serving.addr(), "GET", ACME_PROBE, &oversize, Some(&key))?;
    assert_eq!(status, 413, "one byte past 1 MiB is 413: {body}");
    assert_eq!(
        get(serving.addr(), ACME_PROBE, Some(&key))?.0,
        200,
        "the host serves on after refusing it"
    );
    let (status, stderr) = serving.signal_and_wait("TERM")?;
    assert!(status.success(), "stops clean after a refusal: {stderr}");
    Ok(())
}

#[test]
fn a_bad_address_is_refused_before_binding() -> TestResult {
    for value in ["localhost:8787", "", "127.0.0.1"] {
        let refused = Serving::start(&[("LARRY_SERVE_ADDR", value)]);
        let message = match refused {
            Ok(_) => return Err(format!("{value:?} was served").into()),
            Err(error) => error.to_string(),
        };
        assert!(
            message.contains("exit status: 1")
                && message.contains(&format!("LARRY_SERVE_ADDR={value:?} is not an IP address")),
            "{value:?} is refused naming itself, exit 1: {message}"
        );
    }
    Ok(())
}

#[test]
fn no_state_directory_is_refused_before_binding() -> TestResult {
    let refused = Serving::start(&[("LARRY_SERVE_ADDR", "127.0.0.1:0")]);
    let message = match refused {
        Ok(_) => return Err("a host with no key store was served".into()),
        Err(error) => error.to_string(),
    };
    assert!(
        message.contains("exit status: 1") && message.contains("LARRY_STATE_DIR"),
        "no state directory is refused naming it, exit 1: {message}"
    );
    Ok(())
}

#[test]
fn a_bad_tenant_name_gets_no_key() -> TestResult {
    let state = fresh_state_dir();
    let past = "a".repeat(65);
    for name in ["", "a.b", "a b", past.as_str()] {
        let output = Command::new(env!("CARGO_BIN_EXE_larry"))
            .args(["serve", "key", name])
            .env_clear()
            .env("LARRY_STATE_DIR", &state)
            .stdin(Stdio::null())
            .output()?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{name:?} exits 1: {stderr}");
        assert!(output.stdout.is_empty(), "{name:?} prints no key");
        assert!(
            stderr.contains("is not a tenant name"),
            "{name:?} is named: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn a_taken_port_is_refused_with_its_address() -> TestResult {
    let state = fresh_state_dir();
    let dir = state.display().to_string();
    let holder = TcpListener::bind("127.0.0.1:0")?;
    let taken = holder.local_addr()?.to_string();
    let refused = Serving::start(&[
        ("LARRY_SERVE_ADDR", taken.as_str()),
        ("LARRY_STATE_DIR", dir.as_str()),
    ]);
    let message = match refused {
        Ok(_) => return Err(format!("{taken} was served twice").into()),
        Err(error) => error.to_string(),
    };
    assert!(
        message.contains("exit status: 1") && message.contains(&format!("cannot bind {taken}")),
        "a taken port is refused naming it, exit 1: {message}"
    );
    Ok(())
}

/// `probes` keyed requests for `path` over fresh connections, each answered `status`;
/// their latencies.
fn timed_probes(
    addr: SocketAddr,
    path: &str,
    key: &str,
    probes: usize,
    status: u16,
) -> Result<Vec<Duration>, Box<dyn Error>> {
    let mut latencies = Vec::with_capacity(probes);
    for round in 0..probes {
        let started = Instant::now();
        let (answered, _) = get(addr, path, Some(key))?;
        latencies.push(started.elapsed());
        assert_eq!(answered, status, "probe {round} of {path} answers {status}");
    }
    Ok(latencies)
}

/// One labelled line of nearest-rank p50, p95 and p99 over `latencies`.
fn percentile_line(label: &str, mut latencies: Vec<Duration>) -> String {
    latencies.sort_unstable();
    format!(
        "{label} over {} requests: p50 {:?} p95 {:?} p99 {:?}\n",
        latencies.len(),
        percentile(&latencies, 50),
        percentile(&latencies, 95),
        percentile(&latencies, 99)
    )
}

#[test]
fn a_non_loopback_bind_without_the_plaintext_opt_in_is_refused() -> TestResult {
    let state = fresh_state_dir();
    let message = refusal(start_at("0.0.0.0:0", &state), "0.0.0.0 without the opt-in")?;
    assert!(
        message.contains("exit status: 1")
            && message.contains("0.0.0.0:0")
            && message.contains("LARRY_SERVE_PLAINTEXT=1"),
        "a non-loopback bind is refused naming its address and the opt-in, exit 1: {message}"
    );
    let mut loopback = start_at("127.0.0.1:0", &state)?;
    let (status, stderr) = loopback.signal_and_wait("TERM")?;
    assert!(status.success(), "loopback starts with no opt-in: {stderr}");
    let dir = state.display().to_string();
    let mut opted = Serving::start(&[
        ("LARRY_SERVE_ADDR", "0.0.0.0:0"),
        ("LARRY_SERVE_PLAINTEXT", "1"),
        ("LARRY_STATE_DIR", dir.as_str()),
    ])?;
    let on_loopback = SocketAddr::from(([127, 0, 0, 1], opted.addr().port()));
    assert_eq!(
        get(on_loopback, "/healthz", None)?,
        (200, String::from("ok")),
        "the opted-in host answers on loopback"
    );
    let (status, stderr) = opted.signal_and_wait("TERM")?;
    assert!(status.success(), "the opted-in host stops clean: {stderr}");
    Ok(())
}

#[test]
fn a_plaintext_flag_other_than_one_is_refused() -> TestResult {
    for value in ["true", "0", "", "yes"] {
        let started = Serving::start(&[
            ("LARRY_SERVE_ADDR", "127.0.0.1:0"),
            ("LARRY_SERVE_PLAINTEXT", value),
        ]);
        let message = refusal(started, &format!("{value:?} as the opt-in"))?;
        assert!(
            message.contains("exit status: 1")
                && message.contains(&format!("LARRY_SERVE_PLAINTEXT={value:?}")),
            "{value:?} is refused naming itself, exit 1: {message}"
        );
    }
    Ok(())
}

#[test]
fn a_revoked_key_gets_401_on_its_next_request_with_no_restart() -> TestResult {
    let state = fresh_state_dir();
    let acme = issue_key(&state, "acme")?;
    let beta = issue_key(&state, "beta")?;
    let mut serving = start_on(&state)?;
    let addr = serving.addr();
    let before = timed_probes(addr, ACME_PROBE, &acme, REVOKE_PROBES, 200)?;
    let revoked = revoke(&state, "acme")?;
    assert!(
        revoked.status.success(),
        "the revoke exits 0: {}",
        String::from_utf8_lossy(&revoked.stderr)
    );
    assert_eq!(
        get(addr, ACME_PROBE, Some(&acme))?.0,
        200,
        "the running host keeps the key until it reads the store on SIGHUP"
    );
    serving.signal("HUP")?;
    let ack = serving.next_line()?;
    let printed: Value = json::from_str(&ack)?;
    assert_eq!(
        printed.get("reloaded").and_then(Value::as_bool),
        Some(true),
        "the reload acknowledges on stdout: {ack}"
    );
    assert_eq!(
        get(addr, ACME_PROBE, Some(&acme))?,
        (401, String::from(UNKNOWN)),
        "the revoked key gets 401 on its next request"
    );
    let after = timed_probes(addr, "/t/beta/healthz", &beta, REVOKE_PROBES, 200)?;
    let mut report = percentile_line("acme keyed probes before the revoke", before);
    report.push_str(&percentile_line(
        "beta keyed probes after the reload",
        after,
    ));
    io::stderr().lock().write_all(report.as_bytes())?;
    let (status, stderr) = serving.signal_and_wait("TERM")?;
    assert!(status.success(), "stops clean after the reload: {stderr}");
    Ok(())
}

#[test]
fn a_revoke_of_an_unkeyed_tenant_is_refused() -> TestResult {
    let state = fresh_state_dir();
    issue_key(&state, "acme")?;
    let output = revoke(&state, "ghost")?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(output.stdout.is_empty(), "a refused revoke prints no line");
    assert!(
        stderr.contains("\"ghost\" holds no key to revoke"),
        "the refusal names the tenant: {stderr}"
    );
    Ok(())
}

#[test]
fn a_revoke_survives_a_hard_kill_and_restart() -> TestResult {
    let state = fresh_state_dir();
    let acme = issue_key(&state, "acme")?;
    let mut first = start_on(&state)?;
    let addr = first.addr();
    assert_eq!(
        get(addr, ACME_PROBE, Some(&acme))?.0,
        200,
        "the first host serves the key"
    );
    let revoked = revoke(&state, "acme")?;
    assert!(revoked.status.success(), "the revoke exits 0");
    let (status, _) = first.signal_and_wait("KILL")?;
    assert!(!status.success(), "kill -9 is not a clean exit: {status}");
    let mut second = start_at(&addr.to_string(), &state)?;
    assert_eq!(
        get(addr, ACME_PROBE, Some(&acme))?,
        (401, String::from(UNKNOWN)),
        "the restarted host reads the revoke from executor.db"
    );
    let (status, stderr) = second.signal_and_wait("TERM")?;
    assert!(status.success(), "the second host stops clean: {stderr}");
    Ok(())
}

#[test]
fn a_key_store_past_the_cap_refuses_startup() -> TestResult {
    let state = fresh_state_dir();
    let acme = issue_key(&state, "acme")?;
    fill_key_store(&state, 0, KEY_CAP)?;
    let message = refusal(start_at("127.0.0.1:0", &state), "a store past the cap")?;
    assert!(
        message.contains("exit status: 1")
            && message.contains("65537")
            && message.contains("65536"),
        "startup is refused naming the count and the cap, exit 1: {message}"
    );
    let removed = revoke(&state, "tenant-0")?;
    assert!(
        removed.status.success(),
        "one revoke brings the store to the cap"
    );
    let mut serving = start_on(&state)?;
    assert_eq!(
        get(serving.addr(), ACME_PROBE, Some(&acme))?.0,
        200,
        "at the cap the host starts and serves"
    );
    let (status, stderr) = serving.signal_and_wait("TERM")?;
    assert!(status.success(), "stops clean at the cap: {stderr}");
    Ok(())
}
