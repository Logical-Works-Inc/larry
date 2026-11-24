//! Why `larry governance` could not check at all: each refusal is a typed variant, printed as the
//! text the Python checker raised for the same case, so the one-error report keeps its wording.

use std::io;

use lgwks_ast::ParseError;
use lgwks_macros::Error;
use lgwks_std::pattern::PatternError;

/// A refusal that stops the whole check before or while it reads the documents.
#[derive(Debug, Error)]
pub(crate) enum GovernanceError {
    /// A governed document is a symbolic link (CWE-59).
    #[error("symbolic-link document is not admitted: {relative}")]
    Symlink {
        /// The document, relative to the root.
        relative: String,
    },
    /// A governed name is a directory.
    #[error("[Errno 21] Is a directory: {relative}")]
    IsDirectory {
        /// The directory, relative to the root.
        relative: String,
    },
    /// A governed name is not a regular file: a FIFO, device or socket (larry#336).
    #[error("not a regular file: {relative}")]
    NotRegular {
        /// The entry, relative to the root.
        relative: String,
    },
    /// A document is past the 2 MiB bound.
    #[error("document exceeds byte limit: {relative}")]
    TooLarge {
        /// The document, relative to the root.
        relative: String,
    },
    /// More documents than the bound, counted before any is read.
    #[error("document count exceeds {limit}")]
    TooMany {
        /// The bound.
        limit: usize,
    },
    /// A document or directory could not be read.
    #[error("cannot read {relative}: {cause}")]
    Unreadable {
        /// The path, relative to the root.
        relative: String,
        /// The operating system's cause.
        #[source]
        cause: io::Error,
    },
    /// A document is not UTF-8.
    #[error("{codec}")]
    NotUtf8 {
        /// CPython's `UnicodeDecodeError` text for it.
        codec: String,
    },
    /// A Rust source is past the reader's byte bound.
    #[error("source file exceeds byte limit: {relative} ({bytes} bytes)")]
    SourceTooLarge {
        /// The file, relative to the root.
        relative: String,
        /// The bytes the file holds.
        bytes: usize,
    },
    /// The estate reader refused a Rust source (larry#293, ADR 0058).
    #[error("source file refused: {relative}: {cause}")]
    Source {
        /// The file, relative to the root.
        relative: String,
        /// The reader's typed refusal.
        #[source]
        cause: ParseError,
    },
    /// A Rust source is not UTF-8.
    #[error("source file is not UTF-8: {relative}")]
    SourceNotUtf8 {
        /// The file, relative to the root.
        relative: String,
    },
    /// A workspace member's source root is missing, named so a missing root cannot pass with zero
    /// files read (larry#311).
    #[error("source tree is missing: {relative}")]
    MissingRoot {
        /// The directory, relative to the root.
        relative: String,
    },
    /// The root manifest refused the workspace read: a symlink, an unsupported member form, or
    /// bytes that are not UTF-8 (larry#311).
    #[error("workspace refused: {cause}")]
    Workspace {
        /// The cause the reader reported.
        cause: String,
    },
    /// A checker pattern did not compile.
    #[error("pattern refused: {pattern}: {cause}")]
    Pattern {
        /// The pattern.
        pattern: String,
        /// The engine's cause.
        #[source]
        cause: PatternError,
    },
    /// `urlsplit` refused a bracket placement.
    #[error("Invalid IPv6 URL")]
    InvalidIpv6Url,
    /// `urlsplit` refused an IPvFuture literal.
    #[error("IPvFuture address is invalid")]
    InvalidIpvFuture,
    /// `urlsplit` refused an IPv4 address in brackets.
    #[error("An IPv4 address cannot be in brackets")]
    BracketedIpv4,
    /// `urlsplit` refused a bracketed host that is no address.
    #[error("{host} does not appear to be an IPv4 or IPv6 address")]
    NotAnAddress {
        /// Python's `repr` of the host.
        host: String,
    },
}
