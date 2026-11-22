//! Seeded simulations of the kernel confinement of a command (larry#243, ADR 0046). The
//! argv of each platform is judged against a reference spelling written here from the
//! platform's own documentation (Seatbelt profile language, setpriv(1) and Landlock), the
//! estate's validation is asked about hostile names, and the kernel itself is the oracle for
//! "a write beside the tree is refused": the simulations run real commands and read the
//! folder they must not have changed. Sources: sandbox-exec(1), setpriv(1), landlock(7).

use std::error::Error;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use lgwks_std::seeded::Seeded;

use super::{
    Confine, ConfineError, MAX_READ_ROOTS, PROC_READS, Platform, SANDBOX_EXEC, SETPRIV,
    read_roots_below, supported, supported_with, text,
};
use crate::executor::Ledger;
use crate::executor::run::{Place, run_command};
use crate::executor::sim_tree::{Scratch, path_env, scratch, tree_digest};
use crate::json_at::At;
use crate::sim_error::SimError;

/// What every simulation returns.
type TestResult = Result<(), Box<dyn Error>>;

/// Seeds the kernel simulation runs.
const SEEDS: u64 = 24;

/// Samples of the overhead measurement.
const SAMPLES: usize = 30;

/// Name pieces a hostile directory is built from: quotes, backslashes, spaces, parentheses
/// and non-ASCII, everything a profile string or an argv element must carry whole.
const PIECES: [&str; 9] = [
    "a\"b", "c\\d", "e f", "(g)", "h;i", "j'k", "é→", "l$m", "n`o",
];

/// A state directory holding a workspace as `Island` lays it out: the tree and the scratch
/// directory under `<state>/ws/<name>`, beside the `executor.db` that holds the signing
/// seed. The scratch root is the fake root the walk descends: it links the host's own
/// program, library and configuration directories, so a confined command still runs, and
/// the walk never reads `/tmp`, whose sibling count no test controls (larry#322).
fn layout(label: &str, name: &str) -> Result<(Scratch, PathBuf, PathBuf, PathBuf), SimError> {
    let root = scratch(label)?;
    link_shared(&root)?;
    let state = root.join("state");
    let tree = state.join("ws").join(name).join("tree");
    let tmp = state.join("ws").join(name).join("tmp");
    std::fs::create_dir_all(&tree)?;
    std::fs::create_dir_all(&tmp)?;
    Ok((root, state, tree, tmp))
}

/// Links the host's program, library and configuration directories into `root`, so a
/// command confined by a walk from `root` still starts.
fn link_shared(root: &Path) -> Result<(), SimError> {
    for shared in ["bin", "sbin", "lib", "lib64", "usr", "etc"] {
        let real = Path::new("/").join(shared);
        if real.is_dir() {
            std::os::unix::fs::symlink(&real, root.join(shared))?;
        }
    }
    Ok(())
}

/// The confinement over a layout, its read walk rooted at the layout's fake root. Linux
/// reads are then judged against paths the test built, never the host's `/tmp`, whose
/// entry count would otherwise decide whether the allowlist fits (larry#322).
fn confined(
    platform: Platform,
    state: &Path,
    tree: &Path,
    tmp: &Path,
) -> Result<Confine, ConfineError> {
    let root = state.parent().ok_or_else(|| ConfineError::Path {
        path: state.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "the layout's fake root is the state directory's parent",
        ),
    })?;
    Confine::within(platform, root, state, tree, tmp)
}

/// The Seatbelt profile the reference spelling writes for `state`, the session's `copy`,
/// `tree` and `tmp`.
fn reference_profile(state: &Path, copy: &Path, tree: &Path, tmp: &Path) -> String {
    format!(
        "(version 1)(allow default)(deny file-read* (require-all (subpath \"{}\") (require-not \
         (require-any (subpath \"{}\") (subpath \"{}\") (subpath \"{}\")))))(deny file-write* \
         (require-not (require-any (subpath \"{}\") (subpath \"{}\") (literal \"/dev/null\") \
         (literal \"/dev/tty\"))))(deny sysctl-read (sysctl-name \"kern.procargs2\"))",
        state.display(),
        copy.display(),
        tree.display(),
        tmp.display(),
        tree.display(),
        tmp.display()
    )
}

/// `command` run behind `confine` in its tree under a clean environment.
fn run_confined(
    confine: &Confine,
    command: &[String],
    path: &str,
    tmp: &Path,
) -> Result<std::process::Output, SimError> {
    let argv = confine.wrap(command);
    let (program, arguments) = argv
        .split_first()
        .ok_or(SimError::Missing("an argv to run"))?;
    Ok(Command::new(program)
        .args(arguments)
        .current_dir(confine.tree())
        .env_clear()
        .env("PATH", path)
        .env("TMPDIR", tmp)
        .output()?)
}

/// A path whose bytes are not UTF-8.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn non_utf8_path() -> PathBuf {
    use std::os::unix::ffi::OsStringExt as _;
    PathBuf::from(std::ffi::OsString::from_vec(vec![b'a', 0xff, b'b']))
}

/// No other target can name such a path in a test; it gets a UTF-8 one and the case is moot.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn non_utf8_path() -> PathBuf {
    PathBuf::from("\u{fffd}")
}

/// `/bin/sh -c script` as an argv.
fn shell(script: &str) -> [String; 3] {
    [
        String::from("/bin/sh"),
        String::from("-c"),
        script.to_owned(),
    ]
}

/// Each platform's wrapper argv is exactly the reference spelling, in front of the command.
#[test]
fn sim_confine_argv_per_platform_is_exact() -> TestResult {
    let (_root, state, tree, tmp) = layout("argv", "w")?;
    let copy = tree.parent().ok_or("the workspace copy")?;
    let command = [String::from("echo"), String::from("hi there")];
    let mac = Confine::new(Platform::Macos, &state, &tree, &tmp)?;
    assert_eq!(
        mac.wrap(&command),
        vec![
            String::from(SANDBOX_EXEC),
            String::from("-p"),
            reference_profile(&state, copy, &tree, &tmp),
            String::from("echo"),
            String::from("hi there"),
        ]
    );
    let read = "execute,read-file,read-dir";
    let write = "execute,write-file,read-file,read-dir,remove-dir,remove-file,make-char,make-dir,\
make-reg,make-sock,make-fifo,make-block,make-sym,refer,truncate";
    let linux = confined(Platform::Linux, &state, &tree, &tmp)?;
    let argv = linux.wrap(&command);
    let head = [
        String::from(SETPRIV),
        String::from("--landlock-access"),
        String::from("fs"),
    ];
    assert_eq!(argv.get(..head.len()), Some(head.as_slice()));
    let mut rest = argv.get(head.len()..).ok_or("the argv")?.iter();
    let mut rules: Vec<&str> = Vec::new();
    let mut command_again = Vec::new();
    while let Some(word) = rest.next() {
        if word == "--" {
            command_again = rest.cloned().collect();
            break;
        }
        assert_eq!(word, "--landlock-rule");
        rules.push(rest.next().ok_or("a rule")?.as_str());
    }
    assert_eq!(command_again, command);
    let read_prefix = format!("path-beneath:{read}:");
    let reads: Vec<&str> = rules
        .iter()
        .filter_map(|rule| rule.strip_prefix(&read_prefix))
        .collect();
    let tail: Vec<String> = vec![
        format!("path-beneath:{read}:{}", copy.display()),
        format!("path-beneath:{write}:{}", tree.display()),
        format!("path-beneath:{write}:{}", tmp.display()),
        String::from("path-beneath:write-file,read-file,truncate:/dev/null"),
    ];
    assert_eq!(
        rules.get(rules.len().saturating_sub(4)..),
        Some(
            tail.iter()
                .map(String::as_str)
                .collect::<Vec<&str>>()
                .as_slice()
        )
    );
    assert_eq!(
        reads.len() + 3,
        rules.len(),
        "the other rules are the copy's read rule and reads"
    );
    let copy_path = copy.display().to_string();
    assert!(
        reads.contains(&copy_path.as_str()),
        "the session's copy is a read rule: {reads:?}"
    );
    let roots: Vec<&str> = reads
        .iter()
        .copied()
        .filter(|rule| *rule != copy_path.as_str())
        .collect();
    check_read_roots(&roots, state.parent().ok_or("the fake root")?, &state)?;
    assert_eq!(mac.tree(), tree.as_path());
    assert_eq!(linux.scratch(), tmp.as_path());
    Ok(())
}

/// The read roots a Linux argv carries, judged against the fake root the walk descends: the
/// state directory is a root of none, every other child directory is a root or beneath one,
/// and the `/proc` grants are the command's own entry and the host's tunables.
fn check_read_roots(reads: &[&str], root: &Path, state: &Path) -> TestResult {
    let (top, proc_tail) = reads.split_at(reads.len().saturating_sub(PROC_READS.len()));
    assert_eq!(
        proc_tail, PROC_READS,
        "the command sees itself and the host's tunables"
    );
    let mut sorted = top.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        top,
        sorted.as_slice(),
        "the read roots are sorted and unique"
    );
    let canonical = std::fs::canonicalize(state)?;
    assert!(
        top.iter().all(|granted| !canonical.starts_with(granted)),
        "no read root holds the state directory: {top:?}"
    );
    // Every child directory beside the state directory is a root or beneath one, so a list
    // cut short at any sibling loses a directory the command had.
    let mut siblings = 0_u64;
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        let real = std::fs::canonicalize(&path)?;
        if real == canonical || canonical.starts_with(&real) {
            continue;
        }
        siblings = siblings.saturating_add(1);
        assert!(
            top.iter().any(|granted| real.starts_with(granted)),
            "{} is not a read root and nothing grants it: {top:?}",
            real.display()
        );
    }
    assert!(siblings > 1, "the fake root's siblings were read");
    Ok(())
}

/// Each way a command cannot be confined is its own typed variant, and none runs the command.
#[test]
fn sim_confine_each_refusal_gets_its_exact_variant() -> TestResult {
    let (root, state, tree, tmp) = layout("refuse", "w")?;
    assert!(matches!(
        Confine::new(Platform::Other, &state, &tree, &tmp),
        Err(ConfineError::Unsupported {
            platform: "this platform"
        })
    ));
    assert!(matches!(
        supported(Platform::Other),
        Err(ConfineError::Unsupported { .. })
    ));
    assert!(matches!(
        supported_with(Platform::Macos, &|_| false),
        Err(ConfineError::MissingWrapper { tool: SANDBOX_EXEC })
    ));
    assert!(matches!(
        supported_with(Platform::Linux, &|_| false),
        Err(ConfineError::MissingWrapper { tool: SETPRIV })
    ));
    assert!(supported_with(Platform::Linux, &|_| true).is_ok());
    let gone = root.join("absent");
    assert!(matches!(
        Confine::new(Platform::Macos, &state, &gone, &tmp),
        Err(ConfineError::Path { .. })
    ));
    // A copy that is not beneath the state directory, here the state itself, would open it.
    let copy = tree.parent().ok_or("the workspace copy")?;
    assert!(matches!(
        Confine::new(Platform::Macos, copy, &tree, &tmp),
        Err(ConfineError::NotBeneath { .. })
    ));
    // A path that is not UTF-8 cannot be written into a profile or a rule.
    assert!(matches!(
        text(&non_utf8_path()),
        Err(ConfineError::Invalid(_))
    ));
    assert!(root.is_dir());
    // A wrapper that is not there, one that fails, and one that does not confine.
    let path = path_env()?;
    for (prefix, want) in [
        (
            vec![String::from("/nonexistent/confine-wrapper")],
            "did not start",
        ),
        (vec![String::from("/usr/bin/false")], "exited"),
        (Vec::new(), "was not refused"),
    ] {
        let broken = Confine {
            tree: tree.clone(),
            scratch: tmp.clone(),
            prefix,
        };
        match broken.probe(&path) {
            Err(ConfineError::Probe { detail }) => {
                assert!(detail.contains(want), "{want}: {detail}");
            }
            other => return Err(format!("{want}: expected a probe refusal, got {other:?}").into()),
        }
    }
    let outside = tmp.parent().ok_or("no parent")?.join("probe-outside");
    assert!(!outside.exists(), "a probe leaves nothing beside the tree");
    Ok(())
}

/// Whether the parentheses of an SBPL profile balance outside its strings.
fn balanced(profile: &str) -> bool {
    let (mut depth, mut inside, mut escaped) = (0_usize, false, false);
    for character in profile.chars() {
        if inside {
            match (escaped, character) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => inside = false,
                (false, _) => {}
            }
        } else {
            match character {
                '"' => inside = true,
                '(' => depth = depth.saturating_add(1),
                ')' => match depth.checked_sub(1) {
                    Some(left) => depth = left,
                    None => return false,
                },
                _ => {}
            }
        }
    }
    depth == 0 && !inside
}

/// Hostile directory names stay one string inside the profile and the estate accepts the
/// profile; on the host's own platform the kernel then confines to exactly that directory.
#[test]
fn sim_confine_profile_is_validated_by_the_estate_owner() -> TestResult {
    let path = path_env()?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let first = PIECES.get(rng.index(PIECES.len())?).ok_or("a piece")?;
        let second = PIECES.get(rng.index(PIECES.len())?).ok_or("a piece")?;
        let name = format!("{first}{second}");
        let (_root, state, tree, tmp) = layout("names", &name)?;
        let mac = Confine::new(Platform::Macos, &state, &tree, &tmp)?;
        let profile = mac.prefix.get(2).ok_or("the profile")?;
        assert!(balanced(profile), "seed {seed} {name}: {profile}");
        confined(Platform::host(), &state, &tree, &tmp)?.probe(&path)?;
    }
    Ok(())
}

/// The kernel refuses a write beside the tree, through every route a command has, and the
/// folder beside it stays byte-identical; a write inside the tree lands.
#[test]
fn sim_confine_kernel_refuses_a_write_beside_the_tree() -> TestResult {
    let path = path_env()?;
    for seed in 0..SEEDS {
        let (root, state, tree, tmp) = layout("kernel", "w")?;
        let beside = root.join("beside");
        std::fs::create_dir_all(&beside)?;
        let victim = beside.join("victim.txt");
        std::fs::write(&victim, format!("original {seed}"))?;
        let before = tree_digest(&beside, false)?;
        let confine = confined(Platform::host(), &state, &tree, &tmp)?;
        let routes = [
            format!("printf x > {}/new.txt", beside.display()),
            format!("printf x > {}", victim.display()),
            format!("printf x >> {}", victim.display()),
            format!("rm {}", victim.display()),
            format!("mv {} moved.txt", victim.display()),
            format!("ln {} hard.txt", victim.display()),
            format!("ln -s {} soft.txt && printf x > soft.txt", victim.display()),
            format!("cd {} && printf x > pwned.txt", beside.display()),
            String::from("printf x > ../escape.txt"),
        ];
        for route in routes {
            let out = run_confined(&confine, &shell(&route), &path, &tmp)?;
            assert!(!out.status.success(), "seed {seed}: `{route}` ran");
            let said = String::from_utf8_lossy(&out.stderr).to_lowercase();
            // Landlock answers a link or rename that would reparent a file across the rule
            // boundary with EXDEV, not EACCES, when the refer right is not granted (Linux
            // Landlock documentation, LANDLOCK_ACCESS_FS_REFER); that is the kernel's refusal.
            assert!(
                said.contains("not permitted")
                    || said.contains("permission denied")
                    || said.contains("cross-device link"),
                "seed {seed}: `{route}` failed for another reason: {said}"
            );
        }
        assert_eq!(tree_digest(&beside, false)?, before, "seed {seed}");
        assert!(!root.join("escape.txt").exists(), "seed {seed}");
        let inside = "printf ok > inside.txt && printf ok > \"$TMPDIR/t\" && : > /dev/null";
        let out = run_confined(&confine, &shell(inside), &path, &tmp)?;
        assert!(
            out.status.success(),
            "seed {seed}: a write inside is refused"
        );
        assert_eq!(std::fs::read_to_string(tree.join("inside.txt"))?, "ok");
    }
    Ok(())
}

/// The kernel refuses every read of the state directory, where `executor.db` holds one key
/// and the tenant key digests, so no confined command recovers a signing seed, and the work
/// tree stays readable (larry#322, ADR 0081).
#[test]
fn sim_confine_kernel_refuses_the_state_directory_for_every_seed() -> TestResult {
    let path = path_env()?;
    for seed in 0..SEEDS {
        let (_root, state, tree, tmp) = layout("state-read", "w")?;
        let db = state.join("executor.db");
        let seed_bytes = format!(
            "signing seed {seed} {:016x}",
            seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        );
        std::fs::write(&db, seed_bytes.as_bytes())?;
        let before = std::fs::read(&db)?;
        std::fs::write(tree.join("inside.txt"), b"readable")?;
        let confine = confined(Platform::host(), &state, &tree, &tmp)?;
        let inside = run_confined(&confine, &shell("cat inside.txt"), &path, &tmp)?;
        assert!(
            inside.status.success() && inside.stdout == b"readable",
            "seed {seed}: the work tree is not readable"
        );
        // The session's own copy stays readable: its git directory is what every `git`
        // command in the tree resolves first, and it sits beside the tree (larry#322).
        let copy = tree.parent().ok_or("the workspace copy")?;
        let git = copy.join("git");
        std::fs::create_dir_all(&git)?;
        std::fs::write(git.join("HEAD"), b"ref: refs/heads/base\n")?;
        let resolved = run_confined(
            &confine,
            &shell(&format!("cat {}/git/HEAD", copy.display())),
            &path,
            &tmp,
        )?;
        assert!(
            resolved.status.success() && resolved.stdout == b"ref: refs/heads/base\n",
            "seed {seed}: the copy's git directory is not readable"
        );
        let routes = [
            format!("cat {}", db.display()),
            format!("dd if={} bs=4096 count=1 2>/dev/null", db.display()),
            format!("base64 {}", db.display()),
            format!("cat {}../../../executor.db", tree.display()),
        ];
        for route in routes {
            let out = run_confined(&confine, &shell(&route), &path, &tmp)?;
            let shown = String::from_utf8_lossy(&out.stdout);
            assert!(
                !shown.contains("signing seed") && !shown.contains(&seed_bytes),
                "seed {seed}: `{route}` read the key store"
            );
        }
        assert_eq!(
            std::fs::read(&db)?,
            before,
            "seed {seed}: the key store changed"
        );
    }
    Ok(())
}

/// The executor's environment holds `LARRY_MODEL_KEY` and every `LARRY_KEY_*`, and a
/// confined command cannot read the environment of the process that spawned it: Linux
/// grants no `/proc` entry but the command's own (larry#322, ADR 0081).
#[cfg(target_os = "linux")]
#[test]
fn sim_confine_kernel_hides_the_executors_environment() -> TestResult {
    let path = path_env()?;
    let (_root, state, tree, tmp) = layout("environ", "w")?;
    let confine = confined(Platform::host(), &state, &tree, &tmp)?;
    let secret = "sk-larry-322-model-key";
    let inner = confine.wrap(&shell("cat /proc/$PPID/environ"));
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg("\"$@\"")
        .arg("larry-322-probe")
        .args(&inner)
        .current_dir(confine.tree())
        .env_clear()
        .env("PATH", &path)
        .env("LARRY_KEY_SIM", secret)
        .env("LARRY_MODEL_KEY", secret);
    let out = command.output()?;
    let shown = String::from_utf8_lossy(&out.stdout);
    assert!(
        !shown.contains(secret),
        "a confined command read the executor's environment: {shown}"
    );
    assert!(
        !out.status.success(),
        "reading the executor's environment was allowed"
    );
    Ok(())
}

/// A confined command reads everywhere but the state directory: a directory beside the
/// state directory's parent stays a read root, a sibling link into the state directory is
/// never one, and a file in the state directory stays refused (larry#322, ADR 0081).
#[test]
fn sim_confine_reads_stay_open_beside_the_state() -> TestResult {
    let root = scratch("beside")?;
    let fake = root.join("fake");
    let state = fake.join("a/b/state");
    let tree = fake.join("proj/tree");
    let tmp = fake.join("proj/tmp");
    let keep = fake.join("a/keep");
    for dir in [&state, &tree, &tmp, &keep] {
        std::fs::create_dir_all(dir)?;
    }
    let note = keep.join("note.txt");
    std::fs::write(&note, b"readable")?;
    std::fs::write(state.join("executor.db"), b"signing seed beside")?;
    let link = fake.join("a/deep");
    std::os::unix::fs::symlink(&state, &link)?;
    link_shared(&fake)?;
    let roots = read_roots_below(&fake, &state)?;
    assert!(
        roots.iter().any(|granted| granted == &keep),
        "a directory beside the state directory stays a read root: {roots:?}"
    );
    assert!(
        roots
            .iter()
            .all(|granted| !granted.starts_with(&state) && !state.starts_with(granted)),
        "no read root holds the state directory or lies beneath it, the link included: {roots:?}"
    );
    #[cfg(target_os = "linux")]
    {
        match Confine::within(Platform::host(), &fake, &state, &tree, &tmp) {
            Ok(confine) => {
                let path = path_env()?;
                kernel_reads_beside_the_state(&confine, &state, &path, &tmp, &note, &link)?;
            }
            Err(ConfineError::TooManyRoots { roots, bound }) => {
                let mut errors = std::io::stderr().lock();
                writeln!(
                    errors,
                    "{{\"metric\":\"kernel_skipped\",\"rules\":{roots},\"bound\":{bound}}}"
                )?;
            }
            Err(other) => return Err(format!("the confinement failed: {other}").into()),
        }
    }
    Ok(())
}

/// The kernel, not only the rule list: the file beside the state directory is read, the
/// sibling link into it and the state directory itself are refused (larry#322).
#[cfg(target_os = "linux")]
fn kernel_reads_beside_the_state(
    confine: &Confine,
    state: &Path,
    path: &str,
    tmp: &Path,
    note: &Path,
    link: &Path,
) -> TestResult {
    let read = run_confined(
        confine,
        &shell(&format!("cat {}", note.display())),
        path,
        tmp,
    )?;
    assert!(
        read.status.success() && read.stdout == b"readable",
        "a file beside the state directory is readable: {}",
        String::from_utf8_lossy(&read.stderr)
    );
    let through = run_confined(
        confine,
        &shell(&format!("cat {}/executor.db", link.display())),
        path,
        tmp,
    )?;
    assert!(
        !through.status.success()
            && !String::from_utf8_lossy(&through.stdout).contains("signing seed"),
        "a sibling link into the state directory is not a way in"
    );
    let db = run_confined(
        confine,
        &shell(&format!("cat {}/executor.db", state.display())),
        path,
        tmp,
    )?;
    assert!(
        !db.status.success() && !String::from_utf8_lossy(&db.stdout).contains("signing seed"),
        "the state directory is refused"
    );
    Ok(())
}

/// A state directory that is the walk's own root cannot be granted around: the walk would
/// grant nothing and the command would read nothing. It is refused typed instead, never
/// built into a confinement that denies the whole host (larry#322).
#[test]
fn sim_confine_a_state_at_the_walk_root_is_refused() {
    assert!(matches!(
        read_roots_below(Path::new("/"), Path::new("/")),
        Err(ConfineError::StateAtRoot)
    ));
}

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
    let mut errors = std::io::stderr().lock();
    match read_roots_below(&root, &state) {
        Ok(roots) => {
            writeln!(
                errors,
                "{{\"metric\":\"read_roots\",\"siblings\":15,\"rules\":{}}}",
                roots.len()
            )?;
            assert!(
                roots.iter().any(|granted| granted == &root.join("sib-0")),
                "a sibling of the state directory is a read root: {roots:?}"
            );
        }
        // A host whose path to the state directory already holds more entries than the
        // allowlist leaves is refused typed with both counts, never built past the bound.
        Err(ConfineError::TooManyRoots { roots, bound }) => {
            writeln!(
                errors,
                "{{\"metric\":\"read_roots_refused\",\"rules\":{roots},\"bound\":{bound}}}"
            )?;
        }
        Err(other) => {
            return Err(format!("the roots were refused for another reason: {other}").into());
        }
    }
    let crowded = scratch("crowded")?;
    let crowded_state = crowded.join("state");
    std::fs::create_dir_all(&crowded_state)?;
    for n in 0..=MAX_READ_ROOTS {
        std::fs::create_dir_all(crowded.join(format!("sib-{n}")))?;
    }
    assert!(
        matches!(
            read_roots_below(&crowded, &crowded_state),
            Err(ConfineError::TooManyRoots { .. })
        ),
        "past the bound the confinement is refused typed"
    );
    Ok(())
}

/// The p50, p95 and p99 of `samples`, by nearest rank.
fn percentiles(samples: &mut [u64]) -> Option<[u64; 3]> {
    samples.sort_unstable();
    let at = |per_mille: usize| {
        let rank = samples
            .len()
            .saturating_mul(per_mille)
            .div_ceil(1000)
            .saturating_sub(1);
        samples.get(rank).copied()
    };
    Some([at(500)?, at(950)?, at(990)?])
}

/// The cost of the wrapper against the same spawn without it, whole path (spawn, run, wait).
/// It prints p50/p95/p99 and the sample count and asserts only that every sample ran.
#[test]
fn bench_confine_overhead_is_measured_against_the_unconfined_spawn() -> TestResult {
    let path = path_env()?;
    let (root, _state, tree, tmp) = layout("overhead", "w")?;
    let confine = confined(Platform::host(), &_state, &tree, &tmp)?;
    let ledger = Ledger::open(&root)?;
    let command = shell(":");
    let mut plain = Vec::new();
    let mut confined = Vec::new();
    for (place, into) in [
        (Place::Open(&tree), &mut plain),
        (Place::Confined(&confine), &mut confined),
    ] {
        for sample in 0..SAMPLES {
            let result = run_command(
                &command,
                place,
                &path,
                Duration::from_secs(30),
                &ledger,
                &format!("bench-{sample}"),
                &|| false,
            )?;
            assert_eq!(result.exit_code(), Some(0), "a sample did not run");
            // The run's own measured wall time, spawn to reaped group, in milliseconds.
            let json = result.into_json();
            into.push(json.at("/duration_ms")?.as_u64().ok_or("duration_ms")?);
        }
    }
    let [p50, p95, p99] = percentiles(&mut plain).ok_or("plain samples")?;
    let [c50, c95, c99] = percentiles(&mut confined).ok_or("confined samples")?;
    let mut errors = std::io::stderr().lock();
    writeln!(
        errors,
        "{{\"metric\":\"confine_run_ms\",\"samples\":{SAMPLES},\"plain\":{{\"p50\":{p50},\"p95\":{p95},\"p99\":{p99}}},\"confined\":{{\"p50\":{c50},\"p95\":{c95},\"p99\":{c99}}}}}"
    )?;
    assert_eq!((plain.len(), confined.len()), (SAMPLES, SAMPLES));
    Ok(())
}
