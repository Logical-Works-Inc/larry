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
    Confine, ConfineError, Platform, SANDBOX_EXEC, SETPRIV, supported, supported_with, text,
};
use crate::executor::Ledger;
use crate::executor::run::{Place, run_command};
use crate::executor::sim_tree::{Scratch, path_env, scratch, tree_digest};
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

/// A tree and a scratch directory under a fresh root, as a workspace lays them out.
fn layout(label: &str, name: &str) -> Result<(Scratch, PathBuf, PathBuf), SimError> {
    let root = scratch(label)?;
    let tree = root.join(name).join("tree");
    let tmp = root.join(name).join("tmp");
    std::fs::create_dir_all(&tree)?;
    std::fs::create_dir_all(&tmp)?;
    Ok((root, tree, tmp))
}

/// The Seatbelt profile the reference spelling writes for `tree` and `tmp`.
fn reference_profile(tree: &Path, tmp: &Path) -> String {
    format!(
        "(version 1)(allow default)(deny file-write* (require-not (require-any \
         (subpath \"{}\") (subpath \"{}\") (literal \"/dev/null\") (literal \"/dev/tty\"))))",
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
    let (_root, tree, tmp) = layout("argv", "w")?;
    let command = [String::from("echo"), String::from("hi there")];
    let mac = Confine::new(Platform::Macos, &tree, &tmp)?;
    assert_eq!(
        mac.wrap(&command),
        vec![
            String::from(SANDBOX_EXEC),
            String::from("-p"),
            reference_profile(&tree, &tmp),
            String::from("echo"),
            String::from("hi there"),
        ]
    );
    let read = "execute,read-file,read-dir";
    let write = "execute,write-file,read-file,read-dir,remove-dir,remove-file,make-char,make-dir,\
make-reg,make-sock,make-fifo,make-block,make-sym,refer,truncate";
    let linux = Confine::new(Platform::Linux, &tree, &tmp)?;
    let mut want = vec![
        String::from(SETPRIV),
        String::from("--landlock-access"),
        String::from("fs"),
    ];
    for rule in [
        format!("path-beneath:{read}:/"),
        format!("path-beneath:{write}:{}", tree.display()),
        format!("path-beneath:{write}:{}", tmp.display()),
        String::from("path-beneath:write-file,read-file,truncate:/dev/null"),
    ] {
        want.push(String::from("--landlock-rule"));
        want.push(rule);
    }
    want.push(String::from("--"));
    want.extend(command.iter().cloned());
    assert_eq!(linux.wrap(&command), want);
    assert_eq!(mac.tree(), tree.as_path());
    assert_eq!(linux.scratch(), tmp.as_path());
    Ok(())
}

/// Each way a command cannot be confined is its own typed variant, and none runs the command.
#[test]
fn sim_confine_each_refusal_gets_its_exact_variant() -> TestResult {
    let (root, tree, tmp) = layout("refuse", "w")?;
    assert!(matches!(
        Confine::new(Platform::Other, &tree, &tmp),
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
        Confine::new(Platform::Macos, &gone, &tmp),
        Err(ConfineError::Path { .. })
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
        let (_root, tree, tmp) = layout("names", &name)?;
        let mac = Confine::new(Platform::Macos, &tree, &tmp)?;
        let profile = mac.prefix.get(2).ok_or("the profile")?;
        assert!(balanced(profile), "seed {seed} {name}: {profile}");
        Confine::new(Platform::host(), &tree, &tmp)?.probe(&path)?;
    }
    Ok(())
}

/// The kernel refuses a write beside the tree, through every route a command has, and the
/// folder beside it stays byte-identical; a write inside the tree lands.
#[test]
fn sim_confine_kernel_refuses_a_write_beside_the_tree() -> TestResult {
    let path = path_env()?;
    for seed in 0..SEEDS {
        let (root, tree, tmp) = layout("kernel", "w")?;
        let beside = root.join("beside");
        std::fs::create_dir_all(&beside)?;
        let victim = beside.join("victim.txt");
        std::fs::write(&victim, format!("original {seed}"))?;
        let before = tree_digest(&beside, false)?;
        let confine = Confine::new(Platform::host(), &tree, &tmp)?;
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
    let (root, tree, tmp) = layout("overhead", "w")?;
    let confine = Confine::new(Platform::host(), &tree, &tmp)?;
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
            into.push(json["duration_ms"].as_u64().ok_or("duration_ms")?);
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
