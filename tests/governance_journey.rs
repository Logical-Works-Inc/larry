//! Journeys of `larry governance` and `larry tracker` (larry#53, ADR 0010) as a contributor
//! lives them: check the tree before a push, read the failures, regenerate the tracker page,
//! and trust that the gate changed nothing. Every run drives the shipped binary on a temporary
//! copy of the governed files, from that copy's own directory, with a cleared environment.
//!
//! Source: the Python gate these replace (`tools/check_governance.py`, its CLI contract and
//! messages), POSIX `rename(2)` atomicity for the killed write, and NIST/SEMATECH 7.2.6.2
//! (nearest-rank percentiles) for the latency figures. Requirement: a governed tree passes and
//! prints its counts; a broken one fails with every error on stderr; the report digests exactly
//! the governed documents; a stale page is refused until regenerated; a kill -9 mid-write
//! leaves a whole page, and a write the kernel stops at a seeded file-size limit leaves a cut
//! temporary page and the previous whole page; the checker never writes; local-ci runs no
//! Python governance lane.

#[path = "support/deadline.rs"]
mod deadline;
#[path = "../src/governance/fixture.rs"]
mod fixture;
#[path = "support/ps.rs"]
mod ps;
#[path = "support/temp_dir.rs"]
mod temp_dir;

use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use lgwks_bot::kernel::clock::Clock;
use lgwks_std::hash;
use lgwks_std::json::{self, Value};
use lgwks_std::seeded::Seeded;
use temp_dir::TempDir;

/// What every journey returns.
type TestResult = Result<(), Box<dyn Error>>;

#[path = "support/governed_tree.rs"]
mod governed_tree;

use governed_tree::{
    DOCUMENTS, LARRY, ROOT, Tree, digests, edit_tracker, files_under, larry, text,
};

/// Timed checks for the latency figures.
const RUNS: usize = 32;
/// Kills of a tracker write the killed-write journey makes.
const KILLS: usize = 16;
/// Filler added to the tracker data so its render is large enough for a kill to land
/// inside the write: the data, template and page each stay under the 4 MiB bound.
const TRACKER_FILLER: usize = 3 * 1024 * 1024;
/// How long one kill waits for the tracker's temporary page: the write's own event,
/// ended by one deadline, never a count of probes (INVARIANTS, larry#294). Generous
/// because a loaded host can slow the debug binary's render; a finished child ends
/// the wait at once.
const WRITE_PATIENCE: Duration = Duration::from_secs(60);
/// The signal a write past `RLIMIT_FSIZE` raises (25 on Linux and macOS): the kernel stops
/// the writer inside the write, which is the mid-write kill this journey proves.
const SIGXFSZ: i32 = 25;

impl Tree {
    /// A governed fixture tree the checker passes on its own terms (larry#224, larry#270): the
    /// latency figures are measured on it. `fixture::governed` writes every document; only the
    /// retained receipts are copied from the checkout, a frozen set since the reranker mandate
    /// was retired, so no live document reaches the tree (larry#295).
    fn fixture() -> Result<Tree, Box<dyn Error>> {
        let tree = Tree {
            path: TempDir::new()?,
        };
        fixture::governed(&tree.path, &Path::new(ROOT).join("tools").join("receipts"))?;
        let rendered = larry(&["tracker"], &tree.path)?;
        assert_eq!(
            rendered.status.code(),
            Some(0),
            "the fixture page renders: {}",
            text(&rendered.stderr)
        );
        Ok(tree)
    }
}

#[test]
fn a_governed_tree_passes_from_its_own_directory() -> TestResult {
    let tree = Tree::governed()?;
    let checked = larry(&["governance"], &tree.path)?;
    assert_eq!(
        checked.status.code(),
        Some(0),
        "a governed copy passes: {}",
        text(&checked.stderr)
    );
    let stdout = text(&checked.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.first(),
        Some(&"PASS: document structure and governance inventory"),
        "the PASS block opens: {stdout}"
    );
    let counts: Value = json::from_str(lines.get(1).ok_or("no counts line")?)?;
    for label in [
        "requirements",
        "decisions",
        "falsifiers",
        "execution_cells",
        "axes",
    ] {
        assert!(
            counts
                .get(label)
                .and_then(Value::as_u64)
                .is_some_and(|count| count > 0),
            "{label} is counted: {counts}"
        );
    }
    assert_eq!(
        lines.get(3),
        Some(&"NOT RUN: runtime, model evaluations, hardware and human trials"),
        "the gate says what it does not prove"
    );
    let tracked = larry(&["tracker", "--check"], &tree.path)?;
    assert_eq!(
        tracked.status.code(),
        Some(0),
        "the copied page is current: {}",
        text(&tracked.stderr)
    );
    assert_eq!(
        text(&tracked.stdout),
        "PASS: tools/tracker/index.html is current\n",
        "tracker --check says so"
    );
    Ok(())
}

#[test]
fn a_tree_without_its_documents_fails_with_every_error_on_stderr() -> TestResult {
    let failed = larry(&["governance", "--root", "tests"], Path::new(ROOT))?;
    assert_eq!(
        failed.status.code(),
        Some(1),
        "a tree without documents fails"
    );
    assert!(
        failed.stdout.is_empty(),
        "a failure prints nothing on stdout"
    );
    let stderr = text(&failed.stderr);
    assert!(
        stderr.lines().all(|line| line.starts_with("FAIL: ")),
        "every stderr line is a FAIL line: {stderr}"
    );
    for name in DOCUMENTS {
        assert!(
            stderr.contains(&format!("FAIL: missing document: {name}\n")),
            "{name} is named: {stderr}"
        );
    }
    let refused = larry(&["governance", "--root"], Path::new(ROOT))?;
    assert_eq!(refused.status.code(), Some(1), "a bare --root is refused");
    assert!(
        text(&refused.stderr).contains("usage: larry governance"),
        "with its usage line"
    );
    Ok(())
}

#[test]
fn the_report_digests_exactly_the_governed_documents() -> TestResult {
    let tree = Tree::governed()?;
    let reported = larry(&["governance", "--report"], &tree.path)?;
    assert_eq!(reported.status.code(), Some(0), "the report run passes");
    let report: Value = json::from_str(&text(&reported.stdout))?;
    assert_eq!(
        report.get("ok").and_then(Value::as_bool),
        Some(true),
        "the report says ok"
    );
    let digests = report
        .get("blake3")
        .and_then(Value::as_object)
        .ok_or("no blake3 digests")?;
    let mut expected: Vec<String> = DOCUMENTS.iter().map(|name| String::from(*name)).collect();
    let mut nested = Vec::new();
    files_under(&tree.path, &tree.at("docs"), &mut nested)?;
    expected.extend(nested.into_iter().filter(|name| name.ends_with(".md")));
    expected.sort();
    let named: Vec<&str> = digests.iter().map(|(name, _)| name).collect();
    assert_eq!(
        named, expected,
        "the digests name exactly the governed documents"
    );
    for (name, digest) in digests.iter() {
        let bytes = fs::read(tree.at(name))?;
        assert_eq!(
            digest.as_str(),
            Some(hash::blake3(&bytes).to_hex().as_str()),
            "{name} digest"
        );
    }
    Ok(())
}

#[test]
fn the_checker_leaves_the_tree_byte_identical() -> TestResult {
    let tree = Tree::governed()?;
    let before = digests(&tree)?;
    for args in [
        &["governance"][..],
        &["governance", "--report"],
        &["tracker", "--check"],
    ] {
        let ran = larry(args, &tree.path)?;
        assert_eq!(ran.status.code(), Some(0), "{args:?} passes on the copy");
    }
    fs::remove_file(tree.at("log.md"))?;
    let broken = digests(&tree)?;
    let failed = larry(&["governance", "--report"], &tree.path)?;
    assert_eq!(failed.status.code(), Some(1), "a missing log fails");
    assert_eq!(
        digests(&tree)?,
        broken,
        "a failing check writes nothing either"
    );
    let removed: Vec<&(String, String)> =
        before.iter().filter(|entry| entry.0 != "log.md").collect();
    assert_eq!(
        broken.iter().collect::<Vec<_>>(),
        removed,
        "the passing checks wrote nothing"
    );
    Ok(())
}

#[test]
fn a_stale_tracker_is_refused_until_regenerated() -> TestResult {
    let tree = Tree::governed()?;
    edit_tracker(&tree, 37)?;
    let stale = larry(&["tracker", "--check"], &tree.path)?;
    assert_eq!(stale.status.code(), Some(1), "an unrendered edit is stale");
    assert!(
        text(&stale.stderr).contains("FAIL: tools/tracker/index.html is stale"),
        "and says so: {}",
        text(&stale.stderr)
    );
    let gated = larry(&["governance"], &tree.path)?;
    assert_eq!(
        gated.status.code(),
        Some(1),
        "the governance gate refuses it too"
    );
    assert!(
        text(&gated.stderr).contains("regenerate with `larry tracker`"),
        "naming the fix: {}",
        text(&gated.stderr)
    );
    let written = larry(&["tracker"], &tree.path)?;
    assert_eq!(
        written.status.code(),
        Some(0),
        "the page regenerates: {}",
        text(&written.stderr)
    );
    let bytes = fs::metadata(tree.at("tools/tracker/index.html"))?.len();
    assert_eq!(
        text(&written.stdout),
        format!("wrote tools/tracker/index.html ({bytes} bytes)\n"),
        "the write reports its size"
    );
    for args in [&["tracker", "--check"][..], &["governance"]] {
        assert_eq!(
            larry(args, &tree.path)?.status.code(),
            Some(0),
            "{args:?} passes after the regeneration"
        );
    }
    Ok(())
}

/// Adds [`TRACKER_FILLER`] bytes to the tracker data, so its render is longer than the seeded
/// file-size limit the torn-write run is stopped at, and longer than one write on any host.
fn grow_tracker(tree: &Tree) -> TestResult {
    let path = tree.at("tools/tracker/tracker.json");
    let mut data: Value = json::from_str(&fs::read_to_string(&path)?)?;
    let object = data
        .as_object_mut()
        .ok_or("the tracker data is an object")?;
    let filler = "x".repeat(TRACKER_FILLER);
    object.insert(String::from("_probe"), Value::String(filler.into()));
    fs::write(&path, json::to_string(&data)?)?;
    Ok(())
}

/// The tracker's temporary pages left in its directory: each name and its byte count.
fn temp_pages(tree: &Tree) -> io::Result<Vec<(String, u64)>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(tree.at("tools/tracker"))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".index.html.") && name.ends_with(".tmp") {
            found.push((name, entry.metadata()?.len()));
        }
    }
    Ok(found)
}

/// Ends one tracker write inside the page write itself: the seed draws a file-size limit, so
/// the kernel stops the child at that byte offset, the temporary page is cut partway, and an
/// in-place write could not keep the cut out of the page. Returns the sizes of the temporary
/// pages left.
fn kill_inside_the_write(tree: &Tree, seed: u64) -> Result<Vec<u64>, Box<dyn Error>> {
    let mut rng = Seeded::from_seed(seed);
    // 8 to 2047 blocks of at most 1 KiB is at most 2 MiB, under the 3 MiB page on every shell.
    let blocks = rng.below(2040)?.saturating_add(8);
    let status = Command::new("/bin/sh")
        .arg("-c")
        .arg(format!(
            r#"ulimit -f {blocks} || exit 70; exec "$1" tracker --root "$2""#
        ))
        .arg("sh")
        .arg(LARRY)
        .arg(&*tree.path)
        .env_clear()
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.signal() != Some(SIGXFSZ) {
        return Err(
            format!("the size-limited tracker write ended {status:?}, not by SIGXFSZ").into(),
        );
    }
    let pages: Vec<u64> = temp_pages(tree)?
        .into_iter()
        .map(|(_, size)| size)
        .collect();
    if pages.is_empty() {
        return Err("the stopped tracker write left no temporary page".into());
    }
    Ok(pages)
}

#[test]
fn a_killed_tracker_write_leaves_the_render_whole() -> TestResult {
    let tree = Tree::governed()?;
    let page = tree.at("tools/tracker/index.html");
    let old = fs::read(&page)?;
    edit_tracker(&tree, 61)?;
    grow_tracker(&tree)?;
    let mut observed = Vec::with_capacity(KILLS);
    for _ in 0..KILLS {
        let mut child = Command::new(LARRY)
            .args(["tracker", "--root"])
            .arg(&*tree.path)
            .env_clear()
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let deadline = Instant::now()
            .checked_add(WRITE_PATIENCE)
            .ok_or("the write deadline overflowed")?;
        loop {
            if !temp_pages(&tree)?.is_empty() || child.try_wait()?.is_some() {
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
        }
        if child.try_wait()?.is_none() {
            child.kill()?;
        }
        child.wait()?;
        observed.push(fs::read(&page)?);
    }
    let partial = kill_inside_the_write(&tree, 61)?;
    let finished = larry(&["tracker"], &tree.path)?;
    assert_eq!(
        finished.status.code(),
        Some(0),
        "a later write succeeds after every kill"
    );
    let new = fs::read(&page)?;
    assert_ne!(new, old, "the edit changed the render");
    let written = u64::try_from(new.len())?;
    assert!(
        partial.iter().any(|size| *size < written),
        "the size-limited write was not cut inside the page: temporary pages {partial:?} of {written} bytes"
    );
    for (kill, seen) in observed.iter().enumerate() {
        assert!(
            *seen == old || *seen == new,
            "kill {kill} left a whole page, old or new, never a torn one"
        );
    }
    let mut names = Vec::new();
    files_under(&tree.path, &tree.at("tools/tracker"), &mut names)?;
    for name in names {
        let stray = name
            .strip_prefix("tools/tracker/.index.html.")
            .is_some_and(|rest| rest.ends_with(".tmp"));
        let kept = [
            "tools/tracker/tracker.json",
            "tools/tracker/template.html",
            "tools/tracker/index.html",
        ]
        .contains(&name.as_str());
        assert!(
            kept || stray,
            "only the tracker files and ignored temp pages remain, found {name}"
        );
    }
    assert_eq!(
        larry(&["tracker", "--check"], &tree.path)?.status.code(),
        Some(0),
        "the recovered page is current"
    );
    Ok(())
}

/// The sample at percentile `per_cent` of `sorted`, nearest rank.
fn percentile(sorted: &[Duration], per_cent: usize) -> Duration {
    let rank = sorted.len().saturating_mul(per_cent).div_ceil(100);
    match sorted.get(rank.saturating_sub(1)) {
        Some(&sample) => sample,
        None => Duration::ZERO,
    }
}

/// What one sampled wait saw.
enum Sampled {
    /// The check exited with this status, and the peak RSS some probe caught, if any.
    Exited {
        /// The check's exit status.
        status: ExitStatus,
        /// The peak RSS in KiB, `None` when the check was too short to catch a sample.
        peak: Option<u64>,
    },
    /// The failure deadline passed with the check still running.
    Runaway {
        /// The peak RSS in KiB, `None` when no probe answered.
        peak: Option<u64>,
    },
}

/// Samples `probe` while `exited` answers `None`, and answers as soon as the check exits; once
/// [`deadline::PATIENCE`] has passed on `clock` it reports a runaway instead. The wait is the
/// check's exit, and the bound is one reading of the injected clock, never a count of probes
/// (larry#294, ADR 0053).
fn sample_until_exit<E>(
    clock: &Clock,
    mut probe: impl FnMut(&mut Option<u64>),
    mut exited: impl FnMut() -> Result<Option<ExitStatus>, E>,
) -> Result<Sampled, E> {
    let started = clock.now();
    let mut peak = None;
    loop {
        probe(&mut peak);
        if let Some(status) = exited()? {
            return Ok(Sampled::Exited { status, peak });
        }
        if clock.now().saturating_sub(started) >= deadline::PATIENCE {
            return Ok(Sampled::Runaway { peak });
        }
        std::thread::yield_now();
    }
}

/// The peak RSS in KiB of one check of `root`, sampled from `ps` while it runs: the PTY probe's
/// own read, never `/usr/bin/time` (larry#269), so a host without that tool still measures it.
/// `None` when the check was too short to catch a sample. A check that outlasts
/// [`deadline::PATIENCE`] is a runaway named with that bound, not a probe count (larry#294).
fn peak_rss_kib(root: &Path) -> Result<Option<u64>, Box<dyn Error>> {
    let clock = larry_runtime::live_clock()?;
    let mut child = Command::new(LARRY)
        .args(["governance", "--root"])
        .arg(root)
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let pid = child.id().to_string();
    let sampled = sample_until_exit(
        &clock,
        |peak| {
            if let Some((_, kib)) = ps::sample(&pid) {
                *peak = Some(match *peak {
                    Some(seen) => seen.max(kib),
                    None => kib,
                });
            }
        },
        || child.try_wait(),
    )?;
    match sampled {
        Sampled::Exited { status, peak } if status.success() => Ok(peak),
        Sampled::Exited { status, .. } => {
            Err(format!("the sampled governance check failed: {status}").into())
        }
        Sampled::Runaway { peak } => {
            // The runaway is the failure; a kill or reap that also fails is named in it.
            let reaped = child.kill().and_then(|()| child.wait().map(|_status| ()));
            Err(format!(
                "one governance check did not exit within {:?}; its peak RSS was {peak:?} KiB; kill and reap: {reaped:?}",
                deadline::PATIENCE
            )
            .into())
        }
    }
}

/// Seeds the sampler simulation sweeps.
const SAMPLER_SEEDS: u64 = 64;

/// The largest virtual step a seed draws, in milliseconds.
const SAMPLER_STEP_MAX_MS: u64 = 100;

/// The most probes a seed's check stays running.
const SAMPLER_PROBES_MAX: u64 = 800;

/// The sampler ends on the injected clock, never on a count of probes: for every seed, a check
/// that exits inside the bound is answered, and one still running at the bound is a runaway at
/// the first reading past it (larry#294, ADR 0053).
#[test]
fn sim_the_rss_sampler_ends_on_the_deadline_not_a_probe_count() -> TestResult {
    let bound_ms = u64::try_from(deadline::PATIENCE.as_millis())?;
    for seed in 0..SAMPLER_SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let step_ms = rng.below(SAMPLER_STEP_MAX_MS)?.saturating_add(1);
        let runs_for = rng.below(SAMPLER_PROBES_MAX)?;
        let step = Duration::from_millis(step_ms);
        let elapsed_ms = runs_for
            .checked_mul(step_ms)
            .ok_or("the drawn time overflowed")?;
        let inside = elapsed_ms < bound_ms;
        let clock = Clock::virtual_at(Duration::ZERO);
        let mut probes = 0_u64;
        let sampled = sample_until_exit(
            &clock,
            |peak| *peak = Some(step_ms),
            || -> Result<Option<ExitStatus>, Box<dyn Error>> {
                probes = probes.saturating_add(1);
                clock.advance(step)?;
                Ok((probes > runs_for).then_some(ExitStatus::from_raw(0)))
            },
        )?;
        let reached = step
            .checked_mul(u32::try_from(probes)?)
            .ok_or("the sampled time overflowed")?;
        assert_eq!(
            clock.now(),
            reached,
            "seed {seed}: the sampler read the clock once per probe"
        );
        match sampled {
            Sampled::Exited { peak, .. } => {
                assert!(
                    inside,
                    "seed {seed}: {runs_for} probes of {step_ms} ms is past the bound, yet the check exited"
                );
                assert_eq!(
                    peak,
                    Some(step_ms),
                    "seed {seed}: the sampler kept the peak"
                );
                assert_eq!(
                    probes,
                    runs_for.saturating_add(1),
                    "seed {seed}: the wait ends on the probe that finds the exit"
                );
            }
            Sampled::Runaway { .. } => {
                assert!(
                    !inside,
                    "seed {seed}: {runs_for} probes of {step_ms} ms is inside the bound, yet the sampler gave up"
                );
                let before = clock.now().saturating_sub(step);
                assert!(
                    clock.now() >= deadline::PATIENCE && before < deadline::PATIENCE,
                    "seed {seed}: the sampler stops at the first reading past the bound, read {:?}",
                    clock.now()
                );
            }
        }
    }
    Ok(())
}

/// The check's own budget: a wall-clock figure other tests must not decide, so this test runs
/// alone in the `governance-latency` nextest group (`.config/nextest.toml`) on a fixture tree the
/// test writes (larry#224, larry#270). The peak RSS does not depend on load and is asserted the
/// same way.
#[test]
fn governance_check_latency_and_peak_rss_stay_in_budget() -> TestResult {
    let tree = Tree::fixture()?;
    let mut samples = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let started = Instant::now();
        let checked = larry(&["governance"], &tree.path)?;
        samples.push(started.elapsed());
        assert_eq!(
            checked.status.code(),
            Some(0),
            "the fixture tree passes its own gate: {}",
            text(&checked.stderr)
        );
    }
    samples.sort();
    let (p50, p95, p99) = (
        percentile(&samples, 50),
        percentile(&samples, 95),
        percentile(&samples, 99),
    );
    let rss = peak_rss_kib(&tree.path)?.ok_or(
        "the ps probe took no sample of the governance check, so its peak RSS is unproved",
    )?;
    let figures = format!(
        "governance over {RUNS} runs on the fixture tree: p50 {p50:?} p95 {p95:?} p99 {p99:?}, peak RSS {rss} KiB (ps sample)\n"
    );
    io::stderr().lock().write_all(figures.as_bytes())?;
    assert!(
        p99 < Duration::from_secs(5),
        "one check stays well inside the 5-minute gate budget: p99 {p99:?}"
    );
    assert!(rss < 256 * 1024, "one check stays under 256 MiB: {rss} KiB");
    Ok(())
}

#[test]
fn local_ci_runs_the_rust_gate_and_no_python_governance_lane() -> TestResult {
    let listed = Command::new(env!("CARGO_BIN_EXE_larry-local-ci"))
        .arg("--list")
        .output()?;
    assert_eq!(
        listed.status.code(),
        Some(0),
        "--list runs nothing and succeeds"
    );
    let lanes: Value = json::from_str(&text(&listed.stdout))?;
    let lanes = lanes.as_array().ok_or("--list prints an array")?;
    let command = |name: &str| {
        lanes
            .iter()
            .find(|lane| lane.get("name").and_then(Value::as_str) == Some(name))
            .and_then(|lane| lane.get("command").and_then(Value::as_str))
            .map(String::from)
    };
    assert_eq!(
        command("governance").as_deref(),
        Some("target/debug/larry governance"),
        "governance runs the binary"
    );
    assert_eq!(
        command("tracker").as_deref(),
        Some("target/debug/larry tracker --check"),
        "tracker runs the binary"
    );
    assert_eq!(
        command("mascot").as_deref(),
        Some("target/debug/larry mascot --check"),
        "the mascot render is checked by the binary (larry#152)"
    );
    for lane in lanes {
        let line = lane
            .get("command")
            .and_then(Value::as_str)
            .ok_or("a lane without a command")?;
        assert!(
            !line.contains("check_governance") && !line.contains("generate.py"),
            "no Python governance lane: {line}"
        );
        // larry#53's proof: local CI runs no Python at all once steps 2 and 3 land.
        assert!(
            !line.starts_with("python") && !line.contains(".py"),
            "no lane runs Python: {line}"
        );
    }
    Ok(())
}

/// CI's hosted rust job runs the very test command `cargo local-ci` runs, so a green PR and a
/// green local run attest the same suite (larry#154): `cargo nextest run --workspace --locked`.
#[test]
fn ci_runs_the_test_lane_local_ci_runs() -> TestResult {
    let listed = Command::new(env!("CARGO_BIN_EXE_larry-local-ci"))
        .arg("--list")
        .output()?;
    let lanes: Value = json::from_str(&text(&listed.stdout))?;
    let tests = lanes
        .as_array()
        .ok_or("--list prints an array")?
        .iter()
        .find(|lane| lane.get("name").and_then(Value::as_str) == Some("tests"))
        .and_then(|lane| lane.get("command").and_then(Value::as_str))
        .ok_or("local-ci has a tests lane")?
        .to_owned();
    assert_eq!(
        tests, "cargo nextest run --workspace --locked",
        "the declared test lane"
    );
    let workflow = fs::read_to_string(Path::new(ROOT).join(".github/workflows/ci.yml"))?;
    let hosted = format!("run: {tests}");
    assert!(
        workflow.lines().any(|line| line.trim() == hosted),
        "ci.yml runs `{tests}` on hosted runners"
    );
    Ok(())
}
