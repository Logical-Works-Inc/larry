//! Seeded controls of the governance reader's fault kinds (larry#359, larry#554): a directory, a
//! FIFO, links, hostile names, the byte bound and the window between the check and the open each
//! keep their own refusal, so a reader that reported one as another would fail.
//! Source: CWE-59, CWE-367, CWE-400 and POSIX `mkfifo(3)`.

use std::fs;
use std::path::Path;

use lgwks_std::seeded::Seeded;

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

/// CWE-400: a file of exactly the bound is read whole and one byte past it is refused naming its
/// length, at every seeded bound.
#[test]
fn sim_the_reader_admits_exactly_its_bound() -> SimResult {
    let scratch = Scratch::new("reader-bounds")?;
    for seed in 0..32_u64 {
        let mut rng = Seeded::from_seed(seed);
        let limit = rng.index(4096)?;
        let bound = u64::try_from(limit)?;
        let exact = vec![b'x'; limit];
        scratch.write("exact.txt", &exact)?;
        let read = read::read_bounded(scratch.path(), Path::new("exact.txt"), bound);
        assert!(
            matches!(&read, Ok(bytes) if *bytes == exact),
            "seed {seed}: {limit} bytes at a bound of {limit}: {read:?}"
        );
        scratch.write("past.txt", &vec![b'x'; limit.saturating_add(1)])?;
        let read = read::read_bounded(scratch.path(), Path::new("past.txt"), bound);
        assert!(
            matches!(&read, Err(ReadFault::TooLarge { bytes }) if *bytes == bound.saturating_add(1)),
            "seed {seed}: one past {limit}: {read:?}"
        );
    }
    Ok(())
}

/// CWE-22 and CWE-59: a name that is absent, absolute, empty, climbing, not UTF-8, reached
/// through a link or through a file is refused with its own fault and never read.
#[test]
fn sim_a_hostile_path_is_refused_with_its_own_fault() -> SimResult {
    let scratch = Scratch::new("reader-paths")?;
    scratch.write("ok.txt", b"12345678")?;
    scratch.write("real/inner.txt", b"hi")?;
    let read = |relative: &Path| read::read_bounded(scratch.path(), relative, 4096);
    for missing in ["absent.txt", "", "gone/inner.txt"] {
        let refused = read(Path::new(missing));
        assert!(
            matches!(refused, Err(ReadFault::Missing(_))),
            "{missing:?}: {refused:?}"
        );
    }
    for (name, told) in [
        ("/abs.txt", "/abs.txt is not a relative path"),
        ("../ok.txt", "../ok.txt is not a relative path"),
    ] {
        let refused = read(Path::new(name));
        assert!(
            matches!(&refused, Err(ReadFault::Unreadable(cause)) if cause.to_string() == told),
            "{name}: {refused:?}"
        );
    }
    let refused = read(Path::new("ok.txt/inner.txt"));
    assert!(
        matches!(&refused, Err(ReadFault::Unreadable(cause))
            if cause.to_string().ends_with("ok.txt is not a directory")),
        "{refused:?}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        let raw = Path::new(std::ffi::OsStr::from_bytes(b"h-\xff"));
        let refused = read(raw);
        assert!(
            matches!(&refused, Err(ReadFault::Unreadable(cause))
                if cause.to_string().ends_with("is not a UTF-8 path")),
            "{refused:?}"
        );
        std::os::unix::fs::symlink(
            scratch.path().join("ok.txt"),
            scratch.path().join("link.txt"),
        )?;
        std::os::unix::fs::symlink(scratch.path().join("real"), scratch.path().join("dirlink"))?;
        for linked in ["link.txt", "dirlink/inner.txt"] {
            let refused = read(Path::new(linked));
            assert!(
                matches!(refused, Err(ReadFault::Symlink)),
                "{linked}: {refused:?}"
            );
        }
    }
    let root_file = scratch.path().join("ok.txt");
    let refused = read::read_bounded(&root_file, Path::new("x.txt"), 4096);
    assert!(
        matches!(refused, Err(ReadFault::Unreadable(_))),
        "a file where the governed root belongs: {refused:?}"
    );
    Ok(())
}

/// A directory the walk may not open refuses before anything beneath it is read (CWE-59).
#[cfg(unix)]
#[test]
fn sim_a_locked_directory_on_the_walk_refuses_the_read() -> SimResult {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("reader-mid")?;
    scratch.write("sub/file.txt", b"secret")?;
    let sub = scratch.path().join("sub");
    fs::set_permissions(&sub, fs::Permissions::from_mode(0o000))?;
    let result = read::read_bounded(scratch.path(), Path::new("sub/file.txt"), 4096);
    fs::set_permissions(&sub, fs::Permissions::from_mode(0o755))?;
    assert!(
        matches!(result, Err(ReadFault::Unreadable(_))),
        "{result:?}"
    );
    Ok(())
}

/// CWE-367: the name is swapped, deleted or the seam fails between the check and the open, and
/// each is refused by what the open then finds, never read as the file that was checked.
#[cfg(unix)]
#[test]
fn sim_a_name_changed_between_the_check_and_the_open_is_never_read() -> SimResult {
    let scratch = Scratch::new("reader-window")?;
    scratch.write("target.txt", b"stolen")?;
    let target = scratch.path().join("target.txt");
    let after = |swap: &dyn Fn(&Path) -> std::io::Result<()>| {
        scratch.write("seen.txt", b"good")?;
        let read = read::read_bounded_with(scratch.path(), Path::new("seen.txt"), 4096, swap);
        Ok::<_, super::sim_support::SimError>(read)
    };

    let linked = after(&|current| {
        fs::remove_file(current)?;
        std::os::unix::fs::symlink(&target, current)
    })?;
    assert!(matches!(linked, Err(ReadFault::Symlink)), "{linked:?}");
    fs::remove_file(scratch.path().join("seen.txt"))?;

    let deleted = after(&|current| fs::remove_file(current))?;
    assert!(matches!(deleted, Err(ReadFault::Missing(_))), "{deleted:?}");

    let directory = after(&|current| {
        fs::remove_file(current)?;
        fs::create_dir(current)
    })?;
    assert!(
        matches!(directory, Err(ReadFault::NotRegular { directory: true })),
        "{directory:?}"
    );
    fs::remove_dir(scratch.path().join("seen.txt"))?;

    let broke = after(&|_| Err(std::io::Error::other("seam broke")))?;
    assert!(
        matches!(&broke, Err(ReadFault::Unreadable(cause)) if cause.to_string() == "seam broke"),
        "{broke:?}"
    );
    Ok(())
}

/// CWE-400: a file whose metadata says it is empty but which delivers bytes (every `/proc` file)
/// is still held to the bound by the read itself.
#[cfg(target_os = "linux")]
#[test]
fn sim_a_file_that_understates_its_length_is_still_bounded() {
    let read = read::read_bounded(Path::new("/proc"), Path::new("version"), 0);
    assert!(
        matches!(read, Err(ReadFault::TooLarge { bytes: 1 })),
        "{read:?}"
    );
}
