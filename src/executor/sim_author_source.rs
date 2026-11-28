//! Seeded simulations of the read of a brief's files before the model writes
//! them (larry#617). Source: CWE-367 (a check that another writer can pass
//! between) and CWE-59 (link following). Requirement: the read stops one byte
//! past its bound however the file grows, and everything a brief's file can be
//! gets its own answer.

use std::fs;
use std::path::Path;

use super::super::{AuthorFault, current_content_with};
use super::{SEEDS, TestResult, scratch};
use crate::executor::ExecError;
use crate::executor::sim_tree::Growth;

/// saboteur (CWE-367): for 64 seeds a brief's file grows after its length was checked.
/// Grown past the 256 KiB bound it is refused as too large, having been read one byte past
/// the bound and no further; grown to within it, the whole grown file is read. Catches a
/// length check followed by an unbounded read.
#[test]
fn sim_a_source_grown_after_the_stat_is_refused_too_large() -> TestResult {
    let tree = scratch("grown")?;
    for seed in 0..SEEDS {
        let growth = Growth::seeded(seed)?;
        let name = format!("grown-{seed}.txt");
        let target = tree.join(&name);
        growth.plant(&target)?;
        match current_content_with(|| growth.grow(&target), &tree, &name) {
            Err(ExecError::Author {
                path,
                fault: AuthorFault::TooLarge { len: 262_145 },
            }) if growth.past => assert_eq!(path, name, "seed {seed}: the refused path"),
            Ok(Some(text)) if !growth.past => {
                assert_eq!(text.len(), growth.end, "seed {seed}: the grown file");
            }
            other => {
                let kept = other.map(|text| text.map(|text| text.len()));
                return Err(format!("seed {seed}: grown to {} gave {kept:?}", growth.end).into());
            }
        }
    }
    Ok(())
}

/// The fault `current_content` gives for `path` under `tree`, when it refuses that path.
fn read_fault(tree: &Path, path: &str) -> Option<AuthorFault> {
    match current_content_with(|| Ok(()), tree, path) {
        Err(ExecError::Author {
            path: refused,
            fault,
        }) if refused == path => Some(fault),
        _ => None,
    }
}

/// fat-finger and escaper (CWE-59): a file or folder that is not there yet is no content; the
/// bound admits its limit and refuses past it; a folder, a link at the file or on the way to
/// it, bytes that are not UTF-8, a file used as a folder and a read that fails are each their
/// own fault.
#[test]
fn sim_each_unreadable_source_gets_its_exact_fault() -> TestResult {
    let tree = scratch("faults")?;
    fs::write(tree.join("whole.txt"), "k".repeat(262_144))?;
    fs::write(tree.join("past.txt"), "k".repeat(262_147))?;
    fs::write(tree.join("binary.bin"), [0xff_u8, 0xfe])?;
    fs::create_dir(tree.join("folder"))?;
    for absent in ["absent.txt", "folder/absent.txt", "nowhere/new.rs"] {
        assert_eq!(
            current_content_with(|| Ok(()), &tree, absent)?,
            None,
            "{absent} does not exist yet"
        );
    }
    assert_eq!(
        current_content_with(|| Ok(()), &tree, "whole.txt")?.map(|text| text.len()),
        Some(262_144),
        "the bound admits its limit"
    );
    assert!(matches!(
        read_fault(&tree, "past.txt"),
        Some(AuthorFault::TooLarge { len: 262_147 })
    ));
    assert!(matches!(
        read_fault(&tree, "binary.bin"),
        Some(AuthorFault::NotText(_))
    ));
    assert!(matches!(
        read_fault(&tree, "folder"),
        Some(AuthorFault::NotAFile)
    ));
    assert!(matches!(
        read_fault(&tree, "whole.txt/inside"),
        Some(AuthorFault::Io(_))
    ));
    assert!(matches!(
        current_content_with(|| Err(std::io::Error::other("gone")), &tree, "whole.txt"),
        Err(ExecError::Author {
            fault: AuthorFault::Io(_),
            ..
        })
    ));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(tree.join("whole.txt"), tree.join("leaf"))?;
        std::os::unix::fs::symlink(tree.join("folder"), tree.join("alias"))?;
        for linked in ["leaf", "alias/x.txt"] {
            assert!(
                matches!(read_fault(&tree, linked), Some(AuthorFault::Symlink)),
                "{linked} crosses a link"
            );
        }
    }
    Ok(())
}
