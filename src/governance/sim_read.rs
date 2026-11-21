//! Seeded controls of the governance reader's fault kinds (larry#359): a directory and a FIFO
//! present before the check keep their own refusal flags, so a reader that reported one as the
//! other would fail. Source: CWE-59, CWE-367 and POSIX `mkfifo(3)`.

use std::fs;
use std::path::Path;

use super::read::{self, ReadFault};
use super::sim_support::{Scratch, SimResult};

/// The FIFO the reader refuses by name.
const FIFO: &str = "tools/tracker/tracker.json";

/// A directory and a FIFO already present keep their own refusal flags (larry#359).
#[test]
fn sim_a_directory_and_a_fifo_keep_their_own_refusals() -> SimResult {
    let scratch = Scratch::new("fault-flags")?;
    fs::create_dir_all(scratch.path().join("docs/draft.md"))?;
    let result = read::read_bounded(scratch.path(), Path::new("docs/draft.md"), 4096);
    assert!(
        matches!(&result, Err(ReadFault::NotRegular { directory: true })),
        "{result:?}"
    );
    scratch.write(FIFO, b"{}")?;
    let fifo = scratch.path().join(FIFO);
    fs::remove_file(&fifo)?;
    let status = std::process::Command::new("mkfifo").arg(&fifo).status()?;
    assert!(status.success(), "mkfifo refused: {status}");
    let result = read::read_bounded(scratch.path(), Path::new(FIFO), 4096);
    assert!(
        matches!(&result, Err(ReadFault::NotRegular { directory: false })),
        "{result:?}"
    );
    Ok(())
}
