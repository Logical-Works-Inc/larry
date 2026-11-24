//! The ship checklist keeps its promise for the first-real-use sweep.
//!
//! Source: `docs/product/SHIP.md` step 3 and ADR 0002: a line is ticked only when its proof ran
//! and printed, so an open line must say what its proof is. larry#372, 2026-11-21: the #321
//! review line read as of 2026-11-19 after ten of its children had merged.
//! Requirement: every open line of the "Sweep #200" section names `Proof:` or `Acceptance:`,
//! and the #321 line names what shipped and what is open, in order.

use std::error::Error;
use std::fs;
use std::path::Path;

/// What every test returns.
type TestResult = Result<(), Box<dyn Error>>;

/// The checklist, relative to the repository root.
const SHIP: &str = "docs/product/SHIP.md";
/// The heading word of the sweep section.
const SWEEP: &str = "Sweep #200";
/// More bytes than this in the checklist is refused before it is read.
const MAX_SHIP_BYTES: u64 = 1024 * 1024;

/// The `- ` lines of the section whose `## ` heading contains `title`, up to the next heading.
fn section<'text>(checklist: &'text str, title: &str) -> Vec<&'text str> {
    checklist
        .lines()
        .skip_while(|line| !(line.starts_with("## ") && line.contains(title)))
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .filter(|line| line.starts_with("- "))
        .collect()
}

/// The open lines that name no proof.
fn unproven<'text>(lines: &[&'text str]) -> Vec<&'text str> {
    lines
        .iter()
        .copied()
        .filter(|line| line.starts_with("- [ ]"))
        .filter(|line| !line.contains("Proof:") && !line.contains("Acceptance:"))
        .collect()
}

#[test]
fn the_sweep_section_names_each_open_lines_proof() -> TestResult {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(SHIP);
    let bytes = fs::metadata(&path)?.len();
    assert!(bytes <= MAX_SHIP_BYTES, "{SHIP} is {bytes} bytes");
    let checklist = fs::read_to_string(&path)?;
    let lines = section(&checklist, SWEEP);
    let review = lines.iter().find(|line| line.contains("#321"));
    assert!(
        review.is_some_and(|line| line.contains("Shipped") && line.contains("Open in order:")),
        "the #321 line names what shipped and what is open: {review:?}"
    );
    let missing = unproven(&lines);
    assert!(
        missing.is_empty(),
        "each open line names its proof: {missing:?}"
    );
    Ok(())
}

/// One order governs every agent (larry#424): the steps pick the next issue from the work graph
/// #423 and file new work under its epics, so no second order competes with it.
#[test]
fn the_ship_steps_pick_from_the_work_graph() -> TestResult {
    let checklist = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(SHIP))?;
    let steps: Vec<&str> = checklist
        .lines()
        .skip_while(|line| !line.starts_with("## How a session ships"))
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .collect();
    let first = steps.iter().find(|line| line.starts_with("1. "));
    assert!(
        first.is_some_and(|line| line.contains("#423") && line.contains("-is:blocked")),
        "step 1 picks from the #423 ready query: {first:?}"
    );
    let file = steps.iter().find(|line| line.starts_with("4. "));
    assert!(
        file.is_some_and(|line| line.contains("under its epic in #423")),
        "step 4 files new work under its epic in #423: {file:?}"
    );
    Ok(())
}

#[test]
fn an_open_line_with_no_proof_is_named_and_a_ticked_one_is_not() {
    let sample = "## 7. Sweep #200\n- [x] #1 done\n- [ ] #2 Proof: a run\n- [ ] #3 bare\n\
                  - [ ] #4 Acceptance: a run\n## 8. Next\n- [ ] #5 bare\n";
    let lines = section(sample, SWEEP);
    assert_eq!(
        lines.len(),
        4,
        "the section stops at the next heading: {lines:?}"
    );
    assert_eq!(unproven(&lines), ["- [ ] #3 bare"]);
    assert!(
        section(sample, "Missing").is_empty(),
        "no section, no lines"
    );
}
