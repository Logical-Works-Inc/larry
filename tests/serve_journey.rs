//! Journeys against the shipped `larry serve` (larry#88), as an operator and an
//! orchestrator live them: start it, probe it, stop it, kill it, start it again.
//!
//! Source: the Kubernetes liveness and readiness probe contract (an HTTP `GET`
//! answered `200` while serving), the SIGTERM-then-drain shutdown every
//! orchestrator sends before SIGKILL, RFC 9110 sections 15.5.1 and 15.5.14
//! (`400`, `413`), and POSIX exit-status conventions. Requirement proved: the
//! binary, started with a cleared environment from a directory that is not the
//! repository, prints where it serves, answers the tenant probe, refuses an
//! oversize body at the edge, exits 0 on SIGTERM or SIGINT and stops accepting,
//! leaves its port free after a kill -9, and refuses a malformed or taken
//! address with a message naming it and exit 1 before serving anything.

#[path = "support/serve_cli.rs"]
mod serve_cli;

use std::error::Error;
use std::io::{self, Write as _};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use serve_cli::{Serving, exchange, get, probe_body};

/// What every test here returns.
type TestResult = Result<(), Box<dyn Error>>;

/// Probes the latency case samples.
const SAMPLES: usize = 256;

/// The edge's body ceiling, 1 MiB; one byte past it is refused.
const BODY_CEILING: usize = 1024 * 1024;

/// A host on a free loopback port, the only setting a test needs.
fn start() -> Result<Serving, Box<dyn Error>> {
    Serving::start(&[("LARRY_SERVE_ADDR", "127.0.0.1:0")])
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
    let mut serving = start()?;
    assert_eq!(
        get(serving.addr(), "/healthz")?,
        (200, String::from("ok")),
        "the edge answers liveness with no tenant"
    );
    let mut latencies = Vec::with_capacity(SAMPLES);
    for round in 0..SAMPLES {
        let started = Instant::now();
        let (status, body) = get(serving.addr(), "/t/acme/healthz")?;
        latencies.push(started.elapsed());
        assert_eq!(status, 200, "probe {round} answers 200");
        assert_eq!(body, probe_body(), "probe {round} answers the fixed body");
    }
    let (status, stderr) = serving.signal_and_wait("TERM")?;
    assert!(status.success(), "SIGTERM exits 0: {status}, {stderr}");
    assert!(stderr.is_empty(), "a clean stop prints nothing: {stderr}");
    latencies.sort_unstable();
    let line = format!(
        "larry serve probe over {SAMPLES} requests: p50 {:?} p95 {:?} p99 {:?}\n",
        percentile(&latencies, 50),
        percentile(&latencies, 95),
        percentile(&latencies, 99)
    );
    io::stderr().lock().write_all(line.as_bytes())?;
    Ok(())
}

#[test]
fn sigterm_and_sigint_drain_and_exit_zero() -> TestResult {
    for signal in ["TERM", "INT"] {
        let mut serving = start()?;
        assert_eq!(
            get(serving.addr(), "/t/acme/healthz")?.0,
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
fn a_kill_9_leaves_the_port_for_the_next_start() -> TestResult {
    let mut first = start()?;
    let addr = first.addr();
    assert_eq!(
        get(addr, "/t/acme/healthz")?.0,
        200,
        "the first host serves"
    );
    let (status, _) = first.signal_and_wait("KILL")?;
    assert!(!status.success(), "kill -9 is not a clean exit: {status}");
    let pinned = addr.to_string();
    let mut second = Serving::start(&[("LARRY_SERVE_ADDR", pinned.as_str())])?;
    assert_eq!(
        second.addr(),
        addr,
        "the second host binds the same address"
    );
    let (status, body) = get(addr, "/t/acme/healthz")?;
    assert_eq!(
        (status, body),
        (200, probe_body()),
        "the restarted host serves"
    );
    let (status, stderr) = second.signal_and_wait("TERM")?;
    assert!(status.success(), "the second host stops clean: {stderr}");
    Ok(())
}

#[test]
fn a_body_past_the_ceiling_is_refused_at_the_edge() -> TestResult {
    let mut serving = start()?;
    let oversize = vec![b'x'; BODY_CEILING.saturating_add(1)];
    let (status, body) = exchange(serving.addr(), "GET", "/t/acme/healthz", &oversize)?;
    assert_eq!(status, 413, "one byte past 1 MiB is 413: {body}");
    assert_eq!(
        get(serving.addr(), "/t/acme/healthz")?.0,
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
fn a_taken_port_is_refused_with_its_address() -> TestResult {
    let holder = TcpListener::bind("127.0.0.1:0")?;
    let taken = holder.local_addr()?.to_string();
    let refused = Serving::start(&[("LARRY_SERVE_ADDR", taken.as_str())]);
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
