//! Every ADR number names one decision.
//!
//! Source: larry#319 and larry#330, 2026-11-21. Two workers each took the next free number,
//! 0067, within an hour, and nothing refused the second, so "ADR 0067" named two decisions.
//! Requirement: each `docs/adr/NNNN-slug.md` number is held by one file, and an ADR's
//! `resource:` line names its own path, so a renamed ADR cannot keep pointing at its old one.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::Path;

/// What every test returns.
type TestResult = Result<(), Box<dyn Error>>;

/// The ADR directory, relative to the repository root.
const ADR_DIR: &str = "docs/adr";
/// More ADR files than this is a tree the check refuses to read whole.
const MAX_ADRS: usize = 10_000;
/// More bytes than this in one ADR is refused before it is read.
const MAX_ADR_BYTES: u64 = 2 * 1024 * 1024;

/// The four-digit number of an ADR file name `NNNN-slug.md`, or `None` for any other name.
fn adr_number(file: &str) -> Option<&str> {
    let stem = file.strip_suffix(".md")?;
    let (number, slug) = stem.split_once('-')?;
    (number.len() == 4 && number.bytes().all(|byte| byte.is_ascii_digit()) && !slug.is_empty())
        .then_some(number)
}

/// Each number held by more than one file, with every file holding it, in name order.
fn shared_numbers<'a>(files: impl IntoIterator<Item = &'a str>) -> Vec<(String, Vec<String>)> {
    let mut by_number: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for file in files {
        if let Some(number) = adr_number(file) {
            by_number.entry(number).or_default().push(file.to_owned());
        }
    }
    by_number
        .into_iter()
        .filter(|entry| entry.1.len() > 1)
        .map(|(number, mut files)| {
            files.sort();
            (number.to_owned(), files)
        })
        .collect()
}

/// The `resource:` value of an ADR's frontmatter, when it has one.
fn resource_of(text: &str) -> Option<&str> {
    let frontmatter = text.strip_prefix("---\n")?.split_once("\n---\n")?.0;
    frontmatter
        .lines()
        .find_map(|line| line.strip_prefix("resource:"))
        .map(str::trim)
}

/// Every ADR file name and its text, refused past the bounds rather than read without end.
fn adrs(root: &Path) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(root.join(ADR_DIR))? {
        let entry = entry?;
        let Ok(file) = entry.file_name().into_string() else {
            return Err(format!("{ADR_DIR} holds a file name that is not UTF-8").into());
        };
        if adr_number(&file).is_none() {
            continue;
        }
        let meta = entry.path().symlink_metadata()?;
        if !meta.is_file() || meta.len() > MAX_ADR_BYTES {
            return Err(format!("{ADR_DIR}/{file} is not a regular file under 2 MiB").into());
        }
        if out.len() == MAX_ADRS {
            return Err(format!("{ADR_DIR} holds more than {MAX_ADRS} ADRs").into());
        }
        out.push((file, fs::read_to_string(entry.path())?));
    }
    out.sort();
    Ok(out)
}

/// The four-digit numbers `text` cites after `ADR `.
fn citations(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some((_, after)) = rest.split_once("ADR ") {
        let number: String = after.chars().take(4).collect();
        if number.len() == 4 && number.bytes().all(|byte| byte.is_ascii_digit()) {
            out.push(number);
        }
        rest = after;
    }
    out
}

#[test]
fn every_adr_number_names_one_document() -> TestResult {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let adrs = adrs(root)?;
    assert!(adrs.len() > 60, "the tree holds its ADRs: {}", adrs.len());
    let shared = shared_numbers(adrs.iter().map(|adr| adr.0.as_str()));
    assert!(
        shared.is_empty(),
        "ADR numbers held by more than one file: {shared:?}"
    );
    let drift: Vec<&str> = adrs
        .iter()
        .filter(|adr| {
            resource_of(&adr.1).is_some_and(|resource| resource != format!("/{ADR_DIR}/{}", adr.0))
        })
        .map(|adr| adr.0.as_str())
        .collect();
    assert!(
        drift.is_empty(),
        "these ADRs name another path as their resource: {drift:?}"
    );
    // A renumbered ADR must not leave a governed document citing its old number at a
    // different decision (larry#322's move to 0081 followed main's 0080).
    let held: Vec<String> = adrs
        .iter()
        .filter_map(|adr| adr_number(&adr.0).map(str::to_owned))
        .collect();
    let mut missing = Vec::new();
    for document in ["INVARIANTS.md", "CODEBOOK.md"] {
        let text = fs::read_to_string(root.join(document))?;
        for number in citations(&text) {
            if !held.contains(&number) {
                missing.push(format!("{document} cites ADR {number}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "documents cite numbers no ADR holds: {missing:?}"
    );
    Ok(())
}

#[test]
fn a_shared_number_is_named_with_both_files_and_other_names_are_not_adrs() {
    let collided = shared_numbers(["0067-fenced.md", "0068-next.md", "0067-short-read.md"]);
    assert_eq!(
        collided,
        [(
            String::from("0067"),
            vec![
                String::from("0067-fenced.md"),
                String::from("0067-short-read.md")
            ],
        )],
        "a shared number is named with both files"
    );
    assert!(
        shared_numbers(["0001-a.md", "0002-b.md"]).is_empty(),
        "distinct numbers share nothing"
    );
    for not_an_adr in [
        "README.md",
        "067-short.md",
        "00a7-x.md",
        "0067-.md",
        "0067.md",
        "0067-x.txt",
    ] {
        assert_eq!(
            adr_number(not_an_adr),
            None,
            "{not_an_adr} is not an ADR name"
        );
    }
    assert_eq!(
        resource_of("---\ntitle: t\nresource: /docs/adr/0001-a.md\n---\nbody\n"),
        Some("/docs/adr/0001-a.md")
    );
    assert_eq!(
        resource_of("resource: /x\n"),
        None,
        "no frontmatter, no resource"
    );
}
