//! Every lane runs every time, and only an all-green run writes a receipt (larry#53, ADR 0010;
//! larry#440, ADR 0123). Source: CWE-390 (an error condition detected and not acted on), CWE-754
//! (improper check for an unusual condition: a signal death has no exit code), POSIX.1-2024 `exec`
//! (a missing program) and `wait` (exit status or terminating signal). Requirement: a red lane is
//! reported by name with how it ended, the lanes after it still run, the receipt stays as it was,
//! and a green run's receipt names exactly the checkout, branch and head it ran on.

use std::collections::BTreeMap;
use std::fs;

use lgwks_std::json::{self, Value, json};
use lgwks_std::seeded::Seeded;

use crate::Check;
use crate::rig::{LANES, Rig, attests, entries, exactly, lane_log, lanes_run, ok_line, whole_run};

/// Seeds the red-lane simulation runs.
const SEEDS: u64 = 48;

/// What the plan makes one lane do.
#[derive(Debug, Clone, Copy)]
enum Fate {
    /// Exit zero.
    Pass,
    /// Exit with this non-zero code.
    Exit(u8),
    /// Die by `SIGKILL`, as the out-of-memory killer ends a build.
    Killed,
}

impl Fate {
    /// The plan word.
    fn how(self) -> String {
        match self {
            Fate::Pass => "0".to_owned(),
            Fate::Exit(code) => code.to_string(),
            Fate::Killed => "killed".to_owned(),
        }
    }

    /// How the log says a red lane ended; `None` for a green one.
    fn ended(self) -> Option<String> {
        match self {
            Fate::Pass => None,
            Fate::Exit(code) => Some(format!("failed with exit {code}")),
            Fate::Killed => Some("was killed by a signal".to_owned()),
        }
    }
}

/// The fate of every lane for `seed`: one seed in four is all green, the rest draw each lane.
fn fates(seed: u64) -> Check<BTreeMap<&'static str, Fate>> {
    let mut draw = Seeded::from_seed(seed);
    let green = draw.below(4)? == 0;
    let mut fates = BTreeMap::new();
    for (_, line) in LANES {
        let fate = if green {
            Fate::Pass
        } else {
            match draw.below(4)? {
                0 => Fate::Exit(u8::try_from(draw.below(255)?.saturating_add(1))?),
                1 => Fate::Killed,
                _ => Fate::Pass,
            }
        };
        fates.insert(line, fate);
    }
    Ok(fates)
}

/// One seeded run against the reference: its log, its refusal, what ran and what the receipt holds.
/// Answers whether the run was green.
fn seeded_run(rig: &Rig, seed: u64) -> Check<bool> {
    let fates = fates(seed)?;
    let words: Vec<(&str, String)> = fates
        .iter()
        .map(|(&line, &fate)| (line, fate.how()))
        .collect();
    let plan: Vec<(&str, &str)> = words
        .iter()
        .map(|pair| (pair.0, pair.1.as_str()))
        .filter(|&(_, how)| how != "0")
        .collect();
    rig.plan(&plan)?;
    let directory = rig.default_receipts();
    rig.write_earlier_receipt()?;
    let output = rig.run()?;
    let ended = |line: &str| fates.get(line).and_then(|fate| fate.ended());
    let red: Vec<&str> = LANES
        .iter()
        .filter(|&&(_, line)| ended(line).is_some())
        .map(|&(name, _)| name)
        .collect();
    let log = lane_log(&|_, line| ended(line));
    let ran = rig.take_ran()?;
    if red.is_empty() {
        let head = rig.head()?;
        let short = head.get(..7).ok_or("a head shorter than seven")?;
        let whole = format!("{log}{}", ok_line(&directory, short));
        exactly(&output, 0, &whole, "");
        assert_eq!(ran, whole_run(), "seed {seed}: every call in order");
        attests(&directory, rig.root(), "main", &head)?;
        return Ok(true);
    }
    let refusal = format!(
        "local-ci: red lanes: {}; no receipt written\n",
        red.join(", ")
    );
    exactly(&output, 1, &log, &refusal);
    assert_eq!(
        ran,
        lanes_run(),
        "seed {seed}: every lane ran, then nothing asked for a branch or head"
    );
    rig.earlier_receipt_stands(&format!("seed {seed}"))?;
    Ok(false)
}

/// Any set of red lanes, each by exit code or signal: every lane still runs in order, each red one
/// is reported as it fails and named in the refusal, and no receipt is written.
#[test]
fn sim_every_set_of_red_lanes_runs_every_lane_and_writes_no_receipt() -> Check {
    let rig = Rig::new()?;
    let mut green = 0_u64;
    for seed in 0..SEEDS {
        if seeded_run(&rig, seed)? {
            green = green.saturating_add(1);
        }
    }
    assert!(
        green > 0 && green < SEEDS,
        "the seeds reach both a green and a red run: {green} of {SEEDS} green"
    );
    Ok(())
}

/// A lane whose program is not there (a tool never installed, a build that left no binary) is red
/// by name with the cause, and every other lane still runs.
#[test]
fn a_missing_lane_program_is_red_by_name_and_the_rest_still_run() -> Check {
    let rig = Rig::new()?;
    fs::remove_file(rig.bin("lgwks-deps"))?;
    fs::remove_file(rig.root().join("target/debug/larry"))?;
    let output = rig.run()?;
    let gone = std::io::Error::from_raw_os_error(2).to_string();
    let log = lane_log(&|name, _| {
        ["governance", "tracker", "mascot", "deps"]
            .contains(&name)
            .then(|| format!("did not start: {gone}"))
    });
    exactly(
        &output,
        1,
        &log,
        "local-ci: red lanes: governance, tracker, mascot, deps; no receipt written\n",
    );
    let started: Vec<String> = lanes_run()
        .into_iter()
        .filter(|line| !line.starts_with("target/") && !line.starts_with("lgwks-deps"))
        .collect();
    assert_eq!(rig.take_ran()?, started, "every lane that can start ran");
    assert!(
        !rig.default_receipts().exists(),
        "no receipt directory is made for a red run"
    );
    Ok(())
}

/// The whitespace lane is the real `git diff --check`: a tracked line with trailing blanks is red.
#[test]
fn a_trailing_blank_in_a_tracked_file_turns_the_whitespace_lane_red() -> Check {
    let rig = Rig::new()?;
    fs::write(rig.root().join("README"), "trailing blank \n")?;
    let output = rig.run()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("README:1: trailing whitespace.\n"),
        "git names the line: {stdout}"
    );
    assert!(
        stdout.ends_with("\nlocal-ci: RED lane whitespace failed with exit 2\n"),
        "the lane is red with git's status: {stdout}"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "local-ci: red lanes: whitespace; no receipt written\n",
        "the refusal"
    );
    assert_eq!(output.status.code(), Some(1), "exit status");
    assert!(!rig.default_receipts().exists(), "no receipt");
    Ok(())
}

/// Started deep in the checkout, the run moves to the root: the relative `target/debug/larry`
/// resolves there and the default receipt lands under the root, not under the caller.
#[test]
fn a_run_from_a_subdirectory_runs_the_lanes_from_the_root() -> Check {
    let rig = Rig::new()?;
    let inner = rig.root().join("src");
    let output = rig.command(&inner, &[]).output()?;
    let head = rig.head()?;
    let short = head.get(..7).ok_or("a head shorter than seven")?;
    let log = format!(
        "{}{}",
        lane_log(&|_, _| None),
        ok_line(&rig.default_receipts(), short)
    );
    exactly(&output, 0, &log, "");
    assert_eq!(rig.take_ran()?, whole_run(), "every call in order");
    attests(&rig.default_receipts(), rig.root(), "main", &head)?;
    assert!(
        !inner.join("target").exists(),
        "nothing is written beside the caller"
    );
    Ok(())
}

/// A detached head, as a CI checkout leaves it, is attested as the branch `HEAD`.
#[test]
fn a_detached_head_is_attested_as_head() -> Check {
    let rig = Rig::new()?;
    rig.git(&["checkout", "-q", "--detach"])?;
    let output = rig.run()?;
    assert_eq!(output.status.code(), Some(0), "a detached run is green");
    attests(&rig.default_receipts(), rig.root(), "HEAD", &rig.head()?)
}

/// The receipt goes under `--receipts`, else `LARRY_STATE_DIR`, and a relative one means where the
/// caller stood, though the run has moved to the root by the time it writes.
#[test]
fn a_relative_receipt_directory_is_written_beside_the_caller() -> Check {
    let rig = Rig::new()?;
    let inner = rig.root().join("src");
    let head = rig.head()?;
    let by_state = rig
        .command(&inner, &[])
        .env("LARRY_STATE_DIR", "state")
        .output()?;
    assert_eq!(by_state.status.code(), Some(0), "the state run is green");
    attests(&inner.join("state"), rig.root(), "main", &head)?;
    let by_flag = rig
        .command(&inner, &["--receipts", "flag/deep"])
        .env("LARRY_STATE_DIR", "never")
        .output()?;
    assert_eq!(by_flag.status.code(), Some(0), "the flag run is green");
    attests(&inner.join("flag/deep"), rig.root(), "main", &head)?;
    assert!(
        !inner.join("never").exists() && !rig.default_receipts().exists(),
        "the losing sources get nothing"
    );
    Ok(())
}

/// With `CARGO_TARGET_DIR` set, the `target/` lanes run the binary built there and the receipt
/// goes under it: a relocated target directory is never answered by a stale in-tree binary.
#[test]
fn a_relocated_target_directory_runs_its_binary_and_holds_the_receipt() -> Check {
    let rig = Rig::new()?;
    let moved = rig.beside("moved target");
    rig.stub_larry(&moved)?;
    fs::write(rig.root().join("target/debug/larry"), "#!/bin/sh\nexit 9\n")?;
    let output = rig
        .command(rig.root(), &[])
        .env("CARGO_TARGET_DIR", &moved)
        .output()?;
    let head = rig.head()?;
    let short = head.get(..7).ok_or("a head shorter than seven")?;
    let directory = moved.join("local-ci");
    let log = format!("{}{}", lane_log(&|_, _| None), ok_line(&directory, short));
    exactly(&output, 0, &log, "");
    assert_eq!(rig.take_ran()?, whole_run(), "every call in order");
    attests(&directory, rig.root(), "main", &head)
}

/// The log's short head is the first seven bytes, or the whole head when it is shorter or byte
/// seven splits a character: a hostile `git` never makes the run slice through one.
#[test]
fn the_logged_short_head_holds_at_every_boundary() -> Check {
    let rig = Rig::new()?;
    let directory = rig.default_receipts();
    for (head, short) in [
        ("abcdef", "abcdef"),
        ("abcdefg", "abcdefg"),
        ("abcdefgh", "abcdefg"),
        ("abcdef\u{e9}h", "abcdef\u{e9}h"),
    ] {
        let say = format!("say:{head}");
        rig.plan(&[("git rev-parse HEAD", &say)])?;
        let output = rig.run()?;
        let log = format!("{}{}", lane_log(&|_, _| None), ok_line(&directory, short));
        exactly(&output, 0, &log, "");
        attests(&directory, rig.root(), "main", head)?;
    }
    Ok(())
}

/// `--list` prints every lane in order as JSON and runs nothing, even inside a checkout.
#[test]
fn list_prints_every_lane_in_order_and_runs_nothing() -> Check {
    let rig = Rig::new()?;
    let output = rig.command(rig.root(), &["--list"]).output()?;
    assert_eq!(output.status.code(), Some(0), "exit status");
    assert_eq!(output.stderr, b"", "nothing on standard error");
    let listed: Value = json::from_slice(&output.stdout)?;
    let declared: Vec<Value> = LANES
        .iter()
        .map(|&(name, line)| json!({ "name": name, "command": line }))
        .collect();
    assert_eq!(listed, Value::Array(declared), "the declared lanes");
    assert_eq!(rig.take_ran()?, Vec::<String>::new(), "nothing ran");
    assert_eq!(
        entries(&rig.root().join("target"))?,
        ["debug"],
        "nothing was written"
    );
    Ok(())
}
