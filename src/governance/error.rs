//! Why `larry governance` could not check at all: each refusal is a typed variant, printed as the
//! text the Python checker raised for the same case, so the one-error report keeps its wording.

use std::fmt;
use std::io;

use lgwks_ast::ParseError;
use lgwks_std::pattern::PatternError;

/// A refusal that stops the whole check before or while it reads the documents.
#[derive(Debug)]
pub(crate) enum GovernanceError {
    /// A governed document is a symbolic link (CWE-59).
    Symlink {
        /// The document, relative to the root.
        relative: String,
    },
    /// A governed name is a directory.
    IsDirectory {
        /// The directory, relative to the root.
        relative: String,
    },
    /// A governed name is not a regular file: a FIFO, device or socket (larry#336).
    NotRegular {
        /// The entry, relative to the root.
        relative: String,
    },
    /// A document is past the 2 MiB bound.
    TooLarge {
        /// The document, relative to the root.
        relative: String,
    },
    /// More documents than the bound, counted before any is read.
    TooMany {
        /// The bound.
        limit: usize,
    },
    /// A document or directory could not be read.
    Unreadable {
        /// The path, relative to the root.
        relative: String,
        /// The operating system's cause.
        cause: io::Error,
    },
    /// A document is not UTF-8.
    NotUtf8 {
        /// CPython's `UnicodeDecodeError` text for it.
        codec: String,
    },
    /// A Rust source is past the reader's byte bound.
    SourceTooLarge {
        /// The file, relative to the root.
        relative: String,
        /// The bytes the file holds.
        bytes: usize,
    },
    /// The estate reader refused a Rust source (larry#293, ADR 0058).
    Source {
        /// The file, relative to the root.
        relative: String,
        /// The reader's typed refusal.
        cause: ParseError,
    },
    /// A Rust source is not UTF-8.
    SourceNotUtf8 {
        /// The file, relative to the root.
        relative: String,
    },
    /// A workspace member's source root is missing, named so a missing root cannot pass with zero
    /// files read (larry#311).
    MissingRoot {
        /// The directory, relative to the root.
        relative: String,
    },
    /// The root manifest refused the workspace read: a symlink, an unsupported member form, or
    /// bytes that are not UTF-8 (larry#311).
    Workspace {
        /// The cause the reader reported.
        cause: String,
    },
    /// A checker pattern did not compile.
    Pattern {
        /// The pattern.
        pattern: String,
        /// The engine's cause.
        cause: PatternError,
    },
    /// `urlsplit` refused a bracket placement.
    InvalidIpv6Url,
    /// `urlsplit` refused an IPvFuture literal.
    InvalidIpvFuture,
    /// `urlsplit` refused an IPv4 address in brackets.
    BracketedIpv4,
    /// `urlsplit` refused a bracketed host that is no address.
    NotAnAddress {
        /// Python's `repr` of the host.
        host: String,
    },
}

impl fmt::Display for GovernanceError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Symlink { ref relative } => {
                write!(out, "symbolic-link document is not admitted: {relative}")
            }
            Self::IsDirectory { ref relative } => {
                write!(out, "[Errno 21] Is a directory: {relative}")
            }
            Self::NotRegular { ref relative } => {
                write!(out, "not a regular file: {relative}")
            }
            Self::TooLarge { ref relative } => {
                write!(out, "document exceeds byte limit: {relative}")
            }
            Self::TooMany { limit } => write!(out, "document count exceeds {limit}"),
            Self::Unreadable {
                ref relative,
                ref cause,
            } => write!(out, "cannot read {relative}: {cause}"),
            Self::NotUtf8 { ref codec } => out.write_str(codec),
            Self::SourceTooLarge {
                ref relative,
                bytes,
            } => {
                write!(
                    out,
                    "source file exceeds byte limit: {relative} ({bytes} bytes)"
                )
            }
            Self::Source {
                ref relative,
                ref cause,
            } => write!(out, "source file refused: {relative}: {cause}"),
            Self::SourceNotUtf8 { ref relative } => {
                write!(out, "source file is not UTF-8: {relative}")
            }
            Self::MissingRoot { ref relative } => {
                write!(out, "source tree is missing: {relative}")
            }
            Self::Workspace { ref cause } => write!(out, "workspace refused: {cause}"),
            Self::Pattern {
                ref pattern,
                ref cause,
            } => write!(out, "pattern refused: {pattern}: {cause}"),
            Self::InvalidIpv6Url => out.write_str("Invalid IPv6 URL"),
            Self::InvalidIpvFuture => out.write_str("IPvFuture address is invalid"),
            Self::BracketedIpv4 => out.write_str("An IPv4 address cannot be in brackets"),
            Self::NotAnAddress { ref host } => {
                write!(out, "{host} does not appear to be an IPv4 or IPv6 address")
            }
        }
    }
}

impl std::error::Error for GovernanceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Unreadable { ref cause, .. } => Some(cause),
            Self::Pattern { ref cause, .. } => Some(cause),
            Self::Source { ref cause, .. } => Some(cause),
            Self::Symlink { .. }
            | Self::IsDirectory { .. }
            | Self::NotRegular { .. }
            | Self::TooLarge { .. }
            | Self::TooMany { .. }
            | Self::NotUtf8 { .. }
            | Self::SourceTooLarge { .. }
            | Self::SourceNotUtf8 { .. }
            | Self::MissingRoot { .. }
            | Self::Workspace { .. }
            | Self::InvalidIpv6Url
            | Self::InvalidIpvFuture
            | Self::BracketedIpv4
            | Self::NotAnAddress { .. } => None,
        }
    }
}
