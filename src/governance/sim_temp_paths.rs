//! Seeded controls of the temp-path rule over parsed names (larry#293), the follow-up to
//! larry#267's text rule. Source: The Rust Reference on path expressions, use declarations and
//! macros-by-example, and CWE-59 for the symlink walk. Requirement: a process id that reaches a
//! temp path through a split binding, a `use` alias, a shadow or a macro is refused at the temp
//! call's line, a string literal that only looks like code passes, and the walk never follows a
//! symlink.

use lgwks_std::hash::Hasher;
use lgwks_std::seeded::Seeded;

use super::sim_support::{Scratch, SimError, SimResult};
use super::temp_paths;

/// The refusal the rule writes for `relative` at `line`.
fn refusal(relative: &str, line: usize) -> String {
    format!("a process id names a temp directory outside the guard: {relative}:{line}")
}

/// One fixture: where it lives, its source text and the lines the rule must refuse.
struct Case {
    /// The root-relative path the fixture is written at.
    name: &'static str,
    /// The source text the fixture holds.
    source: &'static str,
    /// The lines the rule must refuse, in order.
    lines: &'static [usize],
}

/// Every naming case the issue lists: a split binding, each `use` form, a shadow and the guard.
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "tests/split_binding.rs",
            source: "fn state() -> std::path::PathBuf {\n    let pid = std::process::id();\n    std::env::temp_dir().join(format!(\"x-{pid}\"))\n}\n",
            lines: &[3],
        },
        Case {
            name: "tests/use_module.rs",
            source: "use std::process;\n\nfn state() -> std::path::PathBuf {\n    std::env::temp_dir().join(format!(\"x-{}\", process::id()))\n}\n",
            lines: &[4],
        },
        Case {
            name: "tests/use_function.rs",
            source: "use std::env::temp_dir;\nuse std::process::id;\n\nfn state() -> std::path::PathBuf {\n    temp_dir().join(format!(\"x-{}\", id()))\n}\n",
            lines: &[5],
        },
        Case {
            name: "tests/use_alias.rs",
            source: "use std::env::temp_dir as td;\nuse std::process as p;\n\nfn state() -> std::path::PathBuf {\n    td().join(format!(\"x-{}\", p::id()))\n}\n",
            lines: &[5],
        },
        Case {
            name: "tests/use_group.rs",
            source: "use std::{env::temp_dir, process};\n\nfn state() -> std::path::PathBuf {\n    temp_dir().join(format!(\"x-{}\", process::id()))\n}\n",
            lines: &[4],
        },
        Case {
            name: "tests/shadowed.rs",
            source: "fn state() -> std::path::PathBuf {\n    let pid = std::process::id();\n    let pid = 7;\n    std::env::temp_dir().join(format!(\"x-{pid}\"))\n}\n",
            lines: &[],
        },
        Case {
            name: "tests/inner_shadow.rs",
            source: "fn state() -> std::path::PathBuf {\n    let pid = std::process::id();\n    let path = {\n        let pid = 7;\n        std::env::temp_dir().join(format!(\"x-{pid}\"))\n    };\n    path\n}\n",
            lines: &[],
        },
        Case {
            name: "tests/guard_route.rs",
            source: "fn state() -> origin::TempDir {\n    origin::TempDir::new()\n}\n",
            lines: &[],
        },
        Case {
            name: "tests/pid_elsewhere.rs",
            source: "fn name() -> String {\n    format!(\"origin-{}-0\", std::process::id())\n}\n",
            lines: &[],
        },
        Case {
            name: "tests/local_temp_dir.rs",
            source: "fn temp_dir() -> String {\n    String::from(\"fixture\")\n}\n\nfn state() -> String {\n    let pid = std::process::id();\n    format!(\"{}-{pid}\", crate::temp_dir())\n}\n",
            lines: &[],
        },
        Case {
            name: "src/production.rs",
            source: "fn state() -> std::path::PathBuf {\n    std::env::temp_dir().join(format!(\"x-{}\", std::process::id()))\n}\n",
            lines: &[],
        },
    ]
}

/// The case names in the order one seed draws: the fixture is one order, one case at a time.
fn order(seed: u64) -> Result<Vec<String>, SimError> {
    let mut names: Vec<String> = cases().iter().map(|case| String::from(case.name)).collect();
    let mut rng = Seeded::from_seed(seed);
    for slot in (1..names.len()).rev() {
        let pick = rng.index(slot.saturating_add(1))?;
        names.swap(slot, pick);
    }
    Ok(names)
}

/// The issue's first proof, seeded: a split binding, a `use` alias of each name, a shadow and the
/// guard route are each refused or passed exactly, and one seed draws one order.
#[test]
fn sim_split_bindings_and_use_aliases_get_the_reference_verdict() -> SimResult {
    for probe in [
        "let pid = std::process::id();",
        "use std::env::temp_dir as td;",
        "let pid = 7;",
        "origin::TempDir::new()",
        "crate::temp_dir()",
    ] {
        assert!(
            cases().iter().any(|case| case.source.contains(probe)),
            "a case drives {probe:?}"
        );
    }
    assert_eq!(order(0x293)?, order(0x293)?, "one seed draws one order");
    assert_ne!(
        order(0x293)?,
        order(0x293_u64.wrapping_add(1))?,
        "another seed draws another order"
    );
    for case in cases() {
        let tree = Scratch::new("temp-paths")?;
        tree.write(case.name, case.source.as_bytes())?;
        let wanted: Vec<String> = case
            .lines
            .iter()
            .map(|line| refusal(case.name, *line))
            .collect();
        assert_eq!(temp_paths::errors(tree.path()), wanted, "{}", case.name);
    }
    Ok(())
}

/// The issue's macro proof: the code a macro receives is code, the literal it receives is data,
/// and a `format!` capture of a bound process id is a temp path that names it.
#[test]
fn sim_macros_are_code_and_string_literals_are_data() -> SimResult {
    let fixtures: [(&str, &str, &[usize]); 4] = [
        (
            "tests/macro_argument.rs",
            "fn state() -> std::path::PathBuf {\n    std::env::temp_dir().join(format!(\"x-{}\", std::process::id()))\n}\n",
            &[2],
        ),
        (
            "tests/macro_capture.rs",
            "fn state() -> std::path::PathBuf {\n    let pid = std::process::id();\n    std::env::temp_dir().join(format!(\"x-{pid}\"))\n}\n",
            &[3],
        ),
        (
            "tests/literal_only.rs",
            "fn state() -> std::path::PathBuf {\n    let label = \"std::process::id()\";\n    std::env::temp_dir().join(label)\n}\n",
            &[],
        ),
        (
            "tests/macro_literal_only.rs",
            "fn state() -> std::path::PathBuf {\n    std::env::temp_dir().join(format!(\"std::process::id()\"))\n}\n",
            &[],
        ),
    ];
    let probe = r#"format!("x-{pid}")"#;
    assert!(
        fixtures.iter().any(|case| case.1.contains(probe)),
        "the capture case drives {probe}"
    );
    for (name, source, lines) in fixtures {
        let tree = Scratch::new("temp-macros")?;
        tree.write(name, source.as_bytes())?;
        let wanted: Vec<String> = lines.iter().map(|line| refusal(name, *line)).collect();
        assert_eq!(temp_paths::errors(tree.path()), wanted, "{name}");
    }
    Ok(())
}

/// The issue's symlink proof: a cycle under the walked tree ends and refuses nothing extra.
#[test]
fn sim_a_symlink_cycle_ends_and_refuses_nothing_extra() -> SimResult {
    let tree = Scratch::new("temp-cycle")?;
    tree.write("tests/plain.rs", b"fn state() {}\n")?;
    let cycle = "tests/loop";
    std::os::unix::fs::symlink(tree.path().join("tests"), tree.path().join(cycle))?;
    assert!(
        temp_paths::errors(tree.path()).is_empty(),
        "a symlink cycle is not followed"
    );
    Ok(())
}

/// One root of generated sources: its guard and the refusals the rule must report for it.
struct Root {
    /// The scratch guard that removes the root.
    tree: Scratch,
    /// The refusals, in path order.
    wanted: Vec<String>,
}

/// `TREES` roots, each holding `FILES` generated sources: half name a pid temp path and half take
/// the one guard, drawn on `seed`.
fn generated(seed: u64) -> Result<Vec<Root>, SimError> {
    const TREES: usize = 8;
    const FILES: usize = 512;
    assert_eq!(
        TREES.saturating_mul(FILES),
        4_096,
        "8 roots of 512 files hold 4_096 sources"
    );
    let mut rng = Seeded::from_seed(seed);
    let mut roots = Vec::with_capacity(TREES);
    for index in 0..TREES {
        let tree = Scratch::new(&format!("temp-many-{index}"))?;
        let mut wanted = Vec::new();
        for file in 0..FILES {
            let relative = format!("tests/generated/f{file}.rs");
            if rng.below(2)? == 0 {
                tree.write(
                    &relative,
                    b"fn state() -> std::path::PathBuf {\n    std::env::temp_dir().join(format!(\"x-{}\", std::process::id()))\n}\n",
                )?;
                wanted.push(refusal(&relative, 2));
            } else {
                tree.write(
                    &relative,
                    b"fn state() -> std::path::PathBuf {\n    origin::TempDir::new().path().to_path_buf()\n}\n",
                )?;
            }
        }
        wanted.sort();
        roots.push(Root { tree, wanted });
    }
    Ok(roots)
}

/// The verdicts of every root, in order, as one trace hash.
fn trace_of(roots: &[Root]) -> Result<String, SimError> {
    let mut trace = Hasher::new();
    for root in roots {
        for error in temp_paths::errors(root.tree.path()) {
            trace.write_framed(error.as_bytes());
        }
    }
    Ok(lgwks_std::hex::encode(trace.finalize().as_bytes()))
}

/// The issue's many-root proof, seeded: 4_096 generated sources across 8 roots are each refused
/// or passed, each root names only its own files, and one seed gives one trace hash.
#[test]
fn sim_many_roots_of_generated_sources_name_only_their_own() -> SimResult {
    let expected = 8_usize.saturating_mul(512);
    assert_eq!(
        expected, 4_096,
        "eight roots of 512 files hold 4_096 sources"
    );
    let first = generated(0x293_4096)?;
    for root in &first {
        assert_eq!(temp_paths::errors(root.tree.path()), root.wanted);
    }
    let second = generated(0x293_4096)?;
    assert_eq!(
        trace_of(&first)?,
        trace_of(&second)?,
        "one seed gives one trace hash"
    );
    let other = generated(0x293_4097)?;
    assert_ne!(
        trace_of(&first)?,
        trace_of(&other)?,
        "another seed draws another trace"
    );
    Ok(())
}
