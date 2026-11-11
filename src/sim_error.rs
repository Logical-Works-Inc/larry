//! The one typed error every in-crate simulation helper returns (test builds only):
//! the brief and write-guard simulations (larry#123), the terminal session's and
//! the ACP agent's (larry#93). A helper that cannot build its input says why with
//! a variant a caller can match, never a boxed string.

use std::error::Error;
use std::fmt;
use std::io;
use std::num::TryFromIntError;
use std::string::FromUtf8Error;

use lgwks_std::json;
use lgwks_std::seeded::DrawError;

/// Why a simulation helper could not build its input.
#[derive(Debug)]
pub(crate) enum SimError {
    /// The seeded stream refused a draw.
    Draw(DrawError),
    /// A drawn number does not fit its target type.
    Range(TryFromIntError),
    /// A scratch directory or a stream could not be read or written.
    Io(io::Error),
    /// A generated document failed to encode or a printed one to parse.
    Json(json::Error),
    /// Printed output was not UTF-8.
    Utf8(FromUtf8Error),
    /// A fixed table or a path lacked what the helper needs.
    Missing(&'static str),
}

impl fmt::Display for SimError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Draw(ref cause) => write!(out, "seeded draw refused: {cause}"),
            Self::Range(ref cause) => write!(out, "drawn number out of range: {cause}"),
            Self::Io(ref cause) => write!(out, "scratch directory: {cause}"),
            Self::Json(ref cause) => write!(out, "json: {cause}"),
            Self::Utf8(ref cause) => write!(out, "output is not UTF-8: {cause}"),
            Self::Missing(what) => write!(out, "missing: {what}"),
        }
    }
}

impl Error for SimError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match *self {
            Self::Draw(ref cause) => Some(cause),
            Self::Range(ref cause) => Some(cause),
            Self::Io(ref cause) => Some(cause),
            Self::Json(ref cause) => Some(cause),
            Self::Utf8(ref cause) => Some(cause),
            Self::Missing(_) => None,
        }
    }
}

impl From<DrawError> for SimError {
    fn from(cause: DrawError) -> Self {
        Self::Draw(cause)
    }
}

impl From<TryFromIntError> for SimError {
    fn from(cause: TryFromIntError) -> Self {
        Self::Range(cause)
    }
}

impl From<io::Error> for SimError {
    fn from(cause: io::Error) -> Self {
        Self::Io(cause)
    }
}

impl From<json::Error> for SimError {
    fn from(cause: json::Error) -> Self {
        Self::Json(cause)
    }
}

impl From<FromUtf8Error> for SimError {
    fn from(cause: FromUtf8Error) -> Self {
        Self::Utf8(cause)
    }
}
