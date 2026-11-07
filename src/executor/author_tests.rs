//! Fixed-case tests of the write guards (larry#123); the seeded simulations live
//! in `sim_author.rs`. Source: CWE-59 (link following). Requirement: a symlink in
//! the clone is never written through, and a fenced reply is written as its body.

use std::error::Error;
use std::fs;

use super::sim::scratch;
use super::{AuthorFault, checked_target, file_text};
use crate::executor::ExecError;

/// What every test returns.
type TestResult = Result<(), Box<dyn Error>>;

#[cfg(unix)]
#[test]
fn a_symlink_anywhere_on_the_path_is_never_written_through() -> TestResult {
    let tree = scratch("tree")?;
    let outside = scratch("outside")?;
    std::os::unix::fs::symlink(&outside, tree.join("escape"))?;
    std::os::unix::fs::symlink(outside.join("victim"), tree.join("leaf"))?;
    for path in ["escape/x.rs", "escape", "leaf"] {
        assert!(
            matches!(
                checked_target(&tree, path),
                Err(ExecError::Author {
                    fault: AuthorFault::Symlink,
                    ..
                })
            ),
            "{path} crosses a symlink"
        );
    }
    let fresh = checked_target(&tree, "src/new/mod.rs")?;
    assert_eq!(fresh, tree.join("src/new/mod.rs"), "a new path is joined");
    assert_eq!(
        fs::read_dir(&outside)?.count(),
        0,
        "nothing was written outside the tree"
    );
    fs::remove_dir_all(&tree)?;
    fs::remove_dir_all(&outside)?;
    Ok(())
}

#[test]
fn a_fenced_reply_is_written_as_its_body_with_a_final_newline() {
    assert_eq!(file_text("```rust\nfn a() {}\n```"), "fn a() {}\n");
    assert_eq!(file_text("fn a() {}"), "fn a() {}\n", "a newline is added");
    assert_eq!(file_text("```"), "```\n", "a lone fence is kept as text");
    assert_eq!(
        file_text("```rust\nopen"),
        "```rust\nopen\n",
        "an unclosed fence is kept"
    );
    assert_eq!(file_text(""), "", "an empty reply stays empty");
}
