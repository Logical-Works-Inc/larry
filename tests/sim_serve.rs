//! Seeded simulation of `larry serve`'s edge answers (larry#88) behind per-tenant
//! bearer keys (larry#89).
//!
//! Source: RFC 9110 sections 15.5.1, 15.5.5 and 15.5.6 (`400`, `404`, `405`),
//! RFC 6750 section 3 (`401` for a missing or wrong bearer token), the
//! lgwks_bot app edge's documented order (tenant name, then the authenticator,
//! then the verb, then the body, crates#880) and refusals, and the estate's
//! seeded-simulation rule (one seed, same seed same trace hash). Requirement
//! proved: across every request family the seed draws, including the 1 and 64
//! byte tenant boundaries and one past them, no key, another tenant's key and a
//! tampered key, the shipped binary answers exactly what an independent
//! reference answers; the same seed on a fresh host and store gives the same
//! trace hash; and many keyed tenants at once, two of them on one path, are each
//! answered as themselves with no cross-talk.

#[path = "support/serve_cli.rs"]
mod serve_cli;

use std::error::Error;
use std::path::Path;
use std::sync::Barrier;

use lgwks_std::hash::Hasher;
use lgwks_std::seeded::Seeded;
use serve_cli::{Serving, exchange, fresh_state_dir, get, issue_key, probe_body};

/// What every test here returns.
type TestResult = Result<(), Box<dyn Error>>;

/// The seed the sequential passes draw from, identical in both.
const SIM_SEED: u64 = 0x5EED_0089;

/// Requests one sequential pass sends.
const CASES: usize = 384;

/// Keyed tenants one sequential pass draws from, the two boundary names among them.
const KEYED: usize = 6;

/// The seeds the concurrent case sweeps.
const CONCURRENT_SEEDS: [u64; 4] = [1, 2, 0x5EED_0089, 0xDEAD_BEEF];

/// Threads sending at once: 100, each sending [`PER_THREAD`] probes, so 1,000
/// probes per seed.
const THREADS: usize = 100;

/// Probes each thread sends.
const PER_THREAD: usize = 10;

/// Distinct keyed tenants the concurrent probes spread over.
const TENANTS: usize = 64;

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

/// The edge's answer to a request with no valid key for its tenant.
const UNKNOWN: &str = "name yourself";

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
    /// The bearer key sent, if any.
    key: Option<String>,
    /// The expected status.
    status: u16,
    /// The expected body.
    answer: String,
}

/// A tenant with its current key.
struct Keyed {
    /// The tenant name.
    name: String,
    /// Its key.
    key: String,
}

/// A byte of `alphabet`, drawn.
fn pick(rng: &mut Seeded, alphabet: &[u8]) -> Result<u8, Box<dyn Error>> {
    Ok(*alphabet
        .get(rng.index(alphabet.len())?)
        .ok_or("an empty alphabet")?)
}

/// A valid tenant name of `len` bytes.
fn tenant(rng: &mut Seeded, len: usize) -> Result<String, Box<dyn Error>> {
    let mut name = Vec::with_capacity(len);
    for _ in 0..len {
        name.push(pick(rng, TENANT_BYTES)?);
    }
    Ok(String::from_utf8(name)?)
}

/// One of `items`, drawn.
fn choose<T: Copy>(rng: &mut Seeded, items: &[T]) -> Result<T, Box<dyn Error>> {
    Ok(*items.get(rng.index(items.len())?).ok_or("no items")?)
}

/// One of `items` by reference, drawn.
fn choose_ref<'item, T>(rng: &mut Seeded, items: &'item [T]) -> Result<&'item T, Box<dyn Error>> {
    Ok(items.get(rng.index(items.len())?).ok_or("no items")?)
}

/// Keys for [`KEYED`] tenants drawn from `rng` in `state`: the first is 1 byte,
/// the second 64, the rest any valid length.
fn key_tenants(rng: &mut Seeded, state: &Path) -> Result<Vec<Keyed>, Box<dyn Error>> {
    let mut keyed: Vec<Keyed> = Vec::with_capacity(KEYED);
    for index in 0..KEYED {
        let len = match index {
            0 => 1,
            1 => TENANT_MAX,
            _ => rng.index(TENANT_MAX)?.saturating_add(1),
        };
        let name = tenant(rng, len)?;
        if keyed.iter().any(|seen| seen.name == name) {
            continue;
        }
        let key = issue_key(state, &name)?;
        keyed.push(Keyed { name, key });
    }
    Ok(keyed)
}

/// A case of `method`, `path`, `body` and `key` the reference expects `status` and `answer` for.
fn case(
    method: &'static str,
    path: String,
    body: &[u8],
    key: Option<&str>,
    status: u16,
    answer: &str,
) -> Case {
    Case {
        method,
        path,
        body: body.to_vec(),
        key: key.map(String::from),
        status,
        answer: String::from(answer),
    }
}

/// One request from the seed, with the reference answer: the rule the edge
/// documents, written out here rather than read from the binary.
fn draw(rng: &mut Seeded, keyed: &[Keyed]) -> Result<Case, Box<dyn Error>> {
    let probe = probe_body();
    let own = choose_ref(rng, keyed)?;
    let (name, key) = (own.name.as_str(), Some(own.key.as_str()));
    let any_key = if rng.index(2)? == 0 { None } else { key };
    Ok(match rng.index(13)? {
        0 => case("GET", format!("/t/{name}/healthz"), b"", key, 200, &probe),
        1 => {
            let past = TENANT_MAX.saturating_add(1).saturating_add(rng.index(16)?);
            let long = tenant(rng, past)?;
            case(
                "GET",
                format!("/t/{long}/healthz"),
                b"",
                any_key,
                400,
                BAD_TENANT,
            )
        }
        2 => {
            let mut bytes = name.as_bytes().to_vec();
            let at = rng.index(bytes.len())?;
            let foreign = pick(rng, FOREIGN_BYTES)?;
            if let Some(byte) = bytes.get_mut(at) {
                *byte = foreign;
            }
            let bad = String::from_utf8(bytes)?;
            case(
                "GET",
                format!("/t/{bad}/healthz"),
                b"",
                any_key,
                400,
                BAD_TENANT,
            )
        }
        3 => case(
            "GET",
            String::from("/t//healthz"),
            b"",
            any_key,
            400,
            BAD_TENANT,
        ),
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
            case("GET", String::from(path), b"", any_key, 400, NO_TENANT)
        }
        5 => {
            let rest = choose(
                rng,
                &["", "health", "healthz/", "HEALTHZ", "readyz", "whoami/x"],
            )?;
            case("GET", format!("/t/{name}/{rest}"), b"", key, 404, NO_ROUTE)
        }
        6 => case(
            "POST",
            format!("/t/{name}/healthz"),
            b"",
            key,
            404,
            NO_ROUTE,
        ),
        7 => {
            let verb = choose(rng, &["PUT", "DELETE", "PATCH", "OPTIONS"])?;
            case(verb, format!("/t/{name}/healthz"), b"", key, 405, BAD_VERB)
        }
        8 => {
            let body = choose(rng, &["null", "{}", "0", "[]", "\"\"", "true", "{"])?;
            let path = format!("/t/{name}/healthz");
            match body {
                "null" => case("GET", path, body.as_bytes(), key, 200, &probe),
                _ => case("GET", path, body.as_bytes(), key, 400, BAD_BODY),
            }
        }
        9 => match choose(rng, &["GET", "HEAD"])? {
            "HEAD" => case("HEAD", String::from("/healthz"), b"", any_key, 200, ""),
            verb => case(verb, String::from("/healthz"), b"", any_key, 200, LIVE),
        },
        10 => {
            let other = choose_ref(rng, keyed)?;
            let tampered = {
                let mut bytes = own.key.clone().into_bytes();
                let at = rng.index(bytes.len())?;
                if let Some(byte) = bytes.get_mut(at) {
                    *byte = if *byte == b'0' { b'1' } else { b'0' };
                }
                String::from_utf8(bytes)?
            };
            let sent = match rng.index(3)? {
                0 => None,
                1 if other.name != own.name => Some(other.key.clone()),
                _ => Some(tampered),
            };
            let verb = choose(rng, &["GET", "POST", "PUT"])?;
            case(
                verb,
                format!("/t/{name}/healthz"),
                b"",
                sent.as_deref(),
                401,
                UNKNOWN,
            )
        }
        11 => {
            let stranger = format!("x{}", tenant(rng, 8)?);
            case(
                "GET",
                format!("/t/{stranger}/healthz"),
                b"",
                key,
                401,
                UNKNOWN,
            )
        }
        12 => {
            let answer = format!("{{\"principal\":\"{name}\"}}");
            case("GET", format!("/t/{name}/whoami"), b"", key, 200, &answer)
        }
        family => return Err(format!("no request family {family}").into()),
    })
}

/// One pass on a fresh host over a fresh store: every drawn request checked
/// against its reference answer, and the trace hash of what was sent and
/// answered. Keys are random, so the trace hashes the request with its key
/// slot named, not the key's bytes.
fn run_pass(seed: u64) -> Result<String, Box<dyn Error>> {
    let state = fresh_state_dir();
    let mut rng = Seeded::from_seed(seed);
    let keyed = key_tenants(&mut rng, &state)?;
    let dir = state.display().to_string();
    let mut serving = Serving::start(&[
        ("LARRY_SERVE_ADDR", "127.0.0.1:0"),
        ("LARRY_STATE_DIR", dir.as_str()),
    ])?;
    let mut trace = Hasher::new();
    for index in 0..CASES {
        let drawn = draw(&mut rng, &keyed)?;
        let sent = drawn.key.as_deref();
        let (status, answer) =
            exchange(serving.addr(), drawn.method, &drawn.path, &drawn.body, sent)?;
        assert_eq!(
            (status, answer.as_str()),
            (drawn.status, drawn.answer.as_str()),
            "seed {seed:#x} case {index}: {} {} {:?} keyed {}",
            drawn.method,
            drawn.path,
            String::from_utf8_lossy(&drawn.body),
            sent.is_some()
        );
        let slot = match sent {
            None => "none",
            Some(token) if keyed.iter().any(|own| own.key == token) => "issued",
            Some(_) => "forged",
        };
        trace.write_framed(drawn.method.as_bytes());
        trace.write_framed(drawn.path.as_bytes());
        trace.write_framed(&drawn.body);
        trace.write_framed(slot.as_bytes());
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
        "the same seed on a fresh host and store gives the same trace"
    );
    assert_ne!(
        run_pass(SIM_SEED.wrapping_add(1))?,
        first,
        "another seed draws another trace"
    );
    Ok(())
}

/// One sender of the concurrent case: [`PER_THREAD`] probes of keyed tenants
/// drawn from `seed`, each of which must answer as its own tenant.
fn send_probes(addr: std::net::SocketAddr, seed: u64, keyed: &[Keyed]) -> Result<usize, String> {
    let mut rng = Seeded::from_seed(seed);
    for _ in 0..PER_THREAD {
        let own = choose_ref(&mut rng, keyed).map_err(|error| error.to_string())?;
        let path = format!("/t/{}/whoami", own.name);
        let answered = get(addr, &path, Some(&own.key)).map_err(|error| error.to_string())?;
        let expected = (200, format!("{{\"principal\":\"{}\"}}", own.name));
        if answered != expected {
            return Err(format!("seed {seed:#x} {path}: {answered:?}"));
        }
    }
    Ok(PER_THREAD)
}

#[test]
fn sim_serve_concurrent_tenants_are_all_answered_for_every_seed() -> TestResult {
    let state = fresh_state_dir();
    let mut keyed = Vec::with_capacity(TENANTS);
    for index in 0..TENANTS {
        let name = format!("tenant-{index}");
        let key = issue_key(&state, &name)?;
        keyed.push(Keyed { name, key });
    }
    let dir = state.display().to_string();
    let mut serving = Serving::start(&[
        ("LARRY_SERVE_ADDR", "127.0.0.1:0"),
        ("LARRY_STATE_DIR", dir.as_str()),
    ])?;
    let addr = serving.addr();
    for seed in CONCURRENT_SEEDS {
        std::thread::scope(|scope| -> TestResult {
            let mut senders = Vec::with_capacity(THREADS);
            for thread in (0_u64..).take(THREADS) {
                let keyed = keyed.as_slice();
                senders.push(scope.spawn(move || send_probes(addr, seed ^ thread, keyed)));
            }
            let mut answered = 0_usize;
            for sender in senders {
                answered = answered
                    .saturating_add(sender.join().map_err(|_panicked| "a sender panicked")??);
            }
            assert_eq!(
                answered,
                THREADS.saturating_mul(PER_THREAD),
                "seed {seed:#x}: every probe answered as its own tenant"
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
fn sim_serve_two_identities_on_one_path_at_once_each_see_only_themselves() -> TestResult {
    let state = fresh_state_dir();
    let alpha = issue_key(&state, "alpha")?;
    let beta = issue_key(&state, "beta")?;
    let dir = state.display().to_string();
    let mut serving = Serving::start(&[
        ("LARRY_SERVE_ADDR", "127.0.0.1:0"),
        ("LARRY_STATE_DIR", dir.as_str()),
    ])?;
    let addr = serving.addr();
    let pairs = 32_usize;
    let gate = Barrier::new(pairs.saturating_mul(4));
    std::thread::scope(|scope| -> TestResult {
        let mut senders = Vec::with_capacity(pairs.saturating_mul(4));
        for _ in 0..pairs {
            for (tenant, key, expected) in [
                (
                    "alpha",
                    alpha.as_str(),
                    (200, String::from("{\"principal\":\"alpha\"}")),
                ),
                (
                    "beta",
                    beta.as_str(),
                    (200, String::from("{\"principal\":\"beta\"}")),
                ),
                ("alpha", beta.as_str(), (401, String::from(UNKNOWN))),
                ("beta", alpha.as_str(), (401, String::from(UNKNOWN))),
            ] {
                let gate = &gate;
                senders.push(scope.spawn(move || {
                    gate.wait();
                    get(addr, &format!("/t/{tenant}/whoami"), Some(key))
                        .map(|answered| (answered, expected))
                        .map_err(|error| error.to_string())
                }));
            }
        }
        for sender in senders {
            let (answered, expected) = sender.join().map_err(|_panicked| "a sender panicked")??;
            assert_eq!(answered, expected, "each identity sees only itself");
        }
        Ok(())
    })?;
    let (status, stderr) = serving.signal_and_wait("TERM")?;
    assert!(status.success(), "the host stops clean: {stderr}");
    Ok(())
}
