//! The temp-path rule analyses only what its text can raise (larry#415): the verdict pass needs
//! `process` and a temp source, the follow pass needs `#[path` or `include`, and the checked parse
//! always runs, so a syntax fault in a skipped file still refuses. Requirement proved: the report
//! over a tree is the one every pass raises over every source, refusals and order included, and
//! every arm `first_temp` and `first_pid` read (`syntax.rs`, `uses.rs`) keeps its gate open.
//! Source: the Rust Reference on paths, `use` declarations and macros (every name the rule reads
//! is spelled in the source text, since a syntax tree expands no macro).

use lgwks_std::hash::Hasher;
use lgwks_std::seeded::Seeded;

use super::super::sim_support::{Scratch, SimError, SimResult};
use super::super::temp_paths;
use super::{cases, fixture, named, package};

/// Seeds the simulation sweeps.
const SEEDS: u64 = 64;

/// The refusal text a verdict raises, which opens the verdict gate.
const LEAK_START: &str = "a process id names a temp directory outside the guard";

/// A function that reaches a temp directory and names the process id by their std paths.
const STD_LEAK: &str = "fn f() -> String {\n    let d = std::env::temp_dir();\n    format!(\"{d:?}{}\", std::process::id())\n}\n";

/// The pieces a generated source is built from: noise; decoys naming every gate token in a comment,
/// a string and an identifier; one needle per arm the verdict pass reads (a std path, a module
/// alias, a self group, an imported function, an alias of it, both globs, a `/tmp` string, a raw
/// string, a byte string, a `/var/tmp` prefix, `var("TMPDIR")` called and imported, `env!`);
/// lookalikes that must pass (`TMP`, `/var/folders`, `var_os`, a rebound name); every
/// followed-module spelling and a syntax fault.
const FRAGMENTS: &[&str] = &[
    "fn noise() { let value = 1_u8; drop(value); }\n",
    "// process temp_dir /tmp TMPDIR #[path include\n",
    "fn decoy() -> &'static str { \"process temp_dir /tmp TMPDIR #[path include\" }\n",
    "fn processor_included_tmp() {}\n",
    "fn temp_only() -> std::path::PathBuf { std::env::temp_dir().join(\"x\") }\n",
    "fn pid_only() -> u32 { std::process::id() }\n",
    STD_LEAK,
    "use std::process as p;\nuse std::env as e;\nfn aliased() -> String { let dir = e::temp_dir(); format!(\"{dir:?}{}\", p::id()) }\n",
    "use std::{env::{self}, process::{self}};\nfn grouped() -> String { let dir = env::temp_dir(); format!(\"{dir:?}{}\", process::id()) }\n",
    "use std::env::temp_dir;\nuse std::process::id;\nfn imported() -> String { let dir = temp_dir(); format!(\"{dir:?}{}\", id()) }\n",
    "use std::env::temp_dir as td;\nuse std::process::id as pid;\nfn renamed() -> String { let dir = td(); format!(\"{dir:?}{}\", pid()) }\n",
    "use std::process::*;\nuse std::env::*;\nfn globbed() -> String { let dir = temp_dir(); format!(\"{dir:?}{}\", id()) }\n",
    "fn literal() -> String { let dir = \"/tmp/x\"; format!(\"{dir}{}\", std::process::id()) }\n",
    "fn raw() -> String { let dir = r#\"/var/tmp/y\"#; format!(\"{dir}{}\", std::process::id()) }\n",
    "fn bytes() -> String { let dir = b\"/tmpfile\"; format!(\"{dir:?}{}\", std::process::id()) }\n",
    "fn variable() -> String { let set = std::env::var(\"TMPDIR\").is_ok(); format!(\"{set}{}\", std::process::id()) }\n",
    "use std::env::var;\nfn imported_var() -> String { let set = var(\"TMPDIR\"); format!(\"{set:?}{}\", std::process::id()) }\n",
    "fn compiled() -> String { let dir = env!(\"TMPDIR\"); format!(\"{dir}{}\", std::process::id()) }\n",
    "fn not_temp() -> String { format!(\"{:?}{:?}{}\", std::env::var(\"TMP\"), std::env::var_os(\"TMPDIR\"), std::process::id()) }\n",
    "fn folders() -> String { let dir = \"/var/folders/x\"; format!(\"{dir}{}\", std::process::id()) }\n",
    "use std::env::temp_dir;\nuse std::process::id;\nfn rebound() { let temp_dir = 1_u8; let id = 2_u8; drop((temp_dir, id)); }\n",
    "#[cfg(test)]\nmod sim_marker {}\n",
    "#[path = \"gen/a.rs\"]\nmod followed;\n",
    "#[path = \"gen/absent.rs\"]\nmod absent;\n",
    "#[path =\"/etc/absolute.rs\"]\nmod absolute;\n",
    "#[ path = \"/etc/spaced.rs\"]\nmod spaced;\n",
    "#[cfg_attr(test, path = \"/etc/conditional.rs\")]\nmod conditional;\n",
    "fn includes() { include!(\"gen/inc.rs\"); }\n",
    "fn qualified() { std::include!(\"/etc/qualified.rs\"); }\n",
    "fn broken( {\n",
    "fn shadow() { let process = 1_u8; drop(process); }\n",
];

/// The names a generated tree's `tests` sources take: test code by their directory.
const TEST_NAMES: [&str; 4] = ["tests/g0", "tests/g1", "tests/g2", "tests/g3"];

/// The names its `src` sources take: two plain, one test code by its `sim_` stem and one by its
/// `_tests` stem; a `#[cfg(test)]` fragment makes a plain one test code too.
const SRC_NAMES: [&str; 4] = ["src/p0", "src/p1", "src/sim_g2", "src/g3_tests"];

/// One generated tree: a package with the two followed targets and eight sources, each built from
/// up to three drawn fragments.
fn tree_of(seed: u64) -> Result<Scratch, SimError> {
    let mut rng = Seeded::from_seed(seed.wrapping_add(7 << 40));
    let tree = package("temp-gate")?;
    named(&tree, "tests/gen/a.rs", "fn followed() {}\n")?;
    named(&tree, "tests/gen/inc.rs", "fn included() {}\n")?;
    for name in TEST_NAMES.iter().chain(SRC_NAMES.iter()) {
        let mut source = String::from("fn empty() {}\n");
        for _ in 0..rng.below(4)? {
            let piece = FRAGMENTS
                .get(rng.index(FRAGMENTS.len())?)
                .ok_or_else(|| SimError::Absent(String::from("a fragment")))?;
            source.push_str(piece);
        }
        named(&tree, &format!("{name}.rs"), &source)?;
    }
    Ok(tree)
}

/// BLAKE3 of a report, as hex.
fn hash_of(report: &[String]) -> String {
    let mut hasher = Hasher::new();
    hasher.update(report.join("\n").as_bytes());
    hasher.finalize().to_hex()
}

/// For 64 generated trees the gated report equals the ungated one, and one seed gives one trace
/// hash. Every arm of the verdict and follow passes, each lookalike and each syntax fault is drawn
/// among up to three fragments a file, so a token the gate forgot would change a report here, and
/// the counts hold that the sweep reaches the verdict pass, the follow pass and a refused file.
#[test]
fn sim_the_analysis_gate_never_changes_a_report() -> SimResult {
    let (mut leaks, mut followed, mut refused) = (0_u32, 0_u32, 0_u32);
    for seed in 0..SEEDS {
        let tree = tree_of(seed)?;
        let gated = temp_paths::errors(tree.path())?;
        let ungated = temp_paths::errors_ungated(tree.path())?;
        assert_eq!(gated, ungated, "seed {seed}: the gate changed the report");
        let again = temp_paths::errors(tree.path())?;
        assert_eq!(
            hash_of(&gated),
            hash_of(&again),
            "seed {seed}: one seed gives one trace hash"
        );
        if gated.iter().any(|error| error.starts_with(LEAK_START)) {
            leaks = leaks.saturating_add(1);
        }
        if gated.iter().any(|error| error.contains("followed module")) {
            followed = followed.saturating_add(1);
        }
        if gated.iter().any(|error| error.contains("refused")) {
            refused = refused.saturating_add(1);
        }
    }
    assert!(
        leaks >= 32,
        "{leaks} trees leaked: the sweep must reach the verdict pass"
    );
    assert!(
        followed >= 32,
        "{followed} trees followed: the sweep must reach the follow pass"
    );
    assert!(
        refused >= 8,
        "{refused} trees held a refused file: the sweep must reach the parse refusal"
    );
    Ok(())
}

/// One arm of the rule: where it lives, its source, the refusals the reference passes raise over it
/// and the answer the gate gives, as (verdict pass, follow pass), worked by hand from the tokens.
struct Arm {
    /// The root-relative path the fixture is written at.
    name: &'static str,
    /// The source text the fixture holds.
    source: &'static str,
    /// How many refusals every pass raises over the source.
    errors: usize,
    /// Whether the gate runs the verdict pass and the follow pass.
    gate: (bool, bool),
}

/// One arm that raises one refusal through the verdict pass.
const fn leak(name: &'static str, source: &'static str) -> Arm {
    Arm {
        name,
        source,
        errors: 1,
        gate: (true, false),
    }
}

/// One arm the rule passes, with the gate answer worked by hand.
const fn passes(name: &'static str, source: &'static str, gate: (bool, bool)) -> Arm {
    Arm {
        name,
        source,
        errors: 0,
        gate,
    }
}

/// One arm that raises one refusal through the follow pass.
const fn follow(name: &'static str, source: &'static str) -> Arm {
    Arm {
        name,
        source,
        errors: 1,
        gate: (false, true),
    }
}

/// Every arm of `first_temp` (`syntax.rs:106`) and `first_pid` (`uses.rs:83`), then the lookalikes
/// no arm reads and the follow spellings. Each leak holds `process` and a temp token; each
/// lookalike is a source the rule passes whatever the gate answers.
const ARMS: &[Arm] = &[
    leak("tests/std_path.rs", STD_LEAK),
    leak(
        "tests/std_prefix.rs",
        "fn f() -> String {\n    let d = ::std::env::temp_dir();\n    format!(\"{d:?}{}\", ::std::process::id())\n}\n",
    ),
    leak(
        "tests/module_alias.rs",
        "use std::env as e;\nuse std::process as p;\nfn f() -> String {\n    let d = e::temp_dir();\n    format!(\"{d:?}{}\", p::id())\n}\n",
    ),
    leak(
        "tests/self_group.rs",
        "use std::{env::{self}, process::{self}};\nfn f() -> String {\n    let d = env::temp_dir();\n    format!(\"{d:?}{}\", process::id())\n}\n",
    ),
    leak(
        "tests/imported.rs",
        "use std::env::temp_dir;\nuse std::process::id;\nfn f() -> String {\n    let d = temp_dir();\n    format!(\"{d:?}{}\", id())\n}\n",
    ),
    leak(
        "tests/renamed.rs",
        "use std::env::temp_dir as td;\nuse std::process::id as pid;\nfn f() -> String {\n    let d = td();\n    format!(\"{d:?}{}\", pid())\n}\n",
    ),
    leak(
        "tests/globs.rs",
        "use std::env::*;\nuse std::process::*;\nfn f() -> String {\n    let d = temp_dir();\n    format!(\"{d:?}{}\", id())\n}\n",
    ),
    leak(
        "tests/tmp_literal.rs",
        "fn f() -> String {\n    let d = \"/tmp/x\";\n    format!(\"{d}{}\", std::process::id())\n}\n",
    ),
    leak(
        "tests/var_tmp_literal.rs",
        "fn f() -> String {\n    let d = \"/var/tmp/x\";\n    format!(\"{d}{}\", std::process::id())\n}\n",
    ),
    leak(
        "tests/tmp_prefix.rs",
        "fn f() -> String {\n    let d = \"/tmpfile\";\n    format!(\"{d}{}\", std::process::id())\n}\n",
    ),
    leak(
        "tests/raw_literal.rs",
        "fn f() -> String {\n    let d = r#\"/tmp/x\"#;\n    format!(\"{d}{}\", std::process::id())\n}\n",
    ),
    leak(
        "tests/byte_literal.rs",
        "fn f() -> String {\n    let d = b\"/tmp/x\";\n    format!(\"{d:?}{}\", std::process::id())\n}\n",
    ),
    leak(
        "tests/var_call.rs",
        "fn f() -> String {\n    let d = std::env::var(\"TMPDIR\");\n    format!(\"{d:?}{}\", std::process::id())\n}\n",
    ),
    leak(
        "tests/var_import.rs",
        "use std::env::var;\nfn f() -> String {\n    let d = var(\"TMPDIR\");\n    format!(\"{d:?}{}\", std::process::id())\n}\n",
    ),
    leak(
        "tests/var_in_macro.rs",
        "fn f() -> String {\n    format!(\"{:?}{}\", std::env::var(\"TMPDIR\"), std::process::id())\n}\n",
    ),
    leak(
        "tests/env_macro.rs",
        "fn f() -> String {\n    let d = env!(\"TMPDIR\");\n    format!(\"{d}{}\", std::process::id())\n}\n",
    ),
    leak("src/sim_leak.rs", STD_LEAK),
    leak("src/leak_tests.rs", STD_LEAK),
    leak(
        "src/marked.rs",
        "#[cfg(test)]\nmod marker {}\nfn f() -> String {\n    let d = \"/tmp/x\";\n    format!(\"{d}{}\", std::process::id())\n}\n",
    ),
    passes("src/production.rs", STD_LEAK, (true, false)),
    passes(
        "tests/temp_only.rs",
        "fn f() -> std::path::PathBuf {\n    std::env::temp_dir().join(\"x\")\n}\n",
        (false, false),
    ),
    passes(
        "tests/pid_only.rs",
        "fn f() -> u32 {\n    std::process::id()\n}\n",
        (false, false),
    ),
    passes(
        "tests/two_functions.rs",
        "fn a() -> std::path::PathBuf {\n    std::env::temp_dir()\n}\nfn b() -> u32 {\n    std::process::id()\n}\n",
        (true, false),
    ),
    passes(
        "tests/tmp_name.rs",
        "fn f() -> String {\n    format!(\"{:?}{}\", std::env::var(\"TMP\"), std::process::id())\n}\n",
        (false, false),
    ),
    passes(
        "tests/var_os.rs",
        "fn f() -> String {\n    format!(\"{:?}{}\", std::env::var_os(\"TMPDIR\"), std::process::id())\n}\n",
        (true, false),
    ),
    passes(
        "tests/folders.rs",
        "fn f() -> String {\n    let d = \"/var/folders/x\";\n    format!(\"{d}{}\", std::process::id())\n}\n",
        (false, false),
    ),
    passes(
        "tests/upper.rs",
        "fn f() -> String {\n    let d = \"/TMP/x\";\n    format!(\"{d}{}\", std::process::id())\n}\n",
        (false, false),
    ),
    passes(
        "tests/usr_tmp.rs",
        "fn f() -> String {\n    let d = \"/usr/tmp/x\";\n    format!(\"{d}{}\", std::process::id())\n}\n",
        (true, false),
    ),
    passes(
        "tests/escaped.rs",
        "fn f() -> String {\n    let d = \"\\x2ftmp/x\";\n    format!(\"{d}{}\", std::process::id())\n}\n",
        (true, false),
    ),
    passes(
        "tests/reexport.rs",
        "fn f() -> String {\n    let d = std::env::temp_dir();\n    format!(\"{d:?}{}\", helper::pid())\n}\n",
        (false, false),
    ),
    passes(
        "tests/comment_only.rs",
        "// std::process::id temp_dir\nfn f() {}\n",
        (true, false),
    ),
    passes(
        "tests/rebound.rs",
        "use std::env::temp_dir;\nuse std::process::id;\nfn f() {\n    let temp_dir = 1_u8;\n    let id = 2_u8;\n    drop((temp_dir, id));\n}\n",
        (true, false),
    ),
    follow("tests/path_attr.rs", "#[path = \"/etc/x.rs\"]\nmod m;\n"),
    follow(
        "tests/path_attr_tight.rs",
        "#[path =\"/etc/x.rs\"]\nmod m;\n",
    ),
    follow(
        "tests/include_macro.rs",
        "fn f() {\n    include!(\"/etc/x.rs\");\n}\n",
    ),
    passes(
        "tests/path_spaced.rs",
        "#[ path = \"/etc/x.rs\"]\nmod m;\n",
        (false, false),
    ),
    passes(
        "tests/path_conditional.rs",
        "#[cfg_attr(test, path = \"/etc/x.rs\")]\nmod m;\n",
        (false, false),
    ),
    passes(
        "tests/include_qualified.rs",
        "fn f() {\n    std::include!(\"/etc/x.rs\");\n}\n",
        (false, true),
    ),
    passes("tests/include_word.rs", "fn included() {}\n", (false, true)),
];

/// Each arm gives the reference refusals, the hand-worked gate answer and the same report gated and
/// ungated; and whichever pass raised a refusal, its gate is open, so the table cannot be right by
/// listing the answer the code gives.
#[test]
fn sim_every_arm_of_the_rule_is_inside_its_gate() -> SimResult {
    for arm in ARMS {
        let tree = fixture("temp-arm", arm.name, arm.source)?;
        let gated = temp_paths::errors(tree.path())?;
        let ungated = temp_paths::errors_ungated(tree.path())?;
        assert_eq!(gated, ungated, "{}: the gate changed the report", arm.name);
        assert_eq!(ungated.len(), arm.errors, "{}: {ungated:?}", arm.name);
        let gate = (
            temp_paths::may_refuse(arm.source),
            temp_paths::may_follow(arm.source),
        );
        assert_eq!(gate, arm.gate, "{}: the gate answered {gate:?}", arm.name);
        if ungated.iter().any(|error| error.starts_with(LEAK_START)) {
            assert!(gate.0, "{}: a verdict needs its gate open", arm.name);
        }
        if ungated
            .iter()
            .any(|error| error.contains("followed module"))
        {
            assert!(
                gate.1,
                "{}: a followed module needs its gate open",
                arm.name
            );
        }
    }
    Ok(())
}

/// Every reference case of the rule (`sim_temp_paths.rs`, larry#293) keeps its gate open when it
/// refuses and gives the same report gated and ungated when it passes.
#[test]
fn sim_every_reference_case_of_the_rule_keeps_its_gate_open() -> SimResult {
    for case in cases() {
        let tree = fixture("temp-reference", case.name, case.source)?;
        let gated = temp_paths::errors(tree.path())?;
        assert_eq!(
            gated,
            temp_paths::errors_ungated(tree.path())?,
            "{}: the gate changed the report",
            case.name
        );
        if !case.lines.is_empty() {
            assert!(
                temp_paths::may_refuse(case.source),
                "{}: the case refuses, so its gate must be open",
                case.name
            );
        }
    }
    Ok(())
}
