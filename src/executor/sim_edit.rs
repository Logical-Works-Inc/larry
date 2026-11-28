//! Seeded simulations of a session's file read (larry#586): the digest is the
//! file's BLAKE3 for every generated body, and every path or file the read
//! cannot take gets its own typed refusal. Source: CWE-59 (a link that leaves
//! the tree) and the brief's per-file bound.

use lgwks_std::seeded::Seeded;

use super::{EditError, read_file};
use crate::executor::sim_tree::{Scratch, scratch};
use crate::sim_error::SimError;

/// What every simulation returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Seeds the digest simulation runs.
const SEEDS: u64 = 512;

/// The pieces a generated body is built from: line ends, a tab and non-ASCII.
const PIECES: [&str; 4] = ["alpha\n", "beta\r\n", "\tgamma", " é "];

/// A scratch snapshot holding `files`: each path with its bytes.
fn snapshot(label: &str, files: &[(&str, &[u8])]) -> Result<Scratch, SimError> {
    let root = scratch(label)?;
    for (path, bytes) in files.iter().copied() {
        std::fs::write(root.join(path), bytes)?;
    }
    Ok(root)
}

/// For every seed, the read returns the file whole and the BLAKE3 hex of its
/// bytes, so the digest names exactly what the model was shown.
#[test]
fn sim_a_read_shows_the_file_and_its_digest_for_every_seed() -> TestResult {
    let root = scratch("sim-read-digest")?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let mut body = String::new();
        for _ in 0..rng.below(40)? {
            let pick = rng.index(PIECES.len())?;
            body.push_str(PIECES.get(pick).ok_or(SimError::Missing("piece"))?);
        }
        let name = format!("read-{seed}.txt");
        std::fs::write(root.join(&name), body.as_bytes())?;
        let view = read_file(&root, &name)?;
        assert_eq!(view.text, body, "seed {seed}: the whole file");
        assert_eq!(
            view.digest,
            lgwks_std::hash::blake3(body.as_bytes()).to_hex(),
            "seed {seed}: the digest of its bytes"
        );
    }
    Ok(())
}

/// A path outside the snapshot is refused before anything is read.
#[test]
fn sim_a_path_outside_the_snapshot_is_refused() -> TestResult {
    let root = snapshot("sim-read-outside", &[("inside.txt", b"kept\n")])?;
    for outside in [
        "../escape.txt",
        "/etc/hosts",
        "sub/../../escape.txt",
        ".git/hooks/x",
        "",
    ] {
        match read_file(&root, outside) {
            Err(EditError::PathOutside { path }) => assert_eq!(path, outside),
            other => return Err(format!("{outside:?}: wrong outcome: {other:?}").into()),
        }
    }
    Ok(())
}

/// A link in the tree is refused as outside, whether it is the file itself or a
/// directory on the way to it.
#[cfg(unix)]
#[test]
fn sim_a_link_in_the_tree_is_refused() -> TestResult {
    let outside = snapshot("sim-read-link-target", &[("secret.txt", b"secret\n")])?;
    let root = scratch("sim-read-link")?;
    std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("file.txt"))?;
    std::os::unix::fs::symlink(&*outside, root.join("folder"))?;
    for path in ["file.txt", "folder/secret.txt"] {
        match read_file(&root, path) {
            Err(EditError::PathOutside { .. }) => {}
            other => return Err(format!("{path}: wrong outcome: {other:?}").into()),
        }
    }
    Ok(())
}

/// Every bound of a read admits its limit and refuses one past it, and what is
/// not a UTF-8 regular file gets its own error.
#[test]
fn sim_each_read_bound_admits_its_limit_and_refuses_past_it() -> TestResult {
    let large = "k\n".repeat(131_072);
    let past = format!("{large}k");
    let root = snapshot(
        "sim-read-bounds",
        &[
            ("large.txt", large.as_bytes()),
            ("past.txt", past.as_bytes()),
            ("binary.bin", &[0xff, 0xfe]),
        ],
    )?;
    std::fs::create_dir(root.join("folder"))?;
    assert_eq!(read_file(&root, "large.txt")?.text.len(), 262_144);
    assert!(matches!(
        read_file(&root, "past.txt"),
        Err(EditError::TooLarge { len: 262_145, .. })
    ));
    assert!(matches!(
        read_file(&root, "binary.bin"),
        Err(EditError::NotText { .. })
    ));
    assert!(matches!(
        read_file(&root, "folder"),
        Err(EditError::NotAFile { .. })
    ));
    assert!(matches!(
        read_file(&root, "absent.txt"),
        Err(EditError::Io { .. })
    ));
    Ok(())
}
