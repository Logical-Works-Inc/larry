//! `larry serve`: the HTTP host on lgwks_bot's app edge (larry#88, ADR 0021),
//! with a bearer key per service tenant (larry#89, ADR 0022), a revoke that
//! lands on SIGHUP, a plaintext bind refused unless opted in, and a key cap
//! (larry#210, ADR 0040).
//!
//! Larry builds one [`App`] and serves it with `App::serve`, so the estate's
//! edge owns the framing, the `/t/{tenant}/...` prefix, the body ceiling, the
//! mailboxes and their refusals; Larry owns the routes, the keys, the address and
//! the stop. The authenticator does no I/O, so it reads a snapshot of the
//! digests; the signal watcher re-reads the store on SIGHUP and swaps it. The
//! edge has no TLS, so a non-loopback bind needs `LARRY_SERVE_PLAINTEXT=1`.

mod error;

use std::any::Any;
use std::collections::BTreeMap;
use std::io::{self, Write as _};
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, RwLock};

use lgwks_bot::app::{App, Effect, Method, Principal, RequestHeaders};
use lgwks_bot::rt::edge::EdgeStop;
use lgwks_bot::rt::sync::CancellationToken;
use lgwks_std::hash::Digest;

use crate::executor::{ExecError, state_ledger};

pub use error::ServeError;

/// The variable naming the address `larry serve` binds.
const ADDR_VAR: &str = "LARRY_SERVE_ADDR";

/// The variable that opts a non-loopback bind into plaintext HTTP; it must be
/// exactly [`PLAINTEXT_OPT_IN`] when set.
const PLAINTEXT_VAR: &str = "LARRY_SERVE_PLAINTEXT";

/// The one value of [`PLAINTEXT_VAR`] that opts in.
const PLAINTEXT_OPT_IN: &str = "1";

/// The address bound when [`ADDR_VAR`] is unset: loopback only.
const DEFAULT_ADDR: &str = "127.0.0.1:8787";

/// The probe's path below the tenant prefix.
pub const HEALTH_PATH: &str = "/healthz";

/// The path that answers who a key names.
const WHOAMI_PATH: &str = "/whoami";

/// The app's name on the edge.
const APP_NAME: &str = "larry";

/// The mark every issued key starts with, so a leaked one is recognisable.
const KEY_PREFIX: &str = "lk_";

/// Random bytes in a key: 256 bits.
const KEY_BYTES: usize = 32;

/// Tenants with a key: the edge's own tenant ceiling.
const MAX_TENANT_KEYS: u32 = 65_536;

/// A tenant name is 1..=64 bytes, as the edge reads it.
const MAX_TENANT_BYTES: usize = 64;

/// The probe's fixed body: no clock, no counter, nothing a tenant could read
/// of another.
const HEALTH_BODY: &str = concat!(
    "{\"status\":\"ok\",\"service\":\"larry\",\"version\":\"",
    env!("CARGO_PKG_VERSION"),
    "\"}"
);

/// Where `larry serve` listens, the key digests it admits and the state
/// directory it reloads them from.
#[derive(Debug)]
pub struct Settings {
    /// The address to bind; port 0 asks the system for a free one.
    addr: SocketAddr,
    /// Each keyed tenant's key digest, read at start and on each SIGHUP.
    keys: Keys,
    /// `LARRY_STATE_DIR`, the directory a reload reads the store from.
    state: String,
}

impl Settings {
    /// Settings from `read` (the process environment in the binary): [`ADDR_VAR`]
    /// when set, else [`DEFAULT_ADDR`]; a non-loopback address only with
    /// [`PLAINTEXT_VAR`] exactly [`PLAINTEXT_OPT_IN`]; then the key digests in
    /// `LARRY_STATE_DIR`'s store.
    ///
    /// # Errors
    ///
    /// [`ServeError::Address`], [`ServeError::PlaintextFlag`],
    /// [`ServeError::Plaintext`], [`ServeError::Store`] (unset or failed state
    /// directory), [`ServeError::KeyCap`] past the cap, and [`ServeError::Digest`]
    /// for a damaged row.
    pub fn from_env(read: impl Fn(&str) -> Option<String>) -> Result<Self, ServeError> {
        let value = match read(ADDR_VAR) {
            Some(set) => set,
            None => String::from(DEFAULT_ADDR),
        };
        let addr = match value.trim().parse::<SocketAddr>() {
            Ok(addr) => addr,
            Err(cause) => return Err(ServeError::Address { value, cause }),
        };
        match read(PLAINTEXT_VAR) {
            Some(flag) if flag == PLAINTEXT_OPT_IN => {}
            Some(flag) => return Err(ServeError::PlaintextFlag { value: flag }),
            None if addr.ip().is_loopback() => {}
            None => return Err(ServeError::Plaintext { addr }),
        }
        let state = read("LARRY_STATE_DIR")
            .filter(|value| !value.is_empty())
            .ok_or(ServeError::Store(ExecError::StateDirUnset))?;
        let keys = Keys::load(&store_env(&state))?;
        Ok(Self { addr, keys, state })
    }
}

/// A reader that answers only `LARRY_STATE_DIR`, with `state`, so the store opens
/// the same directory at start and on every reload.
fn store_env(state: &str) -> impl Fn(&str) -> Option<String> + '_ {
    move |name: &str| (name == "LARRY_STATE_DIR").then(|| String::from(state))
}

/// Each keyed tenant's key digest, and the fixed digest an unknown tenant is
/// compared with.
#[derive(Debug)]
struct Keys {
    /// Each keyed tenant's digest, in name order.
    digests: BTreeMap<Box<str>, Digest>,
    /// The digest of a fixed token no key holds: an unknown tenant pays the same
    /// one compare as a known tenant with a wrong key.
    dummy: Digest,
}

impl Keys {
    /// The keys over `digests`, with the fixed dummy digest.
    fn from_digests(digests: BTreeMap<Box<str>, Digest>) -> Self {
        Self {
            digests,
            dummy: lgwks_std::hash::blake3(b"larry serve: no tenant holds this key"),
        }
    }

    /// The digests in the store, at most [`MAX_TENANT_KEYS`]. A store past the cap
    /// is refused with its count, never truncated; a count that cannot be read as
    /// `usize` is treated as past the cap.
    fn load(read: &impl Fn(&str) -> Option<String>) -> Result<Self, ServeError> {
        let ledger = state_ledger(read).map_err(ServeError::Store)?;
        let rows = ledger
            .tenant_keys(MAX_TENANT_KEYS.saturating_add(1))
            .map_err(ServeError::Store)?;
        let over_cap = match u64::try_from(rows.len()) {
            Ok(seen) => seen > u64::from(MAX_TENANT_KEYS),
            Err(_) => true,
        };
        if over_cap {
            let count = ledger.tenant_key_count().map_err(ServeError::Store)?;
            return Err(ServeError::KeyCap {
                count,
                cap: MAX_TENANT_KEYS,
            });
        }
        let mut digests = BTreeMap::new();
        for (tenant, hex) in rows {
            let digest = Digest::from_hex(&hex).map_err(|cause| ServeError::Digest {
                tenant: tenant.clone(),
                cause,
            })?;
            digests.insert(tenant.into_boxed_str(), digest);
        }
        Ok(Self::from_digests(digests))
    }

    /// The digest `tenant` is compared with: its own, or the dummy when it holds
    /// no key. Every tenant gets one, so the compare always runs.
    fn stored(&self, tenant: &str) -> &Digest {
        match self.digests.get(tenant) {
            Some(digest) => digest,
            None => &self.dummy,
        }
    }

    /// The principal `token` names for `tenant`: the tenant, when the token hashes
    /// to [`Keys::stored`] in constant time and the tenant holds a key. The compare
    /// runs for an unknown tenant too, so the answer does not show which names hold
    /// keys.
    fn admit(&self, tenant: &str, token: &str) -> Option<Principal> {
        let matched = lgwks_std::hash::blake3(token.as_bytes()).ct_eq(self.stored(tenant));
        if matched && self.digests.contains_key(tenant) {
            Principal::new(tenant)
        } else {
            None
        }
    }
}

/// The edge's judgement of one request against the snapshot. A poisoned snapshot
/// admits no one, so a panicked writer fails closed.
fn judge(snapshot: &RwLock<Keys>, tenant: &str, token: &str) -> Option<Principal> {
    snapshot.read().ok()?.admit(tenant, token)
}

/// Re-reads the store and swaps the snapshot, returning how many tenants hold a
/// key now. A store that cannot be read, or is past the cap, swaps in no keys and
/// returns its cause: the reload fails closed.
fn reload(snapshot: &RwLock<Keys>, state: &str) -> Result<usize, ServeError> {
    let (keys, outcome) = match Keys::load(&store_env(state)) {
        Ok(keys) => (keys, Ok(())),
        Err(cause) => (Keys::from_digests(BTreeMap::new()), Err(cause)),
    };
    let count = keys.digests.len();
    let mut current = snapshot.write().map_err(|poisoned| ServeError::Snapshot {
        cause: poisoned.to_string(),
    })?;
    *current = keys;
    outcome.map(|()| count)
}

/// Refuses a tenant name the edge would refuse, before any store is touched.
fn check_tenant(name: &str) -> Result<(), ServeError> {
    let valid = (1..=MAX_TENANT_BYTES).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
    if valid {
        Ok(())
    } else {
        Err(ServeError::BadTenant {
            value: String::from(name),
        })
    }
}

/// `larry serve key <tenant>`: draw a key for `tenant`, store its digest in the
/// state directory and return the key, which is never stored or shown again.
///
/// # Errors
///
/// [`ServeError::BadTenant`] for a name the edge would refuse,
/// [`ServeError::Entropy`] when the OS has none, and [`ServeError::Store`] when
/// the store fails.
pub fn issue_key(
    read: impl Fn(&str) -> Option<String>,
    tenant: &str,
) -> Result<String, ServeError> {
    check_tenant(tenant)?;
    let secret = lgwks_std::random::bytes::<KEY_BYTES>().map_err(ServeError::Entropy)?;
    let key = format!("{KEY_PREFIX}{}", lgwks_std::hex::encode(secret));
    let digest = lgwks_std::hash::blake3(key.as_bytes()).to_hex();
    state_ledger(&read)
        .and_then(|ledger| ledger.set_tenant_key(tenant, &digest))
        .map_err(ServeError::Store)?;
    Ok(key)
}

/// `larry serve revoke <tenant>`: delete the tenant's key digest from the state
/// directory's store. A running host refuses the key from its next SIGHUP, and a
/// stopped one at its next start.
///
/// # Errors
///
/// [`ServeError::BadTenant`] for a name the edge would refuse,
/// [`ServeError::NoKey`] when the tenant holds no key, and [`ServeError::Store`]
/// when the store fails.
pub fn revoke_key(read: impl Fn(&str) -> Option<String>, tenant: &str) -> Result<(), ServeError> {
    check_tenant(tenant)?;
    let removed = state_ledger(&read)
        .and_then(|ledger| ledger.remove_tenant_key(tenant))
        .map_err(ServeError::Store)?;
    if removed {
        Ok(())
    } else {
        Err(ServeError::NoKey {
            tenant: String::from(tenant),
        })
    }
}

/// The routes Larry serves, each a transition on its tenant's cell, behind the
/// key check.
fn app(snapshot: Arc<RwLock<Keys>>) -> App<()> {
    App::new(APP_NAME)
        .authenticate(move |tenant: &str, headers: &RequestHeaders<'_>| {
            judge(&snapshot, tenant, headers.bearer()?)
        })
        .route(Method::GET, HEALTH_PATH, |_: &mut (), (): ()| {
            vec![Effect::reply(200, HEALTH_BODY.as_bytes().to_vec())]
        })
        .route_as(
            Method::GET,
            WHOAMI_PATH,
            |_: &mut (), who: &Principal, (): ()| {
                let body = lgwks_std::json::json!({ "principal": who.as_str() });
                vec![Effect::reply(200, body.to_string().into_bytes())]
            },
        )
}

/// The one line a reload prints on standard output: how many tenants hold a key
/// now, or why none do.
fn announce_reload(outcome: &Result<usize, ServeError>) -> io::Result<()> {
    let line = match *outcome {
        Ok(count) => lgwks_std::json::json!({ "reloaded": true, "keyed_tenants": count }),
        Err(ref cause) => lgwks_std::json::json!({
            "reloaded": false,
            "keyed_tenants": 0,
            "refused": cause.to_string(),
        }),
    };
    let mut out = io::stdout().lock();
    writeln!(out, "{line}").and_then(|()| out.flush())
}

/// The text of a panic payload, when it carries one.
fn panic_text(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        return String::from(*text);
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    String::from("a panic without text")
}

/// The signals `larry serve` watches, registered before the serving line is
/// written, so a stop sent as soon as it is read is never the default action.
/// SIGHUP re-reads the key store.
#[cfg(unix)]
struct Signals {
    /// SIGTERM: the orchestrator's stop.
    terminate: lgwks_bot::rt::signal::unix::Signal,
    /// SIGINT: an operator's Ctrl-C.
    interrupt: lgwks_bot::rt::signal::unix::Signal,
    /// SIGHUP: the operator's reload after `larry serve revoke`.
    hangup: lgwks_bot::rt::signal::unix::Signal,
}

#[cfg(unix)]
impl Signals {
    /// Register the three handlers on the process driver.
    fn register() -> Result<Self, ServeError> {
        use lgwks_bot::rt::signal::unix::{SignalKind, signal};
        lgwks_bot::block_on(async {
            Ok(Self {
                terminate: signal(SignalKind::terminate()).map_err(ServeError::Signal)?,
                interrupt: signal(SignalKind::interrupt()).map_err(ServeError::Signal)?,
                hangup: signal(SignalKind::hangup()).map_err(ServeError::Signal)?,
            })
        })
    }

    /// Run `on_reload` on each SIGHUP until SIGTERM, SIGINT or `done`, which the
    /// edge cancels once it stopped by itself.
    fn wait(
        mut self,
        done: &CancellationToken,
        mut on_reload: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        lgwks_bot::block_on(async move {
            let mut serving = true;
            while serving {
                lgwks_bot::select! {
                    _ = self.terminate.recv() => serving = false,
                    _ = self.interrupt.recv() => serving = false,
                    // A closed SIGHUP stream yields None: the pattern then disables this arm, so no reload loops.
                    Some(()) = self.hangup.recv() => on_reload()?,
                    () = done.cancelled() => serving = false,
                }
            }
            Ok(())
        })
    }
}

/// Ctrl-C, the one stop signal outside unix. A reload needs SIGHUP, so off unix a
/// revoke takes effect at the next start.
#[cfg(not(unix))]
struct Signals;

#[cfg(not(unix))]
impl Signals {
    /// Nothing to register ahead: Ctrl-C is watched once waiting starts.
    fn register() -> Result<Self, ServeError> {
        Ok(Self)
    }

    /// Wait for Ctrl-C, or for `done` once the edge stopped by itself.
    fn wait(
        self,
        done: &CancellationToken,
        _on_reload: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        lgwks_bot::block_on(async move {
            lgwks_bot::select! {
                _ = lgwks_bot::rt::signal::ctrl_c() => {}
                () = done.cancelled() => {}
            }
        });
        Ok(())
    }
}

/// `larry serve`: bind, register the stop and reload signals, `announce` the bound
/// address and how many tenants hold a key, then serve until SIGTERM or SIGINT and
/// drain. Each SIGHUP re-reads the key store and prints one reload line.
///
/// The signal watcher is a scoped thread the scope joins before this returns, so
/// no task outlives the serve.
///
/// # Errors
///
/// [`ServeError::Bind`], [`ServeError::Signal`], [`ServeError::Announce`] from
/// `announce` or a reload line, [`ServeError::Watcher`] when the watcher panicked,
/// or [`ServeError::Host`] from the edge.
pub fn run(
    settings: Settings,
    announce: impl FnOnce(SocketAddr, usize) -> io::Result<()>,
) -> Result<(), ServeError> {
    let bind_error = |cause| ServeError::Bind {
        addr: settings.addr,
        cause,
    };
    let listener = TcpListener::bind(settings.addr).map_err(bind_error)?;
    let local = listener.local_addr().map_err(bind_error)?;
    let signals = Signals::register()?;
    announce(local, settings.keys.digests.len()).map_err(ServeError::Announce)?;
    let state = settings.state;
    let snapshot = Arc::new(RwLock::new(settings.keys));
    let app = app(Arc::clone(&snapshot));
    let stop = EdgeStop::new();
    let done = CancellationToken::new();
    std::thread::scope(|scope| {
        let watcher = scope.spawn(|| {
            let outcome = signals.wait(&done, || announce_reload(&reload(&snapshot, &state)));
            stop.stop();
            outcome
        });
        let served = app.serve(listener, &stop).map_err(ServeError::Host);
        done.cancel();
        let watched = watcher.join().map_err(|panic| ServeError::Watcher {
            cause: panic_text(&*panic),
        })?;
        served.and(watched.map_err(ServeError::Announce))
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::error::Error;
    use std::hint::black_box;
    use std::sync::{Arc, RwLock};

    use lgwks_std::seeded::Seeded;

    use super::{
        ADDR_VAR, DEFAULT_ADDR, HEALTH_BODY, Keys, ServeError, Settings, app, check_tenant, judge,
    };

    /// Keys for each of `names`, each holding `lk_<name>`.
    fn keys_for(names: &[&str]) -> Keys {
        let mut digests = BTreeMap::new();
        for name in names {
            let key = format!("lk_{name}");
            digests.insert(Box::from(*name), lgwks_std::hash::blake3(key.as_bytes()));
        }
        Keys::from_digests(digests)
    }

    #[test]
    fn an_unset_address_binds_loopback_only() -> Result<(), ServeError> {
        let dir = std::env::temp_dir()
            .join(format!("larry-serve-unit-{}-loopback", std::process::id()))
            .display()
            .to_string();
        let settings = Settings::from_env(|name| (name == "LARRY_STATE_DIR").then(|| dir.clone()))?;
        assert_eq!(
            settings.addr.to_string(),
            DEFAULT_ADDR,
            "the default is loopback"
        );
        assert!(
            settings.addr.ip().is_loopback(),
            "a first run exposes nothing"
        );
        assert_eq!(
            settings.keys.digests.len(),
            0,
            "a fresh store admits no one"
        );
        Ok(())
    }

    #[test]
    fn a_hostname_or_empty_address_is_refused_with_its_value() {
        for value in [
            "",
            "localhost:8787",
            "127.0.0.1",
            ":8787",
            "127.0.0.1:65536",
        ] {
            let refused =
                Settings::from_env(|name| (name == ADDR_VAR).then(|| String::from(value)));
            assert!(
                matches!(refused, Err(ServeError::Address { value: ref seen, .. }) if seen == value),
                "{value:?} is refused as an address, naming itself: {refused:?}"
            );
        }
    }

    #[test]
    fn no_state_directory_is_refused_before_binding() {
        let refused =
            Settings::from_env(|name| (name == ADDR_VAR).then(|| String::from("127.0.0.1:0")));
        assert!(
            matches!(refused, Err(ServeError::Store(_))),
            "no state directory, no key store: {refused:?}"
        );
    }

    #[test]
    fn tenant_names_follow_the_edge_rule_at_both_bounds() {
        let longest = "a".repeat(64);
        let past = "a".repeat(65);
        for (name, valid) in [
            ("a", true),
            (longest.as_str(), true),
            ("tenant_1-B", true),
            ("", false),
            (past.as_str(), false),
            ("a.b", false),
            ("a b", false),
            ("ü", false),
        ] {
            assert_eq!(check_tenant(name).is_ok(), valid, "{name:?}");
        }
    }

    #[test]
    fn a_key_admits_only_its_own_tenant() {
        let key = "lk_alpha";
        let keys = keys_for(&["alpha"]);
        assert_eq!(
            keys.admit("alpha", key)
                .map(|who| String::from(who.as_str())),
            Some(String::from("alpha")),
            "the key names its tenant"
        );
        assert!(
            keys.admit("beta", key).is_none(),
            "another tenant's path refuses it"
        );
        assert!(
            keys.admit("alpha", "lk_alphb").is_none(),
            "a near key is refused"
        );
        assert!(keys.admit("alpha", "").is_none(), "an empty key is refused");
    }

    #[test]
    fn sim_unknown_tenant_takes_the_compare_path_for_every_seed() -> Result<(), Box<dyn Error>> {
        let keys = keys_for(&["alpha", "beta", "gamma"]);
        for seed in [1_u64, 2, 0x5EED_0089] {
            let mut rng = Seeded::from_seed(seed);
            for _ in 0..64 {
                let len = rng.index(64)?.saturating_add(1);
                let mut stranger = String::with_capacity(len);
                for _ in 0..len {
                    stranger.push(char::from(
                        b'a'.saturating_add(u8::try_from(rng.index(26)?)?),
                    ));
                }
                if keys.digests.contains_key(stranger.as_str()) {
                    continue;
                }
                assert!(
                    std::ptr::eq(keys.stored(&stranger), &raw const keys.dummy),
                    "seed {seed:#x}: an unknown tenant is compared with the dummy"
                );
                assert!(
                    keys.admit(&stranger, "lk_alpha").is_none(),
                    "seed {seed:#x}: an unknown tenant is refused"
                );
            }
            for known in ["alpha", "beta", "gamma"] {
                let stored = keys
                    .digests
                    .get(known)
                    .ok_or("a keyed tenant has a digest")?;
                assert!(
                    std::ptr::eq(keys.stored(known), stored),
                    "seed {seed:#x}: a known tenant is compared with its own digest"
                );
                assert!(
                    keys.admit(known, "lk_wrong").is_none(),
                    "seed {seed:#x}: a known tenant with a wrong key is refused"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn a_poisoned_key_lock_admits_no_one() {
        let snapshot = RwLock::new(keys_for(&["alpha"]));
        std::thread::scope(|scope| {
            let crashed = scope
                .spawn(|| {
                    let _held = snapshot.write();
                    assert!(black_box(false), "the writer dies holding the lock");
                })
                .join();
            assert!(crashed.is_err(), "the writer panicked holding the lock");
        });
        assert!(
            judge(&snapshot, "alpha", "lk_alpha").is_none(),
            "a poisoned snapshot admits even the right key"
        );
    }

    #[test]
    fn the_app_passes_its_own_check() -> Result<(), lgwks_bot::app::AppError> {
        app(Arc::new(RwLock::new(keys_for(&[])))).check()
    }

    #[test]
    fn the_probe_body_is_json_naming_the_version() -> Result<(), lgwks_std::json::Error> {
        let body: lgwks_std::json::Value = lgwks_std::json::from_str(HEALTH_BODY)?;
        assert_eq!(
            body.get("version").and_then(lgwks_std::json::Value::as_str),
            Some(env!("CARGO_PKG_VERSION")),
            "the probe names the running version"
        );
        assert_eq!(
            body.get("status").and_then(lgwks_std::json::Value::as_str),
            Some("ok"),
            "the probe says ok"
        );
        Ok(())
    }
}
