//! Hostile seeded simulations of `larry serve`'s new rules (larry#210, ADR 0040),
//! one seed each against the shipped binary: a non-loopback bind, a revoke and its
//! SIGHUP, a revoke that waits for its signal, a pulled plug after a revoke, the
//! key cap boundary, a store grown past the cap under a running host, and a burst
//! on one tenant beside another. Each sim's reference is the rule the ADR states,
//! written here rather than read from the binary.

#[path = "support/serve_cli.rs"]
mod serve_cli;

use std::error::Error;
use std::net::SocketAddr;
use std::path::Path;

use lgwks_std::hash::Hasher;
use lgwks_std::seeded::Seeded;
use serve_cli::{
    Serving, exchange, fill_key_store, fresh_state_dir, get, issue_key, probe_body, revoke,
};

/// What every test here returns.
type TestResult = Result<(), Box<dyn Error>>;

/// The seeds every sim sweeps.
const SEEDS: [u64; 4] = [1, 2, 0x5EED_0089, 0xDEAD_BEEF];

/// The tenants the revoke sims hold keys for.
const NAMES: [&str; 3] = ["alpha", "beta", "gamma"];

/// The key cap the edge admits, `MAX_TENANT_KEYS`.
const KEY_CAP_ROWS: usize = 65_536;

/// Bind addresses one bind pass draws.
const BIND_CASES: usize = 48;

/// Threads bursting on one tenant at once.
const BURST_THREADS: usize = 64;

/// Requests each burst thread sends.
const BURST_PER_THREAD: usize = 8;

/// The edge's answer to a request with no valid key for its tenant.
const UNKNOWN: &str = "name yourself";

/// A host on `addr` over the store in `state`.
fn start_pinned(addr: &str, state: &Path) -> Result<Serving, Box<dyn Error>> {
    let dir = state.display().to_string();
    Serving::start(&[
        ("LARRY_SERVE_ADDR", addr),
        ("LARRY_STATE_DIR", dir.as_str()),
    ])
}

/// A key issued for each of `NAMES` in `state`, as (name, key) pairs.
fn issue_all(state: &Path) -> Result<Vec<(&'static str, String)>, Box<dyn Error>> {
    let mut keyed = Vec::with_capacity(NAMES.len());
    for name in NAMES {
        keyed.push((name, issue_key(state, name)?));
    }
    Ok(keyed)
}

/// One of `NAMES`, drawn from `rng`.
fn victim_of(rng: &mut Seeded) -> Result<&'static str, Box<dyn Error>> {
    Ok(*NAMES
        .get(rng.index(NAMES.len())?)
        .ok_or("no tenant names")?)
}

/// Probes every keyed tenant: `200` with the probe body, or `401` for `revoked`
/// (pass "" to revoke nobody). Each answer goes into `trace`.
fn check_all(
    addr: SocketAddr,
    keyed: &[(&str, String)],
    revoked: &str,
    seed: u64,
    trace: &mut Hasher,
) -> TestResult {
    for pair in keyed {
        let name = pair.0;
        let (expected, body) = if name == revoked {
            (401, String::from(UNKNOWN))
        } else {
            (200, probe_body())
        };
        let answered = get(addr, &format!("/t/{name}/healthz"), Some(pair.1.as_str()))?;
        assert_eq!(
            answered,
            (expected, body),
            "seed {seed:#x}: {name} answers {expected}"
        );
        trace.write_framed(name.as_bytes());
        trace.write_framed(&answered.0.to_be_bytes());
    }
    Ok(())
}

/// The finished trace as hex.
fn hex(trace: Hasher) -> String {
    lgwks_std::hex::encode(trace.finalize().as_bytes())
}

/// One IPv4 address from `rng` that is not loopback: any first octet but 127, port 0.
fn outside_loopback(rng: &mut Seeded) -> Result<String, Box<dyn Error>> {
    let first = rng.index(255)?;
    let first = u8::try_from(if first >= 127 {
        first.saturating_add(1)
    } else {
        first
    })?;
    let mut octets = vec![first];
    for _ in 0..3 {
        octets.push(u8::try_from(rng.index(256)?)?);
    }
    Ok(format!(
        "{}.{}.{}.{}:0",
        octets[0], octets[1], octets[2], octets[3]
    ))
}

/// One bind pass: each drawn non-loopback address is refused naming itself and the
/// opt-in, before anything binds. Returns the trace hash of the refusals.
fn bind_pass(seed: u64) -> Result<String, Box<dyn Error>> {
    let mut rng = Seeded::from_seed(seed);
    let mut trace = Hasher::new();
    for index in 0..BIND_CASES {
        let addr = outside_loopback(&mut rng)?;
        let message = match Serving::start(&[("LARRY_SERVE_ADDR", addr.as_str())]) {
            Ok(_) => return Err(format!("seed {seed:#x} case {index}: {addr} was served").into()),
            Err(error) => error.to_string(),
        };
        assert!(
            message.contains("exit status: 1")
                && message.contains(&addr)
                && message.contains("LARRY_SERVE_PLAINTEXT=1"),
            "seed {seed:#x} case {index}: {addr} is refused naming itself: {message}"
        );
        trace.write_framed(addr.as_bytes());
    }
    Ok(hex(trace))
}

#[test]
fn sim_a_non_loopback_bind_is_refused_before_binding_for_every_seed() -> TestResult {
    for seed in SEEDS {
        assert_eq!(
            bind_pass(seed)?,
            bind_pass(seed)?,
            "seed {seed:#x}: the same seed draws the same refusals"
        );
    }
    Ok(())
}

/// One revoke pass: a seeded tenant is revoked, a SIGHUP reloads, and its key is
/// refused while every other key still answers. Returns the trace hash.
fn revoke_pass(seed: u64) -> Result<String, Box<dyn Error>> {
    let state = fresh_state_dir();
    let keyed = issue_all(&state)?;
    let victim = victim_of(&mut Seeded::from_seed(seed))?;
    let mut serving = start_pinned("127.0.0.1:0", &state)?;
    let addr = serving.addr();
    check_all(addr, &keyed, "", seed, &mut Hasher::new())?;
    let revoked = revoke(&state, victim)?;
    assert!(
        revoked.status.success(),
        "seed {seed:#x}: the revoke exits 0"
    );
    serving.signal("HUP")?;
    let ack = serving.next_line()?;
    assert!(
        ack.contains("\"reloaded\":true"),
        "seed {seed:#x}: the reload acknowledges: {ack}"
    );
    let mut trace = Hasher::new();
    check_all(addr, &keyed, victim, seed, &mut trace)?;
    let (status, stderr) = serving.signal_and_wait("TERM")?;
    assert!(status.success(), "seed {seed:#x}: stops clean: {stderr}");
    Ok(hex(trace))
}

#[test]
fn sim_revoke_then_sighup_refuses_the_revoked_key_for_every_seed() -> TestResult {
    for seed in SEEDS {
        assert_eq!(
            revoke_pass(seed)?,
            revoke_pass(seed)?,
            "seed {seed:#x}: the same seed gives the same revoke trace"
        );
    }
    Ok(())
}

/// The wrong order: a revoke written to the store is not live until the SIGHUP
/// that reads it. Returns the trace hash of the answers after the signal.
fn wrong_order_pass(seed: u64) -> Result<String, Box<dyn Error>> {
    let state = fresh_state_dir();
    let keyed = issue_all(&state)?;
    let victim = victim_of(&mut Seeded::from_seed(seed))?;
    let mut serving = start_pinned("127.0.0.1:0", &state)?;
    let addr = serving.addr();
    let revoked = revoke(&state, victim)?;
    assert!(
        revoked.status.success(),
        "seed {seed:#x}: the revoke exits 0"
    );
    check_all(addr, &keyed, "", seed, &mut Hasher::new())?;
    serving.signal("HUP")?;
    assert!(
        serving.next_line()?.contains("\"reloaded\":true"),
        "seed {seed:#x}: the reload acknowledges"
    );
    let mut trace = Hasher::new();
    check_all(addr, &keyed, victim, seed, &mut trace)?;
    let (status, stderr) = serving.signal_and_wait("TERM")?;
    assert!(status.success(), "seed {seed:#x}: stops clean: {stderr}");
    Ok(hex(trace))
}

#[test]
fn sim_a_revoke_is_not_live_before_the_sighup_for_every_seed() -> TestResult {
    for seed in SEEDS {
        assert_eq!(
            wrong_order_pass(seed)?,
            wrong_order_pass(seed)?,
            "seed {seed:#x}: the same seed gives the same trace"
        );
    }
    Ok(())
}

/// The pulled plug: a seeded tenant is revoked, the host is killed with `kill -9`
/// before any SIGHUP, and the restarted host must still refuse it. Returns the
/// trace hash of the restarted host's answers.
fn pulled_plug_pass(seed: u64) -> Result<String, Box<dyn Error>> {
    let state = fresh_state_dir();
    let keyed = issue_all(&state)?;
    let victim = victim_of(&mut Seeded::from_seed(seed))?;
    let mut first = start_pinned("127.0.0.1:0", &state)?;
    let addr = first.addr();
    let revoked = revoke(&state, victim)?;
    assert!(
        revoked.status.success(),
        "seed {seed:#x}: the revoke exits 0"
    );
    let (status, _) = first.signal_and_wait("KILL")?;
    assert!(
        !status.success(),
        "seed {seed:#x}: kill -9 is not a clean exit"
    );
    let mut second = start_pinned(&addr.to_string(), &state)?;
    let mut trace = Hasher::new();
    check_all(second.addr(), &keyed, victim, seed, &mut trace)?;
    let (status, stderr) = second.signal_and_wait("TERM")?;
    assert!(
        status.success(),
        "seed {seed:#x}: the restart stops clean: {stderr}"
    );
    Ok(hex(trace))
}

#[test]
fn sim_a_pulled_plug_after_a_revoke_keeps_the_revoke_for_every_seed() -> TestResult {
    for seed in SEEDS {
        assert_eq!(
            pulled_plug_pass(seed)?,
            pulled_plug_pass(seed)?,
            "seed {seed:#x}: the same seed gives the same trace"
        );
    }
    Ok(())
}

#[test]
fn sim_key_cap_boundary_serves_at_cap_minus_one_and_cap_and_refuses_cap_plus_one() -> TestResult {
    let state = fresh_state_dir();
    let acme = issue_key(&state, "acme")?;
    fill_key_store(&state, 0, KEY_CAP_ROWS.saturating_sub(2))?;
    for (step, (rows, expect_serving)) in [("cap-1", true), ("cap", true), ("cap+1", false)]
        .into_iter()
        .enumerate()
    {
        match start_pinned("127.0.0.1:0", &state) {
            Ok(mut serving) => {
                assert!(
                    expect_serving,
                    "{rows} rows: the store is past the cap and was served"
                );
                assert_eq!(get(serving.addr(), "/t/acme/whoami", Some(&acme))?.0, 200);
                let (status, stderr) = serving.signal_and_wait("TERM")?;
                assert!(status.success(), "{rows} rows: stops clean: {stderr}");
            }
            Err(error) => {
                assert!(!expect_serving, "{rows} rows: refused at the cap: {error}");
                assert!(
                    error.to_string().contains("65537") && error.to_string().contains("65536"),
                    "{rows} rows: the refusal names the count and the cap: {error}"
                );
            }
        }
        fill_key_store(&state, KEY_CAP_ROWS.saturating_add(step), 1)?;
    }
    Ok(())
}

#[test]
fn sim_a_store_grown_past_the_cap_under_a_running_host_reloads_closed() -> TestResult {
    let state = fresh_state_dir();
    let keyed = issue_all(&state)?;
    let mut serving = start_pinned("127.0.0.1:0", &state)?;
    let addr = serving.addr();
    check_all(addr, &keyed, "", 0, &mut Hasher::new())?;
    fill_key_store(&state, 0, KEY_CAP_ROWS)?;
    serving.signal("HUP")?;
    let ack = serving.next_line()?;
    assert!(
        ack.contains("\"reloaded\":false") && ack.contains("65539"),
        "the reload refuses the grown store, naming its count: {ack}"
    );
    for pair in &keyed {
        let answered = get(
            addr,
            &format!("/t/{}/healthz", pair.0),
            Some(pair.1.as_str()),
        )?;
        assert_eq!(
            answered.0, 401,
            "fail closed: {} is refused after a refused reload",
            pair.0
        );
    }
    let (status, stderr) = serving.signal_and_wait("TERM")?;
    assert!(
        status.success(),
        "stops clean after a refused reload: {stderr}"
    );
    Ok(())
}

#[test]
fn sim_a_burst_on_one_tenant_neither_starves_nor_leaks_into_another() -> TestResult {
    let state = fresh_state_dir();
    let alpha = issue_key(&state, "alpha")?;
    let beta = issue_key(&state, "beta")?;
    let mut serving = start_pinned("127.0.0.1:0", &state)?;
    let addr = serving.addr();
    let alpha_answer = (200, String::from("{\"principal\":\"alpha\"}"));
    let beta_answer = (200, String::from("{\"principal\":\"beta\"}"));
    std::thread::scope(|scope| -> TestResult {
        let mut burst = Vec::with_capacity(BURST_THREADS);
        for _ in 0..BURST_THREADS {
            let key = alpha.as_str();
            let expected = alpha_answer.clone();
            burst.push(scope.spawn(move || -> Result<(), String> {
                for _ in 0..BURST_PER_THREAD {
                    let answered = exchange(addr, "GET", "/t/alpha/whoami", b"", Some(key))
                        .map_err(|error| error.to_string())?;
                    if answered != expected {
                        return Err(format!("alpha burst answered {answered:?}"));
                    }
                }
                Ok(())
            }));
        }
        for _ in 0..BURST_THREADS {
            let answered = exchange(addr, "GET", "/t/beta/whoami", b"", Some(&beta))?;
            assert_eq!(
                answered, beta_answer,
                "beta is answered during alpha's burst"
            );
        }
        for sender in burst {
            sender
                .join()
                .map_err(|_panicked| "a burst thread panicked")??;
        }
        Ok(())
    })?;
    let revoked = revoke(&state, "alpha")?;
    assert!(revoked.status.success(), "the revoke of alpha exits 0");
    serving.signal("HUP")?;
    assert!(
        serving.next_line()?.contains("\"reloaded\":true"),
        "the reload acknowledges"
    );
    assert_eq!(
        get(addr, "/t/alpha/whoami", Some(&alpha))?.0,
        401,
        "alpha is revoked"
    );
    assert_eq!(
        get(addr, "/t/beta/whoami", Some(&beta))?,
        beta_answer,
        "beta still answers"
    );
    let (status, stderr) = serving.signal_and_wait("TERM")?;
    assert!(status.success(), "the burst host stops clean: {stderr}");
    Ok(())
}
