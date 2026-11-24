//! The one write of an input to a child's stdin that tolerates a child which stopped reading
//! (larry#444). Included by `tests/support/larry_cli.rs` and `tests/support/eval_arm.rs`, so the
//! rule has one source.

use std::io::{self, ErrorKind, Write};
use std::process::ChildStdin;

/// Writes `input` to `stdin` and closes it. A child that refuses its input exits before it has
/// read it all, and a write larger than the pipe then fails with a broken pipe: that is the
/// child having stopped reading, so its status and stderr are what the caller reads next, not a
/// fault of the harness. Every other failure is returned.
pub(crate) fn feed_stdin(mut stdin: ChildStdin, input: &[u8]) -> io::Result<()> {
    match stdin.write_all(input) {
        Ok(()) => Ok(()),
        Err(ref cause) if cause.kind() == ErrorKind::BrokenPipe => Ok(()),
        Err(cause) => Err(cause),
    }
}
