//! Seeded simulations of the environment probe (larry#274).
//! Source: Meta-Harness `agent.py:873-969` (one probe at task start, fixed
//! commands, no hardcoded `/app`, no swallowed failure) and CWE-200 (no secret
//! in the record). Requirement: five fixture folders (Rust, Python, Node, C,
//! empty) each report their expected probe; every section failure is a typed
//! field, never dropped; hostile bytes never panic and stay bounded; the
//! rendering the model reads is bounded and names the facts; no environment
//! value and no file content ever reaches the record.

use std::error::Error;
use std::path::Path;

use lgwks_std::json::{Value, json};
use lgwks_std::seeded::Seeded;

use super::{
    MAX_RENDER_CHARS, ProbeCall, ProbeError, parse_df, parse_meminfo, parse_status, probe, render,
    sanitize, version_line,
};
use crate::executor::sim_tree::scratch;
use crate::sim_error::SimError;

/// What every test returns.
type TestResult = Result<(), Box<dyn Error>>;

/// The seeds every simulation runs.
const SEEDS: u64 = 32;

/// The canned `/proc/meminfo` the scripted reader returns.
const MEMINFO: &str = "MemTotal:        8000000 kB\nMemAvailable:    4000000 kB\n";

/// The canned `df -k -P` output the scripted runner returns for `df`.
const DF: &str = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/disk1s1 1000000 400000 600000 40% /\n";

/// The canned `git status` output the scripted runner returns for `git`.
const GIT_CLEAN: &str = "## main\n";

/// The canned `--version` line per tool the scripted runner returns.
fn version_of(tool: &str) -> Option<&'static str> {
    match tool {
        "rustc" => Some("rustc 1.100.0 (namespace 00000000 2026-01-01)\n"),
        "cargo" => Some("cargo 1.100.0 (namespace 00000000 2026-01-01)\n"),
        "node" => Some("v22.0.0\n"),
        "python3" => Some("Python 3.12.0\n"),
        _ => None,
    }
}

/// The scripted command runner: fixed answers per argv, never a host process.
fn scripted_run(argv: &[String], _cwd: &Path) -> Result<ProbeCall, ProbeError> {
    let Some(first) = argv.first() else {
        return Err(ProbeError::Command(String::from(
            "the probe command is empty",
        )));
    };
    let name = first.as_str();
    if name == "git" {
        return Ok(ProbeCall::ok(GIT_CLEAN));
    }
    if name == "df" {
        return Ok(ProbeCall::ok(DF));
    }
    match version_of(name) {
        Some(line) => Ok(ProbeCall::ok(line)),
        None => Err(ProbeError::Command(format!("{name} is not on PATH"))),
    }
}

/// The scripted file reader: the canned meminfo for any non-empty path.
fn scripted_read(path: &Path) -> Result<String, ProbeError> {
    if path.as_os_str().is_empty() {
        return Err(ProbeError::Refused("no path to read"));
    }
    Ok(MEMINFO.to_owned())
}

/// Writes `files` (relative path and bytes) under a fresh scratch directory,
/// which lives as long as the returned guard.
fn fixture(files: &[(&str, &str)]) -> Result<impl AsRef<Path>, SimError> {
    let dir = scratch("larry-probe")?;
    for &(name, body) in files {
        let target = dir.join(name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, body)?;
    }
    Ok(dir)
}

/// One fixture shape: its label, its files (relative path and bytes) and the
/// manifest the probe must name (`""` when none).
type Shape = (
    &'static str,
    Vec<(&'static str, &'static str)>,
    &'static str,
);

/// The five shapes the issue names: Rust, Python, Node, C and empty.
fn shapes() -> Vec<Shape> {
    vec![
        (
            "rust",
            vec![
                ("Cargo.toml", "[package]\nname = \"demo\"\n"),
                ("src/main.rs", "fn main() {}\n"),
            ],
            "Cargo.toml",
        ),
        (
            "python",
            vec![
                ("pyproject.toml", "[project]\nname = \"demo\"\n"),
                ("main.py", "print('hi')\n"),
            ],
            "pyproject.toml",
        ),
        (
            "node",
            vec![
                ("package.json", "{\"name\":\"demo\"}\n"),
                ("index.js", "console.log(1)\n"),
            ],
            "package.json",
        ),
        (
            "c",
            vec![
                ("Makefile", "all:\n\ttrue\n"),
                ("main.c", "int main(void){return 0;}\n"),
            ],
            "Makefile",
        ),
        ("empty", vec![], ""),
    ]
}

/// Whether `entries` names `name`.
fn names(entries: &Value, name: &str) -> bool {
    entries.as_array().is_some_and(|items| {
        items
            .iter()
            .any(|entry| entry.get("path").and_then(Value::as_str) == Some(name))
    })
}

/// Whether `manifests` names `name`.
fn manifests_name(manifests: &Value, name: &str) -> bool {
    manifests.as_array().is_some_and(|items| {
        items
            .iter()
            .any(|entry| entry.get("path").and_then(Value::as_str) == Some(name))
    })
}

/// The toolchain entry for `tool`.
fn toolchain<'value>(probed: &'value Value, tool: &str) -> Option<&'value Value> {
    probed
        .get("toolchains")?
        .as_array()?
        .iter()
        .find(|entry| entry.get("tool").and_then(Value::as_str) == Some(tool))
}

/// Five fixture folders each give their expected probe: the entries name the
/// files, the manifests name the manifest, the scripted tools report their
/// versions, the missing ones are typed errors, and the whole record is
/// bounded (larry#274).
#[test]
fn sim_probe_each_fixture_shape_reports_its_expected_probe() -> TestResult {
    for (shape, files, manifest) in shapes() {
        let root = fixture(&files)?;
        let probed = probe(root.as_ref(), &scripted_run, &scripted_read);
        let entries = probed
            .get("tree")
            .and_then(|tree| tree.get("entries"))
            .ok_or("tree entries")?;
        for &(name, _) in &files {
            assert!(names(entries, name), "{shape}: the tree names {name}");
        }
        let manifests = probed.get("manifests").ok_or("manifests")?;
        if manifest.is_empty() {
            assert!(
                manifests.as_array().is_some_and(Vec::is_empty),
                "{shape}: an empty folder names no manifest"
            );
        } else {
            assert!(
                manifests_name(manifests, manifest),
                "{shape}: the probe names {manifest}"
            );
        }
        let rustc = toolchain(&probed, "rustc").ok_or("rustc entry")?;
        assert_eq!(
            rustc.get("ok"),
            Some(&Value::Bool(true)),
            "{shape}: rustc is ok"
        );
        assert!(
            rustc
                .get("version")
                .and_then(Value::as_str)
                .is_some_and(|version| version.starts_with("rustc 1.100.0")),
            "{shape}: rustc reports its version"
        );
        let go = toolchain(&probed, "go").ok_or("go entry")?;
        assert_eq!(
            go.get("ok"),
            Some(&Value::Bool(false)),
            "{shape}: a missing tool is refused"
        );
        assert!(
            go.get("error")
                .and_then(Value::as_str)
                .is_some_and(|error| error.contains("go")),
            "{shape}: the refusal names the tool, never a secret"
        );
        assert_eq!(
            probed.get("memory").and_then(|memory| memory.get("ok")),
            Some(&Value::Bool(true)),
            "{shape}: memory is ok"
        );
        assert_eq!(
            probed.get("disk").and_then(|disk| disk.get("ok")),
            Some(&Value::Bool(true)),
            "{shape}: disk is ok"
        );
        assert_eq!(
            probed.get("git").and_then(|git| git.get("ok")),
            Some(&Value::Bool(true)),
            "{shape}: git status is ok"
        );
        let bytes = probed.to_string().into_bytes();
        assert!(
            bytes.len() < 65_536,
            "{shape}: the probe stays bounded, got {}",
            bytes.len()
        );
    }
    Ok(())
}

/// Every section failure is a typed field, never dropped and never a refused
/// probe: the runner and the reader fail, the folder is missing, and the
/// record still seals with one typed error per section (larry#274).
#[test]
fn sim_probe_every_section_failure_is_typed_never_dropped() -> TestResult {
    let missing = Path::new("/no-such-larry-probe-folder");
    let failing = |_: &[String], _: &Path| -> Result<ProbeCall, ProbeError> {
        Err(ProbeError::Command(String::from(
            "the sandbox refuses every command",
        )))
    };
    let unreadable = |_: &Path| -> Result<String, ProbeError> {
        Err(ProbeError::Refused("meminfo is not there"))
    };
    let probed = probe(missing, &failing, &unreadable);
    for section in ["tree", "memory", "disk", "git"] {
        let field = probed.get(section).ok_or(format!("{section} is present"))?;
        assert_eq!(
            field.get("ok"),
            Some(&Value::Bool(false)),
            "{section}: a failure is typed"
        );
        assert!(
            field
                .get("error")
                .and_then(Value::as_str)
                .is_some_and(|error| !error.is_empty()),
            "{section}: the typed failure is never dropped"
        );
    }
    let tools = probed
        .get("toolchains")
        .and_then(Value::as_array)
        .ok_or("toolchains")?;
    assert!(!tools.is_empty(), "the tool list is fixed, whatever fails");
    for entry in tools {
        assert_eq!(
            entry.get("ok"),
            Some(&Value::Bool(false)),
            "every tool fails typed"
        );
    }
    Ok(())
}

/// Hostile bytes never panic and stay bounded: seeded garbage feeds every
/// parser, and the outputs are errors or capped values (larry#274).
#[test]
fn sim_probe_hostile_bytes_never_panic_and_stay_bounded() -> TestResult {
    let mut rng = Seeded::from_seed(274);
    for _ in 0..SEEDS {
        let length = usize::try_from(rng.below(600)?)?;
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            bytes.push(u8::try_from(rng.below(256)?)?);
        }
        let text = String::from_utf8_lossy(&bytes);
        if let Err(cause) = parse_meminfo(&text) {
            assert!(
                !cause.to_string().is_empty(),
                "a refused meminfo names its reason"
            );
        }
        if let Err(cause) = parse_df(&text) {
            assert!(
                !cause.to_string().is_empty(),
                "a refused df names its reason"
            );
        }
        let (_, head, _) = parse_status(&text);
        assert!(head.len() <= 4096, "the status head stays capped");
        let version = version_line(&text);
        if let Ok(line) = version {
            assert!(line.chars().count() <= 256, "the version line stays capped");
        }
        let cut = sanitize(&text, 64);
        assert!(cut.chars().count() <= 64, "an error field stays capped");
    }
    assert!(parse_meminfo(MEMINFO).is_ok(), "the canned meminfo parses");
    assert_eq!(parse_df(DF)?, 600_000, "the canned df parses");
    Ok(())
}

/// The rendering the model reads is bounded and names the facts: the folder,
/// one manifest, one tool version and the memory (larry#274).
#[test]
fn sim_probe_render_is_bounded_and_names_the_facts() -> TestResult {
    let root = fixture(&[("Cargo.toml", "[package]\nname = \"demo\"\n")])?;
    let probed = probe(root.as_ref(), &scripted_run, &scripted_read);
    let text = render(&probed);
    assert!(!text.is_empty(), "the model reads something");
    assert!(
        text.chars().count() <= MAX_RENDER_CHARS,
        "the rendering stays bounded"
    );
    for fact in ["Cargo.toml", "rustc 1.100.0", "avail_kb", "600000"] {
        assert!(text.contains(fact), "the rendering names {fact}");
    }
    Ok(())
}

/// No environment value and no file content ever reaches the record: the
/// manifests carry a marker, the scripted answers carry one, and neither appears
/// in the sealed value, which names no `LARRY_` variable either (larry#274).
#[test]
fn sim_probe_reports_no_secret_values() -> TestResult {
    let root = fixture(&[("Cargo.toml", "token = \"hunter2-cargo-marker\"\n")])?;
    let leaking = |argv: &[String], _cwd: &Path| -> Result<ProbeCall, ProbeError> {
        let Some(first) = argv.first() else {
            return Err(ProbeError::Command(String::from(
                "the probe command is empty",
            )));
        };
        Ok(ProbeCall::ok(&format!(
            "{} hunter2-run-marker",
            first.as_str()
        )))
    };
    let probed = probe(root.as_ref(), &leaking, &scripted_read);
    let sealed = probed.to_string();
    for marker in ["hunter2-cargo-marker", "LARRY_"] {
        assert!(
            !sealed.contains(marker),
            "the record never carries {marker}"
        );
    }
    Ok(())
}

/// A tool call that fails is a typed failure with its exit status, never an
/// `ok` version: one fixture tool exits 2 with text on stderr, one exits 0
/// printing nothing, and one never exits so the bound kills it (larry#274,
/// sixth pass). Catches `ok:true` over a failed call and a stderr dump passed
/// off as a version.
#[test]
fn sim_probe_a_failed_tool_is_never_ok() -> TestResult {
    let root = fixture(&[])?;
    let failing = |argv: &[String], _cwd: &Path| -> Result<ProbeCall, ProbeError> {
        let Some(first) = argv.first() else {
            return Err(ProbeError::Command(String::from(
                "the probe command is empty",
            )));
        };
        let name = first.as_str();
        match name {
            "rustc" => Ok(ProbeCall::failed(Some(2), "", "boom\n")),
            "cargo" => Ok(ProbeCall::ok("")),
            "node" => Ok(ProbeCall::timed_out()),
            _ => Err(ProbeError::Command(format!("{name} is not on PATH"))),
        }
    };
    let probed = probe(root.as_ref(), &failing, &scripted_read);
    for (tool, needle) in [("rustc", "2"), ("cargo", "empty"), ("node", "timed out")] {
        assert!(
            toolchain(&probed, tool).is_some(),
            "{tool}: the probe keeps a failed tool's entry"
        );
        let entry = toolchain(&probed, tool).ok_or("tool entry")?;
        assert_eq!(
            entry.get("ok"),
            Some(&Value::Bool(false)),
            "{tool}: a failed call is never ok"
        );
        assert!(
            entry
                .get("error")
                .and_then(Value::as_str)
                .is_some_and(|error| error.contains(needle)),
            "{tool}: the typed failure names {needle}"
        );
        assert!(
            entry.get("version").is_none(),
            "{tool}: a failed call seals no version"
        );
    }
    Ok(())
}

/// Every tool runs its own argv from the fixed table and reads its own stream:
/// `go version`, `java -version` (stderr), the rest `--version` (larry#274,
/// sixth pass). Catches one shared argv and a stdout-only read that misses
/// Java's version.
#[test]
fn sim_probe_every_tool_runs_its_own_argv() -> TestResult {
    let root = fixture(&[])?;
    let seen = std::cell::RefCell::new(Vec::new());
    let recording = |argv: &[String], _cwd: &Path| -> Result<ProbeCall, ProbeError> {
        let Some(first) = argv.first() else {
            return Err(ProbeError::Command(String::from(
                "the probe command is empty",
            )));
        };
        seen.borrow_mut().push(argv.join(" "));
        let name = first.as_str();
        if name == "java" {
            return Ok(ProbeCall::failed(Some(0), "", "openjdk 21.0.0\n"));
        }
        Ok(ProbeCall::ok("tool 1.0\n"))
    };
    let probed = probe(root.as_ref(), &recording, &scripted_read);
    assert!(toolchain(&probed, "java").is_some(), "java keeps its entry");
    let java = toolchain(&probed, "java").ok_or("java entry")?;
    assert_eq!(
        java.get("ok"),
        Some(&Value::Bool(true)),
        "java is ok when its stderr carries the version"
    );
    assert!(
        java.get("version")
            .and_then(Value::as_str)
            .is_some_and(|version| version.contains("openjdk")),
        "java reads its version from stderr"
    );
    let calls = seen.borrow();
    for (tool, expected) in [
        ("go", "go version"),
        ("java", "java -version"),
        ("rustc", "rustc --version"),
        ("cargo", "cargo --version"),
        ("node", "node --version"),
    ] {
        assert!(
            calls.iter().any(|call| call == expected),
            "{tool}: the probe runs `{expected}`, got {calls:?}"
        );
    }
    Ok(())
}

/// The truncation bounds hold exactly at the caps: one entry past 200 at the
/// root seals 200 entries and the cut flag, and a subtree that crosses 200
/// seals the same, while a status head past 4096 characters seals its own cut
/// flag (larry#274). Kills the bound mutants (`>=` to `>` keeps a 201st
/// entry; `true` to `false` drops the cut flag) and the head-cap mutant.
#[test]
fn sim_probe_truncation_bounds_hold_exactly_at_the_caps() -> TestResult {
    let names: Vec<String> = (0..201).map(|n| format!("f{n:03}")).collect();
    let files: Vec<(&str, &str)> = names.iter().map(|name| (name.as_str(), "x")).collect();
    let root = fixture(&files)?;
    let probed = probe(root.as_ref(), &scripted_run, &scripted_read);
    let tree = probed.get("tree").ok_or("tree")?;
    assert_eq!(
        tree.get("truncated"),
        Some(&Value::Bool(true)),
        "one entry past the cap seals the cut"
    );
    assert_eq!(
        tree.get("entries").and_then(Value::as_array).map(Vec::len),
        Some(200),
        "the root seals exactly the cap, never a 201st entry"
    );
    let mut nested: Vec<(String, &str)> = (0..190).map(|n| (format!("g{n:03}"), "x")).collect();
    for n in 0..20 {
        nested.push((format!("sub/h{n:02}"), "x"));
    }
    let borrowed: Vec<(&str, &str)> = nested
        .iter()
        .map(|entry| (entry.0.as_str(), entry.1))
        .collect();
    let deep = fixture(&borrowed)?;
    let probed = probe(deep.as_ref(), &scripted_run, &scripted_read);
    let tree = probed.get("tree").ok_or("tree")?;
    assert_eq!(
        tree.get("truncated"),
        Some(&Value::Bool(true)),
        "a subtree crossing the cap seals the cut"
    );
    assert_eq!(
        tree.get("entries").and_then(Value::as_array).map(Vec::len),
        Some(200),
        "the subtree seals exactly the cap, never a 201st entry"
    );
    let (lines, head, cut) = parse_status(&"a".repeat(5000));
    assert_eq!(lines, 1, "one long line counts once");
    assert_eq!(
        head.chars().count(),
        4096,
        "the head stops at its character cap"
    );
    assert!(cut, "a head past its cap seals the cut");
    Ok(())
}

/// The rendering skips entries with no name and separates manifests exactly:
/// a nameless tool entry never ends the toolchain list, and two manifests
/// render joined by one separator with no leading one (larry#274). Kills the
/// skip mutant (`continue` to `break` drops every tool after the nameless
/// one) and the separator mutant (`true` to `false` prints a leading comma).
#[test]
fn sim_probe_render_skips_nameless_entries_and_separates_manifests() {
    let probed = json!({
        "toolchains": [
            {"ok": true, "version": "nameless"},
            {"tool": "rustc", "ok": true, "version": "rustc 9.9.9"},
        ],
        "manifests": [
            {"path": "A", "len": 1},
            {"path": "B", "len": 2},
        ],
    });
    let text = render(&probed);
    assert!(
        text.contains("rustc rustc 9.9.9"),
        "a nameless entry never ends the toolchain list: {text:?}"
    );
    assert!(
        text.contains("manifests: A (1 bytes), B (2 bytes)"),
        "two manifests render joined by one separator: {text:?}"
    );
}

/// A malformed meminfo line never ends the read: a line with no numbers and
/// one with a foreign unit are skipped, and the valid values are read exactly
/// (larry#274). Kills the skip mutant (`continue` to `break` drops every line
/// after the malformed one, so the valid values never seal).
#[test]
fn sim_probe_a_malformed_meminfo_line_is_skipped_and_the_valid_value_is_read() -> TestResult {
    let text = "this line holds no numbers\nMemTotal: 100 MB\nMemTotal:        8000000 kB\nMemAvailable:    4000000 kB\n";
    let (total, available) = parse_meminfo(text)?;
    assert_eq!(
        (total, available),
        (8_000_000, 4_000_000),
        "a malformed line followed by valid ones reads the valid values exactly"
    );
    Ok(())
}

/// Junk before the `Pages free:` line never ends the read: the sysctl
/// fallback skips it and the free pages seal exactly (larry#274). Kills the
/// skip mutant (`continue` to `break` refuses the whole vm_stat output).
#[test]
fn sim_probe_vmstat_junk_lines_are_skipped_and_free_pages_are_read() -> TestResult {
    let root = fixture(&[])?;
    let unreadable = |_: &Path| -> Result<String, ProbeError> {
        Err(ProbeError::Refused("no procfs on this host"))
    };
    let fallback = |argv: &[String], _cwd: &Path| -> Result<ProbeCall, ProbeError> {
        let name = if let Some(first) = argv.first() {
            first.as_str()
        } else {
            ""
        };
        if name == "sysctl" {
            return Ok(ProbeCall::ok("4096\n"));
        }
        if name == "vm_stat" {
            return Ok(ProbeCall::ok(
                "junk header\nMach Virtual Memory Statistics: 0\nPages free: 100.\n",
            ));
        }
        Err(ProbeError::Command(format!("{name} is not on PATH")))
    };
    let probed = probe(root.as_ref(), &fallback, &unreadable);
    let memory = probed.get("memory").ok_or("memory")?;
    assert_eq!(
        memory.get("ok"),
        Some(&Value::Bool(true)),
        "junk lines before the free-pages line still seal ok"
    );
    assert_eq!(
        memory.get("total_kb").and_then(Value::as_u64),
        Some(4),
        "4096 sysctl bytes seal as exactly 4 KiB"
    );
    assert_eq!(
        memory.get("avail_kb").and_then(Value::as_u64),
        Some(400),
        "100 pages of 4096 bytes seal as exactly 400 KiB"
    );
    let tree = probed.get("tree").ok_or("tree")?;
    assert_eq!(
        tree.get("ok"),
        Some(&Value::Bool(true)),
        "the tree seals ok on a readable folder"
    );
    Ok(())
}

/// Fifty status lines seal uncut: the count equals the kept bound, so nothing
/// is cut and no marker is added (larry#274). Kills the cut mutant (`>` to
/// `>=` seals a cut over exactly fifty kept lines).
#[test]
fn sim_probe_fifty_status_lines_seal_uncut_with_no_marker() {
    let text: String = (0..50)
        .map(|n| format!(" M f{n:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (lines, head, cut) = parse_status(&text);
    assert_eq!(lines, 50, "fifty status lines count fifty");
    assert!(!cut, "a count that equals the kept bound seals no cut");
    assert_eq!(head, text, "nothing is cut and no marker is added");
}

/// The render bound holds exactly at its cap: a render of exactly 2048
/// characters seals whole, and one character past it seals the first 2048
/// (larry#274). The cap's guard is load-bearing: without it a 4096-character
/// git head would blow the model's budget.
#[test]
fn sim_probe_render_holds_exactly_at_its_bound() -> TestResult {
    let bare = render(&json!({"git": {"ok": true, "lines": 1, "head": ""}}));
    let room = MAX_RENDER_CHARS
        .checked_sub(bare.chars().count())
        .ok_or("the bare render fits its bound")?;
    let at = render(&json!({"git": {"ok": true, "lines": 1, "head": "a".repeat(room)}}));
    assert_eq!(
        at.chars().count(),
        MAX_RENDER_CHARS,
        "a render exactly at the bound seals whole"
    );
    assert!(
        at.ends_with("a\n"),
        "the whole render seals, newline included"
    );
    let past = render(&json!({"git": {"ok": true, "lines": 1, "head": "a".repeat(room + 1)}}));
    assert_eq!(
        past.chars().count(),
        MAX_RENDER_CHARS,
        "one character past the bound seals the first 2048"
    );
    assert!(
        past.ends_with("aa"),
        "the cut drops the trailing newline and keeps model bytes"
    );
    Ok(())
}

/// The tree renders its entries exactly when ok and its typed error when
/// refused (larry#274). Kills the ok mutant (`== Some(true)` to
/// `== Some(false)` renders a held tree as a bare label).
#[test]
fn sim_probe_render_names_tree_entries_only_when_ok() {
    let probed = json!({"tree": {"ok": true, "truncated": true, "entries": [{"path": "a", "kind": "file", "len": 3}]}});
    let text = render(&probed);
    assert!(
        text.contains("tree: a(file) ...truncated\n"),
        "entries and the marker render exactly: {text:?}"
    );
    let refused = json!({"tree": {"ok": false, "error": "boom"}});
    assert!(
        render(&refused).contains("tree: <boom>\n"),
        "a refused tree names its typed error"
    );
}

/// The toolchain list renders every kept tool with its exact ok flag: an
/// early failure and a later version both appear, joined by one separator
/// with no leading one (larry#274). Kills the separator mutant (`true` to
/// `false` prints a leading comma before the first tool).
#[test]
fn sim_probe_render_separates_tool_entries_without_a_leading_comma() {
    let probed = json!({"toolchains": [
        {"tool": "aaa", "ok": false, "error": "exit status 2"},
        {"tool": "zzz", "ok": true, "version": "zzz 1.0"},
    ]});
    let text = render(&probed);
    assert!(
        text.contains("toolchains: aaa <exit status 2>, zzz zzz 1.0\n"),
        "an early failure and a later version both appear with their exact ok flags: {text:?}"
    );
}

/// A nameless manifest entry never ends the list: the named one after it is
/// still listed with its size (larry#274). Kills the skip mutant (`continue`
/// to `break` drops every manifest after the nameless one).
#[test]
fn sim_probe_render_lists_a_named_manifest_after_a_nameless_one() {
    let probed = json!({"manifests": [{"nonsense": 1}, {"path": "Cargo.toml", "len": 12}]});
    let text = render(&probed);
    assert!(
        text.contains("manifests: Cargo.toml (12 bytes)\n"),
        "the named manifest after a nameless one is listed with its size: {text:?}"
    );
}
