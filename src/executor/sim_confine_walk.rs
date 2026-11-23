//! The Linux read walk on a crowded home folder (larry#382, ADR 0092). Landlock is an
//! allowlist, so the walk grants each sibling on the path to the state directory. A home
//! folder holds hundreds of entries, files among them, and each one is a rule: the bound is
//! the estate's `LandlockRules::MAX_RULES`, and a file takes the file rights alone, because
//! landlock(7) refuses a directory right on a file.

use std::io::Write as _;

use lgwks_bot::rt::process::LandlockRules;

use super::super::read_roots_bounded;
use super::{Confine, ConfineError, MAX_READ_ROOTS, Platform, link_shared, read_roots_below};
use crate::executor::sim_tree::scratch;

/// What every simulation returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The read-root count stays under its named bound with many siblings, and past it the
/// confinement is refused typed rather than built unbounded (larry#322).
#[test]
fn sim_confine_read_roots_stay_under_their_bound() -> TestResult {
    let root = scratch("many")?;
    let state = root.join("state");
    std::fs::create_dir_all(&state)?;
    for n in 0..15 {
        std::fs::create_dir_all(root.join(format!("sib-{n}")))?;
    }
    let roots = read_roots_below(&root, &state)?;
    let mut errors = std::io::stderr().lock();
    writeln!(
        errors,
        "{{\"metric\":\"read_roots\",\"siblings\":15,\"rules\":{}}}",
        roots.len()
    )?;
    assert!(
        roots.iter().any(|granted| granted == &root.join("sib-0")),
        "a sibling of the state directory is a read root: {roots:?}"
    );
    // At the bound the walk holds, one root past it is refused typed with both counts, and the
    // shipped bound is the estate's allowlist less the five fixed rules (larry#382).
    assert_eq!(
        read_roots_bounded(&root, &state, 15)?.len(),
        15,
        "at the bound every sibling is a root"
    );
    assert!(
        matches!(
            read_roots_bounded(&root, &state, 14),
            Err(ConfineError::TooManyRoots {
                roots: 15,
                bound: 14
            })
        ),
        "past the bound the confinement is refused typed"
    );
    assert_eq!(
        MAX_READ_ROOTS.checked_add(5),
        Some(LandlockRules::MAX_RULES),
        "the walk's bound is the estate's, less the fixed rules"
    );
    Ok(())
}

/// Directories and files the fake home folder holds beside `.gitconfig`, each.
const HALF: usize = 250;

/// A state directory under a home folder of 501 entries, half directories and half files, is
/// confined within the estate's allowlist, a file beside it is a read rule of its own, and the
/// state directory stays refused, by the rule list and on Linux by the kernel.
#[test]
fn sim_confine_a_crowded_home_confines_and_its_files_stay_readable() -> TestResult {
    let root = scratch("crowded-home")?;
    let fake = root.join("fake");
    let home = fake.join("home").join("person");
    let state = home.join(".local").join("state").join("larry");
    let tree = state.join("ws").join("t").join("tree");
    let tmp = state.join("ws").join("t").join("tmp");
    for dir in [&tree, &tmp] {
        std::fs::create_dir_all(dir)?;
    }
    for n in 0..HALF {
        std::fs::create_dir_all(home.join(format!(".cache-{n}")))?;
        std::fs::write(home.join(format!(".rc-{n}")), b"rc")?;
    }
    let gitconfig = home.join(".gitconfig");
    std::fs::write(&gitconfig, b"readable")?;
    std::fs::write(state.join("executor.db"), b"signing seed beside")?;
    link_shared(&fake)?;
    let roots = read_roots_below(&fake, &state)?;
    let mut errors = std::io::stderr().lock();
    writeln!(
        errors,
        "{{\"metric\":\"read_roots\",\"home_entries\":{},\"rules\":{},\"bound\":{MAX_READ_ROOTS}}}",
        HALF.saturating_mul(2).saturating_add(1),
        roots.len()
    )?;
    assert!(
        roots.len() > HALF.saturating_mul(2) && roots.len() <= MAX_READ_ROOTS,
        "every entry beside the walk is a rule, inside the bound: {}",
        roots.len()
    );
    assert!(
        roots.contains(&std::fs::canonicalize(&gitconfig)?),
        "a file beside the state directory is a read root"
    );
    let canonical = std::fs::canonicalize(&state)?;
    assert!(
        roots
            .iter()
            .all(|granted| !granted.starts_with(&canonical) && !canonical.starts_with(granted)),
        "no read root holds the state directory or lies beneath it"
    );
    // The Linux argv on any host: a file's rule carries the file rights alone, because
    // landlock(7) refuses a directory right on a file, and a directory's carries read-dir.
    let argv = Confine::within(Platform::Linux, &fake, &state, &tree, &tmp)?.wrap(&[]);
    for (path, list) in [
        (std::fs::canonicalize(&gitconfig)?, "execute,read-file"),
        (
            std::fs::canonicalize(home.join(".cache-0"))?,
            "execute,read-file,read-dir",
        ),
    ] {
        let rule = format!("path-beneath:{list}:{}", path.display());
        assert!(argv.contains(&rule), "the argv carries `{rule}`");
    }
    #[cfg(target_os = "linux")]
    kernel::reads_beside_and_refuses_the_state(&fake, &state, &tree, &tmp, &gitconfig)?;
    Ok(())
}

/// The kernel as the oracle: the confined command reads the file beside the state directory
/// and is refused the key store inside it.
#[cfg(target_os = "linux")]
mod kernel {
    use std::path::Path;

    use super::super::{Confine, Platform, run_confined, shell};
    use super::TestResult;
    use crate::executor::sim_tree::path_env;

    /// `cat` of `beside` succeeds and `cat` of the state's `executor.db` is refused, both
    /// under the confinement walked from `fake`.
    pub(super) fn reads_beside_and_refuses_the_state(
        fake: &Path,
        state: &Path,
        tree: &Path,
        tmp: &Path,
        beside: &Path,
    ) -> TestResult {
        let confine = Confine::within(Platform::host(), fake, state, tree, tmp)?;
        let path = path_env()?;
        let read = run_confined(
            &confine,
            &shell(&format!("cat {}", beside.display())),
            &path,
            tmp,
        )?;
        assert!(
            read.status.success() && read.stdout == b"readable",
            "the kernel lets a file beside the state directory be read: {}",
            String::from_utf8_lossy(&read.stderr)
        );
        let db = run_confined(
            &confine,
            &shell(&format!("cat {}/executor.db", state.display())),
            &path,
            tmp,
        )?;
        assert!(
            !db.status.success() && !String::from_utf8_lossy(&db.stdout).contains("signing seed"),
            "the state directory is refused"
        );
        Ok(())
    }
}
