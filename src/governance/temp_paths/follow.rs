//! The follow-name resolution of the temp-path rule (larry#311, ADR 0063): the `#[path]`
//! and `include!` names the cached analysis spells, resolved beside the file that names them
//! and kept inside the checked tree. Resolution reads the live tree on every check, so only
//! the names are cached, never their targets.

use std::io;
use std::path::{Component, Path, PathBuf};

use super::Source;
use super::syntax::crate_root;

/// The `#[path = "..."]` and `include!("...")` sources the cached analysis names, resolved
/// beside its file and kept inside the checked tree; an absolute path, a link, a missing file
/// or a path outside the tree is a refusal naming the file that names it. Resolution reads the
/// live tree on every check, so only the names are cached, never their targets.
pub(super) fn follows(
    canonical_root: &Path,
    path: &Path,
    relative: &str,
    names: &[(String, Vec<String>)],
    inherited_test: bool,
    errors: &mut Vec<String>,
) -> Vec<Source> {
    let Some(base) = path.parent() else {
        return Vec::new();
    };
    let mut follows = Vec::new();
    for entry in names {
        let (name, inline) = (&entry.0, &entry.1);
        if name.starts_with('/') {
            errors.push(format!(
                "a followed module names an absolute path: {relative}: {name}"
            ));
            continue;
        }
        let mut rooted = base.to_path_buf();
        if !inline.is_empty()
            && !crate_root(relative)
            && let Some(stem) = Path::new(relative).file_stem()
            && let Some(stem) = stem.to_str()
        {
            rooted.push(stem);
        }
        for part in inline {
            rooted.push(part);
        }
        let target = settle(&rooted.join(name));
        let canonical = match resolve_follow(&target) {
            Ok(canonical) => canonical,
            Err(cause) => {
                let what = if target.strip_prefix(canonical_root).is_ok() {
                    "a followed module names no readable source"
                } else {
                    "a followed module leaves the checked tree"
                };
                errors.push(format!("{what}: {relative}: {name}: {cause}"));
                continue;
            }
        };
        let inner = match canonical.strip_prefix(canonical_root) {
            Ok(inner) => inner,
            Err(_) => {
                errors.push(format!(
                    "a followed module leaves the checked tree: {relative}: {name}"
                ));
                continue;
            }
        };
        let inner = inner.to_string_lossy().into_owned();
        follows.push((canonical, inner, inherited_test));
    }
    follows
}

/// `path` with `.` and `..` applied as text, as rustc roots a followed module (larry#311).
fn settle(path: &Path) -> PathBuf {
    let mut settled = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(settled.components().next_back(), Some(Component::Normal(_))) =>
            {
                settled.pop();
            }
            _ => settled.push(part),
        }
    }
    settled
}

/// Resolves a followed module's path: a symlink is never followed and a missing file is refused.
fn resolve_follow(target: &Path) -> io::Result<PathBuf> {
    let metadata = std::fs::symlink_metadata(target)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::other("a symbolic link is never followed"));
    }
    std::fs::canonicalize(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::governance::sim_support::Scratch;

    /// The canonical root of `tree`, so the settled target and the root share one
    /// symlink-free spelling on hosts where the scratch directory itself is a link.
    fn canonical_root_of(tree: &Scratch) -> Result<(PathBuf, PathBuf), Box<dyn std::error::Error>> {
        let canonical = std::fs::canonicalize(tree.path())?;
        Ok((canonical.join("tests/uses.rs"), canonical))
    }

    /// What `follows` refuses over `names` beside `tests/uses.rs`, and what it follows.
    fn resolved(
        canonical: &Path,
        path: &Path,
        names: &[(String, Vec<String>)],
    ) -> (Vec<Source>, Vec<String>) {
        let mut errors = Vec::new();
        let followed = follows(canonical, path, "tests/uses.rs", names, false, &mut errors);
        (followed, errors)
    }

    /// A fresh tree holding a `tests/uses.rs`, resolved over `names`.
    fn followed(
        tag: &str,
        names: &[(String, Vec<String>)],
    ) -> Result<(Vec<Source>, Vec<String>), Box<dyn std::error::Error>> {
        let tree = Scratch::new(tag)?;
        tree.write("tests/uses.rs", b"#[path = \"x.rs\"]\nmod x;\n")?;
        let (path, canonical) = canonical_root_of(&tree)?;
        Ok(resolved(&canonical, &path, names))
    }

    /// A missing module beside its file is refused as unreadable, never as leaving: the
    /// `is_ok` that tells them apart never becomes `is_err`.
    #[test]
    fn a_missing_module_beside_its_file_names_no_readable_source()
    -> Result<(), Box<dyn std::error::Error>> {
        let names = vec![(String::from("missing.rs"), Vec::new())];
        let (followed, errors) = followed("follow-missing-inside", &names)?;
        assert!(followed.is_empty(), "a missing module follows nothing");
        assert_eq!(
            errors.len(),
            1,
            "one missing module is one refusal: {errors:?}"
        );
        assert!(
            errors.first().is_some_and(|first| first.starts_with(
                "a followed module names no readable source: tests/uses.rs: missing.rs: "
            )),
            "an inside-tree miss keeps its name: {errors:?}"
        );
        Ok(())
    }

    /// A missing module outside the tree is refused as leaving, never as unreadable.
    #[test]
    fn a_missing_module_outside_the_tree_names_leaving() -> Result<(), Box<dyn std::error::Error>> {
        let names = vec![(String::from("../../follow-escape-missing.rs"), Vec::new())];
        let (followed, errors) = followed("follow-missing-outside", &names)?;
        assert!(followed.is_empty(), "a missing module follows nothing");
        assert_eq!(
            errors.len(),
            1,
            "one missing module is one refusal: {errors:?}"
        );
        assert!(
            errors.first().is_some_and(|first| first.starts_with(
                "a followed module leaves the checked tree: tests/uses.rs: ../../follow-escape-missing.rs: "
            )),
            "an outside-tree miss keeps its name: {errors:?}"
        );
        Ok(())
    }

    /// A module that leaves the tree never swallows the next one: the `continue` past a
    /// leaving module never becomes `break`.
    #[test]
    fn a_leaving_module_does_not_swallow_the_next_module() -> Result<(), Box<dyn std::error::Error>>
    {
        let tree = Scratch::new("follow-leaving-first")?;
        let outside = Scratch::new("follow-outside")?;
        tree.write("tests/uses.rs", b"#[path = \"x.rs\"]\nmod x;\n")?;
        outside.write("payload.rs", b"pub fn payload() {}\n")?;
        let (path, canonical) = canonical_root_of(&tree)?;
        std::os::unix::fs::symlink(outside.path(), canonical.join("tests/link"))?;
        let names = vec![
            (String::from("link/payload.rs"), Vec::new()),
            (String::from("missing.rs"), Vec::new()),
        ];
        let (followed, errors) = resolved(&canonical, &path, &names);
        assert!(followed.is_empty(), "neither module follows anything");
        assert_eq!(
            errors.len(),
            2,
            "the leaving module and the missing one are both refused: {errors:?}"
        );
        assert!(
            errors.first().is_some_and(|first| first
                == "a followed module leaves the checked tree: tests/uses.rs: link/payload.rs"),
            "the leaving module is named first: {errors:?}"
        );
        assert!(
            errors.get(1).is_some_and(|second| second.starts_with(
                "a followed module names no readable source: tests/uses.rs: missing.rs: "
            )),
            "the module after the leaving one is still refused: {errors:?}"
        );
        Ok(())
    }
}
