//! `larry serve`: the HTTP host on lgwks_bot's app edge (larry#88, ADR 0021).
//!
//! Larry builds one [`App`] and serves it with `App::serve`, so the estate's
//! edge owns the framing, the `/t/{tenant}/...` prefix, the 1 MiB body
//! ceiling, the bounded per-worker mailboxes and their `400`, `413` and `503`
//! refusals; Larry owns the routes, the address and the stop. Each route is a
//! transition on its tenant's own cell, so no tenant reads another's state.
//!
//! `GET /t/{tenant}/healthz` is the probe: it answers from the tenant's cell and
//! reaches no journal, model or owner, so it stays shallow and fails only when
//! the host does. A tenant-free `/healthz` waits for the edge to answer one
//! (logicalworks-crates#879). SIGTERM or SIGINT stops the edge: it stops
//! accepting, drains its connections and `run` returns.

use std::fmt;
use std::io;
use std::net::{AddrParseError, SocketAddr, TcpListener};

use lgwks_bot::app::{App, AppError, Effect, Method};
use lgwks_bot::rt::edge::EdgeStop;
use lgwks_bot::rt::sync::CancellationToken;

/// The variable naming the address `larry serve` binds.
const ADDR_VAR: &str = "LARRY_SERVE_ADDR";

/// The address bound when [`ADDR_VAR`] is unset: loopback only, so a first run
/// exposes nothing to the network.
const DEFAULT_ADDR: &str = "127.0.0.1:8787";

/// The probe's path below the tenant prefix.
pub const HEALTH_PATH: &str = "/healthz";

/// The app's name on the edge.
const APP_NAME: &str = "larry";

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
            Self::Bind { ref cause, .. } | Self::Announce(ref cause) | Self::Signal(ref cause) => {
                Some(cause)
            }
            Self::Host(ref cause) => Some(cause),
        }
    }
}

/// Where `larry serve` listens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// The address to bind; port 0 asks the system for a free one.
    addr: SocketAddr,
}

impl Settings {
    /// Settings from `read` (the process environment in the binary):
    /// [`ADDR_VAR`] when it is set, and [`DEFAULT_ADDR`] when it is not.
    ///
    /// # Errors
    ///
    /// [`ServeError::Address`] when the value is not an IP address and port.
    pub fn from_env(read: impl Fn(&str) -> Option<String>) -> Result<Self, ServeError> {
        let value = match read(ADDR_VAR) {
            Some(set) => set,
            None => String::from(DEFAULT_ADDR),
        };
        match value.trim().parse::<SocketAddr>() {
            Ok(addr) => Ok(Self { addr }),
            Err(cause) => Err(ServeError::Address { value, cause }),
        }
    }
}

/// The routes Larry serves, each a transition on its tenant's cell.
fn app() -> App<()> {
    App::new(APP_NAME).route(Method::GET, HEALTH_PATH, |_: &mut (), (): ()| {
        vec![Effect::reply(200, HEALTH_BODY.as_bytes().to_vec())]
    })
}

/// Bind `settings`' address.
///
/// # Errors
///
/// [`ServeError::Bind`] when the address is taken, not local or refused.
fn bind(settings: &Settings) -> Result<TcpListener, ServeError> {
    TcpListener::bind(settings.addr).map_err(|cause| ServeError::Bind {
        addr: settings.addr,
        cause,
    })
}

/// Serve Larry's app on `listener` until `stop` is given, then drain.
///
/// # Errors
///
/// [`ServeError::Host`] when the edge refuses the app, cannot start a worker or
/// loses the listener.
fn serve(listener: TcpListener, stop: &EdgeStop) -> Result<(), ServeError> {
    app().serve(listener, stop).map_err(ServeError::Host)
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

/// `larry serve`: bind, register the stop signals, `announce` the bound address,
/// then serve until SIGTERM or SIGINT and drain.
///
/// The signal watcher is a scoped thread the scope joins before this returns,
/// so no task outlives the serve.
///
/// # Errors
///
/// [`ServeError::Bind`], [`ServeError::Signal`], [`ServeError::Announce`] from
/// `announce`, or [`ServeError::Host`] from the edge.
pub fn run(
    settings: &Settings,
    announce: impl FnOnce(SocketAddr) -> io::Result<()>,
) -> Result<(), ServeError> {
    let listener = bind(settings)?;
    let local = listener.local_addr().map_err(|cause| ServeError::Bind {
        addr: settings.addr,
        cause,
    })?;
    let signals = Signals::register()?;
    announce(local).map_err(ServeError::Announce)?;
    let stop = EdgeStop::new();
    let done = CancellationToken::new();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            signals.wait(&done);
            stop.stop();
        });
        let served = serve(listener, &stop);
        done.cancel();
        served
    })
}

#[cfg(test)]
mod tests {
    use super::{ADDR_VAR, DEFAULT_ADDR, HEALTH_BODY, ServeError, Settings, app};

    #[test]
    fn an_unset_address_binds_loopback_only() -> Result<(), ServeError> {
        let settings = Settings::from_env(|_| None)?;
        assert_eq!(
            settings.addr.to_string(),
            DEFAULT_ADDR,
            "the default is loopback"
        );
        assert!(
            settings.addr.ip().is_loopback(),
            "a first run exposes nothing"
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
    fn a_spaced_address_is_read_trimmed() -> Result<(), ServeError> {
        let settings = Settings::from_env(|_| Some(String::from(" 0.0.0.0:0 \n")))?;
        assert_eq!(settings.addr.port(), 0, "port 0 asks for a free port");
        Ok(())
    }

    #[test]
    fn the_app_passes_its_own_check() -> Result<(), lgwks_bot::app::AppError> {
        app().check()
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
