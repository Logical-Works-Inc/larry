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
//! leaves a whole page; the checker never writes; local-ci runs no Python governance lane.

use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use lgwks_std::hash;
use lgwks_std::json::{self, Value};

/// What every journey returns.
type TestResult = Result<(), Box<dyn Error>>;

/// The repository the journeys copy from.
const ROOT: &str = env!("CARGO_MANIFEST_DIR");
/// The shipped binary.
const LARRY: &str = env!("CARGO_BIN_EXE_larry");
/// The top-level governed documents.
const DOCUMENTS: [&str; 7] = [
    "README.md",
    "index.md",
    "specification.md",
    "research-ledger.md",
    "AGENTS.md",
    "GOVERNANCE.md",
    "log.md",
];
/// The directories copied whole beside the documents.
const COPIED: [&str; 3] = ["docs", "tools/receipts", "tools/tracker"];
/// Timed checks for the latency figures.
const RUNS: usize = 32;
/// Kills of a tracker write the killed-write journey makes.
const KILLS: usize = 16;
/// Names the copies of one test process.
static COPIES: AtomicU64 = AtomicU64::new(0);

/// A temporary copy of the governed files, removed when dropped.
struct Tree {
    /// The copy's root.
    path: PathBuf,
}

impl Tree {
    /// A fresh copy of every file `larry governance` and `larry tracker` read. The directory
    /// must not exist yet, so a copy never inherits another run's files.
    fn governed() -> Result<Tree, Box<dyn Error>> {
        let sequence = COPIES.fetch_add(1, Ordering::Relaxed);
        let tree = Tree {
            path: std::env::temp_dir().join(format!(
                "larry-governance-journey-{}-{sequence}",
                std::process::id()
            )),
        };
        fs::create_dir(&tree.path)?;
        for name in DOCUMENTS {
            fs::copy(Path::new(ROOT).join(name), tree.at(name))?;
        }
        for directory in COPIED {
            copy_files(&Path::new(ROOT).join(directory), &tree.at(directory))?;
        }
        Ok(tree)
    }

    /// The path of `relative` inside the copy.
    fn at(&self, relative: &str) -> PathBuf {
        self.path.join(relative)
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        // Best effort: a leftover temp dir never fails a journey.
        drop(fs::remove_dir_all(&self.path));
    }
}

/// Copies every Markdown, JSON and HTML file under `from` to the same place under `to`: the
/// kinds the gate reads, never a host's ignored build litter.
fn copy_files(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let read = [".md", ".json", ".html"]
            .iter()
            .any(|suffix| name.ends_with(suffix));
        if kind.is_dir() {
            copy_files(&entry.path(), &to.join(entry.file_name()))?;
        } else if kind.is_file() && read {
            fs::copy(entry.path(), to.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Every file under `root`, `/`-separated and relative, sorted.
fn files_under(root: &Path, directory: &Path, found: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            files_under(root, &path, found)?;
        } else {
            let relative = path.strip_prefix(root).map_err(io::Error::other)?;
            let parts: Vec<String> = relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            found.push(parts.join("/"));
        }
    }
    found.sort();
    Ok(())
}

/// Every file of the tree with its BLAKE3 digest.
fn digests(tree: &Tree) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let mut names = Vec::new();
    files_under(&tree.path, &tree.path, &mut names)?;
    let mut digested = Vec::with_capacity(names.len());
    for name in names {
        let bytes = fs::read(tree.at(&name))?;
        digested.push((name, hash::blake3(&bytes).to_hex()));
    }
    Ok(digested)
}

/// Runs the shipped `larry` with `args` from `directory`, with only `PATH` set.
fn larry(args: &[&str], directory: &Path) -> Result<Output, Box<dyn Error>> {
    Ok(Command::new(LARRY)
        .args(args)
        .current_dir(directory)
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .stdin(Stdio::null())
        .output()?)
}

/// A stream as text.
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Sets one cell's percentage in the tree's tracker data, by text, so the file stays as written.
fn edit_tracker(tree: &Tree, pct: u32) -> TestResult {
    let path = tree.at("tools/tracker/tracker.json");
    let data = fs::read_to_string(&path)?;
    let edited = data.replacen("\"pct\": 0,", &format!("\"pct\": {pct},"), 1);
    assert_ne!(edited, data, "the tracker data has a zero cell to edit");
    fs::write(path, edited)?;
    Ok(())
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

#[test]
fn a_killed_tracker_write_leaves_the_render_whole() -> TestResult {
    let tree = Tree::governed()?;
    let page = tree.at("tools/tracker/index.html");
    let old = fs::read(&page)?;
    edit_tracker(&tree, 61)?;
    let mut observed = Vec::with_capacity(KILLS);
    for _ in 0..KILLS {
        let mut child = Command::new(LARRY)
            .args(["tracker", "--root"])
            .arg(&tree.path)
            .env_clear()
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        child.kill()?;
        child.wait()?;
        observed.push(fs::read(&page)?);
    }
    let finished = larry(&["tracker"], &tree.path)?;
    assert_eq!(
        finished.status.code(),
        Some(0),
        "a later write succeeds after every kill"
    );
    let new = fs::read(&page)?;
    assert_ne!(new, old, "the edit changed the render");
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

/// The peak RSS in KiB `/usr/bin/time` reports for one check of the repository.
fn peak_rss_kib() -> Result<Option<u64>, Box<dyn Error>> {
    let flag = if cfg!(target_os = "macos") {
        "-l"
    } else {
        "-v"
    };
    if !Path::new("/usr/bin/time").is_file() {
        return Ok(None);
    }
    let output = Command::new("/usr/bin/time")
        .args([flag, LARRY, "governance", "--root", ROOT])
        .output()?;
    let stderr = text(&output.stderr);
    let Some(line) = stderr.lines().find(|line| {
        line.to_ascii_lowercase()
            .contains("maximum resident set size")
    }) else {
        return Ok(None);
    };
    let digits: String = line.chars().filter(char::is_ascii_digit).collect();
    let value: u64 = digits.parse()?;
    Ok(Some(if cfg!(target_os = "macos") {
        value.div_ceil(1024)
    } else {
        value
    }))
}

#[test]
fn governance_check_latency_and_peak_rss_stay_in_budget() -> TestResult {
    let mut samples = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let started = Instant::now();
        let checked = larry(&["governance"], Path::new(ROOT))?;
        samples.push(started.elapsed());
        assert_eq!(
            checked.status.code(),
            Some(0),
            "the repository passes its own gate"
        );
    }
    samples.sort();
    let (p50, p95, p99) = (
        percentile(&samples, 50),
        percentile(&samples, 95),
        percentile(&samples, 99),
    );
    let rss = peak_rss_kib()?;
    let figures = format!(
        "governance over {RUNS} runs: p50 {p50:?} p95 {p95:?} p99 {p99:?}, peak RSS {rss:?} KiB\n"
    );
    io::stderr().lock().write_all(figures.as_bytes())?;
    assert!(
        p99 < Duration::from_secs(5),
        "one check stays well inside the 5-minute gate budget: p99 {p99:?}"
    );
    if let Some(kib) = rss {
        assert!(kib < 256 * 1024, "one check stays under 256 MiB: {kib} KiB");
    }
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
    for lane in lanes {
        let line = lane
            .get("command")
            .and_then(Value::as_str)
            .ok_or("a lane without a command")?;
        assert!(
            !line.contains("check_governance") && !line.contains("generate.py"),
            "no Python governance lane: {line}"
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
