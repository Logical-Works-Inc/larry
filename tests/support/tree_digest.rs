//! The BLAKE3 digest of a folder's whole content, shared by the journeys that prove an
//! executor run leaves its origin byte-identical (larry#85, larry#324): every path and byte
//! under the folder, sorted, folded into one hasher, so a moved, added or changed byte names
//! a different tree. Included by `tests/executor_journey.rs` and
//! `tests/exec_confine_journey.rs` through `#[path = "support/tree_digest.rs"]`.

use std::error::Error;
use std::fs;
use std::path::Path;

/// Collects every file and symlink under `dir` as (path relative to `root`, bytes).
fn collect(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let relative = path.strip_prefix(root)?.to_string_lossy().into_owned();
        let kind = fs::symlink_metadata(&path)?.file_type();
        if kind.is_symlink() {
            let target = fs::read_link(&path)?;
            out.push((relative, format!("link:{}", target.display()).into_bytes()));
        } else if kind.is_dir() {
            collect(root, &path, out)?;
        } else {
            out.push((relative, fs::read(&path)?));
        }
    }
    Ok(())
}

/// BLAKE3 over every path and byte under `dir`, in sorted path order.
pub(crate) fn tree_digest(dir: &Path) -> Result<String, Box<dyn Error>> {
    let mut entries = Vec::new();
    collect(dir, dir, &mut entries)?;
    entries.sort();
    let mut hasher = lgwks_std::hash::Hasher::new();
    for entry in &entries {
        hasher.update(entry.0.as_bytes());
        hasher.update(&[0u8]);
        hasher.update(&entry.1);
    }
    Ok(hasher.finalize().to_hex())
}
