//! Seeded simulation of `larry serve`'s edge answers (larry#88).
//!
//! Source: RFC 9110 sections 15.5.1, 15.5.5 and 15.5.6 (`400`, `404`, `405`),
//! the lgwks_bot app edge's documented refusals (a path without `/t/{tenant}/`,
//! a tenant outside 1..=64 bytes of `[A-Za-z0-9_-]`, a verb other than GET or
//! POST, a body that does not decode as the route's input) and the estate's
//! seeded-simulation rule (one seed, same seed same trace hash). Requirement
//! proved: across every request family the seed draws, including the 1 and 64
//! byte tenant boundaries and one past them, the shipped binary answers exactly
//! what an independent reference answers; the same seed on a fresh host gives
//! the same trace hash; and many tenants at once, two of them on one path, are
//! each answered with no cross-talk.

#[path = "support/serve_cli.rs"]
mod serve_cli;

use std::error::Error;
use std::sync::Barrier;

use lgwks_std::hash::Hasher;
use lgwks_std::seeded::Seeded;
use serve_cli::{Serving, exchange, get, probe_body};

/// What every test here returns.
type TestResult = Result<(), Box<dyn Error>>;

/// The seed the sequential passes draw from, identical in both.
const SIM_SEED: u64 = 0x5EED_0088;

/// Requests one sequential pass sends.
const CASES: usize = 384;

/// The seeds the concurrent case sweeps.
const CONCURRENT_SEEDS: [u64; 4] = [1, 2, 0x5EED_0088, 0xDEAD_BEEF];

/// Threads sending at once: 100, each sending [`PER_THREAD`] probes, so 1,000
/// probes per seed.
const THREADS: usize = 100;

/// Probes each thread sends.
const PER_THREAD: usize = 10;

/// Distinct tenants the concurrent probes spread over.
const TENANTS: u64 = 64;

/// Bytes a tenant name may hold.
const TENANT_MAX: usize = 64;

/// Every byte a tenant name may hold.
const TENANT_BYTES: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";

/// Bytes that are legal in a path and never in a tenant name.
const FOREIGN_BYTES: &[u8] = b"!~.+=,@$*";

/// The edge's own tenant-free liveness answer to `GET /healthz` (crates#879).
const LIVE: &str = "ok";

/// The edge's answer to a path naming no tenant.
const NO_TENANT: &str = "name a tenant: /t/{tenant}/...";

/// The edge's answer to a tenant that is not a valid name.
const BAD_TENANT: &str = "the tenant is not a valid name";

/// The app's answer to a path or verb with no route.
const NO_ROUTE: &str = "no such route";

/// The app's answer to a verb other than GET or POST.
const BAD_VERB: &str = "an app route answers GET or POST";

/// The app's answer to a body that does not decode as the route's input.
const BAD_BODY: &str = "the body does not decode as this route's input";

/// One drawn request and the answer the reference expects.
struct Case {
    /// The request's verb.
    method: &'static str,
    /// The request target.
    path: String,
    /// The request body.
    body: Vec<u8>,
    /// The expected status.
    status: u16,
    /// The expected body.
    answer: String,
}

/// A byte of `alphabet`, drawn.
fn pick(rng: &mut Seeded, alphabet: &[u8]) -> Result<u8, Box<dyn Error>> {
    Ok(*alphabet
        .get(rng.index(alphabet.len())?)
        .ok_or("an empty alphabet")?)
}

/// A valid tenant of `len` bytes.
fn tenant(rng: &mut Seeded, len: usize) -> Result<String, Box<dyn Error>> {
    let mut name = Vec::with_capacity(len);
    for _ in 0..len {
        name.push(pick(rng, TENANT_BYTES)?);
    }
    Ok(String::from_utf8(name)?)
}

/// A valid tenant length: the 1 and 64 byte boundaries one draw in four each.
fn valid_len(rng: &mut Seeded) -> Result<usize, Box<dyn Error>> {
    Ok(match rng.index(4)? {
        0 => 1,
        1 => TENANT_MAX,
        _ => rng.index(TENANT_MAX)?.saturating_add(1),
    })
}

/// One of `items`, drawn.
fn choose<T: Copy>(rng: &mut Seeded, items: &[T]) -> Result<T, Box<dyn Error>> {
    Ok(*items.get(rng.index(items.len())?).ok_or("no items")?)
}

/// A case of `method`, `path` and `body` the reference expects `status` and `answer` for.
fn case(method: &'static str, path: String, body: &[u8], status: u16, answer: &str) -> Case {
    Case {
        method,
        path,
        body: body.to_vec(),
        status,
        answer: String::from(answer),
    }
}

/// One request from the seed, with the reference answer: the rule the edge
/// documents, written out here rather than read from the binary.
fn draw(rng: &mut Seeded) -> Result<Case, Box<dyn Error>> {
    let probe = probe_body();
    let name = {
        let len = valid_len(rng)?;
        tenant(rng, len)?
    };
    Ok(match rng.index(10)? {
        0 => case("GET", format!("/t/{name}/healthz"), b"", 200, &probe),
        1 => {
            let past = TENANT_MAX.saturating_add(1).saturating_add(rng.index(16)?);
            let long = tenant(rng, past)?;
            case("GET", format!("/t/{long}/healthz"), b"", 400, BAD_TENANT)
        }
        2 => {
            let mut bytes = name.into_bytes();
            let at = rng.index(bytes.len())?;
            let foreign = pick(rng, FOREIGN_BYTES)?;
            if let Some(byte) = bytes.get_mut(at) {
                *byte = foreign;
            }
            let bad = String::from_utf8(bytes)?;
            case("GET", format!("/t/{bad}/healthz"), b"", 400, BAD_TENANT)
        }
        3 => case("GET", String::from("/t//healthz"), b"", 400, BAD_TENANT),
        4 => {
            let path = choose(
                rng,
                &[
                    "/healthz/",
                    "/",
                    "/t",
                    "/tt/acme/healthz",
                    "/T/acme/healthz",
                ],
            )?;
            case("GET", String::from(path), b"", 400, NO_TENANT)
        }
        9 => match choose(rng, &["GET", "HEAD"])? {
            "HEAD" => case("HEAD", String::from("/healthz"), b"", 200, ""),
            verb => case(verb, String::from("/healthz"), b"", 200, LIVE),
        },
        5 => {
            let rest = choose(
                rng,
                &["", "health", "healthz/", "HEALTHZ", "readyz", "healthz/x"],
            )?;
            case("GET", format!("/t/{name}/{rest}"), b"", 404, NO_ROUTE)
        }
        6 => case("POST", format!("/t/{name}/healthz"), b"", 404, NO_ROUTE),
        7 => {
            let verb = choose(rng, &["PUT", "DELETE", "PATCH", "OPTIONS"])?;
            case(verb, format!("/t/{name}/healthz"), b"", 405, BAD_VERB)
        }
        8 => {
            let body = choose(rng, &["null", "{}", "0", "[]", "\"\"", "true", "{"])?;
            let path = format!("/t/{name}/healthz");
            match body {
                "null" => case("GET", path, body.as_bytes(), 200, &probe),
                _ => case("GET", path, body.as_bytes(), 400, BAD_BODY),
            }
        }
        family => return Err(format!("no request family {family}").into()),
    })
}

/// One pass on a fresh host: every drawn request checked against its
/// reference answer, and the trace hash of what was sent and answered.
fn run_pass(seed: u64) -> Result<String, Box<dyn Error>> {
    let mut serving = Serving::start(&[("LARRY_SERVE_ADDR", "127.0.0.1:0")])?;
    let mut rng = Seeded::from_seed(seed);
    let mut trace = Hasher::new();
    for index in 0..CASES {
        let drawn = draw(&mut rng)?;
        let (status, answer) = exchange(serving.addr(), drawn.method, &drawn.path, &drawn.body)?;
        assert_eq!(
            (status, answer.as_str()),
            (drawn.status, drawn.answer.as_str()),
            "seed {seed:#x} case {index}: {} {} {:?}",
            drawn.method,
            drawn.path,
            String::from_utf8_lossy(&drawn.body)
        );
        trace.write_framed(drawn.method.as_bytes());
        trace.write_framed(drawn.path.as_bytes());
        trace.write_framed(&drawn.body);
        trace.write_framed(&status.to_be_bytes());
        trace.write_framed(answer.as_bytes());
    }
    let (status, stderr) = serving.signal_and_wait("TERM")?;
    assert!(status.success(), "the pass's host stops clean: {stderr}");
    Ok(lgwks_std::hex::encode(trace.finalize().as_bytes()))
}

#[test]
fn sim_serve_every_request_gets_the_reference_answer_for_every_seed() -> TestResult {
    let first = run_pass(SIM_SEED)?;
    let second = run_pass(SIM_SEED)?;
    assert_eq!(
        first, second,
        "the same seed on a fresh host gives the same trace"
    );
    assert_ne!(
        run_pass(SIM_SEED.wrapping_add(1))?,
        first,
        "another seed draws another trace"
    );
    Ok(())
}

/// One sender of the concurrent case: [`PER_THREAD`] probes of tenants drawn
/// from `seed`, each of which must answer the probe body.
fn send_probes(addr: std::net::SocketAddr, seed: u64, probe: &str) -> Result<usize, String> {
    let mut rng = Seeded::from_seed(seed);
    for _ in 0..PER_THREAD {
        let tenant = rng.below(TENANTS).map_err(|error| error.to_string())?;
        let path = format!("/t/tenant-{tenant}/healthz");
        let answered = get(addr, &path).map_err(|error| error.to_string())?;
        if answered != (200, String::from(probe)) {
            return Err(format!("seed {seed:#x} {path}: {answered:?}"));
        }
    }
    Ok(PER_THREAD)
}

#[test]
fn sim_serve_concurrent_tenants_are_all_answered_for_every_seed() -> TestResult {
    let mut serving = Serving::start(&[("LARRY_SERVE_ADDR", "127.0.0.1:0")])?;
    let addr = serving.addr();
    let probe = probe_body();
    for seed in CONCURRENT_SEEDS {
        std::thread::scope(|scope| -> TestResult {
            let mut senders = Vec::with_capacity(THREADS);
            for thread in (0_u64..).take(THREADS) {
                let probe = probe.as_str();
                senders.push(scope.spawn(move || send_probes(addr, seed ^ thread, probe)));
            }
            let mut answered = 0_usize;
            for sender in senders {
                answered = answered
                    .saturating_add(sender.join().map_err(|_panicked| "a sender panicked")??);
            }
            assert_eq!(
                answered,
                THREADS.saturating_mul(PER_THREAD),
                "seed {seed:#x}: every probe answered"
            );
            Ok(())
        })?;
    }
    let (status, stderr) = serving.signal_and_wait("TERM")?;
    assert!(
        status.success(),
        "the host stops clean after load: {stderr}"
    );
    Ok(())
}

#[test]
fn sim_serve_two_tenants_on_one_path_at_once_are_each_answered() -> TestResult {
    let mut serving = Serving::start(&[("LARRY_SERVE_ADDR", "127.0.0.1:0")])?;
    let addr = serving.addr();
    let probe = probe_body();
    let pairs = 32_usize;
    let gate = Barrier::new(pairs.saturating_mul(2));
    std::thread::scope(|scope| -> TestResult {
        let mut senders = Vec::with_capacity(pairs.saturating_mul(2));
        for _ in 0..pairs {
            for tenant in ["alpha", "beta"] {
                let gate = &gate;
                senders.push(scope.spawn(move || {
                    gate.wait();
                    get(addr, &format!("/t/{tenant}/healthz")).map_err(|error| error.to_string())
                }));
            }
        }
        for sender in senders {
            let answered = sender.join().map_err(|_panicked| "a sender panicked")??;
            assert_eq!(
                answered,
                (200, probe.clone()),
                "each tenant gets its own answer"
            );
        }
        Ok(())
    })?;
    let (status, stderr) = serving.signal_and_wait("TERM")?;
    assert!(status.success(), "the host stops clean: {stderr}");
    Ok(())
}
