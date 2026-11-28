//! The one typed error every in-crate simulation helper returns (test builds only):
//! the brief and write-guard simulations (larry#123), the terminal session's and
//! the ACP agent's (larry#93). A helper that cannot build its input says why with
//! a variant a caller can match, never a boxed string.

use std::env::VarError;
use std::io;
use std::num::TryFromIntError;
use std::string::FromUtf8Error;

use lgwks_macros::Error;
use lgwks_std::json;
use lgwks_std::seeded::DrawError;

use crate::executor::ExecError;

/// Why a simulation helper could not build its input.
#[derive(Debug, Error)]
pub(crate) enum SimError {
    /// The seeded stream refused a draw.
    #[error("seeded draw refused: {0}")]
    Draw(#[from] DrawError),
    /// A drawn number does not fit its target type.
    #[error("drawn number out of range: {0}")]
    Range(#[from] TryFromIntError),
    /// A scratch directory or a stream could not be read or written.
    #[error("scratch directory: {0}")]
    Io(#[from] io::Error),
    /// A generated document failed to encode or a printed one to parse.
    #[error("json: {0}")]
    Json(#[from] json::Error),
    /// Printed output was not UTF-8.
    #[error("output is not UTF-8: {0}")]
    Utf8(#[from] FromUtf8Error),
    /// A fixed table or a path lacked what the helper needs.
    #[error("missing: {0}")]
    Missing(&'static str),
    /// The executor refused a step the helper needs (a ledger, an origin, a workspace).
    #[error("executor: {0}")]
    Exec(#[from] ExecError),
    /// A sim could not read the state database back through `SQLite` (larry#578).
    #[error("state database: {0}")]
    Database(#[from] lgwks_deps::storefront::rusqlite::Error),
    /// An environment variable the helper reads is unset or not text.
    #[error("environment: {0}")]
    Env(#[from] VarError),
    /// An export the helper needs was refused (larry#609).
    #[error("export: {0}")]
    Export(#[from] crate::tui::ExportError),
}
