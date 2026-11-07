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
