//! The ship checklist keeps its promise for the first-real-use sweep.
//!
//! Source: `docs/product/SHIP.md` step 3 and ADR 0002: a line is ticked only when its proof ran
//! and printed, so an open line must say what its proof is. larry#372, 2026-11-21: the #321
//! review line read as of 2026-11-19 after ten of its children had merged.
//! Requirement: every open line of the "Sweep #200" section names `Proof:` or `Acceptance:`,
//! and the #321 line names what shipped and what is open, in order, and every ADR since 0092
//! is named on the line of the PR that shipped it (larry#431).

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

/// The first ADR whose shipping PR named it on its checklist line (larry#382, PR #401). Before
/// it, process ADRs shipped with no line of their own.
const FIRST_TICKED_ADR: u32 = 92;
/// More entries than this in `docs/adr` is refused before they are judged.
const MAX_ADRS: usize = 4096;

/// The ADR numbers at or past `FIRST_TICKED_ADR` that `checklist` never names as `ADR NNNN`.
fn unnamed(checklist: &str, numbers: &[u32]) -> Vec<u32> {
    numbers
        .iter()
        .copied()
        .filter(|&number| number >= FIRST_TICKED_ADR)
        .filter(|number| !checklist.contains(&format!("ADR {number:04}")))
        .collect()
}

/// A shipped decision ticks its line (step 3, larry#431): on 2026-11-24 ADRs 0099 to 0103 had
/// merged and the checklist named none of them, so a session read four fixed issues as open.
#[test]
fn every_adr_since_the_ticking_rule_is_named_on_its_line() -> TestResult {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let checklist = fs::read_to_string(root.join(SHIP))?;
    let mut numbers = Vec::new();
    for entry in fs::read_dir(root.join("docs/adr"))? {
        assert!(
            numbers.len() < MAX_ADRS,
            "docs/adr holds over {MAX_ADRS} entries"
        );
        let name = entry?.file_name();
        let digits = name.to_str().and_then(|name| name.get(..4));
        if let Some(number) = digits.and_then(|digits| digits.parse::<u32>().ok()) {
            numbers.push(number);
        }
    }
    numbers.sort_unstable();
    assert!(
        numbers.last().is_some_and(|&last| last >= FIRST_TICKED_ADR),
        "docs/adr holds the ticked ADRs: {numbers:?}"
    );
    let missing = unnamed(&checklist, &numbers);
    assert!(
        missing.is_empty(),
        "{SHIP} names no line for ADR {missing:?}"
    );
    Ok(())
}

#[test]
fn an_adr_past_the_floor_with_no_line_is_named_and_an_older_one_is_not() {
    let sample = "- [x] #1 a fix (ADR 0092): PR #2. #3 another (ADR 0094).";
    assert_eq!(unnamed(sample, &[5, 91, 92, 93, 94, 120]), [93, 120]);
    assert!(unnamed(sample, &[92, 94]).is_empty(), "named ADRs pass");
    assert_eq!(
        unnamed("ADR 93 without padding", &[93]),
        [93],
        "the number is four digits"
    );
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
