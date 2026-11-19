//! Controls of the governance checker over fixture trees the tests write for themselves
//! (larry#270): no control reads the live checkout's documents, so a half-edited document cannot
//! fail a unit test or the mutation gate. CI's `governance` job and `larry governance` at
//! pre-commit still check the real tree.

use std::path::{Path, PathBuf};

use lgwks_std::hash;

use super::sim_support::Scratch;
use super::{check, fixture, tracker};

/// The repository root, the directory this crate is built from.
fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The product's retained receipts: the checker's one home for the pinned receipt identity.
fn retained_receipts() -> PathBuf {
    root().join("tools").join("receipts")
}

/// A fixture tree with its tracker page written by the product's own renderer.
fn fixture_tree() -> Result<Scratch, Box<dyn std::error::Error>> {
    let tree = Scratch::new("governance")?;
    fixture::governed(tree.path(), &retained_receipts())?;
    tracker::write(tree.path())?;
    Ok(tree)
}

#[test]
fn a_fixture_tree_passes_the_structural_gate() -> Result<(), Box<dyn std::error::Error>> {
    let tree = fixture_tree()?;
    let report = check(tree.path());
    assert!(
        report.ok(),
        "the fixture tree must pass: {:?}",
        report.errors()
    );
    let readme = std::fs::read_to_string(tree.path().join("README.md"))?;
    assert!(
        readme.contains("The governed fixture readme"),
        "the fixture readme is the one the test wrote"
    );
    let digest = hash::blake3(readme.as_bytes()).to_hex();
    assert!(
        report.pretty().contains(&digest),
        "the report digests the fixture's own README, never a checkout file"
    );
    Ok(())
}

#[test]
fn a_document_past_the_line_law_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let tree = fixture_tree()?;
    let part = tree.path().join("docs/specification/part-04-cells.md");
    let text = std::fs::read_to_string(&part)?;
    std::fs::write(&part, format!("{text}{}", "\n".repeat(121)))?;
    let report = check(tree.path());
    assert!(!report.ok(), "a grown document must fail the gate");
    assert!(
        report
            .errors()
            .iter()
            .any(|error| error.contains("over the 120-line law")),
        "the line law still fails its test: {:?}",
        report.errors()
    );
    Ok(())
}

#[test]
fn a_missing_document_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let tree = fixture_tree()?;
    std::fs::remove_file(tree.path().join("README.md"))?;
    let report = check(tree.path());
    assert!(!report.ok(), "a tree without its README must fail the gate");
    assert!(
        report
            .errors()
            .contains(&String::from("missing document: README.md")),
        "the missing document is named: {:?}",
        report.errors()
    );
    Ok(())
}

#[test]
fn the_fixture_tracker_page_matches_its_render() -> Result<(), Box<dyn std::error::Error>> {
    let tree = fixture_tree()?;
    let page = tracker::render(tree.path())?;
    assert_eq!(
        page,
        std::fs::read_to_string(tree.path().join("tools/tracker/index.html"))?,
        "the written page is the render, byte for byte"
    );
    assert!(
        tracker::is_current(tree.path())?,
        "the committed page is current"
    );
    let data = std::fs::read_to_string(tree.path().join("tools/tracker/tracker.json"))?;
    std::fs::write(
        tree.path().join("tools/tracker/tracker.json"),
        data.replace("\"falsifier_total\":54", "\"falsifier_total\":55"),
    )?;
    assert!(
        !tracker::is_current(tree.path())?,
        "an edit left unrendered is stale"
    );
    let report = check(tree.path());
    assert!(
        report
            .errors()
            .iter()
            .any(|error| error.contains("falsifier_total is 55")),
        "the drift is named: {:?}",
        report.errors()
    );
    Ok(())
}

#[test]
fn two_fixture_trees_at_once_keep_their_own_report() -> Result<(), Box<dyn std::error::Error>> {
    let first = fixture_tree()?;
    let second = fixture_tree()?;
    let readme = std::fs::read_to_string(first.path().join("README.md"))?;
    std::fs::write(
        first.path().join("README.md"),
        format!("{readme}\nStray reference R9999.\n"),
    )?;
    let solo = (check(first.path()).pretty(), check(second.path()).pretty());
    assert_ne!(solo.0, solo.1, "the two trees differ");
    let together: Vec<(String, String)> = std::thread::scope(|scope| {
        (0..4)
            .map(|_| scope.spawn(|| (check(first.path()).pretty(), check(second.path()).pretty())))
            .filter_map(|handle| handle.join().ok())
            .collect()
    });
    assert_eq!(together.len(), 4, "every thread finished");
    for pair in together {
        assert_eq!(pair, solo, "a concurrent check equals its solo report");
    }
    Ok(())
}

#[test]
fn a_symlinked_fixture_document_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let tree = fixture_tree()?;
    let readme = tree.path().join("README.md");
    let moved = tree.path().join("moved.txt");
    std::fs::rename(&readme, &moved)?;
    std::os::unix::fs::symlink(&moved, &readme)?;
    let report = check(tree.path());
    assert!(!report.ok(), "a symlinked document must fail the load");
    assert!(
        report.errors().contains(&String::from(
            "symbolic-link document is not admitted: README.md"
        )),
        "the symlink is named: {:?}",
        report.errors()
    );
    Ok(())
}

#[test]
fn reranker_receipts_are_present_and_valid() {
    let errors = super::receipts::receipt_errors(root());
    assert!(
        errors.is_empty(),
        "the retained receipts must be valid: {errors:?}"
    );
}

#[test]
fn missing_reranker_receipt_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let scratch = Scratch::new("receipts-none")?;
    std::fs::create_dir_all(scratch.path().join("tools").join("receipts"))?;
    let errors = super::receipts::receipt_errors(scratch.path());
    assert!(
        errors.iter().any(|error| error.contains("none retained")),
        "an empty receipt directory must be refused: {errors:?}"
    );
    Ok(())
}

/// The temp-path rule (larry#267, larry#293): a fixture that splits a temp directory from the
/// process id is refused with its file and line, while the guard route, the guard file itself, and
/// a process id that names something other than a temp path pass.
#[test]
fn a_pid_temp_path_in_test_code_is_refused_and_the_guard_passes()
-> Result<(), Box<dyn std::error::Error>> {
    let tree = Scratch::new("temp-paths")?;
    tree.write(
        "Cargo.toml",
        b"[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )?;
    tree.write("src/lib.rs", b"pub fn present() {}\n")?;
    tree.write("tests/keep.rs", b"fn keep() {}\n")?;
    let fixture = "tests/old_fixture.rs";
    tree.write(
        fixture,
        b"fn state() -> std::path::PathBuf {\n    \
          let base = std::env::temp_dir();\n    \
          base.join(format!(\"x-{}\", std::process::id()))\n}\n",
    )?;
    assert_eq!(
        super::temp_paths::errors(tree.path())?,
        [String::from(
            "a process id names a temp directory outside the guard: tests/old_fixture.rs:2"
        )],
        "the split binding is refused, naming its file and line"
    );
    tree.write(
        fixture,
        b"fn state() -> origin::TempDir {\n    origin::TempDir::new()\n}\n",
    )?;
    assert!(
        super::temp_paths::errors(tree.path())?.is_empty(),
        "the guard route passes"
    );
    tree.write(
        fixture,
        b"const DEAD_PID: i32 = 2_147_483_646;\n\
          fn name() -> String {\n    format!(\"origin-{DEAD_PID}-0\", DEAD_PID)\n}\n",
    )?;
    assert!(
        super::temp_paths::errors(tree.path())?.is_empty(),
        "a process id that names no temp directory passes"
    );
    tree.write(
        "src/executor/sim_tree.rs",
        b"fn scratch_name() -> std::path::PathBuf {\n    \
          std::env::temp_dir().join(format!(\"x-{}\", std::process::id()))\n}\n",
    )?;
    assert!(
        super::temp_paths::errors(tree.path())?.is_empty(),
        "the guard that empties and removes is the one temp path a process id may name"
    );
    tree.write(
        "src/production.rs",
        b"fn report() -> std::path::PathBuf {\n    \
          std::env::temp_dir().join(format!(\"a-{}\", std::process::id()))\n}\n",
    )?;
    assert!(
        super::temp_paths::errors(tree.path())?.is_empty(),
        "a production file is not test code, so the rule does not reach it"
    );
    Ok(())
}

/// The number of live-tree checks the cost control runs.
const CHECKS: usize = 100;

/// The rule reads every workspace member (larry#311): the live tree names the root package and its
/// two crates, so `crates/*` is scanned and a member's test code cannot hide.
#[test]
fn governance_names_every_workspace_member() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        super::temp_paths::Workspace::read(root())?.members(),
        ["", "crates/larry-contracts", "crates/larry-runtime"],
        "the root manifest names the root package and both members"
    );
    Ok(())
}

/// Cost, measured not gated (ADR 0053): 100 checks of the live tree print p50, p95 and p99, and
/// every one of them passes, so the walk and the parse stay in budget (larry#293). The workspace
/// is read once for the member list and that read is printed too (larry#311).
#[test]
fn a_hundred_checks_of_the_tree_print_their_percentiles() -> Result<(), Box<dyn std::error::Error>>
{
    use std::io::Write as _;

    let started = std::time::Instant::now();
    let workspace = super::temp_paths::Workspace::read(root())?;
    let membership = started.elapsed();
    let mut samples = Vec::with_capacity(CHECKS);
    for _ in 0..CHECKS {
        let started = std::time::Instant::now();
        let errors = super::temp_paths::rule_errors(root(), &workspace)?;
        samples.push(started.elapsed());
        assert!(errors.is_empty(), "the live tree passes: {errors:?}");
    }
    samples.sort_unstable();
    let nearest = |per_cent: usize| {
        let rank = samples.len().saturating_mul(per_cent).div_ceil(100);
        match samples.get(rank.saturating_sub(1)) {
            Some(sample) => *sample,
            None => std::time::Duration::ZERO,
        }
    };
    let line = format!(
        "larry governance temp-path rule over {CHECKS} checks: p50 {:?} p95 {:?} p99 {:?} (workspace read {membership:?})\n",
        nearest(50),
        nearest(95),
        nearest(99)
    );
    std::io::stderr().lock().write_all(line.as_bytes())?;
    Ok(())
}

/// Every version `Cargo.lock` text locks for `name`: a `version` line counts only
/// when it directly follows that package's `name` line, so a dependency list or a
/// comment that mentions the name never does.
fn locked_versions<'a>(lock: &'a str, name: &str) -> Vec<&'a str> {
    let wanted = format!("name = \"{name}\"");
    let mut lines = lock.lines();
    let mut versions = Vec::new();
    while let Some(line) = lines.next() {
        if line != wanted {
            continue;
        }
        let Some(next) = lines.next() else {
            break;
        };
        if let Some(version) = next
            .strip_prefix("version = \"")
            .and_then(|rest| rest.strip_suffix('"'))
        {
            versions.push(version);
        }
    }
    versions
}

#[test]
fn locked_versions_reads_only_whole_package_entries() {
    let lock = "[[package]]\nname = \"a\"\nversion = \"1.0.0\"\ndependencies = [\n \"b\",\n]\n\n\
                [[package]]\nname = \"b\"\nversion = \"2.0.0\"\n\n[[package]]\nname = \"b\"\n\
                version = \"3.0.0\"\n\n[[package]]\nname = \"c\"\nsource = \"x\"\n";
    assert_eq!(locked_versions(lock, "a"), ["1.0.0"]);
    assert_eq!(locked_versions(lock, "b"), ["2.0.0", "3.0.0"]);
    assert!(
        locked_versions(lock, "c").is_empty(),
        "no version line follows"
    );
    assert!(locked_versions(lock, "d").is_empty(), "never locked");
    assert!(locked_versions("name = \"a\"", "a").is_empty(), "truncated");
}

/// The frontend's dependency budget (larry#239, ADR 0045): AppCUI and its
/// macro are gone from the lock, and the two frame crates sit at their pins.
#[test]
fn tui_dep_budget_lock_drops_appcui() -> Result<(), Box<dyn std::error::Error>> {
    let lock = std::fs::read_to_string(root().join("Cargo.lock"))?;
    assert!(
        locked_versions(&lock, "appcui").is_empty(),
        "AppCUI is back in the lock"
    );
    assert!(locked_versions(&lock, "appcui_proc_macro").is_empty());
    assert_eq!(locked_versions(&lock, "crossterm"), ["0.29.0"]);
    assert_eq!(locked_versions(&lock, "unicode-width"), ["0.2.3"]);
    Ok(())
}

/// The lines of the checklist section whose heading contains `title`, up to the next heading.
fn section<'text>(checklist: &'text str, title: &str) -> Vec<&'text str> {
    checklist
        .lines()
        .skip_while(|line| !(line.starts_with("## ") && line.contains(title)))
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .filter(|line| line.starts_with("- "))
        .collect()
}

/// The Director, 2026-11-16: "all changes will be routed through a completed human frontend (in
/// tui, then Mac app then iOS and android apps)" (larry#241). The checklist keeps that order:
/// the TUI line first and ticked, the Mac app before the phone apps, and every open line its proof.
#[test]
fn the_ship_checklist_keeps_the_frontend_order() -> Result<(), Box<dyn std::error::Error>> {
    let checklist = std::fs::read_to_string(root().join("docs/product/SHIP.md"))?;
    let lines = section(&checklist, "Human frontend");
    let at = |needle: &str| lines.iter().position(|line| line.contains(needle));
    assert!(
        lines
            .first()
            .is_some_and(|line| line.starts_with("- [x] #238, #239")),
        "the shipped TUI line leads the section: {lines:?}"
    );
    let (mac, phone) = (at("#247"), at("#115"));
    assert!(
        mac.is_some() && phone.is_some(),
        "both later frontends are named"
    );
    assert!(
        mac < phone,
        "the Mac app comes before the phones: {lines:?}"
    );
    let open: Vec<&&str> = lines
        .iter()
        .filter(|line| line.starts_with("- [ ]") && !line.contains("Proof:"))
        .collect();
    assert!(open.is_empty(), "each open line names its proof: {open:?}");
    let sample = "## 8. Human frontend\n- [x] a\n- [ ] b\n## 9. Next\n- [ ] c\n";
    assert_eq!(section(sample, "Human frontend"), ["- [x] a", "- [ ] b"]);
    assert!(
        section(sample, "Missing").is_empty(),
        "no section, no lines"
    );
    Ok(())
}

/// The one governed document whose title names the old name: the record of the rename itself.
const RENAME_RECORD: &str = "docs/adr/0005-rename-rocco-to-larry.md";

/// Whether a document's frontmatter `title:` or `tags:` line names Rocco in any case. The body
/// may still name real things spelt the old way (ADR 0005); the title and tags say what it is.
fn frontmatter_names_rocco(text: &str) -> bool {
    let Some(frontmatter) = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .map(|(head, _)| head)
    else {
        return false;
    };
    frontmatter
        .lines()
        .filter(|line| line.starts_with("title:") || line.starts_with("tags:"))
        .any(|line| line.to_lowercase().contains("rocco"))
}

#[test]
fn every_governed_document_is_titled_and_tagged_larry() -> Result<(), Box<dyn std::error::Error>> {
    // larry#178: ADR 0005 renamed the repository, and 54 split documents kept `tags: [rocco, ...]`
    // and Rocco titles until this check existed.
    let documents = super::documents::load(root())?;
    let named: Vec<&str> = documents
        .iter()
        .filter(|&(name, text)| name != RENAME_RECORD && frontmatter_names_rocco(text))
        .map(|(name, _)| name)
        .collect();
    assert!(
        named.is_empty(),
        "these documents are still titled or tagged Rocco: {named:?}"
    );
    let tagged = "---\ntitle: Larry part\ntags: [rocco, prd]\n---\nbody\n";
    let titled = "---\ntitle: ROCCO notes\ntags: [larry]\n---\nbody\n";
    let history = "---\ntitle: Larry log\ntags: [larry]\n---\n`srinji-kaggss/Rocco` was its name\n";
    assert!(frontmatter_names_rocco(tagged), "an old tag is caught");
    assert!(
        frontmatter_names_rocco(titled),
        "an old title is caught in any case"
    );
    assert!(
        !frontmatter_names_rocco(history),
        "history in the body is allowed"
    );
    assert!(
        !frontmatter_names_rocco("tags: [rocco]\n"),
        "no frontmatter, nothing to judge"
    );
    Ok(())
}

#[test]
fn governance_names_the_locked_lgwks_deps() -> Result<(), Box<dyn std::error::Error>> {
    let lock = std::fs::read_to_string(root().join("Cargo.lock"))?;
    let governance = std::fs::read_to_string(root().join("GOVERNANCE.md"))?;
    let locked = locked_versions(&lock, "lgwks_deps");
    assert_eq!(
        locked.len(),
        1,
        "one lgwks_deps release is locked: {locked:?}"
    );
    for version in locked {
        let named = format!("(lgwks_deps {version})");
        assert!(
            governance.contains(&named),
            "GOVERNANCE.md must name the locked release as `{named}`"
        );
    }
    Ok(())
}

#[test]
fn the_docs_tree_names_every_documents_directory() -> Result<(), Box<dyn std::error::Error>> {
    // larry#178: docs/README.md's layout listed neither the ADRs nor the split parts, and named
    // a fixed ADR range that went stale with the next record. Each directory holding a governed
    // document is named in the layout as `/docs/<directory>/`.
    let documents = super::documents::load(root())?;
    let tree = std::fs::read_to_string(root().join("docs").join("README.md"))?;
    let mut missing: Vec<String> = documents
        .iter()
        .filter_map(|(name, _)| name.strip_prefix("docs/"))
        .filter_map(|rest| rest.split_once('/'))
        .map(|(directory, _)| format!("/docs/{directory}/"))
        .filter(|named| !tree.contains(named.as_str()))
        .collect();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "docs/README.md must name every documents directory: {missing:?}"
    );
    Ok(())
}

/// Each root-relative Markdown link target (`](/path)`) in `text`, without its `#anchor`, in
/// text order.
fn document_links(text: &str) -> Vec<&str> {
    text.split("](/")
        .skip(1)
        .filter_map(|rest| rest.split_once(')'))
        .map(|(target, _)| match target.split_once('#') {
            Some((path, _)) => path,
            None => target,
        })
        .collect()
}

#[test]
fn every_product_document_link_names_a_tracked_file() -> Result<(), Box<dyn std::error::Error>> {
    // larry#232: the ship checklist and each benchmark link the document that holds their numbers,
    // and a link to a file that was renamed or never committed sends the reader nowhere.
    let documents = super::documents::load(root())?;
    let broken: Vec<String> = documents
        .iter()
        .filter(|&(name, _)| name.starts_with("docs/product/"))
        .flat_map(|(name, text)| {
            document_links(text)
                .into_iter()
                .filter(|target| !root().join(target).exists())
                .map(move |target| format!("{name} -> /{target}"))
        })
        .collect();
    assert!(
        broken.is_empty(),
        "these product document links name no file: {broken:?}"
    );
    assert_eq!(
        document_links("see [a](/docs/a.md) and [b](/docs/b.md#part)"),
        ["docs/a.md", "docs/b.md"],
        "every root-relative link, anchor dropped"
    );
    assert!(
        document_links("[web](https://x.example) and (/docs/a.md)").is_empty(),
        "a web link or a bare path is not a document link"
    );
    assert!(
        document_links("[open](/docs/a.md").is_empty(),
        "an unclosed link names nothing"
    );
    Ok(())
}

/// The estate crates the cargo sweep names "at their newest release", in its order.
const SWEEP_ESTATE: [&str; 4] = ["lgwks_std", "lgwks_bot", "lgwks_ast", "lgwks_deps"];

#[test]
fn the_cargo_sweep_names_every_locked_estate_release() -> Result<(), Box<dyn std::error::Error>> {
    // larry#178 found the sweep saying "Larry ships on 8.0.1" while 8.1.1 was locked, and
    // larry#180 moved the pins again the same day: the sweep's newest-release line follows the lock.
    let lock = std::fs::read_to_string(root().join("Cargo.lock"))?;
    let sweep = std::fs::read_to_string(
        root()
            .join("docs")
            .join("dependencies")
            .join("cargo-sweep.md"),
    )?;
    let Some(line) = sweep.lines().find(|line| line.contains("newest release:")) else {
        return Err("docs/dependencies/cargo-sweep.md has no `newest release:` line".into());
    };
    let named: Vec<&str> = line
        .split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .filter(|word| word.matches('.').count() == 2)
        .collect();
    let mut wanted = Vec::new();
    for name in SWEEP_ESTATE {
        let locked = locked_versions(&lock, name);
        assert_eq!(locked.len(), 1, "one {name} release is locked: {locked:?}");
        wanted.extend(locked);
    }
    assert_eq!(
        named, wanted,
        "the sweep's newest-release line must list the locked {SWEEP_ESTATE:?} in order"
    );
    // larry#187: the product plan's pin ruling names the same releases, so it follows the lock too.
    let plan = std::fs::read_to_string(root().join("docs").join("product").join("README.md"))?;
    let Some(ruling) = plan
        .lines()
        .find(|line| line.contains("Newest estate pins"))
    else {
        return Err("docs/product/README.md has no `Newest estate pins` ruling".into());
    };
    for name in ["lgwks_std", "lgwks_bot", "lgwks_deps"] {
        for version in locked_versions(&lock, name) {
            let pin = format!("{name} {version}");
            assert!(
                ruling.contains(&pin),
                "the product plan's pin ruling must name the locked `{pin}`"
            );
        }
    }
    // larry#189, #191: every locked estate release was moved by a shipped PR, so a ticked ship
    // checklist line names it. Any ticked line counts, so the next bump adds its own line.
    let ship = std::fs::read_to_string(root().join("docs").join("product").join("SHIP.md"))?;
    let ticked: Vec<&str> = ship
        .lines()
        .filter(|line| line.starts_with("- [x]"))
        .collect();
    for name in ["lgwks_std", "lgwks_bot", "lgwks_deps"] {
        for version in locked_versions(&lock, name) {
            let pin = format!("{name} {version}");
            assert!(
                ticked.iter().any(|line| line.contains(&pin)),
                "a ticked ship checklist line must name the locked `{pin}`"
            );
        }
    }
    Ok(())
}
