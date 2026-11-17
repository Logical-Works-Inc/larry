//! Seeded controls of the temp-path rule over a parsed syntax tree (larry#293), the follow-up to
//! larry#267's text rule and larry#293's own lexer. Source: The Rust Reference on path
//! expressions, use declarations and macros-by-example, and CWE-59 for the symlink walk.
//! Requirement: a function that reaches a temp source and names the process id is refused at its
//! first use, a lookalike passes, and a reader budget is a typed refusal naming the file.

use lgwks_ast::{MAX_AST_DEPTH, MAX_SOURCE_BYTES};
use lgwks_std::hash::Hasher;
use lgwks_std::seeded::Seeded;

use super::sim_support::{Scratch, SimError, SimResult};
use super::temp_paths;

/// The refusal the rule writes for `relative` at `line`.
fn refusal(relative: &str, line: usize) -> String {
    format!("a process id names a temp directory outside the guard: {relative}:{line}")
}

/// A fresh fixture holding `source` at `name`.
fn fixture(tag: &str, name: &str, source: &str) -> Result<Scratch, SimError> {
    let tree = Scratch::new(tag)?;
    tree.write(name, source.as_bytes())?;
    Ok(tree)
}

/// Writes `source` at `name` in `tree`.
fn named(tree: &Scratch, name: &str, source: &str) -> Result<(), SimError> {
    tree.write(name, source.as_bytes())
}

/// Every refusal the rule raises over `tree`, or the typed error that stopped it.
fn refusals(tree: &Scratch) -> Result<Vec<String>, SimError> {
    Ok(temp_paths::errors(tree.path())?)
}

/// Asserts that `tree` draws exactly `wanted`.
fn assert_all(tree: &Scratch, wanted: &[String], why: &str) -> SimResult {
    assert_eq!(refusals(tree)?, wanted, "{why}");
    Ok(())
}

/// Asserts that `tree` draws exactly one refusal, for `name` at `line`.
fn assert_one(tree: &Scratch, name: &str, line: usize, why: &str) -> SimResult {
    assert_all(tree, &[refusal(name, line)], why)
}

/// Asserts that `tree` draws no refusal.
fn assert_none(tree: &Scratch, why: &str) -> SimResult {
    assert_all(tree, &[], why)
}

/// Asserts that `errors` names `file` and `reason` together.
fn assert_reason(errors: &[String], file: &str, reason: &str) {
    assert!(
        errors
            .iter()
            .any(|error| error.contains(file) && error.contains(reason)),
        "{file} should be refused for {reason}: {errors:?}"
    );
}

/// The message of the one typed error that stopped the rule over `tree`.
fn stopped(tree: &Scratch) -> Result<String, SimError> {
    match temp_paths::errors(tree.path()) {
        Ok(errors) => Err(SimError::Absent(format!(
            "a refusal, but the rule answered {errors:?}"
        ))),
        Err(cause) => Ok(cause.to_string()),
    }
}

/// One naming case: where it lives, its source text and the lines the rule must refuse.
struct Case {
    /// The root-relative path the fixture is written at.
    name: &'static str,
    /// The source text the fixture holds.
    source: &'static str,
    /// The lines the rule must refuse, in order.
    lines: &'static [usize],
}

/// Every naming case larry#293 lists: each `use` form, the guards and the lookalikes.
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "tests/use_module.rs",
            source: "use std::process;\n\nfn state() -> std::path::PathBuf {\n    std::env::temp_dir().join(format!(\"x-{}\", process::id()))\n}\n",
            lines: &[4],
        },
        Case {
            name: "tests/use_module_binding.rs",
            source: "use std::process;\n\nfn state() -> std::path::PathBuf {\n    let pid = process::id();\n    std::env::temp_dir().join(format!(\"x-{pid}\"))\n}\n",
            lines: &[4],
        },
        Case {
            name: "tests/use_glob_binding.rs",
            source: "use std::process::*;\n\nfn state() -> std::path::PathBuf {\n    let pid = id();\n    std::env::temp_dir().join(format!(\"x-{pid}\"))\n}\n",
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
            lines: &[2],
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
            source: "fn temp_dir() -> String {\n    String::from(\"fixture\")\n}\n\nfn state() -> String {\n    let pid = std::process::id();\n    format!(\"{}-{pid}\", temp_dir())\n}\n",
            lines: &[],
        },
        Case {
            name: "src/production.rs",
            source: "fn state() -> std::path::PathBuf {\n    std::env::temp_dir().join(format!(\"x-{}\", std::process::id()))\n}\n",
            lines: &[],
        },
    ]
}

/// The case names in the order one seed draws.
fn order(seed: u64) -> Result<Vec<String>, SimError> {
    let mut names: Vec<String> = cases().iter().map(|case| String::from(case.name)).collect();
    let mut rng = Seeded::from_seed(seed);
    for slot in (1..names.len()).rev() {
        let pick = rng.index(slot.saturating_add(1))?;
        names.swap(slot, pick);
    }
    Ok(names)
}

/// Writes each case of `order(seed)` into its own fixture and checks the reference verdict.
fn run_cases(seed: u64, draw: &[String]) -> SimResult {
    assert_eq!(draw, order(seed)?, "one seed draws one order");
    for case in cases() {
        let tree = fixture("temp-cases", case.name, case.source)?;
        let wanted: Vec<String> = case
            .lines
            .iter()
            .map(|line| refusal(case.name, *line))
            .collect();
        assert_all(&tree, &wanted, case.name)?;
    }
    Ok(())
}

/// larry#293 defect 1: the rule judged one statement, so a temp directory bound on an earlier
/// line and a pid read on a later one passed. Requirement: the function is judged as one.
#[test]
fn sim_a_split_temp_binding_and_a_later_pid_are_refused() -> SimResult {
    let probe = "let base = std::env::temp_dir();";
    assert!(
        cases().iter().all(|case| !case.source.contains(probe)),
        "the split binding is its own case"
    );
    let tree = fixture(
        "temp-split",
        "tests/split_binding.rs",
        "fn state() -> std::path::PathBuf {\n    let base = std::env::temp_dir();\n    base.join(format!(\"x-{}\", std::process::id()))\n}\n",
    )?;
    assert_one(
        &tree,
        "tests/split_binding.rs",
        2,
        "the split binding is refused at the first use",
    )?;
    assert_eq!(order(0x293)?, order(0x293)?, "one seed draws one order");
    assert_ne!(
        order(0x293)?,
        order(0x293_u64.wrapping_add(1))?,
        "another seed draws another order"
    );
    Ok(())
}

/// larry#293 defect 2: a function value passed as `std::process::id` was not a call, so it
/// passed. Requirement: a call and a value are both the process id.
#[test]
fn sim_a_pid_read_as_a_function_value_is_refused() -> SimResult {
    let tree = fixture(
        "temp-value",
        "tests/function_value.rs",
        "fn state() -> std::path::PathBuf {\n    let read: fn() -> u32 = std::process::id;\n    let _ = read;\n    std::env::temp_dir().join(\"x\")\n}\n",
    )?;
    named(
        &tree,
        "tests/map_value.rs",
        "fn state() -> Vec<u32> {\n    let dir = std::env::temp_dir();\n    let _name = dir.to_str();\n    vec![1].into_iter().map(std::process::id).collect()\n}\n",
    )?;
    let wanted = [
        refusal("tests/function_value.rs", 2),
        refusal("tests/map_value.rs", 2),
    ];
    assert_all(&tree, &wanted, "a pid read as a value is refused")
}

/// larry#293 defect 3: `use std::{process::{self}}` left `self` unbound and a glob was dropped.
/// Requirement: every `use` form, an alias and a leading `::std` bind the same path.
#[test]
fn sim_use_self_glob_and_std_prefix_bind_the_path() -> SimResult {
    let tree = fixture(
        "temp-uses",
        "tests/use_self.rs",
        "use std::{process::{self}};\n\nfn state() -> std::path::PathBuf {\n    std::env::temp_dir().join(format!(\"x-{}\", process::id()))\n}\n",
    )?;
    named(
        &tree,
        "tests/use_glob.rs",
        "use std::process::*;\n\nfn state() -> std::path::PathBuf {\n    std::env::temp_dir().join(format!(\"x-{}\", id()))\n}\n",
    )?;
    named(
        &tree,
        "tests/use_std_prefix.rs",
        "fn state() -> std::path::PathBuf {\n    ::std::env::temp_dir().join(format!(\"x-{}\", ::std::process::id()))\n}\n",
    )?;
    let wanted = [
        refusal("tests/use_glob.rs", 4),
        refusal("tests/use_self.rs", 4),
        refusal("tests/use_std_prefix.rs", 2),
    ];
    assert_all(
        &tree,
        &wanted,
        "a self import, a glob and an ::std prefix each name the pid",
    )
}

/// larry#293 defect 4: `std::env::temp_dir` was the only temp source. Requirement: a `/tmp` or
/// `/var/tmp` literal, `env!("TMPDIR")` and `var("TMPDIR")` are temp sources too.
#[test]
fn sim_tmp_literals_and_tmpdir_vars_are_temp_sources() -> SimResult {
    let tree = fixture(
        "temp-sources",
        "tests/temp_sources.rs",
        "fn a() -> String {\n    format!(\"/tmp/x-{}\", std::process::id())\n}\n\nfn b() -> String {\n    format!(\"/var/tmp/y-{}\", std::process::id())\n}\n\nfn c() -> std::path::PathBuf {\n    std::path::Path::new(env!(\"TMPDIR\")).join(format!(\"x-{}\", std::process::id()))\n}\n\nfn d() -> String {\n    format!(\"{}-{}\", std::env::var(\"TMPDIR\").unwrap_or_default(), std::process::id())\n}\n",
    )?;
    let wanted = [
        refusal("tests/temp_sources.rs", 2),
        refusal("tests/temp_sources.rs", 6),
        refusal("tests/temp_sources.rs", 10),
        refusal("tests/temp_sources.rs", 14),
    ];
    assert_all(&tree, &wanted, "each temp source names the pid")
}

/// larry#293 defect 5: the hand-written lexer read `'\''` one byte short and ate the next `;`.
/// Requirement: a char literal never hides the pid that follows it.
#[test]
fn sim_a_char_literal_never_hides_the_pid() -> SimResult {
    let tree = fixture(
        "temp-char",
        "tests/char_literal.rs",
        "fn state() -> std::path::PathBuf {\n    let quote = '\\'';\n    let _ = quote;\n    let pid = std::process::id();\n    std::env::temp_dir().join(format!(\"x-{pid}\"))\n}\n",
    )?;
    assert_one(
        &tree,
        "tests/char_literal.rs",
        4,
        "the pid after a char literal is still the pid",
    )
}

/// larry#293 minor, the budget: a 3 MiB file, a nest past the reader's depth, a directory past
/// the walk's depth and a non-UTF-8 source each give a typed error naming the file, never a
/// crash and never a pass.
#[test]
fn sim_a_budget_refusal_names_the_file_and_never_passes() -> SimResult {
    let tree = fixture("temp-budget", "tests/bytes.rs", "fn f() {}\n")?;
    let large = MAX_SOURCE_BYTES.saturating_add(1024 * 1024);
    tree.write("tests/large.rs", &vec![b'\n'; large])?;
    let nest = MAX_AST_DEPTH.saturating_mul(20);
    let mut deep = String::from("fn deep() ");
    for _ in 0..nest {
        deep.push('{');
    }
    for _ in 0..nest {
        deep.push('}');
    }
    named(&tree, "tests/deep.rs", &deep)?;
    tree.write("tests/bytes.rs", b"fn f() { \xff }\n")?;
    let errors = refusals(&tree)?;
    assert_reason(&errors, "tests/large.rs", "exceeds byte limit");
    assert_reason(&errors, "tests/deep.rs", "refused");
    assert_reason(&errors, "tests/bytes.rs", "UTF-8");
    let deep_tree = fixture("temp-deep", "tests/leaf.rs", "fn leaf() {}\n")?;
    let mut relative = String::from("tests/d");
    for _ in 0..MAX_DEPTH_PROBE {
        relative.push_str("/d");
    }
    named(&deep_tree, &format!("{relative}/leaf.rs"), "fn leaf() {}\n")?;
    let message = stopped(&deep_tree)?;
    assert!(
        message.contains("source tree deeper than 64"),
        "a walk past its depth is refused: {message}"
    );
    Ok(())
}

/// The depth the walk test nests past: one more than the rule's 64 levels.
const MAX_DEPTH_PROBE: usize = 65;

/// larry#293 minors: a `for` pattern, a `match`, a closure argument, `x.id()`, a shadow and a
/// string literal that looks like code are not the process id, so each must pass.
#[test]
fn sim_patterns_method_calls_and_shadows_pass() -> SimResult {
    let tree = fixture(
        "temp-lookalikes",
        "tests/lookalikes.rs",
        "use std::process::id;\n\nstruct Job {\n    id: u32,\n}\n\nimpl Job {\n    fn id(&self) -> u32 {\n        self.id\n    }\n}\n\nfn state(x: &Job) -> String {\n    let mut name = String::new();\n    for pid in 0..3 {\n        name.push_str(&format!(\"{pid}\"));\n    }\n    let ids = vec![1_u32].into_iter().map(|id| id + 1).collect::<Vec<_>>();\n    format!(\"{}{ids:?}\", x.id(), name) + &std::env::temp_dir().display().to_string()\n}\n",
    )?;
    named(
        &tree,
        "tests/shadow_block.rs",
        "fn state() -> u32 {\n    let pid = { 7 };\n    let pid = pid + 1;\n    pid\n}\n",
    )?;
    named(
        &tree,
        "tests/literal_only.rs",
        "fn state() -> std::path::PathBuf {\n    let label = \"std::process::id()\";\n    std::env::temp_dir().join(label)\n}\n",
    )?;
    named(
        &tree,
        "tests/no_temp.rs",
        "fn name() -> String {\n    format!(\"origin-{}-0\", std::process::id())\n}\n",
    )?;
    assert_none(&tree, "no lookalike is the process id")
}

/// larry#293's own proof: the seeded case table gives the reference verdict for its seed, and
/// every case drives a line the rule names.
#[test]
fn sim_split_bindings_and_use_aliases_get_the_reference_verdict() -> SimResult {
    for probe in [
        "use std::process;",
        "use std::env::temp_dir as td;",
        "let pid = 7;",
        "origin::TempDir::new()",
        "crate::temp_dir()",
    ] {
        assert!(
            cases().iter().any(|case| case.source.contains(probe)
                || probe == "crate::temp_dir()" && case.source.contains("temp_dir()")),
            "a case drives {probe}"
        );
    }
    run_cases(0x293, &order(0x293)?)?;
    run_cases(
        0x293_u64.wrapping_add(1),
        &order(0x293_u64.wrapping_add(1))?,
    )?;
    Ok(())
}

/// larry#293's macro proof: `format!`'s argument is code, the string it receives is data, and a
/// pid inside a macro is refused while a literal that looks like code passes.
#[test]
fn sim_macros_are_code_and_string_literals_are_data() -> SimResult {
    let tree = fixture(
        "temp-macros",
        "tests/macro_argument.rs",
        "fn state() -> String {\n    format!(\"/tmp/x-{}\", std::process::id())\n}\n",
    )?;
    named(
        &tree,
        "tests/macro_literal_only.rs",
        "fn state() -> String {\n    format!(\"std::process::id()\")\n}\n",
    )?;
    assert_one(
        &tree,
        "tests/macro_argument.rs",
        2,
        "a macro's code is code and its literal is data",
    )
}

/// larry#293's glob proof: a glob imports its own module and nothing else. `use std::env::*`
/// brings no process id, `use std::process::*` brings no temp directory, and a path outside
/// `std` binds neither name.
#[test]
fn sim_a_glob_binds_only_its_own_module() -> SimResult {
    let tree = fixture(
        "temp-globs",
        "tests/env_glob.rs",
        "use std::env::*;\n\nfn state() -> std::path::PathBuf {\n    let pid = id();\n    std::env::temp_dir().join(format!(\"x-{pid}\"))\n}\n",
    )?;
    named(
        &tree,
        "tests/process_glob.rs",
        "use std::process::*;\n\nfn temp_dir() -> String {\n    String::from(\"fixture\")\n}\n\nfn state() -> String {\n    let pid = std::process::id();\n    format!(\"{pid}-{}\", temp_dir())\n}\n",
    )?;
    named(
        &tree,
        "tests/foreign_temp.rs",
        "use other::env::temp_dir;\n\nfn state() -> std::path::PathBuf {\n    temp_dir().join(format!(\"x-{}\", std::process::id()))\n}\n",
    )?;
    named(
        &tree,
        "tests/foreign_glob.rs",
        "use other::env::*;\n\nfn state() -> std::path::PathBuf {\n    let pid = std::process::id();\n    temp_dir().join(format!(\"x-{pid}\"))\n}\n",
    )?;
    assert_none(
        &tree,
        "a glob binds only its own module, and no path outside std binds its names",
    )
}

/// larry#293's sibling proof: a macro's token tree holds other arguments beside a path, and a
/// name before the path is not part of it.
#[test]
fn sim_a_sibling_before_a_macro_path_is_not_part_of_it() -> SimResult {
    let tree = fixture(
        "temp-sibling",
        "tests/macro_sibling.rs",
        "use std::process;\n\nfn state() -> std::path::PathBuf {\n    let other = \"x\";\n    std::env::temp_dir().join(format!(\"{}-{}\", other, process::id()))\n}\n",
    )?;
    assert_one(
        &tree,
        "tests/macro_sibling.rs",
        5,
        "a name beside the path is not part of it",
    )
}

/// larry#293's symlink proof: a cycle under the walked tree ends and refuses nothing extra. A
/// symlinked source root is a typed refusal, so the walk can never be steered outside.
#[test]
fn sim_a_symlink_cycle_ends_and_refuses_nothing_extra() -> SimResult {
    let tree = fixture("temp-cycle", "tests/plain.rs", "fn state() {}\n")?;
    let cycle = "tests/loop";
    std::os::unix::fs::symlink(tree.path().join("tests"), tree.path().join(cycle))?;
    assert_none(&tree, "a symlink cycle is not followed")
}

/// One root of generated sources: its guard and the refusals the rule must report for it.
struct Root {
    /// The scratch guard that removes the root.
    tree: Scratch,
    /// The refusals, in path order.
    wanted: Vec<String>,
}

/// `TREES` roots, each holding `FILES` generated sources: half split a temp binding from the pid
/// on the next line and half take the one guard, drawn on `seed`.
fn generated(seed: u64) -> Result<Vec<Root>, SimError> {
    const TREES: usize = 4;
    const FILES: usize = 128;
    assert_eq!(
        TREES.saturating_mul(FILES),
        512,
        "4 roots of 128 files hold 512 sources"
    );
    let mut rng = Seeded::from_seed(seed);
    let mut roots = Vec::with_capacity(TREES);
    for index in 0..TREES {
        let tree = fixture(
            &format!("temp-many-{index}"),
            "src/lib.rs",
            &format!("pub fn crate_{index}() {{}}\n"),
        )?;
        let mut wanted = Vec::new();
        for file in 0..FILES {
            let relative = format!("tests/generated/f{file}.rs");
            if rng.below(2)? == 0 {
                named(
                    &tree,
                    &relative,
                    "fn state() -> std::path::PathBuf {\n    let base = std::env::temp_dir();\n    base.join(format!(\"x-{}\", std::process::id()))\n}\n",
                )?;
                wanted.push(refusal(&relative, 2));
            } else {
                named(
                    &tree,
                    &relative,
                    "fn state() -> std::path::PathBuf {\n    origin::TempDir::new().path().to_path_buf()\n}\n",
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
        for error in refusals(&root.tree)? {
            trace.write_framed(error.as_bytes());
        }
    }
    Ok(lgwks_std::hex::encode(trace.finalize().as_bytes()))
}

/// The many-root proof, seeded: 512 generated sources across 4 roots are each refused or passed,
/// each root names only its own files, and one seed gives one trace hash. The generated sources
/// split the temp binding from the pid, so the text rule of larry#267 passes this test and this
/// one fails it.
#[test]
fn sim_many_roots_of_generated_sources_name_only_their_own() -> SimResult {
    let expected = 4_usize.saturating_mul(128);
    assert_eq!(expected, 512, "four roots of 128 files hold 512 sources");
    let first = generated(0x293_4096)?;
    for root in &first {
        assert_eq!(refusals(&root.tree)?, root.wanted);
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
