//! Why `larry serve` stopped other than by its signal: each variant keeps its
//! cause, so a caller can see why (larry#210, ADR 0040).

use std::fmt;
use std::io;
use std::net::{AddrParseError, SocketAddr};

use lgwks_bot::app::AppError;
use lgwks_std::hash::DigestParseError;

use super::{ADDR_VAR, DEFAULT_ADDR, PLAINTEXT_OPT_IN, PLAINTEXT_VAR};
use crate::executor::ExecError;

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
    /// The address is not loopback and [`PLAINTEXT_VAR`] does not opt in.
    Plaintext {
        /// The address asked for.
        addr: SocketAddr,
    },
    /// [`PLAINTEXT_VAR`] is set to something other than [`PLAINTEXT_OPT_IN`].
    PlaintextFlag {
        /// The value as it was set.
        value: String,
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
    /// The store holds more keys than the cap, so startup is refused.
    KeyCap {
        /// The number of keys the store holds.
        count: i64,
        /// The cap the edge admits.
        cap: u32,
    },
    /// The tenant holds no key, so there is nothing to revoke.
    NoKey {
        /// The tenant named.
        tenant: String,
    },
    /// The key snapshot is poisoned; no key is admitted until a restart.
    Snapshot {
        /// What the lock reported.
        cause: String,
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
    /// The signal watcher panicked; no reload or stop can run.
    Watcher {
        /// The panic's message, when it carried one.
        cause: String,
    },
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
            Self::Plaintext { addr } => write!(
                out,
                "serve: {addr} is not loopback and the edge has no TLS; set {PLAINTEXT_VAR}={PLAINTEXT_OPT_IN} to serve keys over plaintext"
            ),
            Self::PlaintextFlag { ref value } => write!(
                out,
                "serve: {PLAINTEXT_VAR}={value:?} is not {PLAINTEXT_OPT_IN:?}; unset it or set it to {PLAINTEXT_OPT_IN}"
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
            Self::KeyCap { count, cap } => write!(
                out,
                "serve: the key store holds {count} tenant keys, past the cap of {cap}; startup refused"
            ),
            Self::NoKey { ref tenant } => write!(out, "serve: {tenant:?} holds no key to revoke"),
            Self::Snapshot { ref cause } => write!(
                out,
                "serve: the key snapshot is poisoned ({cause}); restart to admit keys"
            ),
            Self::Bind { addr, ref cause } => write!(out, "serve: cannot bind {addr}: {cause}"),
            Self::Announce(ref cause) => write!(out, "serve: stdout unwritable: {cause}"),
            Self::Signal(ref cause) => write!(out, "serve: cannot watch for signals: {cause}"),
            Self::Watcher { ref cause } => {
                write!(out, "serve: the signal watcher panicked: {cause}")
            }
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
            Self::Plaintext { .. }
            | Self::PlaintextFlag { .. }
            | Self::BadTenant { .. }
            | Self::KeyCap { .. }
            | Self::NoKey { .. }
            | Self::Snapshot { .. }
            | Self::Watcher { .. } => None,
        }
    }
}
