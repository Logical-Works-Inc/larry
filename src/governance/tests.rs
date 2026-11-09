//! Governance controls against the real tree, with mutations made in memory only.

use std::path::Path;

use super::check;

/// The repository root, the directory this crate is built from.
fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn current_documents_pass_the_structural_gate() -> Result<(), Box<dyn std::error::Error>> {
    let report = check(root());
    assert!(
        report.ok(),
        "the real tree must pass: {:?}",
        report.errors()
    );
    Ok(())
}

#[test]
fn generated_page_matches_the_committed_one() -> Result<(), Box<dyn std::error::Error>> {
    let page = super::tracker::render(root())?;
    let committed =
        std::fs::read_to_string(root().join("tools").join("tracker").join("index.html"))?;
    assert_eq!(
        page, committed,
        "the rendered tracker must match index.html byte for byte"
    );
    Ok(())
}

#[test]
fn reranker_receipts_are_present_and_valid() -> Result<(), Box<dyn std::error::Error>> {
    let errors = super::receipts::receipt_errors(root());
    assert!(
        errors.is_empty(),
        "the retained receipts must be valid: {errors:?}"
    );
    Ok(())
}

#[test]
fn missing_reranker_receipt_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let scratch =
        std::env::temp_dir().join(format!("larry-governance-{}-none", std::process::id()));
    std::fs::create_dir_all(scratch.join("tools").join("receipts"))?;
    let errors = super::receipts::receipt_errors(&scratch);
    std::fs::remove_dir_all(&scratch)?;
    assert!(
        errors.iter().any(|error| error.contains("none retained")),
        "an empty receipt directory must be refused: {errors:?}"
    );
    Ok(())
}

#[test]
fn tracker_is_current_on_the_real_tree() -> Result<(), Box<dyn std::error::Error>> {
    assert!(
        super::tracker::is_current(root())?,
        "the committed tracker must be current"
    );
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

/// The macro release each locked appcui release was built with. appcui's own
/// requirement (`^0.3.6`) admits newer macros that emit traits only newer appcui
/// defines, so a plain `cargo update` breaks the build (logicalworks-crates#841,
/// larry#170); the lock holds the pair until lgwks_deps pins it upstream.
const APPCUI_MACRO_PAIRS: [(&str, &str); 1] = [("0.5.5", "0.3.6")];

#[test]
fn the_lock_holds_appcui_proc_macro_at_the_release_appcui_was_built_with()
-> Result<(), Box<dyn std::error::Error>> {
    let lock = std::fs::read_to_string(root().join("Cargo.lock"))?;
    let macros = locked_versions(&lock, "appcui_proc_macro");
    for appcui in locked_versions(&lock, "appcui") {
        let Some(&(_, wanted)) = APPCUI_MACRO_PAIRS.iter().find(|pair| pair.0 == appcui) else {
            continue;
        };
        assert_eq!(
            macros,
            [wanted],
            "appcui {appcui} needs appcui_proc_macro {wanted} (logicalworks-crates#841): \
             run `cargo update -p appcui_proc_macro --precise {wanted}`"
        );
    }
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
