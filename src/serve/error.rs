//! Why `larry serve` stopped other than by its signal: each variant keeps its
//! cause, so a caller can see why (larry#210, ADR 0040).

use std::io;
use std::net::{AddrParseError, SocketAddr};

use lgwks_bot::app::AppError;
use lgwks_macros::Error;
use lgwks_std::hash::DigestParseError;

use super::{ADDR_VAR, PLAINTEXT_OPT_IN, PLAINTEXT_VAR, default_addr};
use crate::executor::ExecError;

/// Why `larry serve` stopped other than by its signal; each variant keeps its
/// cause.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ServeError {
    /// [`ADDR_VAR`] is not an IP address and port.
    #[error(
        "serve: {ADDR_VAR}={value:?} is not an IP address and port such as {}: {cause}",
        default_addr()
    )]
    Address {
        /// The value as it was set.
        value: String,
        /// Why it does not parse.
        #[source]
        cause: AddrParseError,
    },
    /// The address is not loopback and [`PLAINTEXT_VAR`] does not opt in.
    #[error(
        "serve: {addr} is not loopback and the edge has no TLS; set {}={} to serve keys over plaintext",
        PLAINTEXT_VAR,
        PLAINTEXT_OPT_IN
    )]
    Plaintext {
        /// The address asked for.
        addr: SocketAddr,
    },
    /// [`PLAINTEXT_VAR`] is set to something other than [`PLAINTEXT_OPT_IN`].
    #[error(
        "serve: {0}={value:?} is not {1:?}; unset it or set it to {1}",
        PLAINTEXT_VAR,
        PLAINTEXT_OPT_IN
    )]
    PlaintextFlag {
        /// The value as it was set.
        value: String,
    },
    /// The state directory's store, which holds the key digests, failed.
    #[error("serve: key store: {0}")]
    Store(#[source] ExecError),
    /// A tenant name outside 1..=64 bytes of `[A-Za-z0-9_-]`.
    #[error("serve: {value:?} is not a tenant name: 1 to 64 of A-Z a-z 0-9 _ -")]
    BadTenant {
        /// The name that was refused.
        value: String,
    },
    /// The OS refused entropy for a new key; there is no weaker fallback.
    #[error("serve: key entropy: {0}")]
    Entropy(#[source] lgwks_std::random::EntropyError),
    /// A stored digest is not 64 hex characters.
    #[error("serve: the stored key digest of {tenant:?} is damaged: {cause}")]
    Digest {
        /// The tenant whose row is damaged.
        tenant: String,
        /// Why it does not parse.
        #[source]
        cause: DigestParseError,
    },
    /// The store holds more keys than the cap, so startup is refused.
    #[error(
        "serve: the key store holds {count} tenant keys, past the cap of {cap}; startup refused"
    )]
    KeyCap {
        /// The number of keys the store holds.
        count: i64,
        /// The cap the edge admits.
        cap: u32,
    },
    /// The store holds the cap of keys, so a new tenant gets none; a rotation
    /// of a tenant that holds one still succeeds (larry#397).
    #[error(
        "serve: the key store holds the cap of {cap} tenant keys; revoke one before keying a new tenant"
    )]
    KeysFull {
        /// The cap the edge admits.
        cap: u32,
    },
    /// The tenant holds no key, so there is nothing to revoke.
    #[error("serve: {tenant:?} holds no key to revoke")]
    NoKey {
        /// The tenant named.
        tenant: String,
    },
    /// The key snapshot is poisoned; no key is admitted until a restart.
    #[error("serve: the key snapshot is poisoned ({cause}); restart to admit keys")]
    Snapshot {
        /// What the lock reported.
        cause: String,
    },
    /// The address could not be bound or read back.
    #[error("serve: cannot bind {addr}: {cause}")]
    Bind {
        /// The address asked for.
        addr: SocketAddr,
        /// The operating system's refusal.
        #[source]
        cause: io::Error,
    },
    /// The serving line could not be written to standard output.
    #[error("serve: stdout unwritable: {0}")]
    Announce(#[source] io::Error),
    /// The stop signals could not be registered.
    #[error("serve: cannot watch for signals: {0}")]
    Signal(#[source] io::Error),
    /// The signal watcher panicked; no reload or stop can run.
    #[error("serve: the signal watcher panicked: {cause}")]
    Watcher {
        /// The panic's message, when it carried one.
        cause: String,
    },
    /// The app edge refused the app or failed while serving.
    #[error("serve: {0}")]
    Host(#[source] AppError),
}
