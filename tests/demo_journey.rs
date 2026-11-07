//! The shipped `larry-demo` fixture driver driven through its stdin/stdout protocol: the four
//! journeys `tools/test_demo_cli.py` ran before larry#53 step 2 ported them (#147), plus seeded
//! simulations that hold the binary's frames to the in-process [`Demo`] for the same command
//! stream and kill the driver at a seeded point. Source: the Python journeys; POSIX `kill(2)`
//! for SIGKILL. Requirement: synthetic data stays visibly DEMO, a rejection changes nothing,
//! input is bounded, and a killed driver leaves the workspace and the next session untouched.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use larry_demo::Demo;
use lgwks_std::hash;
use lgwks_std::json::{self, Value};
use lgwks_std::seeded::Seeded;

/// Failures propagate instead of panicking.
type Journey<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Seeds each simulation runs.
const SEEDS: u64 = 32;
/// Commands each seeded stream holds before its `quit`.
const STREAM: usize = 24;
/// The grammar the simulations draw from: every accepted form and the refusals beside them.
const VOCABULARY: [&str; 20] = [
    "next",
    "folders",
    "agents",
    "continue",
    "reset",
    "scenario change",
    "scenario empty",
    "help",
    "status",
    "",
    "inspect 1",
    "inspect 2",
    "inspect 0",
    "inspect x",
    "feedback bound retries",
    "feedback préserver 日本語",
    "feedback",
    "exec touch larry-unwanted",
    "  next  ",
    "nextx",
];

/// Distinct scratch directories made by this test binary.
static SCRATCHES: AtomicU64 = AtomicU64::new(0);

/// An empty directory the driver runs in, removed when dropped.
struct Scratch {
    /// The directory.
    path: PathBuf,
}

impl Scratch {
    /// A fresh directory under the system temporary directory.
    fn new() -> Journey<Self> {
        let serial = SCRATCHES.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "larry-demo-journey-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    /// The names left in the directory.
    fn entries(&self) -> Journey<Vec<String>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(&self.path)? {
            names.push(entry?.file_name().to_string_lossy().into_owned());
        }
        Ok(names)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Best effort: a leftover scratch directory never fails a journey.
        drop(fs::remove_dir_all(&self.path));
    }
}

/// The shipped driver, run in `dir` with arguments `args`.
fn driver(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_larry-demo"));
    command.args(args).current_dir(dir).env_clear();
    command
}

/// Runs the driver to its end with `input` on stdin.
fn run(dir: &Path, input: &[u8], args: &[&str]) -> Journey<Output> {
    let mut child = driver(dir, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("driver has no stdin")?;
    stdin.write_all(input)?;
    drop(stdin);
    Ok(child.wait_with_output()?)
}

/// Every frame the driver printed, parsed.
fn frames(output: &Output) -> Journey<Vec<Value>> {
    let mut parsed = Vec::new();
    for line in output.stdout.split(|&byte| byte == b'\n') {
        if !line.is_empty() {
            parsed.push(json::from_str::<Value>(std::str::from_utf8(line)?)?);
        }
    }
    Ok(parsed)
}

/// The frame at `index`, or the error naming it.
fn frame(views: &[Value], index: usize) -> Journey<&Value> {
    Ok(views
        .get(index)
        .ok_or_else(|| format!("no frame {index} of {}", views.len()))?)
}

/// Whether the driver's stderr contains `text`.
fn said(output: &Output, text: &str) -> bool {
    String::from_utf8_lossy(&output.stderr).contains(text)
}

/// BLAKE3 of each source file a driver could touch: `src/*.rs`, `Cargo.toml`, `Cargo.lock`.
fn workspace_digests() -> Journey<BTreeMap<PathBuf, String>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut paths = vec![root.join("Cargo.toml"), root.join("Cargo.lock")];
    for entry in fs::read_dir(root.join("src"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "rs") {
            paths.push(path);
        }
    }
    let mut digests = BTreeMap::new();
    for path in paths {
        let digest = hash::blake3(&fs::read(&path)?).to_hex();
        digests.insert(path, digest);
    }
    Ok(digests)
}

#[test]
fn inspect_feedback_independent_work_and_continue() -> Journey {
    let scratch = Scratch::new()?;
    let output = run(
        &scratch.path,
        b"next\nfeedback bound retries\nnext\ninspect 2\nnext\ncontinue\nnext\nquit\n",
        &[],
    )?;
    assert!(output.status.success(), "driver exits 0: {output:?}");
    assert!(
        said(&output, "HeldWork"),
        "a held owner refuses the next step"
    );
    let views = frames(&output)?;
    assert_eq!(
        frame(&views, 4)?["work"]["Held"]["anchor"],
        1,
        "the hold keeps its anchor"
    );
    assert_eq!(
        frame(&views, 4)?["inspection"]["trace"],
        "fixture-02",
        "inspect 2"
    );
    let last = views.last().ok_or("no frames")?;
    assert_eq!(
        last["inspection"]["line"], 18,
        "continue then next reaches line 18"
    );
    assert_eq!(
        last["feedback"][0]["text"], "bound retries",
        "feedback is kept"
    );
    assert!(
        views.iter().all(|view| view["mode"]
            .as_str()
            .is_some_and(|mode| mode.starts_with("DEMO"))),
        "every frame is visibly DEMO"
    );
    Ok(())
}

#[test]
fn panes_empty_scenario_and_invalid_operation_recovery() -> Journey {
    let scratch = Scratch::new()?;
    let output = run(
        &scratch.path,
        b"folders\nagents\nscenario empty\nnext\nexec touch larry-unwanted\nscenario change\nnext\nquit\n",
        &[],
    )?;
    assert!(output.status.success(), "driver exits 0: {output:?}");
    let views = frames(&output)?;
    let folders = frame(&views, 1)?["folders"]
        .as_array()
        .ok_or("no folders")?;
    assert_eq!(folders.len(), 2, "the folders pane lists two folders");
    assert_eq!(
        frame(&views, 2)?["agents"][2]["parent"],
        0,
        "agent 2's parent is 0"
    );
    assert_eq!(
        frame(&views, 3)?["scenario"],
        "empty",
        "the empty scenario shows"
    );
    assert!(
        said(&output, "EndOfFixture"),
        "an empty scenario has no next"
    );
    assert!(
        said(&output, "UnknownCommand"),
        "exec is not in the grammar"
    );
    assert!(
        scratch.entries()?.is_empty(),
        "nothing was written: {:?}",
        scratch.entries()
    );
    let last = views.last().ok_or("no frames")?;
    let events = last["events"].as_array().ok_or("no events")?;
    assert_eq!(
        events.len(),
        1,
        "the change scenario recovers and steps once"
    );
    Ok(())
}

#[test]
fn transport_bounds_utf8_eof_and_arguments() -> Journey {
    let scratch = Scratch::new()?;
    let mut oversize = vec![b'x'; 4097];
    oversize.push(b'\n');
    let refused: [(&[u8], &[&str]); 3] =
        [(&oversize, &[]), (b"\xff\n", &[]), (b"", &["--execute"])];
    for (input, args) in refused {
        let output = run(&scratch.path, input, args)?;
        assert_eq!(
            output.status.code(),
            Some(1),
            "{} bytes, {args:?}",
            input.len()
        );
        assert!(
            said(&output, "DEMO failed"),
            "{} bytes, {args:?}: {output:?}",
            input.len()
        );
    }
    let output = run(&scratch.path, b"help\r\nstatus", &[])?;
    assert!(
        output.status.success(),
        "CRLF and a final line without LF: {output:?}"
    );
    assert_eq!(
        frames(&output)?.len(),
        3,
        "the opening frame and one per command"
    );
    Ok(())
}

/// Starts the driver, sends `input`, reads `frames` lines, kills it with SIGKILL and returns
/// the frames it printed.
fn killed_after(dir: &Path, input: &[u8], count: usize) -> Journey<Vec<Value>> {
    let mut child = driver(dir, &[])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("driver has no stdin")?;
    let stdout = child.stdout.take().ok_or("driver has no stdout")?;
    stdin.write_all(input)?;
    stdin.flush()?;
    let mut lines = BufReader::new(stdout);
    let mut seen = Vec::with_capacity(count);
    let mut line = String::new();
    while seen.len() < count {
        line.clear();
        if lines.read_line(&mut line)? == 0 {
            return Err(format!("driver closed after {} frames", seen.len()).into());
        }
        seen.push(json::from_str::<Value>(line.trim_end())?);
    }
    child.kill()?;
    let status = child.wait()?;
    assert_eq!(
        status.signal(),
        Some(9),
        "the driver died of SIGKILL: {status}"
    );
    Ok(seen)
}

#[test]
fn actual_process_crash_resets_fixture_without_workspace_changes() -> Journey {
    let scratch = Scratch::new()?;
    let before = workspace_digests()?;
    let seen = killed_after(
        &scratch.path,
        b"next\nfeedback preserve original anchor\n",
        3,
    )?;
    assert_eq!(
        seen.last().ok_or("no frames")?["work"]["Held"]["anchor"],
        1,
        "held at 1"
    );
    let restored = run(&scratch.path, b"status\nquit\n", &[])?;
    assert!(
        restored.status.success(),
        "a fresh driver exits 0: {restored:?}"
    );
    let views = frames(&restored)?;
    assert_eq!(
        frame(&views, 0)?["work"],
        "Ready",
        "the fresh session is Ready"
    );
    assert_eq!(
        frame(&views, 0)?["feedback"],
        json::json!([]),
        "no feedback survives"
    );
    assert_eq!(
        before,
        workspace_digests()?,
        "the kill changed no source file"
    );
    assert!(scratch.entries()?.is_empty(), "the driver wrote nothing");
    Ok(())
}

/// A seeded stream of `STREAM` commands from the vocabulary.
fn stream(seed: u64) -> Journey<Vec<&'static str>> {
    let mut rng = Seeded::from_seed(seed);
    let mut commands = Vec::with_capacity(STREAM);
    for _ in 0..STREAM {
        let index = rng
            .index(VOCABULARY.len())
            .map_err(|cause| format!("{cause:?}"))?;
        commands.push(*VOCABULARY.get(index).ok_or("draw past the vocabulary")?);
    }
    Ok(commands)
}

/// What the driver must print for `commands`: every frame and every refusal line, computed by
/// the library the driver wraps.
fn expected(commands: &[&str]) -> Journey<(String, String)> {
    let mut demo = Demo::default();
    let mut frames = demo.snapshot()?;
    frames.push('\n');
    let mut refusals = String::new();
    for command in commands {
        match demo.command(command) {
            Ok(false) => break,
            Ok(true) => {}
            Err(rejected) => {
                refusals.push_str(&rejected.to_string());
                refusals.push('\n');
            }
        }
        frames.push_str(&demo.snapshot()?);
        frames.push('\n');
    }
    Ok((frames, refusals))
}

#[test]
fn sim_driver_frames_match_the_library_for_every_seed() -> Journey {
    let scratch = Scratch::new()?;
    let mut first_trace = None;
    for seed in 0..SEEDS {
        let commands = stream(seed)?;
        let mut input = commands.join("\n");
        input.push_str("\nquit\n");
        let output = run(&scratch.path, input.as_bytes(), &[])?;
        let (frames, refusals) = expected(&commands)?;
        assert!(output.status.success(), "seed {seed}: {output:?}");
        assert_eq!(
            String::from_utf8(output.stdout.clone())?,
            frames,
            "seed {seed}: frames"
        );
        assert_eq!(
            String::from_utf8(output.stderr.clone())?,
            refusals,
            "seed {seed}: refusals"
        );
        assert!(
            scratch.entries()?.is_empty(),
            "seed {seed}: the driver wrote nothing"
        );
        if seed == 0 {
            first_trace = Some(hash::blake3(&output.stdout).to_hex());
        }
    }
    let again = run(
        &scratch.path,
        format!("{}\nquit\n", stream(0)?.join("\n")).as_bytes(),
        &[],
    )?;
    assert_eq!(
        first_trace,
        Some(hash::blake3(&again.stdout).to_hex()),
        "the same seed gives the same trace hash"
    );
    Ok(())
}

#[test]
fn sim_a_kill_at_any_seeded_point_leaves_the_next_session_ready() -> Journey {
    let scratch = Scratch::new()?;
    let before = workspace_digests()?;
    for seed in 0..SEEDS {
        let commands = stream(seed)?;
        let mut rng = Seeded::from_seed(seed.wrapping_add(SEEDS));
        let cut = rng.index(STREAM).map_err(|cause| format!("{cause:?}"))?;
        let sent = commands.get(..cut).ok_or("cut past the stream")?;
        let input: String = sent.iter().map(|command| format!("{command}\n")).collect();
        let lines = cut.saturating_add(1);
        let seen = killed_after(&scratch.path, input.as_bytes(), lines)?;
        assert_eq!(
            seen.len(),
            lines,
            "seed {seed}: one frame per command before the kill"
        );
        let fresh = run(&scratch.path, b"status\nquit\n", &[])?;
        assert!(fresh.status.success(), "seed {seed}: {fresh:?}");
        let views = frames(&fresh)?;
        assert_eq!(
            frame(&views, 0)?["work"],
            "Ready",
            "seed {seed}: fresh session"
        );
        assert!(
            scratch.entries()?.is_empty(),
            "seed {seed}: the driver wrote nothing"
        );
    }
    assert_eq!(
        before,
        workspace_digests()?,
        "no kill changed a source file"
    );
    Ok(())
}
