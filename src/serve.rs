//! `larry serve`: the HTTP host on lgwks_bot's app edge (larry#88, ADR 0021),
//! with a bearer key per service tenant (larry#89, ADR 0022).
//!
//! Larry builds one [`App`] and serves it with `App::serve`, so the estate's
//! edge owns the framing, the `/t/{tenant}/...` prefix, the 1 MiB body
//! ceiling, the bounded per-worker mailboxes and their `400`, `413` and `503`
//! refusals; Larry owns the routes, the keys, the address and the stop. Each
//! route is a transition on its tenant's own cell, so no tenant reads another's
//! state.
//!
//! Every tenant request is judged at the edge by [`App::authenticate`] before
//! its verb, body or mailbox: its `Authorization: Bearer` key must hash to the
//! digest stored for the tenant in its path, or it is `401`. `larry serve key
//! <tenant>` draws a key from OS entropy, prints it once and stores only its
//! BLAKE3 digest in the state directory's owner-only store; issuing again
//! rotates it. The digests are read once at start, so a new key takes effect on
//! the next start (SIGTERM drains first). `GET /healthz` names no tenant and
//! the edge answers it itself, unauthenticated, as liveness.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::net::{AddrParseError, SocketAddr, TcpListener};
use std::sync::Arc;

use lgwks_bot::app::{App, AppError, Effect, Method, Principal, RequestHeaders};
use lgwks_bot::rt::edge::EdgeStop;
use lgwks_bot::rt::sync::CancellationToken;
use lgwks_std::hash::{Digest, DigestParseError};

use crate::executor::{ExecError, state_ledger};

/// The variable naming the address `larry serve` binds.
const ADDR_VAR: &str = "LARRY_SERVE_ADDR";

/// The address bound when [`ADDR_VAR`] is unset: loopback only, so a first run
/// exposes nothing to the network.
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

/// Why `larry serve` stopped other than by its signal; each variant keeps its
/// cause.
#[derive(Debug)]
#[non_exhaustive]
pub enum ServeError {
    /// [`ADDR_VAR`] is not an IP address and port.
    Address {
        /// The value as it was set.
        value: String,
        /// Why it does not parse.
        cause: AddrParseError,
    },
    /// The state directory's store, which holds the key digests, failed.
    Store(ExecError),
    /// A tenant name outside 1..=64 bytes of `[A-Za-z0-9_-]`.
    BadTenant {
        /// The name that was refused.
        value: String,
    },
    /// The OS refused entropy for a new key; there is no weaker fallback.
    Entropy(lgwks_std::random::EntropyError),
    /// A stored digest is not 64 hex characters.
    Digest {
        /// The tenant whose row is damaged.
        tenant: String,
        /// Why it does not parse.
        cause: DigestParseError,
    },
    /// The address could not be bound or read back.
    Bind {
        /// The address asked for.
        addr: SocketAddr,
        /// The operating system's refusal.
        cause: io::Error,
    },
    /// The serving line could not be written to standard output.
    Announce(io::Error),
    /// The stop signals could not be registered.
    Signal(io::Error),
    /// The app edge refused the app or failed while serving.
    Host(AppError),
}

impl fmt::Display for ServeError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Address {
                ref value,
                ref cause,
            } => write!(
                out,
                "serve: {ADDR_VAR}={value:?} is not an IP address and port such as {DEFAULT_ADDR}: {cause}"
            ),
            Self::Store(ref cause) => write!(out, "serve: key store: {cause}"),
            Self::BadTenant { ref value } => write!(
                out,
                "serve: {value:?} is not a tenant name: 1 to 64 of A-Z a-z 0-9 _ -"
            ),
            Self::Entropy(ref cause) => write!(out, "serve: key entropy: {cause}"),
            Self::Digest {
                ref tenant,
                ref cause,
            } => write!(
                out,
                "serve: the stored key digest of {tenant:?} is damaged: {cause}"
            ),
            Self::Bind { addr, ref cause } => write!(out, "serve: cannot bind {addr}: {cause}"),
            Self::Announce(ref cause) => write!(out, "serve: stdout unwritable: {cause}"),
            Self::Signal(ref cause) => write!(out, "serve: cannot watch for SIGTERM: {cause}"),
            Self::Host(ref cause) => write!(out, "serve: {cause}"),
        }
    }
}

impl std::error::Error for ServeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Address { ref cause, .. } => Some(cause),
            Self::Store(ref cause) => Some(cause),
            Self::Entropy(ref cause) => Some(cause),
            Self::Digest { ref cause, .. } => Some(cause),
            Self::Bind { ref cause, .. } | Self::Announce(ref cause) | Self::Signal(ref cause) => {
                Some(cause)
            }
            Self::Host(ref cause) => Some(cause),
            Self::BadTenant { .. } => None,
        }
    }
}

/// Where `larry serve` listens, and the key digests it admits.
#[derive(Debug)]
pub struct Settings {
    /// The address to bind; port 0 asks the system for a free one.
    addr: SocketAddr,
    /// Each keyed tenant's key digest, read once at start.
    keys: Keys,
}

impl Settings {
    /// Settings from `read` (the process environment in the binary): [`ADDR_VAR`]
    /// when it is set and [`DEFAULT_ADDR`] when it is not, then the key digests in
    /// `LARRY_STATE_DIR`'s store.
    ///
    /// # Errors
    ///
    /// [`ServeError::Address`] when the value is not an IP address and port,
    /// [`ServeError::Store`] when the state directory is unset or its store
    /// fails, and [`ServeError::Digest`] for a damaged row.
    pub fn from_env(read: impl Fn(&str) -> Option<String>) -> Result<Self, ServeError> {
        let value = match read(ADDR_VAR) {
            Some(set) => set,
            None => String::from(DEFAULT_ADDR),
        };
        let addr = match value.trim().parse::<SocketAddr>() {
            Ok(addr) => addr,
            Err(cause) => return Err(ServeError::Address { value, cause }),
        };
        let keys = Keys::load(&read)?;
        Ok(Self { addr, keys })
    }
}

/// Each keyed tenant's key digest.
#[derive(Debug, Default)]
struct Keys(HashMap<Box<str>, Digest>);

impl Keys {
    /// The digests in the state directory's store, at most [`MAX_TENANT_KEYS`].
    fn load(read: &impl Fn(&str) -> Option<String>) -> Result<Self, ServeError> {
        let rows = state_ledger(read)
            .and_then(|ledger| ledger.tenant_keys(MAX_TENANT_KEYS))
            .map_err(ServeError::Store)?;
        let mut keys = HashMap::with_capacity(rows.len());
        for (tenant, hex) in rows {
            let digest = Digest::from_hex(&hex).map_err(|cause| ServeError::Digest {
                tenant: tenant.clone(),
                cause,
            })?;
            keys.insert(tenant.into_boxed_str(), digest);
        }
        Ok(Self(keys))
    }

    /// The principal `token` names for `tenant`: the tenant, when the token hashes
    /// to its stored digest, compared in constant time.
    fn admit(&self, tenant: &str, token: &str) -> Option<Principal> {
        let stored = self.0.get(tenant)?;
        if lgwks_std::hash::blake3(token.as_bytes()).ct_eq(stored) {
            Principal::new(tenant)
        } else {
            None
        }
    }
}

/// Whether `name` is a tenant name the edge accepts: 1..=64 of `[A-Za-z0-9_-]`.
fn valid_tenant(name: &str) -> bool {
    (1..=MAX_TENANT_BYTES).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
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
    if !valid_tenant(tenant) {
        return Err(ServeError::BadTenant {
            value: String::from(tenant),
        });
    }
    let secret = lgwks_std::random::bytes::<KEY_BYTES>().map_err(ServeError::Entropy)?;
    let key = format!("{KEY_PREFIX}{}", lgwks_std::hex::encode(secret));
    let digest = lgwks_std::hash::blake3(key.as_bytes()).to_hex();
    state_ledger(&read)
        .and_then(|ledger| ledger.set_tenant_key(tenant, &digest))
        .map_err(ServeError::Store)?;
    Ok(key)
}

/// The routes Larry serves, each a transition on its tenant's cell, behind the
/// key check.
fn app(keys: Keys) -> App<()> {
    let keys = Arc::new(keys);
    App::new(APP_NAME)
        .authenticate(move |tenant: &str, headers: &RequestHeaders<'_>| {
            keys.admit(tenant, headers.bearer()?)
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

/// Bind `addr`.
fn bind(addr: SocketAddr) -> Result<TcpListener, ServeError> {
    TcpListener::bind(addr).map_err(|cause| ServeError::Bind { addr, cause })
}

/// The signals that stop `larry serve`, registered before the serving line is
/// written, so a stop sent as soon as it is read is never the default action.
#[cfg(unix)]
struct Signals {
    /// SIGTERM: the orchestrator's stop.
    terminate: lgwks_bot::rt::signal::unix::Signal,
    /// SIGINT: an operator's Ctrl-C.
    interrupt: lgwks_bot::rt::signal::unix::Signal,
}

#[cfg(unix)]
impl Signals {
    /// Register both handlers on the process driver.
    fn register() -> Result<Self, ServeError> {
        use lgwks_bot::rt::signal::unix::{SignalKind, signal};
        lgwks_bot::block_on(async {
            Ok(Self {
                terminate: signal(SignalKind::terminate()).map_err(ServeError::Signal)?,
                interrupt: signal(SignalKind::interrupt()).map_err(ServeError::Signal)?,
            })
        })
    }

    /// Wait for either signal, or for `done` once the edge stopped by itself.
    fn wait(mut self, done: &CancellationToken) {
        lgwks_bot::block_on(async move {
            lgwks_bot::select! {
                _ = self.terminate.recv() => {}
                _ = self.interrupt.recv() => {}
                () = done.cancelled() => {}
            }
        });
    }
}

/// Ctrl-C, the one stop signal outside unix.
#[cfg(not(unix))]
struct Signals;

#[cfg(not(unix))]
impl Signals {
    /// Nothing to register ahead: Ctrl-C is watched once waiting starts.
    fn register() -> Result<Self, ServeError> {
        Ok(Self)
    }

    /// Wait for Ctrl-C, or for `done` once the edge stopped by itself.
    fn wait(self, done: &CancellationToken) {
        lgwks_bot::block_on(async move {
            lgwks_bot::select! {
                _ = lgwks_bot::rt::signal::ctrl_c() => {}
                () = done.cancelled() => {}
            }
        });
    }
}

/// `larry serve`: bind, register the stop signals, `announce` the bound address
/// and how many tenants hold a key, then serve until SIGTERM or SIGINT and drain.
///
/// The signal watcher is a scoped thread the scope joins before this returns,
/// so no task outlives the serve.
///
/// # Errors
///
/// [`ServeError::Bind`], [`ServeError::Signal`], [`ServeError::Announce`] from
/// `announce`, or [`ServeError::Host`] from the edge.
pub fn run(
    settings: Settings,
    announce: impl FnOnce(SocketAddr, usize) -> io::Result<()>,
) -> Result<(), ServeError> {
    let listener = bind(settings.addr)?;
    let local = listener.local_addr().map_err(|cause| ServeError::Bind {
        addr: settings.addr,
        cause,
    })?;
    let signals = Signals::register()?;
    announce(local, settings.keys.0.len()).map_err(ServeError::Announce)?;
    let app = app(settings.keys);
    let stop = EdgeStop::new();
    let done = CancellationToken::new();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            signals.wait(&done);
            stop.stop();
        });
        let served = app.serve(listener, &stop).map_err(ServeError::Host);
        done.cancel();
        served
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        ADDR_VAR, DEFAULT_ADDR, HEALTH_BODY, Keys, ServeError, Settings, app, valid_tenant,
    };

    /// A fresh state directory for one test.
    fn state_dir(name: &str) -> String {
        std::env::temp_dir()
            .join(format!("larry-serve-unit-{}-{name}", std::process::id()))
            .display()
            .to_string()
    }

    #[test]
    fn an_unset_address_binds_loopback_only() -> Result<(), ServeError> {
        let dir = state_dir("loopback");
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
        assert!(settings.keys.0.is_empty(), "a fresh store admits no one");
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
            assert_eq!(valid_tenant(name), valid, "{name:?}");
        }
    }

    #[test]
    fn a_key_admits_only_its_own_tenant() {
        let key = "lk_alpha";
        let mut digests = HashMap::new();
        digests.insert(Box::from("alpha"), lgwks_std::hash::blake3(key.as_bytes()));
        let keys = Keys(digests);
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
    fn the_app_passes_its_own_check() -> Result<(), lgwks_bot::app::AppError> {
        app(Keys::default()).check()
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
